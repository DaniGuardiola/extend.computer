import { testOrigin } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'
const origin = testOrigin
test('theme', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage({
    colorScheme: 'dark',
    viewport: { width: 320, height: 900 },
  })
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.route('**/api/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname
    const data = path.endsWith('/account')
      ? {
          id: 'test',
          email: 'review@example.invalid',
          mfa_enabled: 0,
          email_verified: 1,
          email_enabled: true,
        }
      : path.endsWith('/devices')
        ? { devices: [] }
        : path.endsWith('/passkeys')
          ? { passkeys: [] }
          : path.endsWith('/sessions')
            ? { sessions: [] }
            : {
                enabled: false,
                totp: false,
                keys: 0,
                recovery_codes: 0,
                totp_available: true,
              }
    return route.fulfill({ json: data })
  })
  const picker = page.getByRole('combobox', { name: 'Appearance' })
  const scheme = () =>
    page.evaluate(() => getComputedStyle(document.documentElement).colorScheme)
  await page.goto(origin + '/account')
  await page.waitForLoadState('networkidle')
  await picker.selectOption('light')
  assert.equal(await scheme(), 'light')
  await page.reload()
  await page.waitForLoadState('networkidle')
  await picker.waitFor()
  assert.equal(await picker.inputValue(), 'light')
  assert.equal(await scheme(), 'light')
  for (const path of [
    '/login',
    '/signup',
    '/recover',
    '/email#reset=' + '0'.repeat(64),
    '/desktop/connect',
    '/desktop/connected',
    '/page-not-found-preview',
  ]) {
    await page.goto(origin + path)
    await page.waitForLoadState('networkidle')
    await picker.waitFor()
    assert.equal(await scheme(), 'light', path)
    assert.equal(await picker.inputValue(), 'light', path)
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
      path,
    )
  }
  await page.goto(origin + '/login')
  await page.waitForLoadState('networkidle')
  await picker.selectOption('dark')
  assert.equal(await scheme(), 'dark')
  await page.emulateMedia({ colorScheme: 'light' })
  assert.equal(await scheme(), 'dark')
  await picker.selectOption('system')
  assert.equal(await scheme(), 'light dark')
  const light = await page.evaluate(
    () => getComputedStyle(document.body).backgroundColor,
  )
  await page.emulateMedia({ colorScheme: 'dark' })
  const dark = await page.evaluate(
    () => getComputedStyle(document.body).backgroundColor,
  )
  assert.notEqual(light, dark)
  await picker.selectOption('light')
  await page.screenshot({ path: '/tmp/extend-theme-login-light.png' })
  await page.goto(origin + '/account')
  await page.waitForLoadState('networkidle')
  await picker.waitFor()
  await page.screenshot({ path: '/tmp/extend-theme-account-light.png' })
  await page.goto(origin + '/')
  assert.equal(await scheme(), 'dark')
  assert.equal(await picker.count(), 0)
  assert.deepEqual(errors, [])
  console.log(
    'Theme preference persists across account/auth/error pages; system preference follows OS; mobile fits; landing remains dark; no hydration errors.',
  )
})
