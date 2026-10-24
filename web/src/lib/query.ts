import { QueryClient, queryOptions } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { api, ApiError, type Account } from './api'

// One client per router (and therefore per SSR request), never a server singleton.
export function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 30_000,
        retry: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
        // Show our existing offline feedback immediately instead of pausing.
        networkMode: 'always',
      },
      mutations: { retry: false, networkMode: 'always', gcTime: 0 },
    },
  })
}

// Remove the previous identity without orphaning mounted query observers.
export async function resetSessionQueries(client: QueryClient) {
  await client.cancelQueries()
  client.removeQueries({ type: 'inactive' })
  client.getMutationCache().clear()
  await client.resetQueries({ type: 'active' }, { throwOnError: true })
}

export function useClientReady() {
  const [ready, setReady] = useState(false)
  useEffect(() => setReady(true), [])
  return ready
}

export const accountQuery = queryOptions({
  queryKey: ['account'],
  queryFn: async ({ signal }): Promise<Account | null> => {
    try {
      return await api('GET /account', undefined, {
        signal,
        redirectOnSessionEnd: false,
      })
    } catch (error) {
      // Guest routes and the app handoff must keep their sign-in form available.
      if (error instanceof ApiError && error.status === 401) return null
      throw error
    }
  },
})
export const devicesQuery = queryOptions({
  queryKey: ['devices'],
  queryFn: ({ signal }) => api('GET /devices', undefined, { signal }),
})
export const passkeysQuery = queryOptions({
  queryKey: ['passkeys'],
  queryFn: ({ signal }) => api('GET /passkeys', undefined, { signal }),
})
export const mfaQuery = queryOptions({
  queryKey: ['mfa'],
  queryFn: ({ signal }) => api('GET /mfa', undefined, { signal }),
})
export const sessionsQuery = queryOptions({
  queryKey: ['sessions'],
  queryFn: ({ signal }) =>
    api('GET /sessions', undefined, {
      signal,
    }),
})
export const serverQuery = queryOptions({
  queryKey: ['server'],
  queryFn: ({ signal }) => api('GET /server', undefined, { signal }),
})
