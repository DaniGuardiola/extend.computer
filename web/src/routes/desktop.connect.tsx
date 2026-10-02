import { useEffect, useState } from 'react'
import { createFileRoute } from '@tanstack/react-router'
import { Auth } from '../components/Auth'
import { Brand } from '../components/Brand'
import { api, ApiError, message, type Account } from '../lib/api'
export const Route = createFileRoute('/desktop/connect')({
  component: DesktopConnect,
  head: () => ({ meta: [{ title: 'Sign in to desktop · extend.computer' }] }),
})
function DesktopConnect() {
  const [account, setAccount] = useState<Account | null>(null)
  const [checked, setChecked] = useState(false)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [request, setRequest] = useState<{
    state: string
    challenge: string
    port: number
  } | null>(null)
  async function refresh() {
    setAccount(await api<Account>('/account'))
  }
  useEffect(() => {
    let active = true
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
      setChecked(true)
      return
    }
    setRequest({ port, state, challenge })
    api<Account>('/account')
      .then((value) => {
        if (active) setAccount(value)
      })
      .catch((e) => {
        if (active && !(e instanceof ApiError && e.status === 401))
          setError(message(e))
      })
      .finally(() => {
        if (active) setChecked(true)
      })
    return () => {
      active = false
    }
  }, [])
  async function connect() {
    if (!request) return
    setBusy(true)
    setError('')
    try {
      const { code } = await api<{ code: string }>(
        '/auth/desktop/authorize',
        'POST',
        { challenge: request.challenge },
      )
      // Only an ephemeral loopback destination; never an arbitrary redirect URL.
      const callback = new URL(`http://127.0.0.1:${request.port}/callback`)
      callback.searchParams.set('state', request.state)
      callback.searchParams.set('code', code)
      location.assign(callback.href)
    } catch (e) {
      setError(message(e))
      setBusy(false)
    }
  }
  if (checked && request && !account && !error)
    return <Auth onSignedIn={refresh} />
  return (
    <main id="main" className="auth-main">
      <div className="auth-box">
        <Brand />
        <h1>Connect the desktop app.</h1>
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
              className="button auth-submit"
              disabled={busy}
              onClick={connect}
            >
              {busy ? 'Connecting…' : 'Continue to desktop'}
            </button>
            <button
              className="text-button"
              disabled={busy}
              onClick={async () => {
                try {
                  await api('/auth/logout', 'POST')
                  setAccount(null)
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
