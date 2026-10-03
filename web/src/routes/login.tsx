import { createFileRoute } from '@tanstack/react-router'
import { Auth } from '../components/Auth'
export const Route = createFileRoute('/login')({
  validateSearch: (
    search: Record<string, unknown>,
  ): { preview?: 'signing-in' } => ({
    preview:
      import.meta.env.DEV && search.preview === 'signing-in'
        ? 'signing-in'
        : undefined,
  }),
  component: Login,
  head: () => ({ meta: [{ title: 'Sign in · extend.computer' }] }),
})

function Login() {
  const { preview } = Route.useSearch()
  return (
    <Auth key={preview ?? 'login'} previewLoading={preview === 'signing-in'} />
  )
}
