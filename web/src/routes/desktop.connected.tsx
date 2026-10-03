import { useEffect } from 'react'
import { createFileRoute, Link } from '@tanstack/react-router'
import { Check, ArrowRight } from 'lucide-react'
import { Brand } from '../components/Brand'

export const Route = createFileRoute('/desktop/connected')({
  component: DesktopConnected,
  head: () => ({
    meta: [
      { title: 'Desktop connected · extend.computer' },
      { name: 'robots', content: 'noindex' },
      { name: 'referrer', content: 'no-referrer' },
    ],
  }),
})

function DesktopConnected() {
  useEffect(() => {
    if (location.hash !== '#close') return
    history.replaceState(null, '', location.pathname)
    // Browsers may only allow closing script-opened tabs. Keep the complete
    // success page visible when this tab was opened by the desktop OS instead.
    window.close()
  }, [])

  return (
    <main id="main" className="auth-main desktop-connect-main">
      <div className="auth-box desktop-connected-box">
        <Brand appearance="app" />
        <div className="desktop-connected-icon" aria-hidden="true">
          <Check size={28} strokeWidth={2} />
        </div>
        <h1>You're signed in.</h1>
        <p>
          Return to the extend.computer app to continue. You can safely close
          this tab.
        </p>
        <Link to="/account" className="button primary auth-submit">
          Manage your account <ArrowRight size={16} aria-hidden="true" />
        </Link>
        <Link to="/" className="text-button">
          Back to home
        </Link>
      </div>
    </main>
  )
}
