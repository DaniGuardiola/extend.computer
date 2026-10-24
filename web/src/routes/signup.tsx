import { createFileRoute } from '@tanstack/react-router'
import { Auth } from '../components/Auth'
import { useGuestRoute } from '../lib/guest-route'
export const Route = createFileRoute('/signup')({
  component: Signup,
  head: () => ({ meta: [{ title: 'Create account · extend.computer' }] }),
})

function Signup() {
  const ready = useGuestRoute()
  if (!ready) return null
  return <Auth signup />
}
