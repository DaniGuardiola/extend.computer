import { useEffect, useState, type FormEvent } from 'react'
import * as Ariakit from '@ariakit/react'
import { ShieldCheck, KeyRound, Smartphone } from 'lucide-react'
import type { PublicKeyCredentialCreationOptionsJSON } from '@simplewebauthn/browser'
import { SessionSignOutChoice } from './SessionSignOutChoice'
import { CodeInput } from './CodeInput'
import { api, message } from '../lib/api'
export type MfaInfo = {
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
  initialData,
}: {
  verify: (work: () => Promise<void>) => void
  onChange: () => Promise<void>
  revision: string
  initialData?: MfaInfo
}) {
  const [info, setInfo] = useState<MfaInfo | null>(initialData ?? null)
  const [setup, setSetup] = useState<{ secret: string; uri: string } | null>(
    null,
  )
  const [qr, setQr] = useState('')
  const [qrError, setQrError] = useState(false)
  const [setupStep, setSetupStep] = useState<'scan' | 'confirm'>('scan')
  const [showSetupKey, setShowSetupKey] = useState(false)
  const [copyStatus, setCopyStatus] = useState<'idle' | 'copied' | 'error'>(
    'idle',
  )
  const [codes, setCodes] = useState<string[] | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [signOutOthers, setSignOutOthers] = useState(false)
  const [sensitive, setSensitive] = useState<{
    title: string
    work: (signOut: boolean) => Promise<void>
  } | null>(null)
  function sensitiveAction(
    title: string,
    work: (signOut: boolean) => Promise<void>,
  ) {
    setSignOutOthers(false)
    setSensitive({ title, work })
  }
  async function refresh() {
    setInfo(await api<MfaInfo>('/mfa'))
    await onChange()
  }
  useEffect(() => {
    if (initialData) {
      setInfo(initialData)
      return
    }
    let active = true
    api<MfaInfo>('/mfa')
      .then((value) => {
        if (active) setInfo(value)
      })
      .catch((e) => {
        if (active) setError(message(e))
      })
    return () => {
      active = false
    }
  }, [revision, initialData])
  useEffect(() => {
    let active = true
    setSignOutOthers(false)
    setQr('')
    setQrError(false)
    setSetupStep('scan')
    setShowSetupKey(false)
    setCopyStatus('idle')
    if (setup)
      import('qrcode')
        .then(({ default: QR }) =>
          QR.toDataURL(setup.uri, { width: 220, margin: 2 }),
        )
        .then((url) => {
          if (active) setQr(url)
        })
        .catch(() => {
          if (active) setQrError(true)
        })
    return () => {
      active = false
    }
  }, [setup])
  async function copySetupKey() {
    if (!setup) return
    try {
      await navigator.clipboard.writeText(setup.secret)
      setCopyStatus('copied')
    } catch {
      setCopyStatus('error')
    }
  }
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
      result(
        await api('/mfa/totp/confirm', 'POST', {
          code,
          sign_out_others: !!info?.totp && signOutOthers,
        }),
      )
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
            <h2 id="mfa-title">Two-factor authentication</h2>
            <p>
              {info?.enabled
                ? 'On. Password sign-ins require a second factor.'
                : 'Require a second factor when signing in.'}
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
                  <h3>One-time codes</h3>
                  <p>
                    {info.totp
                      ? 'One-time codes enabled.'
                      : 'Generate sign-in codes with an app or password manager.'}
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
                      sensitiveAction(
                        'Remove one-time codes?',
                        async (signOut) => {
                          await api('/mfa/totp', 'DELETE', {
                            sign_out_others: signOut,
                          })
                        },
                      )
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
                      ? `${info.keys} registered. Passkeys let you sign in directly; touch-only keys follow your password.`
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
                    <ShieldCheck size={19} />
                    <div>
                      <h3>Turn off two-factor authentication</h3>
                      <p>
                        Password sign-ins will no longer require a second
                        factor.
                      </p>
                    </div>
                  </div>
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      sensitiveAction(
                        'Turn off two-factor authentication?',
                        async (signOut) => {
                          await api('/mfa/disable', 'POST', {
                            sign_out_others: signOut,
                          })
                        },
                      )
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
        open={!!sensitive}
        onClose={() => setSensitive(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>{sensitive?.title}</Ariakit.DialogHeading>
        <SessionSignOutChoice
          checked={signOutOthers}
          onChange={setSignOutOthers}
        />
        <div className="dialog-actions">
          <Ariakit.DialogDismiss className="button button-outline">
            Cancel
          </Ariakit.DialogDismiss>
          <button
            className="button danger"
            onClick={() => {
              const pending = sensitive
              if (!pending) return
              const signOut = signOutOthers
              setSensitive(null)
              action(() => pending.work(signOut))
            }}
          >
            Continue
          </button>
        </div>
      </Ariakit.Dialog>
      <Ariakit.Dialog
        open={!!setup}
        onClose={() => !busy && setSetup(null)}
        className="dialog totp-setup-dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Set up one-time codes</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          {setupStep === 'scan'
            ? 'Scan this QR code with your app or password manager.'
            : 'Enter your six-digit one-time code to finish setup.'}
        </Ariakit.DialogDescription>
        {setupStep === 'scan' ? (
          <>
            <div className="totp-scan">
              {qr ? (
                <img
                  className="totp-qr"
                  src={qr}
                  alt="One-time code setup QR code"
                />
              ) : (
                <p role="status">
                  {qrError
                    ? 'Could not display the QR code. Use the setup key instead.'
                    : 'Loading QR code…'}
                </p>
              )}
              <button
                type="button"
                className="text-button"
                aria-expanded={showSetupKey}
                aria-controls="totp-setup-key"
                onClick={() => setShowSetupKey(!showSetupKey)}
              >
                {showSetupKey ? 'Hide setup key' : 'Can’t scan? Show setup key'}
              </button>
              {showSetupKey && (
                <div id="totp-setup-key" className="totp-manual-setup">
                  <p>Add this key to your app or password manager.</p>
                  <div className="totp-key-row">
                    <code>{setup?.secret}</code>
                    <button
                      type="button"
                      className="text-button"
                      aria-label="Copy setup key"
                      onClick={() => void copySetupKey()}
                    >
                      <span aria-live="polite">
                        {copyStatus === 'copied' ? 'Copied' : 'Copy'}
                      </span>
                    </button>
                  </div>
                  {copyStatus === 'error' && (
                    <p role="status">
                      Could not copy. Select and copy the key manually.
                    </p>
                  )}
                </div>
              )}
            </div>
            <div className="dialog-actions">
              <Ariakit.DialogDismiss
                className="button button-outline"
                disabled={busy}
              >
                Cancel
              </Ariakit.DialogDismiss>
              <button
                type="button"
                className="button"
                disabled={busy || (!qr && !showSetupKey)}
                onClick={() => setSetupStep('confirm')}
              >
                Next
              </button>
            </div>
          </>
        ) : (
          <form onSubmit={confirm}>
            {error && (
              <p role="alert" className="message error-message">
                {error}
              </p>
            )}
            <label className="field">
              One-time code
              <CodeInput disabled={busy} />
            </label>
            {info?.totp && (
              <SessionSignOutChoice
                checked={signOutOthers}
                onChange={setSignOutOthers}
              />
            )}
            <div className="dialog-actions">
              <button
                type="button"
                className="button button-outline"
                disabled={busy}
                onClick={() => {
                  setError('')
                  setSetupStep('scan')
                }}
              >
                Back
              </button>
              <button className="button" disabled={busy}>
                {busy ? 'Verifying…' : 'Enable one-time codes'}
              </button>
            </div>
          </form>
        )}
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
        <ul className="recovery-codes" aria-label="Recovery codes">
          {codes?.map((code) => (
            <li key={code}>
              <code>{code}</code>
            </li>
          ))}
        </ul>
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
