import { testOrigin, createTestHeaders } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test(
  'passkey-usage',
  { tag: '@integration' },
  async ({ newContext, newPage }) => {
    const context = await newContext({ extraHTTPHeaders: createTestHeaders() })
    const page = await context.newPage()
    page.on('response', async (response) => {
      if (response.url().includes('/api/v1/') && response.status() >= 400)
        console.log(
          new URL(response.url()).pathname,
          response.status(),
          (await response.json().catch(() => ({}))).error,
        )
    })
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
    const email = `usage-${Date.now()}@example.invalid`
    const password = 'disposable-usage-test-password'
    await page.goto(origin + '/signup')
    await page.waitForLoadState('networkidle')
    await page.getByRole('textbox', { name: 'Email', exact: true }).fill(email)
    await page
      .getByRole('textbox', { name: 'Password', exact: true })
      .fill(password)
    await page
      .getByRole('button', { name: 'Create account', exact: true })
      .click()
    await page.getByRole('button', { name: 'Add key', exact: true }).waitFor()
    await page.getByRole('button', { name: 'Add key', exact: true }).click()
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page
      .getByRole('button', { name: 'Confirm saved', exact: true })
      .waitFor()
    const recoveryCodes = await page
      .locator('.recovery-codes li')
      .allTextContents()
    assert.equal(recoveryCodes.length, 10)
    await page
      .getByRole('button', { name: 'Confirm saved', exact: true })
      .click()
    const usage = page.getByRole('checkbox', {
      name: /^Allow sign-in without password:/,
    })
    await usage.waitFor()
    assert.equal(await usage.isChecked(), true)
    const keys = async () =>
      (await (await context.request.get(origin + '/api/v1/passkeys')).json())
        .passkeys
    const original = (await keys())[0]
    await usage.click()
    await page
      .getByRole('button', { name: 'Turn off sign-in', exact: true })
      .click()
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use passkey or security key', exact: true })
      .click()
    await page.waitForFunction(
      () =>
        document.querySelector('.credential-usage input')?.checked === false,
    )
    assert.equal((await keys())[0].purpose, 'login-disabled')
    assert.equal((await keys()).length, 1)
    const mfa = await (await context.request.get(origin + '/api/v1/mfa')).json()
    assert.equal(mfa.keys, 1)
    assert.equal(mfa.enabled, true)
    // Registration, account refreshes and the usage change all consume the
    // normal auth budget. Begin the next sign-in phase in a fresh window.
    await new Promise((resolve) => setTimeout(resolve, 60000))
    await page.getByRole('button', { name: 'Sign out', exact: true }).click()
    await page
      .getByRole('button', { name: 'Sign in with a passkey', exact: true })
      .click()
    await page
      .getByText('Could not sign in with that passkey', { exact: true })
      .waitFor()
    await page.getByRole('textbox', { name: 'Email', exact: true }).fill(email)
    await page
      .getByRole('textbox', { name: 'Password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Sign in', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use passkey or security key', exact: true })
      .click()
    await usage.waitFor()
    const forbidden = await context.request.patch(
      origin + '/api/v1/passkeys/' + original.id,
      { headers: { Origin: origin }, data: { sign_in_enabled: true } },
    )
    assert.equal(forbidden.status(), 403)
    await usage.click()
    await page
      .getByRole('button', { name: 'Turn on sign-in', exact: true })
      .click()
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use passkey or security key', exact: true })
      .click()
    await page.waitForFunction(
      () => document.querySelector('.credential-usage input')?.checked === true,
    )
    assert.equal((await keys())[0].purpose, 'login')
    await page.getByRole('button', { name: 'Sign out', exact: true }).click()
    await page
      .getByRole('button', { name: 'Sign in with a passkey', exact: true })
      .click()
    await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
    console.log(
      'Basic sign-in toggle passed. Waiting for the normal local rate-limit window before second-factor checks.',
    )
    await new Promise((resolve) => setTimeout(resolve, 60000))
    const savedCredentials = await cdp.send('WebAuthn.getCredentials', {
      authenticatorId,
    })
    await cdp.send('WebAuthn.removeVirtualAuthenticator', { authenticatorId })
    const olderKey = await cdp.send('WebAuthn.addVirtualAuthenticator', {
      options: {
        protocol: 'u2f',
        transport: 'usb',
        hasResidentKey: false,
        hasUserVerification: false,
        automaticPresenceSimulation: true,
      },
    })
    await page.getByRole('button', { name: 'Add key', exact: true }).click()
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use a recovery code', exact: true })
      .click()
    await page
      .getByRole('textbox', { name: 'Recovery code', exact: true })
      .fill(recoveryCodes[0].replace(/\s/g, ''))
    await page
      .getByRole('heading', { name: 'Security key', exact: true })
      .waitFor()
    assert.equal(
      await page
        .getByRole('dialog', { name: 'Save your recovery codes', exact: true })
        .count(),
      0,
    )
    await cdp.send('WebAuthn.removeVirtualAuthenticator', {
      authenticatorId: olderKey.authenticatorId,
    })
    const restored = await cdp.send('WebAuthn.addVirtualAuthenticator', {
      options: {
        protocol: 'ctap2',
        transport: 'internal',
        hasResidentKey: true,
        hasUserVerification: true,
        isUserVerified: true,
        automaticPresenceSimulation: true,
      },
    })
    for (const credential of savedCredentials.credentials)
      await cdp.send('WebAuthn.addCredential', {
        authenticatorId: restored.authenticatorId,
        credential,
      })
    const factor = (await keys()).find((key) => key.purpose === 'factor')
    assert.ok(factor)
    const patchPattern = '**/api/v1/passkeys/' + original.id
    await page.route(patchPattern, (route) =>
      route.continue({
        url: origin + '/api/v1/passkeys/' + factor.id,
        postData: JSON.stringify({ sign_in_enabled: true }),
      }),
    )
    await usage.click()
    await page
      .getByRole('button', { name: 'Turn off sign-in', exact: true })
      .click()
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use passkey or security key', exact: true })
      .click()
    await page
      .getByText('Add this key as a passkey before enabling sign-in', {
        exact: true,
      })
      .waitFor()
    assert.equal(
      (await keys()).find((key) => key.id === factor.id).purpose,
      'factor',
    )
    await page.keyboard.press('Escape')
    await page.getByRole('dialog').waitFor({ state: 'hidden' })
    await page.unroute(patchPattern)
    await usage.click()
    await page
      .getByRole('button', { name: 'Turn off sign-in', exact: true })
      .click()
    await page
      .getByRole('textbox', { name: 'Current password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Continue', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use passkey or security key', exact: true })
      .click()
    await page.waitForFunction(
      () =>
        document.querySelector('.credential-usage input')?.checked === false,
    )
    const stillProtected = await (
      await context.request.get(origin + '/api/v1/mfa')
    ).json()
    assert.equal(stillProtected.enabled, true)
    assert.equal(stillProtected.keys, 2)
    assert.equal(stillProtected.recovery_codes, 9)
    await page.getByRole('button', { name: 'Sign out', exact: true }).click()
    await page.getByRole('textbox', { name: 'Email', exact: true }).fill(email)
    await page
      .getByRole('textbox', { name: 'Password', exact: true })
      .fill(password)
    await page.getByRole('button', { name: 'Sign in', exact: true }).click()
    await page
      .getByRole('button', { name: 'Use passkey or security key', exact: true })
      .click()
    await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
    console.log(
      'Real passkey sign-in blocked when disabled and restored when enabled; credential and second-factor availability preserved; unverified changes rejected.',
    )
  },
)
