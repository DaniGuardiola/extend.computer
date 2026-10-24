import { ThemeFooter } from './ThemePicker'
import { DeviceSceneIllustration } from './DeviceSceneIllustration'

export function PageError() {
  return (
    <main id="main" className="center-page">
      <DeviceSceneIllustration scene="error" />
      <h1>Something went wrong.</h1>
      <p>Please refresh and try again.</p>
      <a href="/" className="button primary">
        Back home
      </a>
      <ThemeFooter />
    </main>
  )
}
