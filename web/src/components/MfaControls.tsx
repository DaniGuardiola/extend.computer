import { api } from '../lib/api'
import { useMfaAction } from '../lib/mutations'
import { ManageKeysLink } from './ManageKeysLink'
import { DownloadButton } from './DownloadButton'
import { CopyButton } from './CopyButton'
import { ErrorFeedback, useErrorFeedback } from './ErrorFeedback'
import { useEffect, useState, type FormEvent } from 'react'
import * as Ariakit from '@ariakit/react'
import { ShieldCheck, KeyRound, Smartphone } from 'lucide-react'
import { SessionSignOutChoice } from './SessionSignOutChoice'
import { CodeInput } from './CodeInput'
import { message, type Passkey } from '../lib/api'
import type { MfaInfo } from '../lib/contracts'
export type { MfaInfo } from '../lib/contracts'
export function MfaControls({
  verify,
  info,
  credentials,
  issuedRecoveryCodes,
}: {
  verify: (work: () => Promise<void>) => void
  info: MfaInfo
  credentials: Passkey[]
  issuedRecoveryCodes?: string[] | null
}) {
  const operation = useMfaAction()
  const factorKeys = credentials.filter((key) => key.purpose === 'factor')

  const verificationMethods = [
    ...(info?.totp ? ['a one-time code'] : []),
    ...(credentials.some((key) => key.purpose !== 'factor')
      ? ['a passkey']
      : []),
    ...(factorKeys.length ? ['a security key'] : []),
  ]
  const [setup, setSetup] = useState<{ secret: string; uri: string } | null>(
    null,
  )
  const [qr, setQr] = useState('')
  const [qrError, setQrError] = useState(false)
  const [setupStep, setSetupStep] = useState<'scan' | 'confirm'>('scan')
  const [showSetupKey, setShowSetupKey] = useState(false)
  const [setupCopyError, setSetupCopyError] = useState(false)
  const [codes, setCodes] = useState<string[] | null>(null)
  useEffect(() => {
    if (issuedRecoveryCodes) setCodes(issuedRecoveryCodes)
  }, [issuedRecoveryCodes])
  const [codesCopyError, setCodesCopyError] = useState(false)
  useEffect(() => setCodesCopyError(false), [codes])
  const [error, setError, errorAttempt] = useErrorFeedback()
  const busy = operation.isPending
  const [signOutOthers, setSignOutOthers] = useState(false)
  const [sensitive, setSensitive] = useState<{
    title: string
    description?: string
    confirmation: string
    work: (signOut: boolean) => Promise<void>
  } | null>(null)
  function sensitiveAction(
    title: string,
    work: (signOut: boolean) => Promise<void>,
    description?: string,
    confirmation = 'Continue',
  ) {
    setSignOutOthers(false)
    setSensitive({ title, work, description, confirmation })
  }
  useEffect(() => {
    let active = true
    setSignOutOthers(false)
    setQr('')
    setQrError(false)
    setSetupStep('scan')
    setShowSetupKey(false)
    setSetupCopyError(false)
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
  function action(work: () => Promise<void>) {
    setError('')
    verify(async () => {
      try {
        await operation.run(work)
      } catch (e) {
        setError(message(e))
        throw e
      }
    })
  }
  function result(value: { recovery_codes?: string[] }) {
    if (value.recovery_codes?.length) setCodes(value.recovery_codes)
  }
  async function confirm(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const code = String(new FormData(event.currentTarget).get('code') ?? '')
    try {
      await operation.run(async () => {
        result(
          await api('POST /mfa/totp/confirm', {
            code,
            sign_out_others: !!info?.totp && signOutOthers,
          }),
        )
        setSetup(null)
        setError('')
      })
    } catch (e) {
      setError(message(e))
    }
  }
  return (
    <>
      <section className="mfa-section" aria-labelledby="mfa-title">
        <div className="security-heading">
          <div>
            <h2 id="mfa-title">Two-factor authentication</h2>
            <p>
              {info?.enabled
                ? `Password sign-ins require ${new Intl.ListFormat('en', { type: 'disjunction' }).format(verificationMethods)}.`
                : 'Require a second factor for password sign-ins.'}
            </p>
          </div>
          {info?.enabled ? (
            <Ariakit.TooltipProvider showTimeout={0} placement="left">
              <Ariakit.TooltipAnchor
                render={<button type="button" />}
                className="status-label [grid-column:2] [grid-row:1] self-center p-0 [border:0] [background:none] text-app-ink leading-[1.4]"
              >
                On
              </Ariakit.TooltipAnchor>
              <Ariakit.Tooltip
                className="account-tooltip"
                flip="right top bottom"
                overflowPadding={16}
              >
                <Ariakit.TooltipArrow />
                Remove all verification methods to turn off two-factor
                authentication.
              </Ariakit.Tooltip>
            </Ariakit.TooltipProvider>
          ) : (
            <span className="status-label status-label-off [grid-column:2] [grid-row:1] self-center p-0 [border:0] [background:none] text-app-ink leading-[1.4]">
              Off
            </span>
          )}
        </div>
        {error && !setup && (
          <ErrorFeedback error={error} attempt={errorAttempt} />
        )}
        {info && (
          <div className="security-list [border-top:1px_solid_var(--line)] mt-[22px]">
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
                      setSetup(await api('POST /mfa/totp/setup')),
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
                          await api('DELETE /mfa/totp', {
                            sign_out_others: signOut,
                          })
                        },
                        info.enabled && !info.keys
                          ? 'This is your last verification method. Removing it will turn off two-factor authentication and invalidate your recovery codes. Password sign-ins will no longer require a second factor.'
                          : 'You can still verify password sign-ins with a passkey or security key.',
                        info.enabled && !info.keys
                          ? 'Remove codes and turn off 2FA'
                          : 'Remove codes',
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
                  <p>{info.keys} available to verify password sign-ins.</p>
                </div>
              </div>
              <div className="mfa-actions">
                <ManageKeysLink>Manage keys</ManageKeysLink>
                {!info.enabled && !!info.keys && (
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      action(async () => result(await api('POST /mfa/enable')))
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
                        {info.recovery_codes} unused. Save them somewhere safe.
                        Each works once.
                      </p>
                    </div>
                  </div>
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() =>
                      action(async () =>
                        result(await api('POST /mfa/recovery-codes')),
                      )
                    }
                  >
                    Generate new codes
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </section>
      <Ariakit.Dialog
        unmountOnHide
        open={!!sensitive}
        onClose={() => setSensitive(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>{sensitive?.title}</Ariakit.DialogHeading>
        {sensitive?.description && (
          <Ariakit.DialogDescription>
            {sensitive.description}
          </Ariakit.DialogDescription>
        )}
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
            {sensitive?.confirmation}
          </button>
        </div>
      </Ariakit.Dialog>
      <Ariakit.Dialog
        unmountOnHide
        open={!!setup}
        onClose={() => {
          if (busy) return
          setError('')
          setSetup(null)
        }}
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
                  <div className="totp-key-copy-row">
                    <div className="totp-key-box">
                      <code>
                        {setup?.secret
                          .match(/.{1,16}/g)
                          ?.map((line) => line.match(/.{1,4}/g)?.join(' '))
                          .join('\n')}
                      </code>
                    </div>
                    <CopyButton
                      value={setup?.secret ?? ''}
                      ariaLabel="Copy setup key"
                      showIcon
                      onError={setSetupCopyError}
                    />
                  </div>
                  {setupCopyError && (
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
            {error && <ErrorFeedback error={error} attempt={errorAttempt} />}
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
        unmountOnHide
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
        {codesCopyError && (
          <p role="status">
            Could not copy. Select and copy the codes manually.
          </p>
        )}
        <div className="recovery-code-actions">
          <CopyButton
            value={codes?.join('\n') ?? ''}
            label="Copy codes"
            showIcon
            onError={setCodesCopyError}
          />
          <DownloadButton
            value={codes?.join('\n') ?? ''}
            filename="extend-computer-recovery-codes.txt"
          />
        </div>
        <div className="dialog-actions">
          <Ariakit.DialogDismiss className="button">
            Confirm saved
          </Ariakit.DialogDismiss>
        </div>
      </Ariakit.Dialog>
    </>
  )
}
