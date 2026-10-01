import { chromium } from 'playwright'
import assert from 'node:assert/strict'
const url = process.env.EXTEND_TEST_URL ?? 'http://127.0.0.1:3001'
const browser = await chromium.connectOverCDP(process.env.EXTEND_TEST_CDP)
const context = browser.contexts()[0]
const page =
  context.pages().find((p) => p.url().startsWith(url)) ??
  (await context.newPage())
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
await page.getByRole('button', { name: 'Create account', exact: true }).click()
await page.getByRole('button', { name: 'Add passkey', exact: true }).waitFor()
await page.getByRole('button', { name: 'Add passkey', exact: true }).click()
await page
  .getByText('Passkey added. Next time, sign in with a touch.', { exact: true })
  .waitFor()
await page
  .getByRole('button', { name: 'Sign out', exact: true })
  .first()
  .click()
await page
  .getByRole('button', { name: 'Sign in with a passkey', exact: true })
  .click()
await page
  .getByRole('heading', { name: 'Your devices.', exact: true })
  .waitFor()
await page.getByRole('button', { name: 'Remove passkey', exact: true }).click()
const dialog = page.getByRole('dialog')
await dialog.getByRole('button', { name: 'Remove', exact: true }).click()
await page.getByText('Removed.', { exact: true }).waitFor()
await page
  .getByRole('heading', { name: 'A touch beats a password.', exact: true })
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
await browser.close()
