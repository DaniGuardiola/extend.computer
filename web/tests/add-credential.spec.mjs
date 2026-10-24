import { testOrigin } from './test-config.mjs'
const origin = testOrigin
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

test('add-credential', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  const registrations = []
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/passkeys/register/options')) {
      registrations.push(route.request().postDataJSON())
      return route.fulfill({
        status: 400,
        json: { error: 'Test registration stopped' },
      })
    }
    const data = path.endsWith('/account')
      ? {
          id: 'preview',
          email: 'review@example.invalid',
          email_verified: 1,
          mfa_enabled: 0,
        }
      : path.endsWith('/devices')
        ? { devices: [] }
        : path.endsWith('/passkeys')
          ? { passkeys: [] }
          : path.endsWith('/sessions')
            ? { sessions: [] }
            : path.endsWith('/mfa/reauth')
              ? { success: true }
              : {
                  enabled: false,
                  totp: false,
                  keys: 0,
                  recovery_codes: 0,
                  totp_available: true,
                }
    await route.fulfill({ status: 200, json: data })
  })
  await page.goto(origin + '/account')
  await page.getByRole('button', { name: 'Add key', exact: true }).waitFor()
  await page.waitForLoadState('networkidle')
  for (const width of [320, 1024]) {
    await page.setViewportSize({ width, height: 900 })
    await page.getByRole('button', { name: 'Add key', exact: true }).click()
    assert.equal(
      await page
        .getByRole('checkbox', {
          name: 'Allow sign-in without password',
          exact: true,
        })
        .count(),
      0,
    )
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill('disposable-test-password')
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    )
    const requested = page.waitForResponse((response) =>
      response.url().endsWith('/passkeys/register/options'),
    )
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await requested
    assert.deepEqual(registrations.at(-1), { auto: true })
    await page.getByText('Test registration stopped', { exact: true }).waitFor()
    await page.getByRole('button', { name: 'Cancel', exact: true }).click()
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
  }
  console.log(
    'One add-key action verifies identity then requests automatic registration; no upfront usage choice; fits mobile and desktop.',
  )
})
