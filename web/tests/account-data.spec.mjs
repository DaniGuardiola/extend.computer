import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'
import { testOrigin } from './test-config.mjs'

test('account-data', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  const reads = { account: 0, mfa: 0, sessions: 0 }
  let recoveryCount = 5
  let sessions = [
    {
      id: 'current',
      current: 1,
      client: 'Chrome',
      platform: 'macOS',
      location: null,
      created_at: 1791030000,
      last_access_at: 1791030000,
    },
    {
      id: 'other',
      current: 0,
      client: 'Safari',
      platform: 'iOS',
      location: null,
      created_at: 1791030000,
      last_access_at: 1791030000,
    },
  ]
  await page.route('**/api/v1/**', (route) => {
    const p = new URL(route.request().url()).pathname
    const method = route.request().method()
    if (p.endsWith('/sessions/other') && method === 'DELETE')
      sessions = sessions.filter((s) => s.id !== 'other')
    if (p.endsWith('/mfa/recovery-codes')) {
      recoveryCount = 10
      return route.fulfill({
        json: {
          recovery_codes: Array.from({ length: 10 }, (_, i) =>
            String(12340000 + i),
          ),
        },
      })
    }
    if (p.endsWith('/mfa/reauth'))
      return route.fulfill({ json: { success: true } })
    const endpoint = p.split('/').at(-1)
    if (method === 'GET' && Object.hasOwn(reads, endpoint)) reads[endpoint]++
    const json =
      endpoint === 'account'
        ? {
            id: 'test',
            email: 'test@example.invalid',
            email_verified: 1,
            mfa_enabled: 1,
          }
        : endpoint === 'mfa'
          ? {
              enabled: true,
              totp: true,
              keys: 0,
              recovery_codes: recoveryCount,
              totp_available: true,
            }
          : endpoint === 'sessions'
            ? { sessions }
            : endpoint === 'passkeys'
              ? { passkeys: [] }
              : endpoint === 'devices'
                ? { devices: [] }
                : { success: true }
    return route.fulfill({ json })
  })
  await page.goto(testOrigin + '/account')
  await page
    .getByText('5 unused. Save them somewhere safe. Each works once.')
    .waitFor()
  await page.waitForLoadState('networkidle')
  assert.deepEqual(reads, { account: 1, mfa: 1, sessions: 1 })
  await page
    .getByRole('button', { name: 'Generate new codes', exact: true })
    .click()
  await page.getByLabel('Current password').fill('test-password')
  await page.getByRole('button', { name: 'Continue', exact: true }).click()
  await page.getByRole('button', { name: 'Confirm saved', exact: true }).click()
  await page
    .getByText('10 unused. Save them somewhere safe. Each works once.')
    .waitFor()
  await page.waitForLoadState('networkidle')
  assert.deepEqual(
    reads,
    { account: 2, mfa: 2, sessions: 2 },
    'MFA change refreshes account data once',
  )
  await page
    .getByRole('button', { name: 'Sign out Safari on iOS', exact: true })
    .click()
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Sign out', exact: true })
    .click()
  await page.getByText('Session signed out.', { exact: true }).waitFor()
  assert.equal(
    await page
      .getByRole('button', { name: 'Sign out Safari on iOS', exact: true })
      .count(),
    0,
  )
  assert.deepEqual(
    reads,
    { account: 2, mfa: 2, sessions: 3 },
    'Revoking another session refreshes only sessions',
  )
  console.log('Account data stays current without duplicate MFA/session reads.')
})
