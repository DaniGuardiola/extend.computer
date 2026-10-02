import { useState, type FormEvent } from 'react'
import { Fingerprint } from 'lucide-react'
import type { PublicKeyCredentialRequestOptionsJSON } from '@simplewebauthn/browser'
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
  const [error, setError] = useState('')
  async function attempt(work: () => Promise<unknown>) {
    setBusy(true)
    setError('')
    try {
      await work()
      await onVerified()
    } catch (e) {
      setError(message(e))
    } finally {
      setBusy(false)
    }
  }
  async function code(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    await attempt(() =>
      api('/auth/mfa/verify', 'POST', {
        ticket: pending.ticket,
        code: data.get('code'),
      }),
    )
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
      <p>Confirm it’s you with a second factor.</p>
      {error && (
        <p role="alert" className="message error-message">
          {error}
        </p>
      )}
      <form onSubmit={code}>
        <label className="field">
          {pending.totp ? 'Authenticator or recovery code' : 'Recovery code'}
          <input
            name="code"
            autoComplete="one-time-code"
            required
            maxLength={39}
            autoFocus
            spellCheck={false}
            autoCapitalize="none"
          />
        </label>
        <button className="button auth-submit" disabled={busy}>
          Verify code
        </button>
      </form>
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
      <button className="text-button" disabled={busy} onClick={onCancel}>
        Start over
      </button>
    </div>
  )
}
