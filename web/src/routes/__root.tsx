import type { QueryClient } from '@tanstack/react-query'
import { ThemeFooter } from '../components/ThemePicker'
import { DeviceSceneIllustration } from '../components/DeviceSceneIllustration'
import { PageError } from '../components/PageError'
import {
  createRootRouteWithContext,
  HeadContent,
  Outlet,
  Scripts,
  Link,
} from '@tanstack/react-router'
import styles from '../styles.css?url'

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()(
  {
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
      <html lang="en" suppressHydrationWarning>
        <head>
          <script
            dangerouslySetInnerHTML={{
              __html:
                "try{var t=localStorage.getItem('extend-theme');if(t==='light'||t==='dark'||t==='system')document.documentElement.dataset.theme=t}catch(e){}",
            }}
          />
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
        <DeviceSceneIllustration />
        <p className="eyebrow">404 / WRONG SCREEN</p>
        <h1>Nothing over here.</h1>
        <Link to="/" className="button primary">
          Back home
        </Link>
        <ThemeFooter />
      </main>
    ),
    errorComponent: PageError,
  },
)
