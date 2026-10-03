import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from '@tanstack/react-router'
import { Fingerprint, MoveUpRight } from 'lucide-react'
import type { PublicKeyCredentialRequestOptionsJSON } from '@simplewebauthn/browser'
import { SecondFactor, type MfaPending } from './SecondFactor'
import { Brand } from './Brand'
import { ValidatedForm, ValidatedInput } from './ValidatedForm'
import { api, message } from '../lib/api'
export function Auth({
  signup = false,
  previewLoading = false,
  onSignedIn,
}: {
  signup?: boolean
  previewLoading?: boolean
  onSignedIn?: () => void | Promise<void>
}) {
  const navigate = useNavigate()
  const [pending, setPending] = useState<MfaPending | null>(null)
  const [busy, setBusy] = useState(import.meta.env.DEV && previewLoading)
  const [error, setError] = useState('')
  const [errorAttempt, setErrorAttempt] = useState(0)
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    setBusy(true)
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
        setError('')
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
      setErrorAttempt((attempt) => attempt + 1)
    } finally {
      setBusy(false)
    }
  }
  async function passkey() {
    setBusy(true)
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
      setErrorAttempt((attempt) => attempt + 1)
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="auth-layout">
      <aside className="auth-aside">
        <Brand appearance="app" />
        <div>
          <h2>Account</h2>
          <p>Manage your devices and sign-in settings.</p>
        </div>
      </aside>
      <main id="main" className="auth-main auth-account-main">
        <div className="auth-box">
          <Link to="/" className="back-link">
            Back to home <MoveUpRight size={12} />
          </Link>
          <h1 className="auth-title-only">
            {signup ? 'Create an account' : 'Sign in'}
          </h1>
          {error && (
            <p
              key={errorAttempt}
              className="message error-message auth-error-enter"
              role="alert"
            >
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
              <ValidatedForm onSubmit={submit}>
                <label className="field">
                  Email
                  <ValidatedInput
                    aria-label="Email"
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
                  <ValidatedInput
                    aria-label="Password"
                    name="password"
                    type="password"
                    autoComplete={signup ? 'new-password' : 'current-password'}
                    required
                    minLength={12}
                    maxLength={1024}
                    aria-describedby={signup ? 'password-hint' : undefined}
                  />
                </label>
                {!signup && (
                  <p className="auth-recovery-link">
                    <Link to="/recover">Forgot your password?</Link>
                  </p>
                )}
                {signup && (
                  <p id="password-hint" className="field-hint">
                    At least 12 characters. You can add a passkey next.
                  </p>
                )}
                <button
                  className="button auth-submit"
                  disabled={busy}
                  aria-busy={busy}
                  type="submit"
                >
                  {busy ? (
                    <span className="loading-text" role="status">
                      <span className="sr-only">One moment…</span>
                      <span aria-hidden="true">
                        One moment
                        <span className="loading-dots">
                          <span>.</span>
                          <span>.</span>
                          <span>.</span>
                        </span>
                      </span>
                    </span>
                  ) : signup ? (
                    'Create account'
                  ) : (
                    'Sign in'
                  )}
                </button>
              </ValidatedForm>
              {!signup && (
                <>
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
        </div>
        {!pending && (
          <p className="auth-switch">
            {signup ? 'Already have an account?' : 'Need an account?'}
            <Link to={signup ? '/login' : '/signup'}>
              {signup ? 'Sign in' : 'Create an account'}
            </Link>
          </p>
        )}
      </main>
    </div>
  )
}
