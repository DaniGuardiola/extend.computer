import { api } from '../lib/api'
import { useAction } from '../lib/mutations'
import { ErrorFeedback, useErrorFeedback } from './ErrorFeedback'
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Fingerprint } from 'lucide-react'
import { CodeInput } from './CodeInput'
import { message } from '../lib/api'
import type { MfaPending } from '../lib/contracts'
export type { MfaPending } from '../lib/contracts'
export function SecondFactor({
  pending,
  onVerified,
  onCancel,
  onBusyChange,
}: {
  pending: MfaPending
  onVerified: () => void | Promise<void>
  onCancel: () => void
  onBusyChange?: (busy: boolean) => void
}) {
  const operation = useAction()
  const busy = operation.isPending
  const [showVerifying, setShowVerifying] = useState(false)
  const verifying = useRef(false)
  const [error, setError, errorAttempt] = useErrorFeedback()
  useEffect(() => {
    setShowVerifying(false)
    if (!busy) return
    const timer = setTimeout(() => setShowVerifying(true), 1000)
    return () => clearTimeout(timer)
  }, [busy])
  const [method, setMethod] = useState<'totp' | 'recovery' | 'key'>(
    pending.totp ? 'totp' : pending.keys ? 'key' : 'recovery',
  )
  function switchMethod(next: 'totp' | 'recovery') {
    setError('')
    setMethod(next)
  }
  async function attempt(work: () => Promise<unknown>) {
    if (verifying.current) return
    verifying.current = true
    onBusyChange?.(true)
    try {
      await operation.run(async () => {
        await work()
        await onVerified()
      })
    } catch (e) {
      setError(message(e))
    } finally {
      verifying.current = false
      onBusyChange?.(false)
    }
  }
  function verifyCode(value: FormDataEntryValue | null) {
    return attempt(() =>
      api('POST /auth/mfa/verify', {
        ticket: pending.ticket,
        code: String(value ?? ''),
      }),
    )
  }
  async function code(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    await verifyCode(String(data.get('code') ?? ''))
  }
  async function key() {
    await attempt(async () => {
      const { startAuthentication } = await import('@simplewebauthn/browser')
      const optionsJSON = await api('POST /auth/mfa/key/options', {
        ticket: pending.ticket,
      })
      const credential = await startAuthentication({ optionsJSON })
      await api('POST /auth/mfa/key/verify', {
        ...credential,
        ticket: pending.ticket,
      })
    })
  }
  return (
    <div className="second-factor">
      <p className="second-factor-description">
        {method === 'totp'
          ? 'Enter your six-digit one-time code.'
          : method === 'recovery'
            ? 'Enter one of your saved recovery codes. Each works once.'
            : 'Confirm with your passkey or security key.'}
      </p>
      {error && <ErrorFeedback error={error} attempt={errorAttempt} />}
      {method !== 'key' && (
        <form key={method} onSubmit={code}>
          <label className="field">
            {method === 'totp' ? 'One-time code' : 'Recovery code'}
            <CodeInput
              length={method === 'recovery' ? 8 : 6}
              disabled={busy}
              onComplete={(value) => {
                void verifyCode(value)
              }}
            />
          </label>
          <p role="status" className="code-verifying">
            {busy && showVerifying ? 'Verifying…' : ''}
          </p>
        </form>
      )}
      {pending.keys && (
        <button
          className="button button-outline auth-submit gap-[10px]"
          onClick={key}
          disabled={busy}
        >
          <Fingerprint size={17} />
          Use passkey or security key
        </button>
      )}
      <div className="second-factor-alternatives">
        {method !== 'recovery' && pending.recovery && (
          <button
            className="text-button link-button"
            disabled={busy}
            onClick={() => switchMethod('recovery')}
          >
            Use a recovery code
          </button>
        )}
        {method === 'recovery' && pending.totp && (
          <button
            className="text-button link-button"
            disabled={busy}
            onClick={() => switchMethod('totp')}
          >
            Use a one-time code
          </button>
        )}
        <button
          className="text-button link-button"
          disabled={busy}
          onClick={onCancel}
        >
          Start over
        </button>
      </div>
    </div>
  )
}
