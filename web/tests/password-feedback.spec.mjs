import { testOrigin } from './test-config.mjs'
const origin = testOrigin
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

test('password-feedback', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  await page.route('**/api/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname
    if (path.endsWith('/account') && new URL(page.url()).pathname === '/signup')
      return route.fulfill({ status: 401, json: { error: 'Please sign in' } })
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
            ? { sessions: [] }
            : { enabled: false, totp: false, keys: 0, recovery_codes: 0 }
    return route.fulfill({ json: data })
  })
  let sharedStyle
  for (const path of [
    '/signup',
    '/email#reset=' + '0'.repeat(64),
    '/account',
  ]) {
    await page.goto(origin + path)
    await page.waitForLoadState('networkidle')
    if (path === '/account')
      await page.getByRole('button', { name: 'Change', exact: true }).click()
    const input = page.locator('input[autocomplete="new-password"]')
    await input.waitFor()
    const feedback = input.locator('xpath=following-sibling::span')
    assert.equal(await feedback.textContent(), 'At least 12 characters.')
    const style = await feedback.evaluate((el) => {
      const css = getComputedStyle(el)
      return {
        height: el.getBoundingClientRect().height,
        font: css.fontSize,
        line: css.lineHeight,
        color: css.color,
      }
    })
    if (sharedStyle) assert.deepEqual(style, sharedStyle)
    sharedStyle = style
    await input.fill('short')
    await input.blur()
    await page.waitForFunction(
      () =>
        document.querySelector('input[autocomplete="new-password"]')
          .nextElementSibling?.textContent === 'Use at least 12 characters.',
    )
    assert.equal(await feedback.textContent(), 'Use at least 12 characters.')
    assert.equal(
      await feedback.evaluate((el) => el.getBoundingClientRect().height),
      style.height,
    )
    await input.fill('abcdefghijkl')
    assert.equal(await feedback.textContent(), 'Nice, that works!')
    await input.fill('')
    assert.equal(await feedback.textContent(), 'At least 12 characters.')
    await input.evaluate((el) => el.form.requestSubmit())
    assert.equal(await feedback.textContent(), 'Enter a password.')
    await input.fill('abcdefghijkl')
    assert.equal(await feedback.textContent(), 'Nice, that works!')
    assert.equal(
      await feedback.evaluate((el) => el.getBoundingClientRect().height),
      style.height,
    )
    if (path === '/account') {
      for (const dismiss of ['cancel', 'escape']) {
        await input.fill('')
        await input.evaluate((el) => el.form.requestSubmit())
        assert.equal(await feedback.textContent(), 'Enter a password.')
        if (dismiss === 'cancel')
          await page
            .getByRole('button', { name: 'Cancel', exact: true })
            .click()
        else await page.keyboard.press('Escape')
        await page
          .getByRole('dialog', { name: 'Change password' })
          .waitFor({ state: 'hidden' })
        await page.getByRole('button', { name: 'Change', exact: true }).click()
        assert.equal(await input.inputValue(), '')
        assert.equal(await feedback.textContent(), 'At least 12 characters.')
        assert.equal(
          await page.getByRole('dialog').getByRole('alert').count(),
          0,
        )
      }
    }
  }
  console.log(
    'Signup, reset, and password change share hint, validation, success feedback, and stable feedback geometry.',
  )
})
