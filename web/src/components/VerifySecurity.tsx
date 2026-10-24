import { api } from '../lib/api'
import { useAction } from '../lib/mutations'
import { ErrorFeedback, useErrorFeedback } from './ErrorFeedback'
import { useState, type FormEvent } from 'react'
import * as Ariakit from '@ariakit/react'
import { SecondFactor, type MfaPending } from './SecondFactor'
import { message } from '../lib/api'
export function VerifySecurity({
  onVerified,
  onClose,
}: {
  onVerified: () => Promise<void>
  onClose: () => void
}) {
  const operation = useAction()
  const [pending, setPending] = useState<MfaPending | null>(null)
  const [verifying, setVerifying] = useState(false)
  const busy = operation.isPending || verifying
  const [error, setError, errorAttempt] = useErrorFeedback()
  async function finish() {
    try {
      await operation.run(async () => {
        await onVerified()
        onClose()
      })
    } catch (e) {
      setError(message(e))
    }
  }
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const password = String(
      new FormData(event.currentTarget).get('password') ?? '',
    )
    try {
      await operation.run(async () => {
        const result = await api('POST /mfa/reauth', { password })
        if ('mfa_required' in result) {
          setError('')
          setPending(result)
        } else {
          await onVerified()
          onClose()
        }
      })
    } catch (e) {
      setError(message(e))
    }
  }
  return (
    <Ariakit.Dialog
      unmountOnHide
      open
      onClose={() => !busy && onClose()}
      className="dialog"
      backdrop={<div className="dialog-backdrop" />}
    >
      <Ariakit.DialogHeading>Confirm security change</Ariakit.DialogHeading>
      <Ariakit.DialogDescription>
        Verify your password and any enabled second factor.
      </Ariakit.DialogDescription>
      {error && <ErrorFeedback error={error} attempt={errorAttempt} />}
      {pending ? (
        <SecondFactor
          pending={pending}
          onVerified={finish}
          onCancel={() => setPending(null)}
          onBusyChange={setVerifying}
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
