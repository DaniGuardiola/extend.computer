import { useRevokeSession } from '../lib/mutations'
import { ErrorFeedback, useErrorFeedback } from './ErrorFeedback'
import { useState } from 'react'
import * as Ariakit from '@ariakit/react'
import { Globe, Monitor } from 'lucide-react'
import { message } from '../lib/api'

import type { AccountSession } from '../lib/contracts'
export type { AccountSession } from '../lib/contracts'

export function Sessions({
  sessions,
  onSignedOut,
}: {
  sessions: AccountSession[]
  onSignedOut: () => Promise<void>
}) {
  const operation = useRevokeSession()
  const [target, setTarget] = useState<AccountSession | 'all' | null>(null)
  const busy = operation.isPending
  const [error, setError, errorAttempt] = useErrorFeedback()
  const [notice, setNotice] = useState('')
  async function revoke() {
    if (!target) return
    setNotice('')
    try {
      const signedOut = await operation.run(target)
      setTarget(null)
      setError('')
      if (signedOut) await onSignedOut()
      else setNotice('Session signed out.')
    } catch (e) {
      setError(message(e))
    }
  }
  const targetSession = target && target !== 'all' ? target : null
  function clientLabel(session: AccountSession) {
    return session.client === 'Desktop app'
      ? session.platform
        ? `${session.platform} app`
        : 'Desktop app'
      : (session.client ?? 'Session')
  }
  function sessionName(session: AccountSession) {
    const client =
      session.client === 'Desktop app'
        ? `the ${clientLabel(session)}`
        : (session.client ?? 'this session')
    const devices = session.device_names?.length
      ? new Intl.ListFormat('en', { type: 'conjunction' }).format(
          session.device_names,
        )
      : null
    const platform = session.client === 'Desktop app' ? null : session.platform
    return client + (devices || platform ? ` on ${devices ?? platform}` : '')
  }
  const targetName = targetSession ? sessionName(targetSession) : 'this session'
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
          <p>Manage where you’re signed in.</p>
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
        <ErrorFeedback error={error} attempt={errorAttempt} />
      )}
      {notice && (
        <p role="status" className="message success-message">
          {notice}
        </p>
      )}
      <div className="security-list [border-top:1px_solid_var(--line)] mt-[22px]">
        {[...sessions]
          .sort(
            (a, b) =>
              b.current - a.current || b.last_access_at - a.last_access_at,
          )
          .map((session) => (
            <article key={session.id} className="security-row session-row">
              <div>
                {session.client === 'Desktop app' ? (
                  <Monitor size={19} />
                ) : (
                  <Globe size={19} />
                )}
                <div>
                  <h3>
                    {[
                      clientLabel(session),
                      session.device_names?.length
                        ? session.device_names.join(', ')
                        : session.client === 'Desktop app'
                          ? null
                          : session.platform,
                    ]
                      .filter(Boolean)
                      .join(' · ')}{' '}
                    {session.current ? (
                      <span className="session-current">This session</span>
                    ) : null}
                    {!session.current &&
                      Date.now() / 1000 - session.last_access_at >=
                        14 * 86400 && (
                        <span className="session-current session-inactive">
                          Inactive for{' '}
                          {Math.floor(
                            (Date.now() / 1000 - session.last_access_at) /
                              86400,
                          )}{' '}
                          days
                        </span>
                      )}
                  </h3>
                  <p>
                    <Ariakit.TooltipProvider showTimeout={0} placement="left">
                      <Ariakit.TooltipAnchor
                        render={<span tabIndex={0} />}
                        className="session-location"
                      >
                        {session.location ?? 'Location unavailable'}
                      </Ariakit.TooltipAnchor>
                      <Ariakit.Tooltip
                        className="account-tooltip"
                        flip="right top bottom"
                        overflowPadding={16}
                      >
                        <Ariakit.TooltipArrow />
                        {session.location
                          ? 'Estimated from this session’s IP address. VPNs and proxies can make it inaccurate.'
                          : 'No location information was available for this session.'}
                      </Ariakit.Tooltip>
                    </Ariakit.TooltipProvider>
                  </p>
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
                aria-label={`Sign out ${session.current ? 'this session' : sessionName(session)}`}
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
      <Ariakit.Dialog
        unmountOnHide
        open={!!target}
        onClose={() => {
          if (busy) return
          setError('')
          setTarget(null)
        }}
        className="dialog"
        backdrop={<div className="dialog-backdrop" />}
      >
        <Ariakit.DialogHeading>
          {target === 'all'
            ? 'Sign out all sessions?'
            : target?.current
              ? 'Sign out this session?'
              : `Sign out ${targetName}?`}
        </Ariakit.DialogHeading>
        <Ariakit.DialogDescription>
          {target === 'all'
            ? 'You’ll be signed out everywhere, including this browser. Devices will need to sign in again.'
            : target?.current
              ? 'You’ll be signed out of this browser.'
              : `You’ll be signed out of ${targetName}.`}
        </Ariakit.DialogDescription>
        {error && <ErrorFeedback error={error} attempt={errorAttempt} />}
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
