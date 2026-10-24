import { useQuery, useQueryClient } from '@tanstack/react-query'
import { accountQuery, useClientReady } from '../lib/query'
import { useAction } from '../lib/mutations'
import { ThemeHeader } from '../components/ThemePicker'
import { useEffect, useState } from 'react'
import { createFileRoute } from '@tanstack/react-router'
import { Auth } from '../components/Auth'
import { Brand } from '../components/Brand'
import { api, message } from '../lib/api'
export const Route = createFileRoute('/desktop/connect')({
  component: DesktopConnect,
  head: () => ({ meta: [{ title: 'Sign in to desktop · extend.computer' }] }),
})
function DesktopConnect() {
  const operation = useAction()
  const queryClient = useQueryClient()
  const ready = useClientReady()
  const [error, setError] = useState('')
  const [leaving, setLeaving] = useState(false)
  const busy = operation.isPending || leaving
  const [request, setRequest] = useState<{
    state: string
    challenge: string
    port: number
  } | null>(null)
  const session = useQuery({
    ...accountQuery,
    enabled: ready && !!request,
    staleTime: 0,
  })
  const account = session.isFetching ? undefined : session.data
  const checked =
    ready && (!request || (!session.isPending && !session.isFetching))
  async function refresh() {
    await queryClient.ensureQueryData(accountQuery)
  }
  useEffect(() => {
    const params = new URLSearchParams(location.search)
    const port = Number(params.get('port'))
    const state = params.get('state') ?? ''
    const challenge = params.get('challenge') ?? ''
    if (
      !Number.isInteger(port) ||
      port < 1024 ||
      port > 65535 ||
      !/^[a-f0-9]{64}$/.test(state) ||
      !/^[A-Za-z0-9_-]{43}$/.test(challenge)
    ) {
      setError('Invalid desktop sign-in request. Start again from the app.')
      return
    }
    setRequest({ port, state, challenge })
  }, [])
  useEffect(() => {
    if (session.error) setError(message(session.error))
  }, [session.error])
  async function connect() {
    if (!request) return
    setError('')
    try {
      await operation.run(async () => {
        const { code } = await api('POST /auth/desktop/authorize', {
          challenge: request.challenge,
        })
        // Only an ephemeral loopback destination; never an arbitrary redirect URL.
        const callback = new URL(`http://127.0.0.1:${request.port}/callback`)
        callback.searchParams.set('state', request.state)
        callback.searchParams.set('code', code)
        setLeaving(true)
        location.assign(callback.href)
      })
    } catch (e) {
      setError(message(e))
      setLeaving(false)
    }
  }
  if (checked && request && !account && !error)
    return <Auth onSignedIn={refresh} />
  return (
    <main
      id="main"
      className="auth-main desktop-connect-main flex justify-center items-center py-16 px-7 min-h-[100svh]"
    >
      <ThemeHeader />
      <div className="auth-box w-full max-w-[360px]">
        <Brand appearance="app" />
        <h1>Connect the desktop app</h1>
        <p>
          Only continue if you started this sign-in from extend.computer on this
          computer.
        </p>
        {error && (
          <p role="alert" className="message error-message">
            {error}
          </p>
        )}
        {!checked && <p>Checking your account…</p>}
        {account && request && (
          <>
            <p>Signed in as {account.email}.</p>
            <button
              className="button primary auth-submit gap-[10px]"
              disabled={busy}
              onClick={connect}
            >
              {busy ? 'Signing in…' : 'Sign in to app'}
            </button>
            <button
              className="text-button"
              disabled={busy}
              onClick={async () => {
                try {
                  await operation.run(async () => {
                    await api('POST /auth/logout')
                    queryClient.clear()
                    queryClient.setQueryData(accountQuery.queryKey, null)
                  })
                } catch (e) {
                  setError(message(e))
                }
              }}
            >
              Use another account
            </button>
          </>
        )}
      </div>
    </main>
  )
}
