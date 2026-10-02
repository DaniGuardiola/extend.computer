import { useEffect, useState, type FormEvent } from 'react'
import { Link } from '@tanstack/react-router'
import { Brand } from './Brand'
import { api, message } from '../lib/api'

export function EmailFlow({ recovery = false }: { recovery?: boolean }) {
  const [link, setLink] = useState<{
    kind: 'verify' | 'reset'
    token: string
  } | null>(null)
  const [ready, setReady] = useState(recovery)
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => {
    if (recovery) return
    const match = window.location.hash.match(/^#(verify|reset)=([a-f0-9]{64})$/)
    if (match)
      setLink({ kind: match[1] as 'verify' | 'reset', token: match[2]! })
    // Tokens stay out of referrers, server logs and the history entry.
    window.history.replaceState(null, '', window.location.pathname)
    setReady(true)
  }, [recovery])
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    setBusy(true)
    setError('')
    try {
      if (recovery)
        await api('/auth/recovery/request', 'POST', {
          email: data.get('email'),
        })
      else if (link)
        await api(
          link.kind === 'verify'
            ? '/auth/email/verify'
            : '/auth/recovery/complete',
          'POST',
          {
            token: link.token,
            ...(link.kind === 'reset'
              ? { password: data.get('password') }
              : {}),
          },
        )
      setDone(true)
      setLink(null)
    } catch (error) {
      setError(message(error))
    } finally {
      setBusy(false)
    }
  }
  const reset = link?.kind === 'reset'
  return (
    <div className="auth-layout">
      <aside className="auth-aside">
        <Brand />
        <div>
          <span className="eyebrow">YOUR COMPUTERS, TOGETHER</span>
          <h2>
            Your desk.
            <br />
            Your control.
          </h2>
        </div>
        <small>A little peace of mind.</small>
      </aside>
      <main id="main" className="auth-main">
        <div className="auth-box">
          <Link to="/login" className="back-link">
            Back to sign in ↗
          </Link>
          <h1>
            {done
              ? recovery
                ? 'Check your inbox.'
                : 'You’re all set.'
              : recovery
                ? 'Find your way back.'
                : reset
                  ? 'A fresh start.'
                  : 'Confirm your email.'}
          </h1>
          {error ? (
            <p className="message error-message" role="alert">
              {error}
            </p>
          ) : null}
          {done ? (
            <>
              <p className="auth-description" role="status">
                {recovery
                  ? 'If this address belongs to a verified account, a reset link is on its way. It expires in 30 minutes.'
                  : 'You can sign in to your account now.'}
              </p>
              <Link to="/login" className="button primary">
                Sign in
              </Link>
            </>
          ) : !ready ? (
            <p role="status">Opening your link…</p>
          ) : !recovery && !link ? (
            <p className="auth-description">
              This link is incomplete. Request a new email from your account, or
              use password recovery.
            </p>
          ) : (
            <form onSubmit={submit}>
              <p className="auth-description">
                {recovery
                  ? 'Enter your email to request a password reset.'
                  : reset
                    ? 'Choose a new password. This signs out every device and removes existing passkeys; you can add them again after signing in.'
                    : 'Confirm this address belongs to you. This link works once.'}
              </p>
              {recovery ? (
                <label className="field">
                  Email
                  <input
                    name="email"
                    type="email"
                    autoComplete="email"
                    required
                    maxLength={254}
                  />
                </label>
              ) : reset ? (
                <label className="field">
                  New password
                  <input
                    name="password"
                    type="password"
                    autoComplete="new-password"
                    required
                    minLength={12}
                    maxLength={1024}
                  />
                  <span className="field-hint">At least 12 characters.</span>
                </label>
              ) : null}
              <button className="button auth-submit" disabled={busy}>
                {busy
                  ? 'One moment…'
                  : recovery
                    ? 'Send reset link'
                    : reset
                      ? 'Reset password'
                      : 'Verify email'}
              </button>
            </form>
          )}
        </div>
      </main>
    </div>
  )
}
