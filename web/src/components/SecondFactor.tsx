import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Fingerprint } from 'lucide-react'
import type { PublicKeyCredentialRequestOptionsJSON } from '@simplewebauthn/browser'
import { CodeInput } from './CodeInput'
import { api, message } from '../lib/api'
export type MfaPending = {
  mfa_required: true
  ticket: string
  totp: boolean
  keys: boolean
  recovery: boolean
}
export function SecondFactor({
  pending,
  onVerified,
  onCancel,
}: {
  pending: MfaPending
  onVerified: () => void | Promise<void>
  onCancel: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [showVerifying, setShowVerifying] = useState(false)
  const verifying = useRef(false)
  const [error, setError] = useState('')
  const [errorAttempt, setErrorAttempt] = useState(0)
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
    setBusy(true)
    try {
      await work()
      await onVerified()
    } catch (e) {
      setError(message(e))
      setErrorAttempt((attempt) => attempt + 1)
    } finally {
      verifying.current = false
      setBusy(false)
    }
  }
  function verifyCode(value: FormDataEntryValue | null) {
    return attempt(() =>
      api('/auth/mfa/verify', 'POST', { ticket: pending.ticket, code: value }),
    )
  }
  async function code(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    await verifyCode(data.get('code'))
  }
  async function key() {
    await attempt(async () => {
      const { startAuthentication } = await import('@simplewebauthn/browser')
      const optionsJSON = await api<PublicKeyCredentialRequestOptionsJSON>(
        '/auth/mfa/key/options',
        'POST',
        { ticket: pending.ticket },
      )
      const credential = await startAuthentication({ optionsJSON })
      await api('/auth/mfa/key/verify', 'POST', {
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
      {error && (
        <p
          key={errorAttempt}
          role="alert"
          className="message error-message auth-error-enter"
        >
          {error}
        </p>
      )}
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
          className="button button-outline auth-submit"
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
