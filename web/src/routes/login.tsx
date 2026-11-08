import { createFileRoute } from '@tanstack/react-router'
import { Auth } from '../components/Auth'
import { useGuestRoute } from '../lib/guest-route'
export const Route = createFileRoute('/login')({
  validateSearch: (
    search: Record<string, unknown>,
  ): { reason?: 'session-ended' | 'account-deleted' } => ({
    reason:
      search.reason === 'session-ended' || search.reason === 'account-deleted'
        ? search.reason
        : undefined,
  }),
  component: Login,
  head: () => ({ meta: [{ title: 'Sign in · extend.computer' }] }),
})

function Login() {
  const { reason } = Route.useSearch()
  const ready = useGuestRoute()
  if (!ready) return null
  return (
    <Auth
      sessionEnded={reason === 'session-ended'}
      accountDeleted={reason === 'account-deleted'}
    />
  )
}
