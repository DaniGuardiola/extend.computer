import { createFileRoute } from '@tanstack/react-router'
import { Auth } from '../components/Auth'
export const Route = createFileRoute('/login')({
  component: Auth,
  head: () => ({ meta: [{ title: 'Sign in · extend.computer' }] }),
})
