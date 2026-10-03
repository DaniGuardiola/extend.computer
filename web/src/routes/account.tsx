import { useEffect, useRef, useState, type FormEvent } from 'react'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import * as Ariakit from '@ariakit/react'
import {
  Fingerprint,
  KeyRound,
  Laptop,
  LogOut,
  Monitor,
  RefreshCw,
  Trash2,
} from 'lucide-react'
import type { PublicKeyCredentialCreationOptionsJSON } from '@simplewebauthn/browser'
import { AccountSkeleton } from '../components/AccountSkeleton'
import { MfaControls, type MfaInfo } from '../components/MfaControls'
import { Sessions, type AccountSession } from '../components/Sessions'
import { VerifySecurity } from '../components/VerifySecurity'
import { SessionSignOutChoice } from '../components/SessionSignOutChoice'
import { Brand } from '../components/Brand'
import { ValidatedForm, ValidatedInput } from '../components/ValidatedForm'
import {
  api,
  ApiError,
  message,
  type Account,
  type Device,
  type Passkey,
} from '../lib/api'
export const Route = createFileRoute('/account')({
  component: Dashboard,
  head: () => ({ meta: [{ title: 'Your devices · extend.computer' }] }),
})
type Removal = { kind: 'devices' | 'passkeys'; id: string; name: string }
function Dashboard() {
  const [verification, setVerification] = useState<
    (() => Promise<void>) | null
  >(null)
  const navigate = useNavigate()
  const [account, setAccount] = useState<Account | null>(null)
  const [devices, setDevices] = useState<Device[]>([])
  const [passkeys, setPasskeys] = useState<Passkey[]>([])
  const [busy, setBusy] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState('')
  const [removal, setRemoval] = useState<Removal | null>(null)
  const [passwordOpen, setPasswordOpen] = useState(false)
  const [signOutOthers, setSignOutOthers] = useState(false)
  const [sessionsRevision, setSessionsRevision] = useState(0)
  const [mfa, setMfa] = useState<MfaInfo>()
  const [sessions, setSessions] = useState<AccountSession[]>()
  const [loading, setLoading] = useState(true)
  async function loadAccount() {
    return Promise.all([
      api<Account>('/account'),
      api<{ devices: Device[] }>('/devices'),
      api<{ passkeys: Passkey[] }>('/passkeys'),
      api<MfaInfo>('/mfa'),
      api<{ sessions: AccountSession[] }>('/sessions'),
    ])
  }
  const initialLoad = useRef<ReturnType<typeof loadAccount> | null>(null)
  async function refresh() {
    const [user, registered, keys, factors, loggedIn] = await loadAccount()
    setMfa(factors)
    setSessions(loggedIn.sessions)
    setAccount(user)
    setDevices(registered.devices)
    setPasskeys(keys.passkeys)
    setSessionsRevision((value) => value + 1)
  }
  useEffect(() => {
    let active = true
    const request = (initialLoad.current ??= loadAccount())
    request
      .then(([user, registered, keys, factors, loggedIn]) => {
        if (active) {
          setMfa(factors)
          setSessions(loggedIn.sessions)
          setAccount(user)
          setDevices(registered.devices)
          setPasskeys(keys.passkeys)
        }
      })
      .catch((error) => {
        if (active) {
          if (error instanceof ApiError && error.status === 401)
            void navigate({ to: '/login' })
          else setError(message(error))
        }
      })
      .finally(() => {
        if (active) setLoading(false)
      })
    return () => {
      active = false
    }
  }, [navigate])
  async function action(work: () => Promise<void>, notice = '') {
    setBusy(true)
    setError('')
    setSuccess('')
    try {
      await work()
      setSuccess(notice)
    } catch (error) {
      setError(message(error))
    } finally {
      setBusy(false)
    }
  }
  function verify(work: () => Promise<void>) {
    setVerification(() => work)
  }
  function protectedAction(work: () => Promise<void>, notice = '') {
    if (account?.mfa_enabled) verify(() => action(work, notice))
    else void action(work, notice)
  }
  async function addPasskey() {
    protectedAction(async () => {
      const { startRegistration } = await import('@simplewebauthn/browser')
      const optionsJSON = await api<PublicKeyCredentialCreationOptionsJSON>(
        '/passkeys/register/options',
        'POST',
        {},
      )
      await api(
        '/passkeys/register/verify',
        'POST',
        await startRegistration({ optionsJSON }),
      )
      await refresh()
    }, 'Passkey added.')
  }
  useEffect(() => setSignOutOthers(false), [passwordOpen, removal])
  async function changePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    protectedAction(
      async () => {
        await api('/account/password', 'POST', {
          current_password: data.get('current'),
          password: data.get('password'),
          sign_out_others: signOutOthers,
        })
        setPasswordOpen(false)
        await refresh()
      },
      signOutOthers
        ? 'Password changed. Other sessions signed out.'
        : 'Password changed.',
    )
  }
  return (
    <div className="dashboard">
      <header className="site-header">
        <Brand appearance="app" />
        <div className="account-nav">
          <span className="account-email">
            {account ? (
              account.email
            ) : (
              <span className="account-loading-inline" aria-hidden="true">
                <span className="skeleton skeleton-email" />
              </span>
            )}
          </span>
          <button
            className="text-button"
            disabled={busy || !account}
            onClick={() =>
              action(async () => {
                await api('/auth/logout', 'POST')
                await navigate({ to: '/login' })
              })
            }
          >
            <LogOut size={14} />
            Sign out
          </button>
        </div>
      </header>
      <main id="main" className="dashboard-main">
        <div className="dashboard-title">
          <div>
            <h1>Devices</h1>
            <p>Manage your registered computers and account settings.</p>
          </div>
          <button
            className="text-button"
            disabled={busy || loading}
            onClick={() =>
              action(async () => {
                setRefreshing(true)
                try {
                  await refresh()
                } finally {
                  setRefreshing(false)
                }
              })
            }
          >
            <RefreshCw size={14} />
            {refreshing ? 'Refreshing…' : 'Refresh'}
          </button>
        </div>
        {error && (
          <p className="message error-message" role="alert">
            {error}
          </p>
        )}
        {success && (
          <p className="message success-message" role="status">
            {success}
          </p>
        )}
        {!account ? (
          <AccountSkeleton loading={loading} />
        ) : (
          <div className="account-content">
            {account.email_enabled && !account.email_verified ? (
              <section className="email-notice" aria-label="Verify email">
                <div>
                  <h2>Verify your email</h2>
                  <p>Verify {account.email} to enable password recovery.</p>
                </div>
                <button
                  className="button button-outline"
                  disabled={busy}
                  onClick={() =>
                    action(async () => {
                      await api('/auth/email/request', 'POST')
                    }, 'Verification email requested. Check your inbox.')
                  }
                >
                  Send verification email
                </button>
              </section>
            ) : null}
            <section aria-label="Registered devices" className="device-list">
              {devices.length ? (
                devices.map((device) => (
                  <article key={device.id} className="device-row">
                    <div className="device-icon">
                      {device.platform === 'macos' ? (
                        <Laptop size={22} />
                      ) : (
                        <Monitor size={22} />
                      )}
                    </div>
                    <div className="device-detail">
                      <h3>{device.name}</h3>
                      <p>
                        {device.platform} · {device.fingerprint.slice(0, 12)}
                      </p>
                    </div>
                    <span
                      className={
                        'device-status' + (device.online ? ' online' : '')
                      }
                    >
                      <i
                        className={device.online ? 'status-dot' : 'offline-dot'}
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
                <div className="empty-devices">
                  <Monitor size={31} />
                  <h2>No devices registered</h2>
                  <p>
                    Open the desktop app and sign in to add this computer. Pair
                    devices on your local network before connecting.
                  </p>
                </div>
              )}
            </section>
            <MfaControls
              initialData={mfa}
              verify={verify}
              onChange={refresh}
              revision={`${account.mfa_enabled}:${passkeys.length}`}
            />
            <section aria-labelledby="security-title">
              <div className="security-heading">
                <div>
                  <h2 id="security-title">Sign-in methods</h2>
                  <p>Manage how you sign in.</p>
                </div>
              </div>
              <div className="security-list">
                <div
                  className="passkey-method"
                  role="group"
                  aria-label="Passkeys"
                >
                  {passkeys.map((key) => (
                    <div key={key.id} className="security-row">
                      <div>
                        <Fingerprint size={19} />
                        <div>
                          <h3>{key.name}</h3>
                          <p>
                            {key.purpose === 'factor' ? 'Second factor · ' : ''}
                            Added{' '}
                            {new Date(
                              key.created_at * 1000,
                            ).toLocaleDateString()}{' '}
                            · {key.rp_id}
                          </p>
                        </div>
                      </div>
                      <button
                        className="icon-button"
                        aria-label="Remove passkey"
                        disabled={busy}
                        onClick={() =>
                          setRemoval({
                            kind: 'passkeys',
                            id: key.id,
                            name: 'this passkey',
                          })
                        }
                      >
                        <Trash2 size={15} />
                      </button>
                    </div>
                  ))}
                  {!passkeys.length && (
                    <div className="security-row">
                      <div>
                        <Fingerprint size={19} />
                        <div>
                          <h3>No passkeys added</h3>
                          <p>
                            Add a passkey to sign in with your fingerprint,
                            face, or security key.
                          </p>
                        </div>
                      </div>
                    </div>
                  )}
                  <div className="passkey-add-action">
                    <button
                      className="text-button"
                      disabled={busy || passkeys.length >= 10}
                      onClick={addPasskey}
                    >
                      Add passkey
                    </button>
                  </div>
                </div>
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
                    onClick={() => setPasswordOpen(true)}
                  >
                    Change
                  </button>
                </div>
              </div>
            </section>
            <Sessions
              initialData={sessions}
              revision={`${account.mfa_enabled}:${sessionsRevision}`}
              onSignedOut={() => navigate({ to: '/login' })}
            />
          </div>
        )}
      </main>
      <footer className="dashboard-footer">
        <span>extend.computer</span>
        <a href="/">Back to home ↗</a>
      </footer>
      {verification && (
        <VerifySecurity
          onVerified={verification}
          onClose={() => setVerification(null)}
        />
      )}
      <Ariakit.Dialog
        open={!!removal}
        onClose={() => !busy && setRemoval(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Remove {removal?.name}?</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          {removal?.kind === 'devices'
            ? 'This device will stop appearing in your account. Its account connection will be revoked.'
            : 'You can still sign in with your password or another passkey.'}
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
                await api(
                  '/' + target.kind + '/' + encodeURIComponent(target.id),
                  'DELETE',
                  target.kind === 'passkeys'
                    ? { sign_out_others: signOutOthers }
                    : undefined,
                )
                await refresh()
              }
              if (target.kind === 'passkeys') protectedAction(work, 'Removed.')
              else void action(work, 'Removed.')
            }}
          >
            Remove
          </button>
        </div>
      </Ariakit.Dialog>
      <Ariakit.Dialog
        open={passwordOpen}
        onClose={() => !busy && setPasswordOpen(false)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Change password</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          Update the password you use to sign in.
        </Ariakit.DialogDescription>
        {error && (
          <p role="alert" className="message error-message">
            {error}
          </p>
        )}
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
            <ValidatedInput
              aria-label="New password"
              name="password"
              type="password"
              autoComplete="new-password"
              minLength={12}
              maxLength={1024}
              required
            />
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
