import { test } from './browser-fixtures.mjs'
import { jobs, captureVisual } from './account-visual-cases.mjs'
import { compareSnapshot } from './visual-comparison.mjs'

test.beforeAll(() => {
  if (process.platform !== 'linux' || process.arch !== 'arm64')
    throw new Error(
      'Visual baselines require Linux arm64. Run Dockerfile.playwright as described in web/README.md.',
    )
})

for (const job of jobs) {
  const name = `${job.name}-${job.width}-${job.theme}.png`
  test(name, { tag: '@visual' }, async ({ newContext }, testInfo) => {
    const context = await newContext({
      viewport: { width: job.width, height: 1000 },
      colorScheme: job.theme,
      reducedMotion: job.motion ? 'no-preference' : 'reduce',
      locale: 'en-US',
      timezoneId: 'UTC',
      // The loading baseline is the initial SSR shell. Keep this stage stable;
      // account-loading.spec.mjs separately verifies its hydrated lifecycle.
      javaScriptEnabled: job.state !== 'loading',
    })
    const page = await context.newPage()
    const shot = await captureVisual(page, job)
    await compareSnapshot(testInfo, name, shot)
  })
}
