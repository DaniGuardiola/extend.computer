// Local-only, isolated Chromium: passwordless passkeys remain valid with MFA.
import { chromium } from 'playwright'
import assert from 'node:assert/strict'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3006'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = await chromium.launch({ headless: true })
try {
  const context = await browser.newContext()
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  const cdp = await context.newCDPSession(page)
  await cdp.send('WebAuthn.enable')
  const { authenticatorId } = await cdp.send(
    'WebAuthn.addVirtualAuthenticator',
    {
      options: {
        protocol: 'ctap2',
        transport: 'internal',
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true,
      },
    },
  )
  const email = `passkey-mfa-${Date.now()}@example.invalid`
  const password = 'disposable-passkey-mfa-password'
  await page.goto(origin + '/signup')
  await page.waitForLoadState('networkidle')
  await page.getByRole('textbox', { name: 'Email', exact: true }).fill(email)
  await page
    .getByRole('textbox', { name: 'Password', exact: true })
    .fill(password)
  await page
    .getByRole('button', { name: 'Create account', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
  await page.reload()
  await page.waitForLoadState('networkidle')
  await page.getByRole('button', { name: 'Add passkey', exact: true }).click()
  await page
    .getByText('Passkey added.', { exact: true })
    .waitFor()
    .catch(async (error) => {
      console.error(errors, await page.locator('body').innerText())
      throw error
    })
  await page
    .getByRole('button', { name: 'Enable with existing keys', exact: true })
    .click()
  const confirm = page.getByRole('dialog', {
    name: 'Confirm security change',
    exact: true,
  })
  await confirm
    .getByRole('textbox', { name: 'Current password', exact: true })
    .fill(password)
  await confirm.getByRole('button', { name: 'Continue', exact: true }).click()
  await page
    .getByRole('dialog', { name: 'Save your recovery codes', exact: true })
    .getByRole('button', { name: 'Saved', exact: true })
    .click()
  await page
    .getByText('On. Password sign-ins require a second factor.', {
      exact: true,
    })
    .waitFor()
  await page
    .getByRole('button', { name: 'Sign out', exact: true })
    .first()
    .click()
  await page.waitForLoadState('networkidle')
  await page.getByRole('textbox', { name: 'Email', exact: true }).fill(email)
  await page
    .getByRole('textbox', { name: 'Password', exact: true })
    .fill(password)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.locator('.second-factor').waitFor()
  assert.equal(
    await page.getByRole('heading', { name: 'Devices', exact: true }).count(),
    0,
  )
  await page.goto(origin + '/login')
  await page.waitForLoadState('networkidle')
  // A signed assertion without user verification must still be rejected.
  const credential = (
    await cdp.send('WebAuthn.getCredentials', { authenticatorId })
  ).credentials[0]
  await cdp.send('WebAuthn.setUserVerified', {
    authenticatorId,
    isUserVerified: false,
  })
  const denied = await page.evaluate(
    async ({ id }) => {
      const options = await fetch('/api/v1/passkeys/login/options', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{}',
      }).then((r) => r.json())
      const decode = (value) =>
        Uint8Array.from(
          atob(value.replace(/-/g, '+').replace(/_/g, '/')),
          (c) => c.charCodeAt(0),
        )
      const encode = (value) =>
        btoa(String.fromCharCode(...new Uint8Array(value)))
          .replace(/\+/g, '-')
          .replace(/\//g, '_')
          .replace(/=+$/, '')
      const assertion = await navigator.credentials.get({
        publicKey: {
          ...options,
          challenge: decode(options.challenge),
          userVerification: 'discouraged',
          allowCredentials: [{ id: decode(id), type: 'public-key' }],
        },
      })
      return (
        await fetch('/api/v1/passkeys/login/verify', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            id: assertion.id,
            rawId: encode(assertion.rawId),
            type: assertion.type,
            clientExtensionResults: assertion.getClientExtensionResults(),
            response: {
              clientDataJSON: encode(assertion.response.clientDataJSON),
              authenticatorData: encode(assertion.response.authenticatorData),
              signature: encode(assertion.response.signature),
              userHandle: assertion.response.userHandle
                ? encode(assertion.response.userHandle)
                : null,
            },
          }),
        })
      ).status
    },
    { id: credential.credentialId },
  )
  assert.equal(denied, 401)
  await cdp.send('WebAuthn.setUserVerified', {
    authenticatorId,
    isUserVerified: true,
  })
  await page
    .getByRole('button', { name: 'Sign in with a passkey', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
  await page.getByText(email, { exact: true }).waitFor()
  await page
    .getByText('On. Password sign-ins require a second factor.', {
      exact: true,
    })
    .waitFor()
  assert.deepEqual(errors, [])
  console.log(
    'Passed: direct passkey sign-in with MFA, password MFA gate, user-verification enforcement.',
  )
} finally {
  await browser.close()
}
