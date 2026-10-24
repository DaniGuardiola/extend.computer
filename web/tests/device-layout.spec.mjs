import { testOrigin } from './test-config.mjs'
import { installAccountFixture } from './account-fixtures.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test('device-layout', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  await installAccountFixture(page, 'devices')
  await page.goto(origin + '/account')
  await page.getByRole('heading', { name: 'MacBook Pro' }).waitFor()
  await page.waitForLoadState('networkidle')
  for (const width of [292, 320, 375, 600, 768, 1024]) {
    await page.setViewportSize({ width, height: 900 })
    const rows = await page.locator('.device-row').evaluateAll((elements) =>
      elements.map((row) => {
        const detail = row
          .querySelector('.device-detail')
          .getBoundingClientRect()
        const status = row
          .querySelector('.device-status')
          .getBoundingClientRect()
        return {
          detailWidth: detail.width,
          detailBottom: detail.bottom,
          statusTop: status.top,
        }
      }),
    )
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    )
    for (const row of rows) {
      assert.ok(row.detailWidth >= 100, `Text column too narrow at ${width}px`)
      if (width <= 600) assert.ok(row.statusTop >= row.detailBottom)
    }
    for (const fingerprint of ['8e42c7b91af0', '3b7f09a2d861']) {
      await page.getByText(fingerprint, { exact: true }).focus()
      const tooltip = page.getByRole('tooltip')
      await tooltip.waitFor()
      await page.waitForTimeout(200)
      await page.waitForFunction(() => {
        const rect = [...document.querySelectorAll('[role="tooltip"]')]
          .map((el) => el.getBoundingClientRect())
          .find((rect) => rect.width > 0)
        return rect && rect.left >= 15 && rect.right <= innerWidth - 15
      })
      await page.keyboard.press('Escape')
      await tooltip.waitFor({ state: 'hidden' })
    }
  }
  await page.unroute('**/api/v1/**')
  await installAccountFixture(page, 'passkeys')
  await page.goto(origin + '/account')
  await page.getByRole('heading', { name: '1Password' }).waitFor()
  await page.waitForLoadState('networkidle')
  const usage = page.getByRole('checkbox', {
    name: 'Allow sign-in without password: 1Password',
    exact: true,
  })
  await usage.click()
  await page
    .getByRole('button', { name: 'Turn off sign-in', exact: true })
    .click()
  await page.getByLabel('Current password').fill('test-password')
  await page.getByRole('button', { name: 'Continue', exact: true }).click()
  await page.getByRole('dialog').waitFor({ state: 'hidden' })
  assert.equal(await usage.isChecked(), false)
  await page
    .getByText('1 available for sign-in without your password.')
    .waitFor()
  await page.getByText('3 available to verify password sign-ins.').waitFor()
  await usage.click()
  await page
    .getByRole('button', { name: 'Turn on sign-in', exact: true })
    .click()
  await page.getByLabel('Current password').fill('test-password')
  await page.getByRole('button', { name: 'Continue', exact: true }).click()
  await page.getByRole('dialog').waitFor({ state: 'hidden' })
  assert.equal(await usage.isChecked(), true)
  await page
    .getByText('2 available for sign-in without your password.')
    .waitFor()
  for (const width of [320, 1024]) {
    await page.setViewportSize({ width, height: 900 })
    const remove = page.getByRole('button', {
      name: 'Remove passkey: 1Password',
      exact: true,
    })
    assert.equal(await remove.count(), 1)
    assert.equal(
      (await remove.textContent()).trim(),
      width === 320 ? 'Remove passkey' : '',
    )
    if (width === 1024) {
      await page.keyboard.press('Tab')
      await remove.focus()
      await page
        .getByRole('tooltip', { name: 'Remove passkey: 1Password' })
        .waitFor()
    }
    await remove.click()
    await page.getByRole('dialog', { name: 'Remove 1Password?' }).waitFor()
    await page.getByRole('button', { name: 'Cancel', exact: true }).click()
  }
  console.log('Device text stays readable and status moves below it on mobile.')
})
