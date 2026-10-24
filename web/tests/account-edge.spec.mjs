import { testOrigin } from './test-config.mjs'
import {
  installAccountFixture,
  reachAccountState,
} from './account-fixtures.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test('account-edge', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  const errors = []
  page.on('pageerror', (error) => errors.push(error.message))
  let mode = 'server'
  let release
  let held
  let secondFactor = false
  let sessionExpired = false
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname
    if (sessionExpired && path.endsWith('/account'))
      return route.fulfill({ status: 401, json: { error: 'Please sign in' } })
    if (path.endsWith('/auth/mfa/verify')) {
      if (held) await held
      return route.fulfill({
        status: 401,
        json: { error: 'Invalid verification code' },
      })
    }
    if (path.endsWith('/passkeys/register/options')) {
      if (held) await held
      if (mode === 'offline') return route.abort('failed')
      if (mode === 'html')
        return route.fulfill({
          status: 502,
          contentType: 'text/html',
          body: '<h1>Bad gateway</h1>',
        })
      if (mode === 'expired') {
        sessionExpired = true
        return route.fulfill({ status: 401, json: { error: 'Please sign in' } })
      }
      return route.fulfill({
        status: 503,
        json: { error: 'Could not add your key. Please try again.' },
      })
    }
    const data = path.endsWith('/account')
      ? {
          id: 'preview',
          email: 'review@example.invalid',
          email_verified: 1,
          email_enabled: true,
          mfa_enabled: 0,
        }
      : path.endsWith('/devices')
        ? { devices: [] }
        : path.endsWith('/passkeys')
          ? { passkeys: [] }
          : path.endsWith('/sessions')
            ? {
                sessions: [
                  {
                    id: 'current',
                    current: 1,
                    client: 'Chrome',
                    platform: 'macOS',
                    location: null,
                    created_at: 1791030000,
                    last_access_at: 1791030000,
                  },
                ],
              }
            : path.endsWith('/mfa/reauth')
              ? secondFactor
                ? {
                    mfa_required: true,
                    ticket: 'test',
                    totp: true,
                    keys: false,
                    recovery: true,
                  }
                : { success: true }
              : {
                  enabled: false,
                  totp: false,
                  keys: 0,
                  recovery_codes: 0,
                  totp_available: true,
                }
    return route.fulfill({ json: data })
  })
  await page.goto(origin + '/account')
  await page.getByRole('button', { name: 'Add key', exact: true }).waitFor()
  const dialog = page.getByRole('dialog', { name: 'Confirm security change' })
  for (const [nextMode, expected] of [
    ['server', 'Could not add your key. Please try again.'],
    ['offline', 'Could not connect. Check your connection and try again.'],
    ['html', 'Could not complete the request. Please try again.'],
    ['expired', 'Your session ended. Sign in again to continue.'],
  ]) {
    mode = nextMode
    await page.getByRole('button', { name: 'Add key', exact: true }).click()
    await dialog.getByLabel('Current password').fill('test-password')
    if (mode === 'server')
      held = new Promise((resolve) => {
        release = resolve
      })
    await dialog.getByRole('button', { name: 'Continue', exact: true }).click()
    if (mode === 'server') {
      await page.waitForTimeout(150)
      assert.equal(
        await dialog
          .getByRole('button', { name: 'Continue', exact: true })
          .isDisabled(),
        true,
      )
      await page.keyboard.press('Escape')
      assert.equal(await dialog.isVisible(), true)
      release()
      held = undefined
    }
    if (mode === 'expired') {
      await page.waitForURL(origin + '/login?reason=session-ended')
      await page
        .getByRole('alert')
        .filter({ hasText: 'Your session ended.' })
        .waitFor()
      await page.getByRole('button', { name: 'Sign in', exact: true }).waitFor()
      sessionExpired = false
      await page.goto(origin + '/account')
      continue
    }
    await dialog.getByRole('alert').filter({ hasText: expected }).waitFor()
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  }
  secondFactor = true
  await page.getByRole('button', { name: 'Add key', exact: true }).click()
  await dialog.getByLabel('Current password').fill('test-password')
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click()
  held = new Promise((resolve) => {
    release = resolve
  })
  await dialog
    .getByRole('textbox', { name: 'One-time code', exact: true })
    .fill('123456')
  await page.waitForTimeout(150)
  await page.keyboard.press('Escape')
  assert.equal(
    await dialog.isVisible(),
    true,
    'Second-factor request must keep dialog open while pending',
  )
  release()
  held = undefined
  await dialog
    .getByRole('alert')
    .filter({ hasText: 'Invalid verification code' })
    .waitFor()
  await page.keyboard.press('Escape')
  await dialog.waitFor({ state: 'hidden' })
  secondFactor = false
  for (const state of [
    'devices',
    'passkeys',
    'sessions',
    'code-setup',
    'recovery-codes',
  ]) {
    for (const width of [320, 600, 1069]) {
      await page.setViewportSize({ width, height: 900 })
      await page.unroute('**/api/v1/**')
      await installAccountFixture(page, state)
      await page.goto(origin + '/account')
      await reachAccountState(page, state)
      await page
        .getByRole('heading', { name: 'Account', exact: true })
        .waitFor()
      assert.equal(
        await page.evaluate(
          () => document.documentElement.scrollWidth > innerWidth,
        ),
        false,
        `${state}: overflow at ${width}px`,
      )
      if (state === 'code-setup' || state === 'recovery-codes') {
        await page.getByRole('dialog').waitFor()
        await page.keyboard.press('Tab')
        assert.equal(
          await page
            .getByRole('dialog')
            .evaluate((el) => el.contains(document.activeElement)),
          true,
        )
        await page.keyboard.press('Escape')
        await page.getByRole('dialog').waitFor({ state: 'hidden' })
      }
    }
  }
  await page.unroute('**/api/v1/**')
  await installAccountFixture(page)
  await page.goto(origin + '/account')
  const add = page.getByRole('button', { name: 'Add key', exact: true })
  await add.click()
  await dialog.getByLabel('Current password').waitFor()
  assert.equal(await dialog.getByLabel('Current password').inputValue(), '')
  assert.equal(await dialog.getByRole('alert').count(), 0)
  await page.keyboard.press('Escape')
  await dialog.waitFor({ state: 'hidden' })
  assert.equal(await add.evaluate((el) => el === document.activeElement), true)
  assert.deepEqual(errors, [])
  console.log(
    'Pending controls, retryable failures, session expiry, mobile layouts, keyboard focus and dialog reset passed.',
  )
})
