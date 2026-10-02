import { useState, type FormEvent } from 'react'
import * as Ariakit from '@ariakit/react'
import { SecondFactor, type MfaPending } from './SecondFactor'
import { api, message } from '../lib/api'
export function VerifySecurity({
  onVerified,
  onClose,
}: {
  onVerified: () => Promise<void>
  onClose: () => void
}) {
  const [pending, setPending] = useState<MfaPending | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function finish() {
    setBusy(true)
    try {
      await onVerified()
      onClose()
    } catch (e) {
      setError(message(e))
    } finally {
      setBusy(false)
    }
  }
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const password = new FormData(event.currentTarget).get('password')
    setBusy(true)
    setError('')
    try {
      const result = await api<MfaPending | { success: true }>(
        '/mfa/reauth',
        'POST',
        { password },
      )
      if ('mfa_required' in result) setPending(result)
      else await finish()
    } catch (e) {
      setError(message(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Ariakit.Dialog
      open
      onClose={() => !busy && onClose()}
      className="dialog"
      backdrop={<div className="dialog-backdrop" />}
    >
      <Ariakit.DialogHeading>Confirm security change</Ariakit.DialogHeading>
      <Ariakit.DialogDescription>
        Verify your password and any enabled second factor.
      </Ariakit.DialogDescription>
      {error && (
        <p role="alert" className="message error-message">
          {error}
        </p>
      )}
      {pending ? (
        <SecondFactor
          pending={pending}
          onVerified={finish}
          onCancel={() => setPending(null)}
        />
      ) : (
        <form onSubmit={submit}>
          <label className="field">
            Current password
            <input
              name="password"
              type="password"
              autoComplete="current-password"
              required
              autoFocus
            />
          </label>
          <div className="dialog-actions">
            <Ariakit.DialogDismiss
              className="button button-outline"
              disabled={busy}
            >
              Cancel
            </Ariakit.DialogDismiss>
            <button className="button" disabled={busy}>
              Continue
            </button>
          </div>
        </form>
      )}
    </Ariakit.Dialog>
  )
}
