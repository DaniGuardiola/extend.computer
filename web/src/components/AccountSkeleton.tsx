function Rows({
  count,
  session = false,
}: {
  count: number
  session?: boolean
}) {
  return (
    <div className="security-list" aria-hidden="true">
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="security-row skeleton-row">
          <div>
            <span className="skeleton skeleton-icon" />
            <div className="skeleton-detail">
              <span className="skeleton skeleton-label" />
              <span className="skeleton skeleton-description" />
              {session && (
                <>
                  <span className="skeleton skeleton-description" />
                  <span className="skeleton skeleton-description" />
                </>
              )}
            </div>
          </div>
          <span className="skeleton skeleton-action" />
        </div>
      ))}
    </div>
  )
}
export function AccountSkeleton({ loading }: { loading: boolean }) {
  return (
    <div className="account-loading" aria-busy={loading}>
      <span className="sr-only" role="status">
        {loading
          ? 'Loading account…'
          : 'Could not load your account. Try refreshing.'}
      </span>
      <div className="device-list skeleton-devices" aria-hidden="true">
        <span className="skeleton skeleton-device-icon" />
        <span className="skeleton skeleton-label" />
        <span className="skeleton skeleton-description" />
        <span className="skeleton skeleton-description" />
      </div>
      <section className="mfa-section" aria-label="Two-factor authentication">
        <div className="security-heading">
          <div>
            <h2>Two-factor authentication</h2>
            <p>Manage your sign-in security.</p>
          </div>
        </div>
        <Rows count={4} />
      </section>
      <section aria-label="Sign-in methods">
        <div className="security-heading">
          <div>
            <h2>Sign-in methods</h2>
            <p>Manage how you sign in.</p>
          </div>
        </div>
        <Rows count={2} />
      </section>
      <section className="sessions-section" aria-label="Sessions">
        <div className="security-heading">
          <div>
            <h2>Sessions</h2>
            <p>Manage where you’re signed in. Locations are approximate.</p>
          </div>
        </div>
        <Rows count={1} session />
      </section>
    </div>
  )
}
