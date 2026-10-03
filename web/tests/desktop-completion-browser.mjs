import { chromium } from 'playwright'
import assert from 'node:assert/strict'

const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3010'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = await chromium.launch({ headless: true })
try {
  const context = await browser.newContext()
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto(origin + '/')
  // An ordinary tab with previous navigation cannot be automatically closed.
  await page.goto(origin + '/desktop/connected#close')
  await page.getByRole('heading', { name: "You're signed in." }).waitFor()
  await page.waitForURL(origin + '/desktop/connected')
  assert.ok(!page.isClosed())
  assert.ok(
    await page.getByText('You can safely close', { exact: false }).isVisible(),
  )
  assert.equal(await page.evaluate(() => location.search + location.hash), '')
  assert.ok(
    await page.getByRole('link', { name: 'Manage your account' }).isVisible(),
  )
  for (const colorScheme of ['light', 'dark']) {
    await page.emulateMedia({ colorScheme })
    const background = await page
      .locator('.desktop-connected-icon')
      .evaluate((node) => getComputedStyle(node).backgroundColor)
    assert.notEqual(background, 'rgba(0, 0, 0, 0)')
  }
  // A script-opened window can close itself after a successful handoff.
  const closed = new Promise((resolve) => {
    context.once('page', (popup) => popup.once('close', resolve))
  })
  await page.evaluate(
    (url) => window.open(url, '_blank'),
    origin + '/desktop/connected#close',
  )
  await Promise.race([
    closed,
    new Promise((_, reject) =>
      setTimeout(
        () => reject(new Error('Completion popup did not close')),
        10000,
      ),
    ),
  ])
  await page.getByRole('link', { name: 'Back to home' }).click()
  await page.waitForURL(origin + '/')
  assert.deepEqual(errors, [])
  console.log(
    'Desktop completion: styled fallback, clean URL, light/dark themes, auto-close, navigation, no browser errors.',
  )
  await context.close()
} finally {
  await browser.close()
}
