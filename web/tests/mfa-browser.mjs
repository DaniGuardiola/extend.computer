// Runs only in the isolated test Chromium.
import { chromium } from 'playwright'
import { createHmac } from 'node:crypto'
import assert from 'node:assert/strict'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3006'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = process.env.EXTEND_TEST_CDP
  ? await chromium.connectOverCDP(process.env.EXTEND_TEST_CDP)
  : await chromium.launch({ headless: true })
const context = await browser.newContext()
await context.setExtraHTTPHeaders({
  'CF-Connecting-IP': '10.123.' + Math.floor(Math.random() * 250) + '.1',
})
const page = await context.newPage()
const errors = []
page.on('pageerror', (e) => errors.push(e.message))
const cdp = await context.newCDPSession(page)
await cdp.send('WebAuthn.enable')
const password = 'disposable-browser-mfa-password'
let testEmail
async function signup(prefix) {
  await page.goto(origin + '/signup')
  await page.waitForLoadState('networkidle')
  testEmail = prefix + '-' + Date.now() + '@example.invalid'
  await page
    .getByRole('textbox', { name: 'Email', exact: true })
    .fill(testEmail)
  await page
    .getByRole('textbox', { name: 'Password', exact: true })
    .fill(password)
  await page
    .getByRole('button', { name: 'Create account', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
  await page.getByText(testEmail, { exact: true }).waitFor()
}
async function confirmPassword() {
  const d = page.getByRole('dialog', {
    name: 'Confirm security change',
    exact: true,
  })
  await d
    .getByRole('textbox', { name: 'Current password', exact: true })
    .fill(password)
  await d.getByRole('button', { name: 'Continue', exact: true }).click()
}
async function saveCodes() {
  const d = page.getByRole('dialog', {
    name: 'Save your recovery codes',
    exact: true,
  })
  await d.waitFor().catch(async (e) => {
    console.error({
      alerts: await page.getByRole('alert').allTextContents(),
      errors,
    })
    throw e
  })
  const codes = await d.locator('.recovery-codes code').allTextContents()
  assert.equal(codes.length, 10)
  for (const code of codes) assert.match(code, /^\d{4} \d{4}$/)
  await d.getByRole('button', { name: 'Saved', exact: true }).click()
  return codes
}
async function passwordLogin() {
  await page
    .getByRole('button', { name: 'Sign out', exact: true })
    .first()
    .click()
  await page
    .getByRole('textbox', { name: 'Email', exact: true })
    .fill(testEmail)
  await page
    .getByRole('textbox', { name: 'Password', exact: true })
    .fill(password)
  await page.waitForLoadState('networkidle')
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page
    .locator('.second-factor')
    .waitFor()
    .catch(async (e) => {
      console.error({
        url: page.url(),
        alerts: await page.getByRole('alert').allTextContents(),
        errors,
      })
      throw e
    })
  assert.equal(
    await page.getByRole('heading', { name: 'Devices', exact: true }).count(),
    0,
  )
}
const key = await cdp.send('WebAuthn.addVirtualAuthenticator', {
  options: {
    protocol: 'ctap2',
    transport: 'usb',
    hasResidentKey: false,
    hasUserVerification: false,
    isUserVerified: false,
    automaticPresenceSimulation: true,
  },
})
await signup('touch-key')
await page.getByRole('button', { name: 'Add key', exact: true }).click()
await confirmPassword()
await saveCodes()
await page
  .getByText('On. Password sign-ins require a second factor.', { exact: true })
  .waitFor()
await passwordLogin()
await page
  .getByRole('button', { name: 'Use passkey or security key', exact: true })
  .click()
await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
const purpose = await page.evaluate(async () => {
  const r = await fetch('/api/v1/passkeys')
  return (await r.json()).passkeys[0].purpose
})
assert.equal(purpose, 'factor')
await page
  .getByRole('button', { name: 'Generate new codes', exact: true })
  .click()
await confirmPassword()
await page
  .getByRole('dialog', { name: 'Confirm security change', exact: true })
  .getByRole('button', { name: 'Use passkey or security key', exact: true })
  .click()
await saveCodes()
await cdp.send('WebAuthn.removeVirtualAuthenticator', {
  authenticatorId: key.authenticatorId,
})
await page
  .getByRole('button', { name: 'Sign out', exact: true })
  .first()
  .click()
await signup('totp')
await page.getByRole('button', { name: 'Set up', exact: true }).click()
await confirmPassword()
const setup = page.getByRole('dialog', {
  name: 'Set up one-time codes',
  exact: true,
})
await setup
  .getByRole('img', { name: 'Authenticator setup QR code', exact: true })
  .waitFor()
assert.equal(await setup.locator('.totp-key-row code').count(), 0)
assert.equal(
  await setup
    .getByRole('textbox', { name: 'One-time code', exact: true })
    .count(),
  0,
)
await setup
  .getByRole('button', { name: 'Can’t scan? Show setup key', exact: true })
  .click()
const secret = await setup.locator('.totp-key-row code').innerText()
await context.grantPermissions(['clipboard-read', 'clipboard-write'])
await setup.getByRole('button', { name: 'Copy setup key', exact: true }).click()
await setup.getByText('Copied', { exact: true }).waitFor()
assert.equal(await page.evaluate(() => navigator.clipboard.readText()), secret)
function code() {
  let bits = 0,
    value = 0,
    bytes = []
  for (const c of secret) {
    value = (value << 5) | 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.indexOf(c)
    bits += 5
    if (bits >= 8) {
      bits -= 8
      bytes.push((value >>> bits) & 255)
    }
  }
  const counter = Buffer.alloc(8)
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000)))
  const hash = createHmac('sha1', Buffer.from(bytes)).update(counter).digest()
  return ((hash.readUInt32BE(hash[19] & 15) & 0x7fffffff) % 1e6)
    .toString()
    .padStart(6, '0')
}
await setup.getByRole('button', { name: 'Next', exact: true }).click()
assert.equal(
  await setup
    .getByRole('img', { name: 'Authenticator setup QR code', exact: true })
    .count(),
  0,
)
assert.equal(await setup.locator('.totp-key-row code').count(), 0)
await setup.getByRole('button', { name: 'Back', exact: true }).click()
assert.equal(await setup.locator('.totp-key-row code').innerText(), secret)
await setup.getByRole('button', { name: 'Hide setup key', exact: true }).click()
assert.equal(await setup.locator('.totp-key-row code').count(), 0)
await setup.getByRole('button', { name: 'Next', exact: true }).click()
const setupStep = Math.floor(Date.now() / 30000)
await setup
  .getByRole('textbox', { name: 'One-time code', exact: true })
  .fill(code())
await setup
  .getByRole('button', { name: 'Enable one-time codes', exact: true })
  .click()
const recovery = await saveCodes()
await passwordLogin()
await page
  .getByRole('button', { name: 'Use a recovery code', exact: true })
  .click()
await page
  .getByRole('textbox', { name: 'Recovery code', exact: true })
  .fill(recovery[0].replace(/\s/g, ''))
await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
while (Math.floor(Date.now() / 30000) === setupStep)
  await new Promise((resolve) => setTimeout(resolve, 500))
await passwordLogin()
await page
  .getByRole('textbox', { name: 'One-time code', exact: true })
  .fill(code())
await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
await page.getByText(testEmail, { exact: true }).waitFor()
await page
  .getByText('On. Password sign-ins require a second factor.', { exact: true })
  .waitFor()
assert.deepEqual(errors, [])
await page.screenshot({ path: '/tmp/extend-mfa-tested.png', fullPage: true })
console.log(
  JSON.stringify({
    passed: [
      'authenticator QR setup',
      'one-time recovery display',
      'password plus touch-only USB key',
      'password plus recovery code',
      'password plus TOTP',
      'no browser exceptions',
    ],
  }),
)
await context.close()
await browser.close()
