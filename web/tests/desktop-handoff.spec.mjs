import { testOrigin } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test('desktop-handoff', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  let signedIn = false
  let sessionEmail = 'test@example.invalid'
  let authorization
  const state = '0'.repeat(64)
  const challenge = 'A'.repeat(43)
  const request = `${origin}/desktop/connect?port=54321&state=${state}&challenge=${challenge}`
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/account'))
      return route.fulfill({
        status: signedIn ? 200 : 401,
        json: signedIn
          ? { id: sessionEmail, email: sessionEmail, email_verified: 1, mfa_enabled: 0 }
          : { error: 'Please sign in' },
      })
    if (path.endsWith('/auth/login')) {
      signedIn = true
      sessionEmail = route.request().postDataJSON().email
      return route.fulfill({ json: { account: { id: 'test' } } })
    }
    if (path.endsWith('/auth/logout')) {
      signedIn = false
      return route.fulfill({ json: { success: true } })
    }
    if (path.endsWith('/auth/desktop/authorize')) {
      authorization = route.request().postDataJSON()
      return route.fulfill({ json: { code: 'test-code' } })
    }
    throw new Error('Unexpected API request: ' + path)
  })
  await page.route('http://127.0.0.1:54321/**', (route) =>
    route.fulfill({ body: 'App callback received' }),
  )
  await page.goto(request)
  await page.getByRole('heading', { name: 'Sign in', exact: true }).waitFor()
  assert.equal(page.url(), request)
  await page.getByLabel('Email', { exact: true }).fill('test@example.invalid')
  await page.getByLabel('Password', { exact: true }).fill('test-password')
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page
    .getByRole('button', { name: 'Sign in to app', exact: true })
    .waitFor()
  assert.equal(page.url(), request)
  await page
    .getByRole('button', { name: 'Use another account', exact: true })
    .click()
  await page.getByRole('heading', { name: 'Sign in', exact: true }).waitFor()
  await page.getByLabel('Email', { exact: true }).fill('second@example.invalid')
  await page.getByLabel('Password', { exact: true }).fill('test-password')
  await page.getByRole('button', { name: 'Sign in', exact: true }).click()
  await page
    .getByText('Signed in as second@example.invalid.', { exact: true })
    .waitFor()
  assert.equal(
    await page
      .getByText('Signed in as test@example.invalid.', { exact: true })
      .count(),
    0,
  )
  assert.equal(page.url(), request)
  await page
    .getByRole('button', { name: 'Sign in to app', exact: true })
    .click()
  await page.waitForURL('http://127.0.0.1:54321/callback?**')
  const callback = new URL(page.url())
  assert.equal(callback.searchParams.get('state'), state)
  assert.equal(callback.searchParams.get('code'), 'test-code')
  assert.deepEqual(authorization, { challenge })
  console.log(
    'Signed-out app handoff: login, account switching, authorization and callback passed',
  )
})
