import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'
import { testOrigin } from './test-config.mjs'

test('query-cache', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  let user = 'first'
  const reads = { devices: 0, passkeys: 0, mfa: 0, sessions: 0 }
  await page.route('**/api/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname.slice(
      '/api/v1/'.length,
    )
    const method = route.request().method()
    if (path === 'auth/logout') {
      user = null
      return route.fulfill({ json: { success: true } })
    }
    if (path === 'auth/login') {
      user = 'second'
      return route.fulfill({ json: { account: { id: user } } })
    }
    if (!user)
      return route.fulfill({ status: 401, json: { error: 'Please sign in' } })
    if (method === 'GET' && path in reads) reads[path]++
    const json =
      path === 'account'
        ? {
            id: user,
            email: `${user}@example.invalid`,
            email_verified: 1,
            mfa_enabled: 0,
          }
        : path === 'devices'
          ? {
              devices: [
                {
                  id: user,
                  name: `${user} laptop`,
                  platform: 'macos',
                  fingerprint: 'a'.repeat(64),
                  online: true,
                  last_seen: null,
                },
              ],
            }
          : path === 'passkeys'
            ? { passkeys: [] }
            : path === 'sessions'
              ? { sessions: [] }
              : { enabled: false, totp: false, keys: 0, recovery_codes: 0 }
    return route.fulfill({ json })
  })
  await page.goto(testOrigin + '/account')
  await page.getByText('first laptop', { exact: true }).waitFor()
  await page.getByRole('link', { name: 'extend.computer home' }).click()
  await page.getByRole('link', { name: 'Sign in', exact: true }).click()
  await page.waitForURL(testOrigin + '/account')
  await page.getByText('first laptop', { exact: true }).waitFor()
  assert.deepEqual(
    reads,
    { devices: 1, passkeys: 1, mfa: 1, sessions: 1 },
    'Fresh cache survives client navigation without duplicate reads',
  )
  await page
    .locator('.account-nav')
    .getByRole('button', { name: 'Sign out', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Sign in', exact: true }).waitFor()
  await page.getByLabel('Email', { exact: true }).fill('second@example.invalid')
  await page.getByLabel('Password', { exact: true }).fill('test-password')
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.getByText('second laptop', { exact: true }).waitFor()
  assert.equal(await page.getByText('first laptop', { exact: true }).count(), 0)
  assert.equal(
    await page.getByText('first@example.invalid', { exact: true }).count(),
    0,
  )
  assert.deepEqual(
    reads,
    { devices: 2, passkeys: 2, mfa: 2, sessions: 2 },
    'New user gets new data for every protected resource',
  )
  console.log(
    'Query navigation cache, deduplication and account-switch isolation passed.',
  )
})
