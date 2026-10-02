import { createFileRoute } from '@tanstack/react-router'
import { EmailFlow } from '../components/EmailFlow'
export const Route = createFileRoute('/recover')({
  component: () => <EmailFlow recovery />,
  head: () => ({ meta: [{ title: 'Password recovery · extend.computer' }] }),
})
