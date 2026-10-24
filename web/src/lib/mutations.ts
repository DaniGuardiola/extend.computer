import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from './api'
import type { AccountSession, Input } from './contracts'

// Workflows include browser prompts and awaited invalidation. Pending controls
// therefore stay disabled until the visible data is current, even on failure.
export function useAction() {
  const mutation = useMutation({
    mutationFn: (work: () => Promise<void>) => work(),
  })
  return {
    isPending: mutation.isPending,
    run: async (work: () => Promise<void>) => {
      try {
        await mutation.mutateAsync(work)
      } finally {
        mutation.reset()
      }
    },
  }
}

export function useRefreshAccount() {
  const client = useQueryClient()
  return async (
    keys = ['account', 'devices', 'passkeys', 'mfa', 'sessions'],
  ) => {
    await Promise.all(
      keys.map((key) =>
        client.invalidateQueries(
          { queryKey: [key], exact: true },
          { throwOnError: true },
        ),
      ),
    )
  }
}

// Domain actions keep endpoint details and affected query keys together.
// Results are reset after consumption so sensitive one-time data isn't cached.
function useOperation<V, R>(work: (variables: V) => Promise<R>) {
  const mutation = useMutation({ mutationFn: work })
  return {
    isPending: mutation.isPending,
    run: async (variables: V) => {
      try {
        return await mutation.mutateAsync(variables)
      } finally {
        mutation.reset()
      }
    },
  }
}

export function useRegisterKey() {
  const refresh = useRefreshAccount()
  return useOperation(async (onRecoveryCodes: (codes: string[]) => void) => {
    const { startRegistration } = await import('@simplewebauthn/browser')
    const optionsJSON = await api('POST /passkeys/register/options', {
      auto: true,
    })
    const result = await api(
      'POST /passkeys/register/verify',
      await startRegistration({ optionsJSON }),
    )
    // Issued codes cannot be fetched again. Deliver them before a refresh that
    // may fail after registration has already succeeded.
    if (result.recovery_codes?.length) onRecoveryCodes(result.recovery_codes)
    await refresh(['account', 'passkeys', 'mfa'])
  })
}

export function useRemoveCredential() {
  const refresh = useRefreshAccount()
  return useOperation(
    async (target: {
      kind: 'devices' | 'passkeys'
      id: string
      signOutOthers: boolean
    }) => {
      if (target.kind === 'passkeys') {
        await api(
          'DELETE /passkeys/:id',
          { sign_out_others: target.signOutOthers },
          { id: target.id },
        )
        await refresh(['account', 'passkeys', 'mfa', 'sessions'])
      } else {
        await api('DELETE /devices/:id', undefined, { id: target.id })
        await refresh(['devices', 'sessions'])
      }
    },
  )
}

export function usePasskeyUsage() {
  const refresh = useRefreshAccount()
  return useOperation(async (input: { id: string; enabled: boolean }) => {
    await api(
      'PATCH /passkeys/:id',
      { sign_in_enabled: input.enabled },
      { id: input.id },
    )
    await refresh(['passkeys'])
  })
}

export function useChangePassword() {
  const refresh = useRefreshAccount()
  return useOperation(async (input: Input<'POST /account/password'>) => {
    await api('POST /account/password', input)
    await refresh(['account', 'sessions'])
  })
}

export function useMfaAction() {
  const refresh = useRefreshAccount()
  return useOperation(async (work: () => Promise<void>) => {
    await work()
    await refresh(['account', 'mfa', 'sessions'])
  })
}

export function useRevokeSession() {
  const refresh = useRefreshAccount()
  const client = useQueryClient()
  return useOperation(async (target: AccountSession | 'all') => {
    await api(
      target === 'all' ? 'DELETE /sessions' : 'DELETE /sessions/:id',
      undefined,
      { id: target === 'all' ? undefined : target.id },
    )
    const signedOut = target === 'all' || !!target.current
    if (signedOut) client.clear()
    else await refresh(['sessions'])
    return signedOut
  })
}
