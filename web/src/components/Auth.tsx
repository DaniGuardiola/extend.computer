import { api } from '../lib/api'
import { useAction } from '../lib/mutations'
import { useQueryClient } from '@tanstack/react-query'
import { serverQuery, resetSessionQueries } from '../lib/query'
import { ThemeHeader } from './ThemePicker'
import { ErrorFeedback, useErrorFeedback } from './ErrorFeedback'
import { NewPasswordInput } from './NewPasswordInput'
import { useState, type FormEvent } from 'react'
import { Link, useNavigate } from '@tanstack/react-router'
import { Fingerprint, MoveUpRight } from 'lucide-react'
import { SecondFactor, type MfaPending } from './SecondFactor'
import { Brand } from './Brand'
import { ValidatedForm, ValidatedInput } from './ValidatedForm'
import { message } from '../lib/api'
export function Auth({
  signup = false,
  sessionEnded = false,
  onSignedIn,
}: {
  signup?: boolean
  sessionEnded?: boolean
  onSignedIn?: () => void | Promise<void>
}) {
  const operation = useAction()
  const queryClient = useQueryClient()
  async function signedIn() {
    // MFA reauthentication uses the same endpoints as sign-in. Clear only when
    // the Auth flow completes, not whenever a second factor is verified.
    await resetSessionQueries(queryClient)
    await (onSignedIn ? onSignedIn() : navigate({ to: '/account' }))
  }
  const navigate = useNavigate()
  const [pending, setPending] = useState<MfaPending | null>(null)
  const busy = operation.isPending
  const [error, setError, errorAttempt] = useErrorFeedback()
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    try {
      await operation.run(async () => {
        const result = await api(
          signup ? 'POST /auth/signup' : 'POST /auth/login',
          {
            email: String(data.get('email') ?? ''),
            password: String(data.get('password') ?? ''),
          },
        )
        if ('mfa_required' in result) {
          setError('')
          setPending(result)
          return
        }
        if (signup) {
          const settings = await queryClient.fetchQuery(serverQuery)
          if (settings.email_enabled)
            await api('POST /auth/email/request').catch(() => {})
        }
        await signedIn()
      })
    } catch (error) {
      setError(message(error))
    }
  }
  async function passkey() {
    try {
      await operation.run(async () => {
        const { startAuthentication } = await import('@simplewebauthn/browser')
        const optionsJSON = await api('POST /passkeys/login/options')
        await api(
          'POST /passkeys/login/verify',
          await startAuthentication({ optionsJSON }),
        )
        await signedIn()
      })
    } catch (error) {
      setError(message(error))
    }
  }
  return (
    <div className="auth-layout">
      <aside className="auth-aside flex flex-col justify-between py-12 px-16 bg-app-aside [border-right:1px_solid_var(--line)]">
        <Brand appearance="app" />
        <div>
          <h2>Account</h2>
          <p>Manage your devices and account.</p>
        </div>
      </aside>
      <main
        id="main"
        className="auth-main auth-account-main flex justify-center items-center py-16 px-7 flex-col gap-8"
      >
        <ThemeHeader />
        <div className="auth-box w-full max-w-[360px]">
          <Link to="/" className="back-link">
            Back to home <MoveUpRight size={12} />
          </Link>
          <h1 className="auth-title-only">
            {signup ? 'Create an account' : 'Sign in'}
          </h1>
          {sessionEnded && !pending && !error && (
            <ErrorFeedback
              error="Your session ended. Sign in again to continue."
              attempt={1}
            />
          )}
          {error && <ErrorFeedback error={error} attempt={errorAttempt} />}
          {pending ? (
            <SecondFactor
              pending={pending}
              onVerified={signedIn}
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
                  {signup ? (
                    <NewPasswordInput aria-label="Password" name="password" />
                  ) : (
                    <ValidatedInput
                      aria-label="Password"
                      name="password"
                      type="password"
                      autoComplete="current-password"
                      required
                      minLength={12}
                      maxLength={1024}
                    />
                  )}
                </label>
                {!signup && (
                  <p className="auth-recovery-link text-right text-[12px] text-app-accent mt-[-8px] mr-0 mb-0 ml-0">
                    <Link to="/recover">Forgot your password?</Link>
                  </p>
                )}
                <button
                  className="button auth-submit gap-[10px]"
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
                  <div className="auth-divider text-center text-app-muted text-[11px] my-[22px] mx-0">
                    <span>or</span>
                  </div>
                  <button
                    className="button button-outline auth-submit gap-[10px]"
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
          <p className="auth-switch mt-[27px] text-center text-[12px] text-app-muted">
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
