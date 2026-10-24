import { QueryClientProvider } from '@tanstack/react-query'
import { createQueryClient } from './lib/query'
import { createRouter } from '@tanstack/react-router'
import { routeTree } from './routeTree.gen'
import { PageError } from './components/PageError'

export function getRouter() {
  const queryClient = createQueryClient()
  return createRouter({
    context: { queryClient },
    Wrap: ({ children }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    ),
    routeTree,
    defaultErrorComponent: PageError,
    scrollRestoration: true,
    scrollRestorationBehavior: 'instant',
    defaultPreload: 'intent',
  })
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
