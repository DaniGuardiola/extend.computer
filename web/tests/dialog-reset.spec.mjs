import { testOrigin } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test('dialog-reset', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  let rejectVerification = true
  async function checkRetry(dialog, button) {
    const alert = dialog.getByRole('alert')
    await alert.evaluate((el) => {
      window.previousErrorNode = el
    })
    const height = await dialog.evaluate(
      (el) => el.getBoundingClientRect().height,
    )
    await button.click()
    assert.equal(
      await alert.evaluate((el) => el === window.previousErrorNode),
      true,
    )
    await page.waitForFunction(() => !window.previousErrorNode.isConnected)
    assert.equal(
      await alert.evaluate((el) => getComputedStyle(el).animationName),
      'auth-error-enter',
    )
    assert.equal(
      await dialog.evaluate((el) => el.getBoundingClientRect().height),
      height,
    )
  }
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname
    const method = route.request().method()
    if (
      (path.endsWith('/mfa/reauth') && rejectVerification) ||
      path.endsWith('/mfa/totp/confirm') ||
      path.endsWith('/account/password') ||
      (path.endsWith('/sessions') && method === 'DELETE')
    ) {
      await new Promise((resolve) => setTimeout(resolve, 300))
      return route.fulfill({
        status: 400,
        json: { error: 'Test request rejected.' },
      })
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
            ? {
                sessions: [
                  {
                    id: 'session',
                    created_at: 1,
                    last_access_at: 1,
                    client: 'Chrome',
                    platform: 'macOS',
                    location: null,
                    current: 1,
                  },
                ],
              }
            : path.endsWith('/mfa/reauth')
              ? { success: true }
              : path.endsWith('/mfa/totp/setup')
                ? {
                    secret: 'JBSWY3DPEHPK3PXP',
                    uri: 'otpauth://totp/Test?secret=JBSWY3DPEHPK3PXP',
                  }
                : {
                    enabled: false,
                    totp: false,
                    keys: 0,
                    recovery_codes: 0,
                    totp_available: true,
                  }
    return route.fulfill({ json: data })
  })
  await page.goto(origin + '/account')
  await page.waitForLoadState('networkidle')
  const openSetup = () =>
    page.getByRole('button', { name: 'Set up', exact: true }).click()
  const verification = page.getByRole('dialog', {
    name: 'Confirm security change',
    exact: true,
  })
  await openSetup()
  await verification.locator('input').fill('example-password')
  await verification
    .getByRole('button', { name: 'Continue', exact: true })
    .click()
  await verification.getByRole('alert').waitFor()
  await checkRetry(
    verification,
    verification.getByRole('button', { name: 'Continue', exact: true }),
  )
  await verification
    .getByRole('button', { name: 'Cancel', exact: true })
    .click()
  await openSetup()
  assert.equal(await verification.getByRole('alert').count(), 0)
  assert.equal(await verification.locator('input').inputValue(), '')
  rejectVerification = false
  await verification.locator('input').fill('example-password')
  await verification
    .getByRole('button', { name: 'Continue', exact: true })
    .click()
  const setup = page.getByRole('dialog', {
    name: 'Set up one-time codes',
    exact: true,
  })
  await setup.getByRole('button', { name: 'Next', exact: true }).click()
  await setup
    .getByRole('textbox', { name: 'One-time code', exact: true })
    .fill('123456')
  await setup
    .getByRole('button', { name: 'Enable one-time codes', exact: true })
    .click()
  await setup.getByRole('alert').waitFor()
  await checkRetry(
    setup,
    setup.getByRole('button', { name: 'Enable one-time codes', exact: true }),
  )
  await page.keyboard.press('Escape')
  await setup.waitFor({ state: 'hidden' })
  await openSetup()
  await verification.locator('input').fill('example-password')
  await verification
    .getByRole('button', { name: 'Continue', exact: true })
    .click()
  await setup.getByRole('button', { name: 'Next', exact: true }).click()
  assert.equal(await setup.getByRole('alert').count(), 0)
  assert.equal(
    await setup
      .getByRole('textbox', { name: 'One-time code', exact: true })
      .inputValue(),
    '',
  )
  await page.keyboard.press('Escape')
  await setup.waitFor({ state: 'hidden' })
  await page.getByRole('button', { name: 'Sign out all', exact: true }).click()
  const sessions = page.getByRole('dialog', {
    name: 'Sign out all sessions?',
    exact: true,
  })
  await sessions.getByRole('button', { name: 'Sign out', exact: true }).click()
  await sessions.getByRole('alert').waitFor()
  await checkRetry(
    sessions,
    sessions.getByRole('button', { name: 'Sign out', exact: true }),
  )
  await sessions.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('button', { name: 'Sign out all', exact: true }).click()
  assert.equal(await sessions.getByRole('alert').count(), 0)
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: 'Change', exact: true }).click()
  const password = page.getByRole('dialog', {
    name: 'Change password',
    exact: true,
  })
  await password
    .getByRole('textbox', { name: 'Current password', exact: true })
    .fill('example-password')
  await password
    .getByRole('textbox', { name: 'New password', exact: true })
    .fill('different-password')
  await password
    .getByRole('button', { name: 'Save password', exact: true })
    .click()
  await password.getByRole('alert').waitFor()
  await checkRetry(
    password,
    password.getByRole('button', { name: 'Save password', exact: true }),
  )
  await page.emulateMedia({ reducedMotion: 'reduce' })
  assert.equal(
    await password
      .getByRole('alert')
      .evaluate((el) => getComputedStyle(el).animationName),
    'none',
  )
  await password.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('button', { name: 'Change', exact: true }).click()
  assert.equal(await password.getByRole('alert').count(), 0)
  assert.equal(
    await password
      .getByRole('textbox', { name: 'Current password', exact: true })
      .inputValue(),
    '',
  )
  assert.deepEqual(errors, [])
  console.log(
    'Verification, one-time code setup, session sign-out, and password server errors reset after dismissal.',
  )
})
