import { defineConfig } from '@playwright/test'

const origin = new URL(process.env.EXTEND_TEST_URL ?? 'http://localhost:3026')
if (
  !['localhost', '127.0.0.1'].includes(origin.hostname) ||
  origin.pathname !== '/'
)
  throw new Error('Tests require a local server origin')
process.env.EXTEND_TEST_URL = origin.origin

export default defineConfig({
  testDir: './tests',
  testMatch: '**/*.spec.mjs',
  timeout: 240_000,
  expect: { timeout: 10_000 },
  workers: 2,
  fullyParallel: true,
  retries: 0,
  updateSnapshots: 'none',
  reporter: [['list'], ['html', { open: 'never' }]],
  outputDir: './test-results/playwright',
  use: {
    baseURL: origin.origin,
    // Use the same software paint path and grayscale text in local containers
    // and CI. LCD/GPU repaint artifacts otherwise change isolated pixels.
    launchOptions: { args: ['--disable-gpu', '--disable-lcd-text'] },
    trace: 'retain-on-failure',
    actionTimeout: 15_000,
    navigationTimeout: 30_000,
  },
  webServer: {
    command: `npm run dev -- --port ${origin.port || '80'} --strictPort`,
    url: origin.origin,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
})
