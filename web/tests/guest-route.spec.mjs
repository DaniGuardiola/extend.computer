import { testOrigin } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test('guest-route', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  let signedIn = true
  await page.route('**/api/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/account')) {
      return route.fulfill({
        status: signedIn ? 200 : 401,
        json: signedIn
          ? {
              id: 'test',
              email: 'test@example.invalid',
              email_verified: 1,
              mfa_enabled: 0,
            }
          : { error: 'Please sign in' },
      })
    }
    const json = path.endsWith('/devices')
      ? { devices: [] }
      : path.endsWith('/passkeys')
        ? { passkeys: [] }
        : path.endsWith('/sessions')
          ? { sessions: [] }
          : { enabled: false, totp: false, keys: 0, recovery_codes: 0 }
    return route.fulfill({ json })
  })
  await page.goto(origin + '/account')
  await page.getByRole('heading', { name: 'Account', exact: true }).waitFor()
  await page.getByRole('link', { name: 'extend.computer home' }).click()
  await page.getByRole('link', { name: 'Sign in', exact: true }).click()
  await page.waitForURL(origin + '/account')
  for (const path of [
    '/login',
    '/signup',
    '/login?preview=signing-in',
    '/signup?preview=creating-account',
  ]) {
    await page.goto(origin + path)
    await page.waitForURL(origin + '/account')
  }
  signedIn = false
  for (const [path, title] of [
    ['/login', 'Sign in'],
    ['/signup', 'Create an account'],
  ]) {
    await page.goto(origin + path)
    await page.getByRole('heading', { name: title, exact: true }).waitFor()
    assert.equal(new URL(page.url()).pathname, path)
  }
  console.log('Guest routes: signed-in redirects and signed-out forms passed')
})
