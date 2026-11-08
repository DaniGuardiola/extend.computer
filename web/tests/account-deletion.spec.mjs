import { testOrigin } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import { installAccountFixture } from './account-fixtures.mjs'
import assert from 'node:assert/strict'

async function openAccount(page, deletion) {
  const calls = []
  let deleted = false
  await installAccountFixture(page)
  // Registered last, so it runs before the shared fixture.
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request()
    const path = new URL(request.url()).pathname
    if (path.endsWith('/mfa/reauth') || path.endsWith('/account'))
      calls.push(request.method() + ' ' + path.replace('/api/v1', ''))
    if (path.endsWith('/account') && request.method() === 'DELETE') {
      const result = deletion(request.postDataJSON())
      deleted = result.status === 204
      return route.fulfill(result)
    }
    if (deleted && path.endsWith('/account'))
      return route.fulfill({ status: 401, json: { error: 'Please sign in' } })
    return route.fallback()
  })
  await page.goto(testOrigin + '/account')
  await page
    .getByRole('heading', { name: 'Delete account', exact: true })
    .waitFor()
  return calls
}

async function confirmDeletion(page, email) {
  await page
    .getByRole('button', { name: 'Delete account', exact: true })
    .click()
  const dialog = page.getByRole('dialog', { name: 'Delete your account?' })
  const submit = dialog.getByRole('button', {
    name: 'Delete account',
    exact: true,
  })
  const input = dialog.getByRole('textbox', {
    name: 'Enter your email to confirm',
  })
  assert.equal(await submit.isDisabled(), true)
  await input.fill('someone@example.invalid')
  assert.equal(await submit.isDisabled(), true)
  await input.fill(email)
  assert.equal(await submit.isEnabled(), true)
  await submit.click()
  await dialog.waitFor({ state: 'hidden' })
  const verify = page.getByRole('dialog', { name: 'Confirm security change' })
  await verify.getByLabel('Current password').fill('test-password')
  await verify.getByRole('button', { name: 'Continue', exact: true }).click()
  return verify
}

test('account-deletion', { tag: '@ui' }, async ({ newPage }) => {
  const page = await newPage()
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  const bodies = []
  const calls = await openAccount(page, (body) => {
    bodies.push(body)
    return { status: 204 }
  })
  // The typed confirmation resets when the dialog closes.
  await page
    .getByRole('button', { name: 'Delete account', exact: true })
    .click()
  const dialog = page.getByRole('dialog', { name: 'Delete your account?' })
  await dialog
    .getByRole('textbox', { name: 'Enter your email to confirm' })
    .fill('review@example.invalid')
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page
    .getByRole('button', { name: 'Delete account', exact: true })
    .click()
  assert.equal(
    await dialog
      .getByRole('textbox', { name: 'Enter your email to confirm' })
      .inputValue(),
    '',
  )
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  await dialog.waitFor({ state: 'hidden' })

  await confirmDeletion(page, ' Review@Example.invalid ')
  await page.waitForURL(/\/login\?reason=account-deleted$/)
  await page
    .getByRole('status')
    .filter({ hasText: 'Your account was deleted.' })
    .waitFor()
  await page.getByRole('heading', { name: 'Sign in', exact: true }).waitFor()
  assert.deepEqual(bodies, [{ email: 'Review@Example.invalid' }])
  // Fresh verification comes before the deletion request.
  assert.deepEqual(
    calls.filter((call) => call !== 'GET /account'),
    ['POST /mfa/reauth', 'DELETE /account'],
  )
  assert.deepEqual(errors, [])
})

test('account-deletion-refused', { tag: '@ui' }, async ({ newPage }) => {
  const page = await newPage()
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  await openAccount(page, () => ({
    status: 403,
    json: { error: 'Security settings changed. Verify again.' },
  }))
  const verify = await confirmDeletion(page, 'review@example.invalid')
  await verify
    .getByRole('alert')
    .filter({ hasText: 'Security settings changed. Verify again.' })
    .waitFor()
  assert.match(page.url(), /\/account$/)
  await verify.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page.getByText('review@example.invalid').first().waitFor()
  assert.deepEqual(errors, [])
})
