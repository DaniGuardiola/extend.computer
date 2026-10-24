import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  accountQuery,
  devicesQuery,
  passkeysQuery,
  mfaQuery,
  sessionsQuery,
  useClientReady,
} from '../lib/query'
import {
  useAction,
  useRefreshAccount,
  useRegisterKey,
  useRemoveCredential,
  usePasskeyUsage,
  useChangePassword,
} from '../lib/mutations'
import { ThemePicker } from '../components/ThemePicker'
import { ErrorFeedback, useErrorFeedback } from '../components/ErrorFeedback'
import { ManageKeysLink } from '../components/ManageKeysLink'
import { CredentialRow } from '../components/CredentialRow'
import { NewPasswordInput } from '../components/NewPasswordInput'
import { useEffect, useState, type FormEvent } from 'react'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import * as Ariakit from '@ariakit/react'
import {
  Fingerprint,
  Check,
  KeyRound,
  Laptop,
  LogOut,
  Monitor,
  RefreshCw,
  Smartphone,
  Tablet,
  Trash2,
  X,
} from 'lucide-react'
import { AccountSkeleton } from '../components/AccountSkeleton'
import { MfaControls } from '../components/MfaControls'
import { Sessions } from '../components/Sessions'
import { VerifySecurity } from '../components/VerifySecurity'
import { SessionSignOutChoice } from '../components/SessionSignOutChoice'
import { Brand } from '../components/Brand'
import { ValidatedForm, ValidatedInput } from '../components/ValidatedForm'
import { api, ApiError, message, type Passkey } from '../lib/api'
export const Route = createFileRoute('/account')({
  component: Dashboard,
  head: () => ({ meta: [{ title: 'Your account · extend.computer' }] }),
})
type Removal = { kind: 'devices' | 'passkeys'; id: string; name: string }
function platformLabel(platform: string) {
  const labels: Record<string, string> = {
    macos: 'macOS',
    windows: 'Windows',
    linux: 'Linux',
    ios: 'iOS',
    ipados: 'iPadOS',
    android: 'Android',
  }
  return (
    labels[platform.toLowerCase()] ??
    platform.charAt(0).toUpperCase() + platform.slice(1)
  )
}
function Dashboard() {
  const [verification, setVerification] = useState<
    (() => Promise<void>) | null
  >(null)
  const navigate = useNavigate()
  const operation = useAction()
  const emailOperation = useAction()
  const registration = useRegisterKey()
  const removalAction = useRemoveCredential()
  const usageAction = usePasskeyUsage()
  const passwordAction = useChangePassword()
  const queryClient = useQueryClient()
  const enabled = useClientReady()
  const userQuery = useQuery({ ...accountQuery, enabled })
  const registeredQuery = useQuery({ ...devicesQuery, enabled })
  const keysQuery = useQuery({ ...passkeysQuery, enabled })
  const factorsQuery = useQuery({ ...mfaQuery, enabled })
  const loggedInQuery = useQuery({ ...sessionsQuery, enabled })
  const queries = [
    userQuery,
    registeredQuery,
    keysQuery,
    factorsQuery,
    loggedInQuery,
  ]
  const complete = queries.every((query) => query.data !== undefined)
  const loading =
    !complete && (!enabled || queries.some((query) => query.isFetching))
  const account = complete ? userQuery.data : null
  const devices = registeredQuery.data?.devices ?? []
  const passkeys = keysQuery.data?.passkeys ?? []
  const mfa = factorsQuery.data
  const sessions = loggedInQuery.data?.sessions ?? []
  const busy = operation.isPending
  const [refreshState, setRefreshState] = useState<
    'idle' | 'refreshing' | 'refreshed'
  >('idle')
  const [error, setError, errorAttempt] = useErrorFeedback()
  const [deviceRefreshError, setDeviceRefreshError, deviceRefreshAttempt] =
    useErrorFeedback()
  const [success, setSuccess] = useState('')
  const [emailVerifiedNotice, setEmailVerifiedNotice] = useState(false)
  const emailSending = emailOperation.isPending
  const [emailSent, setEmailSent] = useState(0)
  const [emailError, setEmailError, emailErrorAttempt] = useErrorFeedback()
  const [emailRetryAt, setEmailRetryAt] = useState(0)
  const [emailCooldown, setEmailCooldown] = useState(0)
  useEffect(() => {
    if (!emailRetryAt) return
    const update = () =>
      setEmailCooldown(
        Math.max(0, Math.ceil((emailRetryAt - Date.now()) / 1000)),
      )
    update()
    const timer = window.setInterval(update, 1000)
    return () => window.clearInterval(timer)
  }, [emailRetryAt])
  async function sendVerificationEmail() {
    if (emailSending || Date.now() < emailRetryAt) return
    setEmailError('')
    try {
      await emailOperation.run(async () => {
        const result = await api('POST /auth/email/request')
        const cooldown = result.retry_after
        setEmailSent((count) => count + 1)
        setEmailCooldown(cooldown)
        setEmailRetryAt(Date.now() + cooldown * 1000)
      })
    } catch (error) {
      setEmailError(message(error))
      if (error instanceof ApiError && error.status === 429) {
        const cooldown = error.retryAfter ?? 60
        setEmailCooldown(cooldown)
        setEmailRetryAt(Date.now() + cooldown * 1000)
      }
    }
  }
  const [removal, setRemoval] = useState<Removal | null>(null)
  const [passwordOpen, setPasswordOpen] = useState(false)
  const [usageChange, setUsageChange] = useState<Passkey | null>(null)
  const [issuedRecoveryCodes, setIssuedRecoveryCodes] = useState<
    string[] | null
  >(null)
  const [signOutOthers, setSignOutOthers] = useState(false)
  const signInPasskeys = passkeys.filter((key) => key.purpose === 'login')
  const removalKey =
    removal?.kind === 'passkeys'
      ? passkeys.find((key) => key.id === removal.id)
      : undefined
  const removingFactor = removalKey?.purpose === 'factor'
  const removingLastMethod = !!mfa?.enabled && !mfa.totp && mfa.keys <= 1
  function removeCredential(key: Passkey) {
    setRemoval({ kind: 'passkeys', id: key.id, name: key.name })
  }
  const refresh = useRefreshAccount()
  async function refreshDevices() {
    await refresh(['devices'])
  }
  const loadError = !complete
    ? queries.find((query) => query.error)?.error
    : null
  useEffect(() => {
    if (loadError) setError(message(loadError))
  }, [loadError, setError])
  useEffect(() => {
    if (enabled && userQuery.data === null) {
      queryClient.clear()
      window.location.assign('/login?reason=session-ended')
    }
  }, [enabled, userQuery.data, queryClient])
  useEffect(() => {
    if (
      account?.email_verified &&
      sessionStorage.getItem('extend-email-verified')
    ) {
      setEmailVerifiedNotice(true)
      sessionStorage.removeItem('extend-email-verified')
    }
  }, [account?.email_verified])
  async function action(
    work: () => Promise<void>,
    notice = '',
    propagate = false,
  ) {
    setSuccess('')
    try {
      await operation.run(work)
      setError('')
      setSuccess(notice)
    } catch (error) {
      setError(message(error))
      if (propagate) throw error
    }
  }
  function verify(work: () => Promise<void>) {
    setVerification(() => work)
  }
  function protectedAction(work: () => Promise<void>, notice = '') {
    if (account?.mfa_enabled) verify(() => action(work, notice, true))
    else void action(work, notice)
  }
  function addKey() {
    verify(() =>
      action(
        async () => {
          await registration.run(setIssuedRecoveryCodes)
        },
        'Key added.',
        true,
      ),
    )
  }
  function changeCredentialUsage(key: Passkey) {
    const purpose = key.purpose === 'login' ? 'login-disabled' : 'login'
    verify(() =>
      action(
        async () => {
          await usageAction.run({ id: key.id, enabled: purpose === 'login' })
        },
        'Passkey use updated.',
        true,
      ),
    )
  }
  useEffect(() => setSignOutOthers(false), [passwordOpen, removal])
  async function changePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    protectedAction(
      async () => {
        await passwordAction.run({
          current_password: String(data.get('current') ?? ''),
          password: String(data.get('password') ?? ''),
          sign_out_others: signOutOthers,
        })
        setPasswordOpen(false)
      },
      signOutOthers
        ? 'Password changed. Other sessions signed out.'
        : 'Password changed.',
    )
  }
  return (
    <div className="dashboard min-h-[100dvh] flex flex-col max-w-[1120px] m-auto py-0 px-12">
      <header className="site-header">
        <Brand appearance="app" />
        <div className="account-nav flex items-center gap-[15px]">
          <span className="account-email text-app-muted text-[11px]">
            {account ? (
              account.email
            ) : loading ? (
              <span className="account-loading-inline" aria-hidden="true">
                <span className="skeleton skeleton-email w-[150px] h-[13px]" />
              </span>
            ) : null}
          </span>
          <button
            className="text-button"
            disabled={busy || !account}
            onClick={() =>
              action(async () => {
                await api('POST /auth/logout')
                queryClient.clear()
                await navigate({ to: '/login' })
              })
            }
          >
            <LogOut size={14} />
            Sign out
          </button>
        </div>
      </header>
      <main id="main" className="dashboard-main flex-1 py-[66px] px-0">
        {account && emailVerifiedNotice && (
          <section
            className="email-notice email-verified-notice"
            aria-label="Email verified"
          >
            <div role="status">
              <Check size={20} aria-hidden="true" />
              <h2>Email verified!</h2>
            </div>
            <button
              className="icon-button"
              aria-label="Dismiss email verification confirmation"
              onClick={() => setEmailVerifiedNotice(false)}
            >
              <X size={18} />
            </button>
          </section>
        )}
        {account &&
        account.email_enabled &&
        !account.email_verified &&
        !emailVerifiedNotice ? (
          <section className="email-notice" aria-label="Verify email">
            <div>
              <h2>Verify your email</h2>
              <p>Verify {account.email} to enable password recovery.</p>
              {emailSent > 0 && (
                <p
                  key={emailSent}
                  className="email-send-confirmation"
                  role="status"
                >
                  <Check size={16} aria-hidden="true" />
                  <span>Verification email sent. Check your inbox.</span>
                </p>
              )}
              {emailError && (
                <ErrorFeedback error={emailError} attempt={emailErrorAttempt} />
              )}
            </div>
            <button
              className="button button-outline"
              disabled={busy || emailSending || emailCooldown > 0}
              onClick={sendVerificationEmail}
            >
              {emailSending
                ? 'Sending…'
                : emailCooldown > 0
                  ? `Resend in ${emailCooldown >= 60 ? `${Math.ceil(emailCooldown / 60)} min` : `${emailCooldown}s`}`
                  : emailSent
                    ? 'Resend verification email'
                    : 'Send verification email'}
            </button>
          </section>
        ) : null}
        {error && (account || loading) && !passwordOpen && !verification && (
          <ErrorFeedback error={error} attempt={errorAttempt} />
        )}
        {success && (
          <p className="message success-message" role="status">
            {success}
          </p>
        )}
        {(account || loading) && (
          <h1 className="account-page-title text-[48px] tracking-[-2px] mb-10">
            Account
          </h1>
        )}
        <div
          className={
            !account && !loading
              ? 'account-load-failure [width:min(100%,_420px)] flex flex-col items-center gap-6 text-center'
              : 'dashboard-title security-heading'
          }
        >
          {!account && !loading && (
            <>
              <div
                className="account-failure-illustration grid place-items-center w-[180px] h-[104px] text-app-device"
                aria-hidden="true"
              >
                <svg
                  width="180"
                  height="104"
                  viewBox="0 0 180 104"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.7"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect
                    x="8"
                    y="24"
                    width="58"
                    height="43"
                    rx="5"
                    fill="var(--panel)"
                  />
                  <path d="M37 67v12M24 79h26" />
                  <path d="M20 36h14M20 43h23" opacity=".45" />
                  <circle cx="143" cy="46" r="27" fill="var(--panel)" />
                  <circle cx="143" cy="39" r="7" />
                  <path d="M130 59c0-12 26-12 26 0" />
                  <path
                    d="M73 46h9m20 0h7"
                    strokeDasharray="3 4"
                    opacity=".6"
                  />
                  <path d="m88 42 8 8m0-8-8 8" stroke="var(--error-ink)" />
                </svg>
              </div>
              <ErrorFeedback error={error} attempt={errorAttempt} />
            </>
          )}
          {(account || loading) && (
            <div>
              <h2>Devices</h2>
              <p>Manage your registered devices.</p>
            </div>
          )}
          <button
            className={
              !account && !loading
                ? 'button button-outline'
                : 'text-button account-refresh'
            }
            data-state={refreshState}
            aria-label={
              refreshState === 'refreshing'
                ? 'Refreshing…'
                : refreshState === 'refreshed'
                  ? 'Refreshed!'
                  : !account && !loading
                    ? 'Try again'
                    : 'Refresh'
            }
            disabled={busy || loading || refreshState !== 'idle'}
            aria-busy={refreshState === 'refreshing'}
            onClick={async () => {
              if (!account) setError('')
              setDeviceRefreshError('')
              setRefreshState('refreshing')
              const [result] = await Promise.allSettled([
                account ? refreshDevices() : refresh(),
                new Promise((resolve) => setTimeout(resolve, 2400)),
              ])
              if (result.status === 'fulfilled') {
                if (!account) setError('')
                setRefreshState('refreshed')
                await new Promise((resolve) => setTimeout(resolve, 3600))
              } else {
                if (account) setDeviceRefreshError(message(result.reason))
                else setError(message(result.reason))
              }
              setRefreshState('idle')
            }}
          >
            {refreshState === 'refreshed' ? (
              <Check size={14} aria-hidden="true" />
            ) : (
              <RefreshCw size={14} aria-hidden="true" />
            )}
            <span
              className="account-refresh-label"
              role="status"
              aria-live="polite"
            >
              {refreshState === 'refreshing'
                ? 'Refreshing…'
                : refreshState === 'refreshed'
                  ? 'Refreshed!'
                  : !account && !loading
                    ? 'Try again'
                    : 'Refresh'}
            </span>
          </button>
        </div>
        {account && deviceRefreshError && (
          <ErrorFeedback
            error={deviceRefreshError}
            attempt={deviceRefreshAttempt}
          />
        )}
        {!account && loading ? (
          <AccountSkeleton loading={loading} />
        ) : account ? (
          <div className="account-content">
            <section
              aria-label="Registered devices"
              className="device-list mt-[26px] mr-0 mb-[50px] ml-0 [border:1px_solid_var(--line)] rounded-[7px] overflow-hidden"
            >
              {devices.length ? (
                devices.map((device) => (
                  <article
                    key={device.id}
                    className="device-row flex items-center p-[23px] [border-top:1px_solid_var(--line)] gap-5"
                  >
                    <div className="device-icon w-[46px] h-[46px] bg-app-panel [border:1px_solid_var(--line)] rounded-[6px] grid place-items-center text-app-device shrink-0">
                      {device.platform === 'macos' ? (
                        <Laptop size={22} />
                      ) : device.platform === 'android' ? (
                        <Smartphone size={22} />
                      ) : (
                        <Monitor size={22} />
                      )}
                    </div>
                    <div className="device-detail flex-1 min-w-0">
                      <h3>{device.name}</h3>
                      <p>
                        {platformLabel(device.platform)} ·{' '}
                        <Ariakit.TooltipProvider
                          showTimeout={0}
                          placement="left"
                        >
                          <Ariakit.TooltipAnchor render={<span tabIndex={0} />}>
                            {device.fingerprint.slice(0, 12)}
                          </Ariakit.TooltipAnchor>
                          <Ariakit.Tooltip
                            className="account-tooltip"
                            flip="right top bottom"
                            overflowPadding={16}
                          >
                            <Ariakit.TooltipArrow />
                            Device fingerprint: a short identifier derived from
                            this device’s public key.
                          </Ariakit.Tooltip>
                        </Ariakit.TooltipProvider>
                      </p>
                    </div>
                    <span
                      className={
                        'device-status text-[11px] inline-flex gap-[7px] items-center text-app-muted' +
                        (device.online ? ' online' : '')
                      }
                    >
                      <i
                        className={
                          device.online
                            ? 'status-dot'
                            : 'offline-dot w-[6px] h-[6px] rounded-[50%] [background:#747f6b]'
                        }
                      />
                      {device.online ? 'Online' : 'Offline'}
                    </span>
                    <button
                      className="icon-button"
                      aria-label={'Remove ' + device.name}
                      disabled={busy}
                      onClick={() =>
                        setRemoval({
                          kind: 'devices',
                          id: device.id,
                          name: device.name,
                        })
                      }
                    >
                      <Trash2 size={15} />
                    </button>
                  </article>
                ))
              ) : (
                <div className="empty-devices py-[50px] px-[25px] text-center">
                  <div
                    className="empty-device-icons flex justify-center items-center gap-[18px] text-app-accent mb-[18px]"
                    aria-hidden="true"
                  >
                    <Smartphone size={32} />
                    <Laptop size={32} />
                    <Monitor size={32} />
                    <Tablet size={32} className="empty-device-tablet" />
                  </div>
                  <h2>No devices registered</h2>
                  <p>Open the app and sign in to add a device.</p>
                </div>
              )}
            </section>

            <section aria-labelledby="security-title">
              <div className="security-heading">
                <div>
                  <h2 id="security-title">Sign-in methods</h2>
                  <p>Manage how you sign in.</p>
                </div>
              </div>
              <div className="security-list [border-top:1px_solid_var(--line)] mt-[22px]">
                <div className="security-row">
                  <div>
                    <KeyRound size={19} />
                    <div>
                      <h3>Password</h3>
                      <p>Use your password to sign in.</p>
                    </div>
                  </div>
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() => {
                      setError('')
                      setPasswordOpen(true)
                    }}
                  >
                    Change
                  </button>
                </div>
                <div className="security-row">
                  <div>
                    <Fingerprint size={19} />
                    <div>
                      <h3>Passkeys</h3>
                      <p>
                        {signInPasskeys.length} available for sign-in without
                        your password.
                      </p>
                    </div>
                  </div>
                  <ManageKeysLink>Manage passkeys</ManageKeysLink>
                </div>
              </div>
            </section>
            {mfa && (
              <MfaControls
                credentials={passkeys}
                issuedRecoveryCodes={issuedRecoveryCodes}
                info={mfa}
                verify={verify}
              />
            )}
            <section aria-labelledby="credentials-title">
              <div className="security-heading">
                <div>
                  <h2 id="credentials-title" tabIndex={-1}>
                    Passkeys and security keys
                  </h2>
                  <p>Manage your keys and how you use them.</p>
                </div>
              </div>
              <div
                className="security-list [border-top:1px_solid_var(--line)] mt-[22px]"
                role="group"
                aria-label="Passkeys and security keys"
              >
                {passkeys.map((key) => (
                  <CredentialRow
                    key={key.id}
                    credential={key}
                    disabled={busy}
                    onRemove={removeCredential}
                    onUsageChange={setUsageChange}
                  />
                ))}
                {!passkeys.length && (
                  <div className="security-row">
                    <div>
                      <Fingerprint size={19} />
                      <div>
                        <h3>No keys added</h3>
                        <p>
                          Add a passkey to sign in or a key to verify password
                          sign-ins.
                        </p>
                      </div>
                    </div>
                  </div>
                )}
              </div>
              <div className="credential-add-actions flex flex-wrap [gap:8px_20px] mt-3">
                <button
                  className="text-button"
                  disabled={busy || passkeys.length >= 10}
                  onClick={addKey}
                >
                  Add key
                </button>
              </div>
            </section>
            <Sessions
              sessions={sessions}
              onSignedOut={async () => {
                queryClient.clear()
                await navigate({ to: '/login' })
              }}
            />
          </div>
        ) : null}
      </main>
      <footer className="dashboard-footer">
        <span>extend.computer</span>
        <ThemePicker />
        <a href="/">Back to home ↗</a>
      </footer>
      {verification && (
        <VerifySecurity
          onVerified={verification}
          onClose={() => {
            setVerification(null)
          }}
        />
      )}
      <Ariakit.Dialog
        unmountOnHide
        open={!!usageChange}
        onClose={() => !busy && setUsageChange(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>
          {usageChange?.purpose === 'login' ? 'Turn off' : 'Turn on'} sign-in
          with {usageChange?.name}?
        </Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          {usageChange?.purpose === 'login'
            ? 'This passkey will no longer sign you in without your password. You can still use it to verify password sign-ins when two-factor authentication is on.'
            : 'This passkey will let you sign in without your password. You can also keep using it to verify password sign-ins.'}
        </Ariakit.DialogDescription>
        <div className="dialog-actions">
          <Ariakit.DialogDismiss className="button button-outline">
            Cancel
          </Ariakit.DialogDismiss>
          <button
            className="button"
            onClick={() => {
              const key = usageChange
              setUsageChange(null)
              if (key) changeCredentialUsage(key)
            }}
          >
            {usageChange?.purpose === 'login'
              ? 'Turn off sign-in'
              : 'Turn on sign-in'}
          </button>
        </div>
      </Ariakit.Dialog>
      <Ariakit.Dialog
        unmountOnHide
        open={!!removal}
        onClose={() => !busy && setRemoval(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Remove {removal?.name}?</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          {removal?.kind === 'devices'
            ? 'This device will be removed from your account. To reconnect, open the app on that device and sign in again.'
            : removingLastMethod
              ? `${removalKey?.purpose === 'login' ? 'This passkey lets you sign in without your password and also verifies password sign-ins.' : `This ${removingFactor ? 'security key' : 'passkey'} verifies password sign-ins.`} This is your last verification method. Removing it will turn off two-factor authentication and invalidate your recovery codes.`
              : removalKey?.purpose !== 'login'
                ? `This ${removingFactor ? 'security key' : 'passkey'} will no longer verify password sign-ins.${mfa?.enabled ? ' Two-factor authentication stays on with your remaining methods.' : ''}`
                : `You will no longer be able to sign in with this passkey. You can still use your password or another passkey.${mfa?.enabled ? ' Two-factor authentication stays on with your remaining methods.' : ''}`}
        </Ariakit.DialogDescription>
        {removal?.kind === 'passkeys' && (
          <SessionSignOutChoice
            checked={signOutOthers}
            onChange={setSignOutOthers}
          />
        )}
        <div className="dialog-actions">
          <Ariakit.DialogDismiss
            className="button button-outline"
            disabled={busy}
          >
            Cancel
          </Ariakit.DialogDismiss>
          <button
            className="button danger"
            disabled={busy}
            onClick={() => {
              const target = removal
              if (!target) return
              setRemoval(null)
              const work = async () => {
                await removalAction.run({
                  kind: target.kind,
                  id: target.id,
                  signOutOthers,
                })
              }
              if (target.kind === 'passkeys') protectedAction(work, 'Removed.')
              else void action(work, 'Removed.')
            }}
          >
            {removal?.kind === 'passkeys'
              ? removingLastMethod
                ? `Remove ${removingFactor ? 'key' : 'passkey'} and turn off 2FA`
                : removingFactor
                  ? 'Remove security key'
                  : 'Remove passkey'
              : 'Remove device'}
          </button>
        </div>
      </Ariakit.Dialog>
      <Ariakit.Dialog
        unmountOnHide
        open={passwordOpen}
        onClose={() => {
          if (busy) return
          setError('')
          setPasswordOpen(false)
        }}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Change password</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          Update the password you use to sign in.
        </Ariakit.DialogDescription>
        {error && <ErrorFeedback error={error} attempt={errorAttempt} />}
        <ValidatedForm onSubmit={changePassword}>
          <label className="field">
            Current password
            <ValidatedInput
              aria-label="Current password"
              name="current"
              type="password"
              autoComplete="current-password"
              required
            />
          </label>
          <label className="field">
            New password
            <NewPasswordInput aria-label="New password" name="password" />
          </label>
          <SessionSignOutChoice
            checked={signOutOthers}
            onChange={setSignOutOthers}
          />
          <div className="dialog-actions">
            <Ariakit.DialogDismiss
              className="button button-outline"
              disabled={busy}
            >
              Cancel
            </Ariakit.DialogDismiss>
            <button className="button" disabled={busy} type="submit">
              Save password
            </button>
          </div>
        </ValidatedForm>
      </Ariakit.Dialog>
    </div>
  )
}
