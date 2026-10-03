import { useEffect, useState } from 'react'
import * as Ariakit from '@ariakit/react'
import { Monitor } from 'lucide-react'
import { api, message } from '../lib/api'

export type AccountSession = {
  id: string
  created_at: number
  last_access_at: number
  platform: string | null
  client: string | null
  location: string | null
  current: number
}

export function Sessions({
  revision,
  initialData,
  onSignedOut,
}: {
  revision: string
  initialData?: AccountSession[]
  onSignedOut: () => Promise<void>
}) {
  const [sessions, setSessions] = useState<AccountSession[] | null>(
    initialData ?? null,
  )
  const [target, setTarget] = useState<AccountSession | 'all' | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  useEffect(() => {
    if (initialData) {
      setSessions(initialData)
      setError('')
      return
    }
    let active = true
    api<{ sessions: AccountSession[] }>('/sessions')
      .then((result) => {
        if (active) {
          setSessions(result.sessions)
          setError('')
        }
      })
      .catch((e) => {
        if (active) setError(message(e))
      })
    return () => {
      active = false
    }
  }, [revision, initialData])
  async function revoke() {
    if (!target) return
    setBusy(true)
    setError('')
    setNotice('')
    try {
      await api(
        target === 'all' ? '/sessions' : '/sessions/' + target.id,
        'DELETE',
      )
      setTarget(null)
      if (target === 'all' || target.current) await onSignedOut()
      else {
        setSessions(
          (await api<{ sessions: AccountSession[] }>('/sessions')).sessions,
        )
        setNotice('Session signed out.')
      }
    } catch (e) {
      setError(message(e))
    } finally {
      setBusy(false)
    }
  }
  const date = (value: number) =>
    new Date(value * 1000).toLocaleString(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    })
  return (
    <section className="sessions-section" aria-labelledby="sessions-title">
      <div className="security-heading">
        <div>
          <h2 id="sessions-title">Sessions</h2>
          <p>Manage where you’re signed in. Locations are approximate.</p>
        </div>
        <button
          className="text-button"
          disabled={busy || !sessions?.length}
          onClick={() => {
            setError('')
            setTarget('all')
          }}
        >
          Sign out all
        </button>
      </div>
      {error && !target && (
        <p role="alert" className="message error-message">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="message success-message">
          {notice}
        </p>
      )}
      {!sessions ? (
        <p role="status">
          {error
            ? 'Could not load sessions. Try refreshing.'
            : 'Loading sessions…'}
        </p>
      ) : (
        <div className="security-list">
          {sessions.map((session) => (
            <article key={session.id} className="security-row session-row">
              <div>
                <Monitor size={19} />
                <div>
                  <h3>
                    {[session.client ?? 'Session', session.platform]
                      .filter(Boolean)
                      .join(' · ')}{' '}
                    {session.current ? (
                      <span className="session-current">This session</span>
                    ) : null}
                  </h3>
                  <p>{session.location ?? 'Location unavailable'}</p>
                  <p>
                    Last active{' '}
                    <time
                      dateTime={new Date(
                        session.last_access_at * 1000,
                      ).toISOString()}
                    >
                      {date(session.last_access_at)}
                    </time>
                  </p>
                  <p>
                    Signed in{' '}
                    <time
                      dateTime={new Date(
                        session.created_at * 1000,
                      ).toISOString()}
                    >
                      {date(session.created_at)}
                    </time>
                  </p>
                </div>
              </div>
              <button
                className="text-button"
                aria-label={`Sign out ${session.current ? 'this session' : [session.client ?? 'session', session.platform].filter(Boolean).join(' on ')}`}
                disabled={busy}
                onClick={() => {
                  setError('')
                  setTarget(session)
                }}
              >
                Sign out
              </button>
            </article>
          ))}
        </div>
      )}
      <Ariakit.Dialog
        open={!!target}
        onClose={() => !busy && setTarget(null)}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>
          {target === 'all'
            ? 'Sign out all sessions?'
            : 'Sign out this session?'}
        </Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          {target === 'all'
            ? 'You’ll be signed out everywhere, including this browser. Devices will need to sign in again.'
            : target?.current
              ? 'You’ll be signed out of this browser.'
              : 'This session and its connected devices will need to sign in again.'}
        </Ariakit.DialogDescription>
        {error && (
          <p role="alert" className="message error-message">
            {error}
          </p>
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
            onClick={() => void revoke()}
          >
            {busy ? 'Signing out…' : 'Sign out'}
          </button>
        </div>
      </Ariakit.Dialog>
    </section>
  )
}
