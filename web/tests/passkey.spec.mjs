import { testOrigin, createTestHeaders } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'
const url = testOrigin
test('passkey', { tag: '@integration' }, async ({ newContext, newPage }) => {
  const context = await newContext({ extraHTTPHeaders: createTestHeaders() })
  const page = await context.newPage()
  try {
    const errors = []
    page.on('pageerror', (error) => errors.push(error.message))
    const cdp = await context.newCDPSession(page)
    await cdp.send('WebAuthn.enable')
    await cdp.send('WebAuthn.addVirtualAuthenticator', {
      options: {
        protocol: 'ctap2',
        transport: 'internal',
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true,
      },
    })
    const suffix = Date.now()
    const email = `passkey-${suffix}@example.invalid`
    await page.goto(url + '/signup')
    await page.waitForLoadState('networkidle')
    await page.getByRole('textbox', { name: 'Email', exact: true }).fill(email)
    await page
      .getByRole('textbox', { name: 'Password', exact: true })
      .fill('disposable-browser-password')
    await page
      .getByRole('button', { name: 'Create account', exact: true })
      .click()
    await page.getByRole('button', { name: 'Add key', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Add key', exact: true }).click()
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill('disposable-browser-password')
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page.getByText('Key added.', { exact: true }).waitFor()
    await page
      .getByRole('dialog', { name: 'Save your recovery codes' })
      .getByRole('button', { name: 'Confirm saved', exact: true })
      .click()
    await page
      .getByRole('button', { name: 'Sign out', exact: true })
      .first()
      .click()
    await page
      .getByRole('button', { name: 'Sign in with a passkey', exact: true })
      .click()
    await page.getByRole('heading', { name: 'Account', exact: true }).waitFor()
    await page
      .getByRole('group', { name: 'Passkeys and security keys', exact: true })
      .getByRole('button', { name: /^Remove / })
      .click()
    const dialog = page.getByRole('dialog')
    await dialog
      .getByRole('button', {
        name: 'Remove passkey and turn off 2FA',
        exact: true,
      })
      .click()
    await page
      .getByLabel('Current password')
      .fill('disposable-browser-password')
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use passkey or security key', exact: true })
      .click()
    await page.getByText('Removed.', { exact: true }).waitFor()
    await page
      .getByRole('heading', { name: 'No keys added', exact: true })
      .waitFor()
    assert.deepEqual(errors, [])
    await page.screenshot({
      path: '/tmp/extend-account-tested.png',
      fullPage: true,
    })
    console.log(
      JSON.stringify({
        passed: [
          'password signup',
          'passkey registration',
          'passkey login',
          'passkey removal',
          'accessible confirmation dialog',
          'no browser exceptions',
        ],
        test_email: email,
      }),
    )
    await cdp.detach()
  } finally {
  }
})
