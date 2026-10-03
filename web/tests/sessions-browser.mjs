import { chromium } from 'playwright'
import assert from 'node:assert/strict'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3006'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = await chromium.launch({ headless: true })
const context = await browser.newContext()
const page = await context.newPage()
const errors = []
page.on('pageerror', (e) => errors.push(e.message))
const email = `sessions-ui-${Date.now()}@example.invalid`,
  password = 'disposable-session-ui-password'
await page.goto(origin + '/signup')
await page.waitForLoadState('networkidle')
await page.getByRole('textbox', { name: 'Email', exact: true }).fill(email)
await page
  .getByRole('textbox', { name: 'Password', exact: true })
  .fill(password)
await page.getByRole('button', { name: 'Create account', exact: true }).click()
const section = page.getByRole('region', { name: 'Sessions', exact: true })
await section
  .getByRole('button', { name: 'Sign out this session', exact: true })
  .waitFor()
const response = await fetch(origin + '/api/v1/auth/login', {
  method: 'POST',
  headers: {
    Origin: origin,
    'Content-Type': 'application/json',
    'X-Extend-Client': 'desktop',
    'User-Agent': 'extend.computer (Windows)',
  },
  body: JSON.stringify({ email, password }),
})
const native = await response.json()
assert.equal(response.status, 200)
await page.getByRole('button', { name: 'Refresh', exact: true }).click()
await section
  .getByRole('button', { name: 'Sign out Desktop app on Windows', exact: true })
  .waitFor()
assert.equal(await section.locator('article').count(), 2)
await section.scrollIntoViewIfNeeded()
await page.screenshot({ path: '/tmp/extend-sessions-preview.png' })
await section
  .getByRole('button', { name: 'Sign out Desktop app on Windows', exact: true })
  .click()
await page
  .getByRole('dialog', { name: 'Sign out this session?', exact: true })
  .getByRole('button', { name: 'Sign out', exact: true })
  .click()
await section.getByText('Session signed out.', { exact: true }).waitFor()
assert.equal(await section.locator('article').count(), 1)
assert.equal(
  (
    await fetch(origin + '/api/v1/account', {
      headers: { Authorization: 'Bearer ' + native.token },
    })
  ).status,
  401,
)
await section.getByRole('button', { name: 'Sign out all', exact: true }).click()
await page
  .getByRole('dialog', { name: 'Sign out all sessions?', exact: true })
  .getByRole('button', { name: 'Sign out', exact: true })
  .click()
await page.getByRole('heading', { name: 'Sign in', exact: true }).waitFor()
assert.deepEqual(errors, [])
await browser.close()
console.log(
  'PASS: sessions list, confirmation, individual sign-out, bulk sign-out, login redirect, no browser errors',
)
