function Rows({
  count,
  session = false,
}: {
  count: number
  session?: boolean
}) {
  return (
    <div
      className="security-list [border-top:1px_solid_var(--line)] mt-[22px]"
      aria-hidden="true"
    >
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="security-row skeleton-row min-h-[77px]">
          <div>
            <span className="skeleton skeleton-icon w-[19px] h-[19px]" />
            <div className="skeleton-detail min-w-0">
              <span className="skeleton skeleton-label w-[150px] h-[17px]" />
              <span className="skeleton skeleton-description [width:min(270px,_44vw)] h-[12px] mt-2" />
              {session && (
                <>
                  <span className="skeleton skeleton-description [width:min(270px,_44vw)] h-[12px] mt-2" />
                  <span className="skeleton skeleton-description [width:min(270px,_44vw)] h-[12px] mt-2" />
                </>
              )}
            </div>
          </div>
          <span className="skeleton skeleton-action w-[65px] h-[28px]" />
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
      <div
        className="device-list mt-[26px] mr-0 mb-[50px] ml-0 [border:1px_solid_var(--line)] rounded-[7px] overflow-hidden"
        aria-hidden="true"
      >
        {Array.from({ length: 2 }, (_, index) => (
          <div
            key={index}
            className="device-row skeleton-device-row flex items-center p-[23px] [border-top:1px_solid_var(--line)] gap-5"
          >
            <span className="device-icon skeleton w-[46px] h-[46px] bg-app-panel [border:1px_solid_var(--line)] rounded-[6px] grid place-items-center text-app-device shrink-0" />
            <div className="device-detail flex-1 min-w-0">
              <span className="skeleton skeleton-label w-[150px] h-[17px]" />
              <span className="skeleton skeleton-description [width:min(270px,_44vw)] h-[12px] mt-2" />
            </div>
            <span className="device-status skeleton skeleton-device-status text-[11px] inline-flex gap-[7px] items-center text-app-muted w-[55px] h-[12px] shrink-0" />
            <span className="skeleton skeleton-device-remove w-[18px] h-[18px] shrink-0" />
          </div>
        ))}
      </div>
      <section aria-label="Sign-in methods">
        <div className="security-heading">
          <div>
            <h2>Sign-in methods</h2>
            <p>Manage how you sign in.</p>
          </div>
        </div>
        <Rows count={2} />
      </section>
      <section className="mfa-section" aria-label="Two-factor authentication">
        <div className="security-heading">
          <div>
            <h2>Two-factor authentication</h2>
            <p>Manage your sign-in security.</p>
          </div>
          <span
            className="skeleton skeleton-status w-[24px] h-[17px] self-start"
            aria-hidden="true"
          />
        </div>
        <Rows count={3} />
      </section>
      <section aria-label="Passkeys and security keys">
        <div className="security-heading">
          <div>
            <h2>Passkeys and security keys</h2>
            <p>Manage your keys and how you use them.</p>
          </div>
        </div>
        <Rows count={2} />
        <div
          className="credential-add-actions flex flex-wrap [gap:8px_20px] mt-3"
          aria-hidden="true"
        >
          <span className="skeleton skeleton-action w-[65px] h-[28px]" />
        </div>
      </section>
      <section className="sessions-section" aria-label="Sessions">
        <div className="security-heading">
          <div>
            <h2>Sessions</h2>
            <p>Manage where you’re signed in.</p>
          </div>
          <span
            className="skeleton skeleton-action w-[65px] h-[28px]"
            aria-hidden="true"
          />
        </div>
        <Rows count={1} session />
      </section>
    </div>
  )
}
