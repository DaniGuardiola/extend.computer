import { api } from '../lib/api'
import { useAction } from '../lib/mutations'
import { useQueryClient } from '@tanstack/react-query'
import { ThemeHeader } from './ThemePicker'
import { ErrorFeedback, useErrorFeedback } from './ErrorFeedback'
import { NewPasswordInput } from './NewPasswordInput'
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Link, useNavigate } from '@tanstack/react-router'
import { MoveUpRight } from 'lucide-react'
import { Brand } from './Brand'
import { ValidatedForm, ValidatedInput } from './ValidatedForm'
import { message } from '../lib/api'

export function EmailFlow({ recovery = false }: { recovery?: boolean }) {
  const operation = useAction()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const verification = useRef<Promise<unknown> | null>(null)
  const [link, setLink] = useState<{
    kind: 'verify' | 'reset'
    token: string
  } | null>(null)
  const [ready, setReady] = useState(recovery)
  const [kind, setKind] = useState<'verify' | 'reset' | null>(null)
  const busy = operation.isPending
  const [done, setDone] = useState(false)
  const [error, setError, errorAttempt] = useErrorFeedback()
  useEffect(() => {
    if (recovery) return
    const match = window.location.hash.match(/^#(verify|reset)=([a-f0-9]{64})$/)
    if (match) {
      setLink({ kind: match[1] as 'verify' | 'reset', token: match[2]! })
      setKind(match[1] as 'verify' | 'reset')
    }
    // Tokens stay out of referrers, server logs and the history entry.
    window.history.replaceState(null, '', window.location.pathname)
    setReady(true)
  }, [recovery])
  useEffect(() => {
    if (link?.kind !== 'verify') return
    let active = true
    const request = (verification.current ??= api('POST /auth/email/verify', {
      token: link.token,
    }))
    request
      .then(() => {
        if (!active) return
        void queryClient.invalidateQueries({ queryKey: ['account'] })
        sessionStorage.setItem('extend-email-verified', '1')
        void navigate({ to: '/account' })
      })
      .catch((error) => {
        if (!active) return
        setError(message(error))
      })
    return () => {
      active = false
    }
  }, [link, navigate])
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    try {
      await operation.run(async () => {
        if (recovery)
          await api('POST /auth/recovery/request', {
            email: String(data.get('email') ?? ''),
          })
        else if (link?.kind === 'reset')
          await api('POST /auth/recovery/complete', {
            token: link.token,
            password: String(data.get('password') ?? ''),
          })
        if (!recovery) queryClient.clear()
        setDone(true)
        setError('')
        setLink(null)
      })
    } catch (error) {
      setError(message(error))
    }
  }
  const reset = kind === 'reset'
  const resetLinkInvalid = reset && error.startsWith('Invalid or expired link')
  return (
    <div className="auth-layout">
      <aside className="auth-aside flex flex-col justify-between py-12 px-16 bg-app-aside [border-right:1px_solid_var(--line)]">
        <Brand appearance="app" />
        <div>
          <h2>
            {recovery || reset
              ? 'Account recovery'
              : kind === 'verify'
                ? 'Email verification'
                : 'Account'}
          </h2>
          <p>
            {recovery || reset
              ? 'Reset your password.'
              : kind === 'verify'
                ? 'Confirm your email address.'
                : 'Manage your account.'}
          </p>
        </div>
      </aside>
      <main
        id="main"
        className="auth-main flex justify-center items-center py-16 px-7"
      >
        <ThemeHeader />
        <div className="auth-box w-full max-w-[360px]">
          <Link to="/login" className="back-link">
            Back to sign in <MoveUpRight size={12} />
          </Link>
          <h1>
            {done
              ? recovery
                ? 'Check your inbox'
                : reset
                  ? 'Password reset'
                  : 'Email verified'
              : recovery
                ? 'Reset your password'
                : reset
                  ? resetLinkInvalid
                    ? 'Unable to reset password'
                    : 'Choose a new password'
                  : !ready
                    ? 'Opening your link'
                    : link
                      ? error
                        ? 'Unable to verify email'
                        : 'Verifying your email'
                      : 'This link is incomplete'}
          </h1>
          {error ? (
            <ErrorFeedback error={error} attempt={errorAttempt} />
          ) : null}
          {done ? (
            <>
              <p
                className="auth-description text-app-muted text-[13px] leading-[1.7] mb-[30px]"
                role="status"
              >
                {recovery
                  ? 'If an account exists for this email, we’ll send a reset link. It expires in 30 minutes.'
                  : 'You can sign in to your account now.'}
              </p>
            </>
          ) : resetLinkInvalid ? (
            <Link to="/recover" className="button auth-submit gap-[10px]">
              Request new reset link
            </Link>
          ) : !ready ? (
            <p role="status">Opening your link…</p>
          ) : link?.kind === 'verify' ? (
            error ? null : (
              <p
                className="auth-description text-app-muted text-[13px] leading-[1.7] mb-[30px]"
                role="status"
              >
                One moment…
              </p>
            )
          ) : !recovery && !link ? (
            <p className="auth-description text-app-muted text-[13px] leading-[1.7] mb-[30px]">
              Request a new email from your account, or use password recovery.
            </p>
          ) : (
            <ValidatedForm onSubmit={submit}>
              <p className="auth-description text-app-muted text-[13px] leading-[1.7] mb-[30px]">
                {recovery
                  ? 'Enter your email to request a password reset.'
                  : reset
                    ? 'You’ll be signed out on all devices. If you use two-factor authentication, you’ll still need it to sign in.'
                    : 'Confirm this address belongs to you. This link works once.'}
              </p>
              {recovery ? (
                <label className="field">
                  Email
                  <ValidatedInput
                    aria-label="Email"
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
                  <NewPasswordInput aria-label="New password" name="password" />
                </label>
              ) : null}
              <button className="button auth-submit gap-[10px]" disabled={busy}>
                {busy
                  ? 'One moment…'
                  : recovery
                    ? 'Send reset link'
                    : reset
                      ? 'Reset password'
                      : 'Verify email'}
              </button>
            </ValidatedForm>
          )}
        </div>
      </main>
    </div>
  )
}
