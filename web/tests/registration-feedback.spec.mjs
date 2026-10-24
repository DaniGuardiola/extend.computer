import { test, expect } from '@playwright/test'
import { installAccountFixture } from './account-fixtures.mjs'

test(
  'issued recovery codes survive a failed refresh',
  { tag: '@ui' },
  async ({ page, context, baseURL }) => {
    await installAccountFixture(page, 'security-off')
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
    let registered = false
    const codes = Array.from({ length: 10 }, (_, index) =>
      String(12340000 + index),
    )
    await page.route('**/api/v1/passkeys/register/options', (route) =>
      route.fulfill({
        json: {
          challenge: 'A'.repeat(43),
          rp: { name: 'Test', id: new URL(baseURL).hostname },
          user: {
            id: 'Zml4dHVyZQ',
            name: 'review@example.invalid',
            displayName: 'Review',
          },
          pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
          authenticatorSelection: {
            residentKey: 'preferred',
            userVerification: 'preferred',
          },
          attestation: 'none',
        },
      }),
    )
    await page.route('**/api/v1/passkeys/register/verify', (route) => {
      registered = true
      return route.fulfill({ json: { recovery_codes: codes } })
    })
    await page.route('**/api/v1/passkeys', (route) =>
      registered
        ? route.fulfill({
            status: 503,
            json: { error: 'Could not refresh keys.' },
          })
        : route.fallback(),
    )
    await page.goto('/account')
    await page.getByRole('button', { name: 'Add key', exact: true }).click()
    await page.getByLabel('Current password').fill('disposable-test-password')
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    const recovery = page.getByRole('dialog', {
      name: 'Save your recovery codes',
    })
    await expect(recovery).toBeVisible()
    await expect(recovery.locator('.recovery-codes li')).toHaveText(codes)
    await recovery
      .getByRole('button', { name: 'Confirm saved', exact: true })
      .click()
    await expect(
      page.getByText('Could not refresh keys.', { exact: true }).first(),
    ).toBeVisible()
    await expect(
      page.getByRole('button', { name: 'Continue', exact: true }),
    ).toBeEnabled()
  },
)
