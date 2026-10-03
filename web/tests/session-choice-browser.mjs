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
  await page.goto(origin + '/signup')
  await page.waitForLoadState('networkidle')
  await page
    .getByRole('textbox', { name: 'Email', exact: true })
    .fill(`choice-${Date.now()}@example.invalid`)
  await page
    .getByRole('textbox', { name: 'Password', exact: true })
    .fill('disposable-choice-password')
  await page
    .getByRole('button', { name: 'Create account', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Devices', exact: true }).waitFor()
  assert.equal(await page.getByRole('checkbox').count(), 0)
  await page.getByRole('button', { name: 'Change', exact: true }).click()
  const dialog = page.getByRole('dialog', {
    name: 'Change password',
    exact: true,
  })
  const choice = dialog.getByRole('checkbox', {
    name: /Sign out other sessions/,
  })
  assert.equal(await choice.isChecked(), false)
  await choice.check()
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('button', { name: 'Change', exact: true }).click()
  assert.equal(await choice.isChecked(), false)
  await page.screenshot({ path: '/tmp/extend-session-choice.png' })
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByRole('button', { name: 'Set up', exact: true }).click()
  const verify = page.getByRole('dialog', {
    name: 'Confirm security change',
    exact: true,
  })
  await verify
    .getByRole('textbox', { name: 'Current password', exact: true })
    .fill('disposable-choice-password')
  await verify.getByRole('button', { name: 'Continue', exact: true }).click()
  const setup = page.getByRole('dialog', {
    name: 'Set up one-time codes',
    exact: true,
  })
  await setup.getByRole('button', { name: 'Next', exact: true }).click()
  assert.equal(await setup.getByRole('checkbox').count(), 0)
  assert.deepEqual(errors, [])
  console.log(
    'Passed: accessible password sign-out choice, unchecked/reset defaults, no option for authenticator addition, no browser errors.',
  )
} finally {
  await browser.close()
}
