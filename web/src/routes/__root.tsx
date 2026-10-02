import {
  createRootRoute,
  HeadContent,
  Outlet,
  Scripts,
  Link,
} from '@tanstack/react-router'
import styles from '../styles.css?url'

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      { title: 'extend.computer — One desk. Zero borders.' },
      {
        name: 'description',
        content:
          'Your screens, keyboard and mouse, across your devices. Extend displays, share input, mirror screens and access another workspace. Starting on macOS.',
      },
      { name: 'theme-color', content: '#141714' },
      { property: 'og:title', content: 'One desk. Zero borders.' },
      {
        property: 'og:description',
        content:
          'Your screens, keyboard and mouse, across your devices. Meet extend.computer.',
      },
      { property: 'og:type', content: 'website' },
    ],
    links: [
      { rel: 'stylesheet', href: styles },
      { rel: 'icon', href: '/favicon.svg', type: 'image/svg+xml' },
    ],
  }),
  component: () => (
    <html lang="en">
      <head>
        <HeadContent />
      </head>
      <body>
        <a className="skip-link" href="#main">
          Skip to content
        </a>
        <Outlet />
        <Scripts />
      </body>
    </html>
  ),
  notFoundComponent: () => (
    <main id="main" className="center-page">
      <p className="eyebrow">404 / WRONG SCREEN</p>
      <h1>Nothing over here.</h1>
      <Link to="/" className="button primary">
        Back home
      </Link>
    </main>
  ),
  errorComponent: () => (
    <main id="main" className="center-page">
      <h1>Something got disconnected.</h1>
      <p>Please refresh and try again.</p>
      <a href="/" className="button primary">
        Back home
      </a>
    </main>
  ),
})
