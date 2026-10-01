import { createFileRoute } from '@tanstack/react-router'
import { Auth } from '../components/Auth'
export const Route = createFileRoute('/signup')({
  component: () => <Auth signup />,
  head: () => ({ meta: [{ title: 'Create account · extend.computer' }] }),
})
