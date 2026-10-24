import { testOrigin } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test('email-verification', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  let verifications = 0
  let rejected = false
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/auth/email/verify')) {
      verifications++
      assert.equal(route.request().method(), 'POST')
      return route.fulfill(
        rejected
          ? {
              status: 400,
              json: { error: 'Invalid or expired link. Request a new one.' },
            }
          : { status: 200, json: { success: true } },
      )
    }
    const data = path.endsWith('/account')
      ? {
          id: 'preview',
          email: 'review@example.invalid',
          email_verified: 1,
          email_enabled: true,
          mfa_enabled: 0,
        }
      : path.endsWith('/devices')
        ? { devices: [] }
        : path.endsWith('/passkeys')
          ? { passkeys: [] }
          : path.endsWith('/sessions')
            ? { sessions: [] }
            : { enabled: false, totp: false, keys: 0, recovery_codes: 0 }
    await route.fulfill({ status: 200, json: data })
  })
  await page.goto(origin + '/email#verify=' + '0'.repeat(64))
  await page
    .getByRole('region', { name: 'Email verified', exact: true })
    .waitFor()
  assert.equal(new URL(page.url()).pathname, '/account')
  assert.equal(new URL(page.url()).hash, '')
  assert.equal(verifications, 1)
  await page
    .getByRole('button', { name: 'Dismiss email verification confirmation' })
    .click()
  assert.equal(
    await page
      .getByRole('region', { name: 'Email verified', exact: true })
      .count(),
    0,
  )
  await page.reload()
  await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
  assert.equal(
    await page
      .getByRole('region', { name: 'Email verified', exact: true })
      .count(),
    0,
  )
  rejected = true
  await page.goto(origin + '/email#verify=' + '1'.repeat(64))
  await page.getByRole('alert').waitFor()
  assert.equal(new URL(page.url()).pathname, '/email')
  assert.equal(
    await page
      .getByRole('button', { name: 'Verify email', exact: true })
      .count(),
    0,
  )
  assert.equal(verifications, 2)
  console.log(
    'Automatic verification, redirect, dismissible one-time banner, and invalid-link handling passed.',
  )
})
