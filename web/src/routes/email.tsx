import { createFileRoute } from '@tanstack/react-router'
import { EmailFlow } from '../components/EmailFlow'
export const Route = createFileRoute('/email')({
  component: EmailFlow,
  head: () => ({ meta: [{ title: 'Your account · extend.computer' }] }),
})
