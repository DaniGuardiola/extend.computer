import { useEffect, useState, type FormEvent } from 'react'
import * as Ariakit from '@ariakit/react'
import { ShieldCheck, KeyRound, Smartphone } from 'lucide-react'
import type { PublicKeyCredentialCreationOptionsJSON } from '@simplewebauthn/browser'
import { api, message } from '../lib/api'
type Info = {
  enabled: boolean
  totp: boolean
  keys: number
  recovery_codes: number
  totp_available: boolean
}
export function MfaControls({
  verify,
  onChange,
  revision,
}: {
  verify: (work: () => Promise<void>) => void
  onChange: () => Promise<void>
  revision: string
}) {
  const [info, setInfo] = useState<Info | null>(null)
  const [setup, setSetup] = useState<{ secret: string; uri: string } | null>(
    null,
  )
  const [qr, setQr] = useState('')
  const [codes, setCodes] = useState<string[] | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  async function refresh() {
    setInfo(await api<Info>('/mfa'))
    await onChange()
  }
  useEffect(() => {
    let active = true
    api<Info>('/mfa')
      .then((value) => {
        if (active) setInfo(value)
      })
      .catch((e) => {
        if (active) setError(message(e))
      })
    return () => {
      active = false
    }
  }, [revision])
  useEffect(() => {
    let active = true
    setQr('')
    if (setup)
      import('qrcode')
        .then(({ default: QR }) =>
          QR.toDataURL(setup.uri, { width: 220, margin: 2 }),
        )
        .then((url) => {
          if (active) setQr(url)
        })
        .catch(() => {})
    return () => {
      active = false
    }
  }, [setup])
  function action(work: () => Promise<void>) {
    setError('')
    verify(async () => {
      setBusy(true)
      try {
        await work()
        await refresh()
      } catch (e) {
        setError(message(e))
        throw e
      } finally {
        setBusy(false)
      }
    })
  }
  function result(value: { recovery_codes?: string[] }) {
    if (value.recovery_codes?.length) setCodes(value.recovery_codes)
  }
  async function confirm(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const code = new FormData(event.currentTarget).get('code')
    setBusy(true)
    setError('')
    try {
      result(await api('/mfa/totp/confirm', 'POST', { code }))
      setSetup(null)
      await refresh()
    } catch (e) {
      setError(message(e))
    } finally {
      setBusy(false)
    }
  }
  function addKey() {
    action(async () => {
      const { startRegistration } = await import('@simplewebauthn/browser')
      const optionsJSON = await api<PublicKeyCredentialCreationOptionsJSON>(
        '/passkeys/register/options',
        'POST',
        { factor: true },
      )
      await api(
        '/passkeys/register/verify',
        'POST',
        await startRegistration({ optionsJSON }),
      )
      if (!info?.enabled) result(await api('/mfa/enable', 'POST'))
    })
  }
  return (
    <>
      <section className="mfa-section" aria-labelledby="mfa-title">
        <div className="security-heading">
          <div>
            <h2 id="mfa-title">
              <ShieldCheck size={20} />
              Two-factor authentication
            </h2>
            <p>
              {info?.enabled
                ? 'On. Password sign-ins require a second factor.'
                : 'Add another layer to your account.'}
            </p>
          </div>
          <span className="status-label">{info?.enabled ? 'On' : 'Off'}</span>
        </div>
        {error && (
          <p role="alert" className="message error-message">
            {error}
          </p>
        )}
        {info && (
          <div className="security-list">
            <div className="security-row">
              <div>
                <Smartphone size={19} />
                <div>
                  <h3>Authenticator app</h3>
                  <p>
                    {info.totp
                      ? 'Time-based codes enabled.'
                      : 'Use 1Password, Ente Auth, or another TOTP app.'}
                  </p>
                </div>
              </div>
              <div className="mfa-actions">
                <button
                  className="text-button"
                  disabled={busy || !info.totp_available}
                  onClick={() =>
                    action(async () =>
                      setSetup(await api('/mfa/totp/setup', 'POST')),
                    )
                  }
                >
                  {info.totp ? 'Replace' : 'Set up'}
                </button>
                {info.totp && (
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      action(async () => {
                        await api('/mfa/totp', 'DELETE')
                      })
                    }
                  >
                    Remove
                  </button>
                )}
              </div>
            </div>
            <div className="security-row">
              <div>
                <KeyRound size={19} />
                <div>
                  <h3>Passkeys and security keys</h3>
                  <p>
                    {info.keys
                      ? `${info.keys} registered. Use a passkey or touch a USB/NFC key after your password.`
                      : 'Add a passkey or a USB/NFC security key.'}
                  </p>
                </div>
              </div>
              <div className="mfa-actions">
                <button
                  className="text-button"
                  disabled={busy || info.keys >= 10}
                  onClick={addKey}
                >
                  Add key
                </button>
                {!info.enabled && !!info.keys && (
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      action(async () =>
                        result(await api('/mfa/enable', 'POST')),
                      )
                    }
                  >
                    Enable with existing keys
                  </button>
                )}
              </div>
            </div>
            {info.enabled && (
              <>
                <div className="security-row">
                  <div>
                    <ShieldCheck size={19} />
                    <div>
                      <h3>Recovery codes</h3>
                      <p>
                        {info.recovery_codes} unused. Save them somewhere safe;
                        each works once.
                      </p>
                    </div>
                  </div>
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      action(async () =>
                        result(await api('/mfa/recovery-codes', 'POST')),
                      )
                    }
                  >
                    Generate new codes
                  </button>
                </div>
                <div className="security-row">
                  <div>
                    <div>
                      <h3>Turn off two-factor authentication</h3>
                      <p>
                        Other sessions will be signed out when security settings
                        change.
                      </p>
                    </div>
                  </div>
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      action(async () => {
                        await api('/mfa/disable', 'POST')
                      })
                    }
                  >
                    Turn off
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </section>
      <Ariakit.Dialog
        open={!!setup}
        onClose={() => !busy && setSetup(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Set up authenticator</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          Scan this QR code, or enter the setup key in your authenticator. Enter
          its six-digit code to finish.
        </Ariakit.DialogDescription>
        {qr && (
          <img className="totp-qr" src={qr} alt="Authenticator setup QR code" />
        )}
        <label className="field">
          Setup key
          <input readOnly value={setup?.secret ?? ''} spellCheck={false} />
        </label>
        {error && (
          <p role="alert" className="message error-message">
            {error}
          </p>
        )}
        <form onSubmit={confirm}>
          <label className="field">
            Authenticator code
            <input
              name="code"
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9]{6}"
              maxLength={6}
              required
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
              Enable authenticator
            </button>
          </div>
        </form>
      </Ariakit.Dialog>
      <Ariakit.Dialog
        open={!!codes}
        onClose={() => setCodes(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Save your recovery codes</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          These codes are shown once. Each replaces your second factor for one
          sign-in. New codes invalidate older ones.
        </Ariakit.DialogDescription>
        <pre className="recovery-codes">{codes?.join('\n')}</pre>
        <div className="dialog-actions">
          <button
            className="button button-outline"
            onClick={() => {
              const url = URL.createObjectURL(
                new Blob([codes!.join('\n')], { type: 'text/plain' }),
              )
              const a = document.createElement('a')
              a.href = url
              a.download = 'extend-computer-recovery-codes.txt'
              a.click()
              URL.revokeObjectURL(url)
            }}
          >
            Download codes
          </button>
          <Ariakit.DialogDismiss className="button">
            Saved
          </Ariakit.DialogDismiss>
        </div>
      </Ariakit.Dialog>
    </>
  )
}
