import { chromium } from 'playwright'
import assert from 'node:assert/strict'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3006'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = await chromium.launch({ headless: true })
try {
  const context = await browser.newContext()
  let submitted
  let holdVerification
  await context.route('**/api/v1/auth/login', (route) =>
    route.fulfill({
      json: {
        mfa_required: true,
        ticket: 'review',
        totp: true,
        recovery: true,
        keys: false,
      },
    }),
  )
  await context.route('**/api/v1/auth/mfa/verify', async (route) => {
    submitted = route.request().postDataJSON().code
    if (holdVerification) await holdVerification
    await route.fulfill({ status: 401, json: { error: 'Invalid code' } })
  })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  await page.goto(origin + '/login')
  await page.waitForLoadState('networkidle')
  await page
    .getByRole('textbox', { name: 'Email', exact: true })
    .fill('review@example.invalid')
  await page
    .getByRole('textbox', { name: 'Password', exact: true })
    .fill('disposable-review-password')
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  const input = page.getByRole('textbox', {
    name: 'One-time code',
    exact: true,
  })
  await input.waitFor()
  assert.equal(await page.locator('.otp-slot').count(), 6)
  await input.fill('12345')
  assert.deepEqual(await page.locator('.otp-slot').allTextContents(), [
    '1',
    '2',
    '3',
    '4',
    '5',
    '',
  ])
  await input.press('Backspace')
  assert.equal(await input.inputValue(), '1234')
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await page.evaluate(() => navigator.clipboard.writeText('654 321'))
  await input.fill('')
  const verified = page.waitForResponse((response) =>
    response.url().endsWith('/auth/mfa/verify'),
  )
  await input.press('ControlOrMeta+V')
  await verified
  assert.equal(await input.inputValue(), '654321')
  assert.equal(
    await page
      .getByRole('button', { name: 'Verify code', exact: true })
      .count(),
    0,
  )
  await page.getByRole('alert').waitFor()
  assert.equal(submitted, '654321')
  const beforeRetry = await page.locator('.otp-slots').boundingBox()
  await page.getByRole('alert').evaluate((element) => {
    element.dataset.previousAttempt = 'true'
  })
  let releaseVerification
  holdVerification = new Promise((resolve) => {
    releaseVerification = resolve
  })
  await input.fill('12345')
  await input.press('6')
  await page.waitForTimeout(200)
  assert.equal(await page.getByRole('status').textContent(), '')
  await page.getByRole('status').filter({ hasText: 'Verifying' }).waitFor()
  assert.deepEqual(await page.locator('.otp-slots').boundingBox(), beforeRetry)
  assert.equal(await page.getByRole('alert').textContent(), 'Invalid code')
  assert.equal(await input.evaluate((element) => document.activeElement === element), true)
  assert.equal(await input.getAttribute('readonly'), '')
  releaseVerification()
  holdVerification = undefined
  await page.getByRole('status').filter({ hasText: 'Verifying' }).waitFor({ state: 'hidden' })
  assert.equal(await page.getByRole('alert').getAttribute('data-previous-attempt'), null)
  await page
    .getByRole('button', { name: 'Use a recovery code', exact: true })
    .click()
  const recovery = page.getByRole('textbox', {
    name: 'Recovery code',
    exact: true,
  })
  assert.equal(await page.locator('.otp-slot').count(), 8)
  assert.equal(await page.locator('.otp-group').count(), 2)
  assert.equal(await page.getByRole('alert').count(), 0)
  await page.evaluate(() => navigator.clipboard.writeText('1234 5678'))
  const backupVerified = page.waitForResponse((response) =>
    response.url().endsWith('/auth/mfa/verify'),
  )
  await recovery.press('ControlOrMeta+V')
  await backupVerified
  await page.getByRole('alert').waitFor()
  assert.equal(submitted, '12345678')
  for (const width of [320, 375]) {
    await page.setViewportSize({ width, height: 667 })
    await page.waitForTimeout(100)
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    )
  }
  await page
    .getByRole('button', { name: 'Use a one-time code', exact: true })
    .click()
  assert.equal(await input.inputValue(), '')
  for (const width of [320, 375, 1440]) {
    await page.setViewportSize({ width, height: 667 })
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    )
  }
  assert.deepEqual(errors, [])
  console.log(
    'Passed: six slots, typing, backspace, formatted paste, form submission, recovery switch, mobile layout.',
  )
} finally {
  await browser.close()
}
