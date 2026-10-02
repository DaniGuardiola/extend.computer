import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from '@tanstack/react-router'
import { ArrowRight, Fingerprint, MoveUpRight } from 'lucide-react'
import type { PublicKeyCredentialRequestOptionsJSON } from '@simplewebauthn/browser'
import { SecondFactor, type MfaPending } from './SecondFactor'
import { Brand } from './Brand'
import { api, message } from '../lib/api'
export function Auth({
  signup = false,
  onSignedIn,
}: {
  signup?: boolean
  onSignedIn?: () => void | Promise<void>
}) {
  const navigate = useNavigate()
  const [pending, setPending] = useState<MfaPending | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    setBusy(true)
    setError('')
    try {
      const result = await api<MfaPending | { account: unknown }>(
        '/auth/' + (signup ? 'signup' : 'login'),
        'POST',
        {
          email: data.get('email'),
          password: data.get('password'),
        },
      )
      if ('mfa_required' in result) {
        setPending(result)
        return
      }
      if (signup) {
        const settings = await api<{ email_enabled: boolean }>('/server')
        if (settings.email_enabled)
          await api('/auth/email/request', 'POST').catch(() => {})
      }
      await (onSignedIn ? onSignedIn() : navigate({ to: '/account' }))
    } catch (error) {
      setError(message(error))
    } finally {
      setBusy(false)
    }
  }
  async function passkey() {
    setBusy(true)
    setError('')
    try {
      const { startAuthentication } = await import('@simplewebauthn/browser')
      const optionsJSON = await api<PublicKeyCredentialRequestOptionsJSON>(
        '/passkeys/login/options',
        'POST',
      )
      await api(
        '/passkeys/login/verify',
        'POST',
        await startAuthentication({ optionsJSON }),
      )
      await (onSignedIn ? onSignedIn() : navigate({ to: '/account' }))
    } catch (error) {
      setError(message(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="auth-layout">
      <aside className="auth-aside">
        <Brand />
        <div>
          <span className="eyebrow">YOUR COMPUTERS, TOGETHER</span>
          <h2>
            Less switching.
            <br />
            More doing.
          </h2>
          <p>A home for every device on your desk.</p>
        </div>
        <small>Your desk. Your control.</small>
      </aside>
      <main id="main" className="auth-main">
        <div className="auth-box">
          <Link to="/" className="back-link">
            Back to the desk <MoveUpRight size={12} />
          </Link>
          <h1>{signup ? 'Make yourself at home.' : 'Welcome back.'}</h1>
          <p className="auth-description">
            {signup
              ? 'Create your account. Bring your devices together.'
              : 'Your devices are right where you left them.'}
          </p>
          {error && (
            <p className="message error-message" role="alert">
              {error}
            </p>
          )}
          {pending ? (
            <SecondFactor
              pending={pending}
              onVerified={() =>
                onSignedIn ? onSignedIn() : navigate({ to: '/account' })
              }
              onCancel={() => setPending(null)}
            />
          ) : (
            <>
              <form onSubmit={submit}>
                <label className="field">
                  Email
                  <input
                    name="email"
                    type="email"
                    autoComplete="email"
                    required
                    maxLength={254}
                    placeholder="you@example.com"
                  />
                </label>
                <label className="field">
                  Password
                  <input
                    name="password"
                    type="password"
                    autoComplete={signup ? 'new-password' : 'current-password'}
                    required
                    minLength={12}
                    maxLength={1024}
                    aria-describedby={signup ? 'password-hint' : undefined}
                  />
                </label>
                {signup && (
                  <p id="password-hint" className="field-hint">
                    At least 12 characters. You can add a passkey next.
                  </p>
                )}
                <button
                  className="button auth-submit"
                  disabled={busy}
                  type="submit"
                >
                  {busy ? 'One moment…' : signup ? 'Create account' : 'Sign in'}
                  <ArrowRight size={16} />
                </button>
              </form>
              {!signup && (
                <>
                  <p className="auth-switch">
                    <Link to="/recover">Forgot your password?</Link>
                  </p>
                  <div className="auth-divider">
                    <span>or</span>
                  </div>
                  <button
                    className="button button-outline auth-submit"
                    disabled={busy}
                    onClick={passkey}
                  >
                    <Fingerprint size={17} />
                    Sign in with a passkey
                  </button>
                </>
              )}
            </>
          )}
          <p className="auth-switch">
            {signup ? 'Already have an account?' : 'New here?'}
            <Link to={signup ? '/login' : '/signup'}>
              {signup ? 'Sign in' : 'Create an account'}
            </Link>
          </p>
        </div>
      </main>
    </div>
  )
}
