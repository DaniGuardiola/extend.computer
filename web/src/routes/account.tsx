import { useEffect, useState, type FormEvent } from 'react'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import * as Ariakit from '@ariakit/react'
import {
  Fingerprint,
  KeyRound,
  Laptop,
  LogOut,
  Monitor,
  Plus,
  RefreshCw,
  Shield,
  Trash2,
} from 'lucide-react'
import type { PublicKeyCredentialCreationOptionsJSON } from '@simplewebauthn/browser'
import { Brand } from '../components/Brand'
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
  const navigate = useNavigate()
  const [account, setAccount] = useState<Account | null>(null)
  const [devices, setDevices] = useState<Device[]>([])
  const [passkeys, setPasskeys] = useState<Passkey[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState('')
  const [removal, setRemoval] = useState<Removal | null>(null)
  const [passwordOpen, setPasswordOpen] = useState(false)
  const [sessionsOpen, setSessionsOpen] = useState(false)
  async function refresh() {
    const [user, registered, keys] = await Promise.all([
      api<Account>('/account'),
      api<{ devices: Device[] }>('/devices'),
      api<{ passkeys: Passkey[] }>('/passkeys'),
    ])
    setAccount(user)
    setDevices(registered.devices)
    setPasskeys(keys.passkeys)
  }
  useEffect(() => {
    let active = true
    Promise.all([
      api<Account>('/account'),
      api<{ devices: Device[] }>('/devices'),
      api<{ passkeys: Passkey[] }>('/passkeys'),
    ])
      .then(([user, registered, keys]) => {
        if (active) {
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
  async function addPasskey() {
    await action(async () => {
      const { startRegistration } = await import('@simplewebauthn/browser')
      const optionsJSON = await api<PublicKeyCredentialCreationOptionsJSON>(
        '/passkeys/register/options',
        'POST',
      )
      await api(
        '/passkeys/register/verify',
        'POST',
        await startRegistration({ optionsJSON }),
      )
      await refresh()
    }, 'Passkey added. Next time, sign in with a touch.')
  }
  async function changePassword(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    await action(async () => {
      await api('/account/password', 'POST', {
        current_password: data.get('current'),
        password: data.get('password'),
      })
      setPasswordOpen(false)
      await refresh()
    }, 'Password changed. Other sessions signed out.')
  }
  return (
    <div className="dashboard">
      <header className="site-header">
        <Brand />
        <div className="account-nav">
          <span className="account-email">{account?.email}</span>
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
            <span className="eyebrow">YOUR CORNER OF THE INTERNET</span>
            <h1>Your devices.</h1>
            <p>One account. All your computers.</p>
          </div>
          <button
            className="text-button"
            disabled={busy}
            onClick={() => action(refresh)}
          >
            <RefreshCw size={14} />
            Refresh
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
          <p role="status">
            {error
              ? 'Could not load your account. Try refreshing.'
              : 'Loading your desk…'}
          </p>
        ) : (
          <>
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
                  <h2>Your desk starts here.</h2>
                  <p>
                    No devices registered yet. Desktop account sign-in is coming
                    next. For now, the desktop app connects directly on your
                    local network.
                  </p>
                </div>
              )}
            </section>
            <section aria-labelledby="security-title">
              <div className="security-heading">
                <div>
                  <h2 id="security-title">Make it yours. Keep it yours.</h2>
                  <p>Manage how you sign in.</p>
                </div>
                <button
                  className="button button-outline"
                  disabled={busy || passkeys.length >= 10}
                  onClick={addPasskey}
                >
                  <Plus size={14} />
                  Add passkey
                </button>
              </div>
              <div className="security-list">
                {passkeys.map((key) => (
                  <div key={key.id} className="security-row">
                    <div>
                      <Fingerprint size={19} />
                      <div>
                        <h3>{key.name}</h3>
                        <p>
                          Added{' '}
                          {new Date(key.created_at * 1000).toLocaleDateString()}{' '}
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
                        <h3>A touch beats a password.</h3>
                        <p>
                          Add a passkey to sign in with your fingerprint, face,
                          or security key.
                        </p>
                      </div>
                    </div>
                  </div>
                )}
                <div className="security-row">
                  <div>
                    <KeyRound size={19} />
                    <div>
                      <h3>Password</h3>
                      <p>Keep a password as another way in.</p>
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
                <div className="security-row">
                  <div>
                    <Shield size={19} />
                    <div>
                      <h3>Other sessions</h3>
                      <p>
                        Sign out everywhere else. Those devices will go offline.
                      </p>
                    </div>
                  </div>
                  <button
                    className="text-button"
                    disabled={busy}
                    onClick={() => setSessionsOpen(true)}
                  >
                    Sign out
                  </button>
                </div>
              </div>
            </section>
          </>
        )}
      </main>
      <footer className="dashboard-footer">
        <span>Your devices. Your control.</span>
        <a href="/">Back to home ↗</a>
      </footer>
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
            onClick={() =>
              action(async () => {
                if (!removal) return
                await api(
                  '/' + removal.kind + '/' + encodeURIComponent(removal.id),
                  'DELETE',
                )
                setRemoval(null)
                await refresh()
              }, 'Removed.')
            }
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
          Other sessions will be signed out.
        </Ariakit.DialogDescription>
        {error && (
          <p role="alert" className="message error-message">
            {error}
          </p>
        )}
        <form onSubmit={changePassword}>
          <label className="field">
            Current password
            <input
              name="current"
              type="password"
              autoComplete="current-password"
              required
            />
          </label>
          <label className="field">
            New password
            <input
              name="password"
              type="password"
              autoComplete="new-password"
              minLength={12}
              maxLength={1024}
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
            <button className="button" disabled={busy} type="submit">
              Save password
            </button>
          </div>
        </form>
      </Ariakit.Dialog>
      <Ariakit.Dialog
        open={sessionsOpen}
        onClose={() => !busy && setSessionsOpen(false)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>Sign out other sessions?</Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          This session stays signed in. Devices using other sessions will go
          offline until they sign in again.
        </Ariakit.DialogDescription>
        <div className="dialog-actions">
          <Ariakit.DialogDismiss
            className="button button-outline"
            disabled={busy}
          >
            Cancel
          </Ariakit.DialogDismiss>
          <button
            className="button"
            disabled={busy}
            onClick={() =>
              action(async () => {
                await api('/sessions/others', 'DELETE')
                setSessionsOpen(false)
                await refresh()
              }, 'Other sessions signed out.')
            }
          >
            Sign out
          </button>
        </div>
      </Ariakit.Dialog>
    </div>
  )
}
