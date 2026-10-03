import { chromium } from 'playwright'
import assert from 'node:assert/strict'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3006'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = await chromium.launch({ headless: true })
try {
  const page = await browser.newPage()
  let requests = 0
  let holdLogin
  await page.route('**/api/v1/auth/login', async (route) => {
    requests++
    if (holdLogin) await holdLogin
    await route.fulfill({ status: 401, json: { error: 'Invalid credentials' } })
  })
  await page.goto(origin + '/login')
  await page.waitForLoadState('networkidle')
  const email = page.getByRole('textbox', { name: 'Email', exact: true })
  const password = page.getByRole('textbox', { name: 'Password', exact: true })
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.getByText('Enter your email.', { exact: true }).waitFor()
  assert.equal(requests, 0)
  assert.equal(
    await email.evaluate((el) => el === document.activeElement),
    true,
  )
  await email.fill('invalid')
  await page
    .getByText('Enter a valid email address.', { exact: true })
    .waitFor()
  await email.fill('review@example.invalid')
  assert.equal(await email.getAttribute('aria-invalid'), null)
  await password.fill('short')
  await page.getByText('Use at least 12 characters.', { exact: true }).waitFor()
  assert.equal(await password.getAttribute('aria-invalid'), 'true')
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  assert.equal(requests, 0)
  await password.fill('long-enough-password')
  assert.equal(await password.getAttribute('aria-invalid'), null)
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.getByText('Invalid credentials', { exact: true }).waitFor()
  assert.equal(requests, 1)
  const beforeRetry = await password.boundingBox()
  await page.getByRole('alert').evaluate((element) => {
    element.dataset.previousAttempt = 'true'
  })
  let releaseLogin
  holdLogin = new Promise((resolve) => {
    releaseLogin = resolve
  })
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page.getByRole('status').waitFor()
  assert.deepEqual(await password.boundingBox(), beforeRetry)
  assert.equal(
    await page.getByRole('alert').textContent(),
    'Invalid credentials',
  )
  releaseLogin()
  holdLogin = undefined
  await page.getByRole('status').waitFor({ state: 'hidden' })
  assert.deepEqual(await password.boundingBox(), beforeRetry)
  assert.equal(
    await page.getByRole('alert').getAttribute('data-previous-attempt'),
    null,
  )
  await page.goto(origin + '/recover')
  await page.waitForLoadState('networkidle')
  await page
    .getByRole('button', { name: 'Send reset link', exact: true })
    .click()
  await page.getByText('Enter your email.', { exact: true }).waitFor()
  await page.goto(origin + '/signup')
  await page.waitForLoadState('networkidle')
  await page
    .getByRole('textbox', { name: 'Email', exact: true })
    .fill('review@example.invalid')
  await page
    .getByRole('textbox', { name: 'Password', exact: true })
    .fill('short')
  await page
    .getByRole('button', { name: 'Create account', exact: true })
    .click()
  await page.getByText('Use at least 12 characters.', { exact: true }).waitFor()
  console.log(
    'Passed: inline validation, focus, correction, blocked invalid requests, signup and recovery.',
  )
} finally {
  await browser.close()
}
