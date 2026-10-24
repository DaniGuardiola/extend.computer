// Local physical Android account checks. Never used by the production build.
import { defineConfig } from 'vite'
import { cloudflare } from '@cloudflare/vite-plugin'
import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import react, { reactCompilerPreset } from '@vitejs/plugin-react'
import babel from '@rolldown/plugin-babel'
import tailwindcss from '@tailwindcss/vite'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const state = process.env.EXTEND_ANDROID_ACCOUNT_FIXTURE
if (!state) throw new Error('Prepare an isolated Android account fixture first.')
const root = resolve(state)
const fixture = JSON.parse(readFileSync(join(root, 'fixture.json'), 'utf8'))
const origin = new URL(fixture.origin)
if (fixture.version !== 1 || origin.hostname !== '127.0.0.1' || origin.protocol !== 'http:')
  throw new Error('Physical account fixtures require a loopback HTTP origin.')

export default defineConfig({
  plugins: [
    cloudflare({
      configPath: join(root, 'wrangler.json'),
      persistState: { path: join(root, 'state') },
      viteEnvironment: { name: 'ssr' },
      remoteBindings: false,
      inspectorPort: false,
      tunnel: false,
    }),
    tailwindcss(),
    tanstackStart(),
    react(),
    babel({ presets: [reactCompilerPreset()] }),
  ],
  server: { host: '127.0.0.1', port: Number(origin.port), strictPort: true },
})
