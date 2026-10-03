import { chromium } from 'playwright'
import assert from 'node:assert/strict'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3006'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = await chromium.launch({ headless: true })
try {
  const context = await browser.newContext({
    viewport: { width: 1280, height: 900 },
  })
  const signup = await context.request.post(origin + '/api/v1/auth/signup', {
    headers: { Origin: origin },
    data: {
      email: `loading-${Date.now()}@example.invalid`,
      password: 'disposable-loading-password',
    },
  })
  assert.equal(signup.status(), 201)
  let release
  let gate = new Promise((resolve) => {
    release = resolve
  })
  const requests = {}
  await context.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname
    requests[path] = (requests[path] ?? 0) + 1
    await gate
    await route.continue()
  })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto(origin + '/account')
  await page.locator('.account-loading').waitFor({ state: 'attached' })
  assert.equal(
    await page
      .locator('.account-loading')
      .evaluate((el) => getComputedStyle(el).opacity),
    '0',
  )
  await page.waitForFunction(
    () =>
      getComputedStyle(document.querySelector('.account-loading')).opacity ===
      '1',
  )
  for (const name of [
    'Devices',
    'Two-factor authentication',
    'Sign-in methods',
    'Sessions',
  ])
    assert.equal(
      await page.getByRole('heading', { name, exact: true }).count(),
      1,
    )
  await page.screenshot({
    path: '/tmp/extend-account-loading.png',
    fullPage: true,
  })
  release()
  await page
    .getByRole('heading', { name: 'No devices registered', exact: true })
    .waitFor()
  await page.getByText('This session', { exact: true }).waitFor()
  assert.equal(
    await page
      .locator('.account-content')
      .evaluate((el) => getComputedStyle(el).animationName),
    'account-fade-in',
  )
  await page.waitForLoadState('networkidle')
  for (const endpoint of ['account', 'devices', 'passkeys', 'mfa', 'sessions'])
    assert.equal(
      requests['/api/v1/' + endpoint],
      1,
      endpoint + ' must load once',
    )
  gate = new Promise((resolve) => {
    release = resolve
  })
  await page.getByRole('button', { name: 'Refresh', exact: true }).click()
  await page.getByRole('button', { name: 'Refreshing…', exact: true }).waitFor()
  assert.equal(await page.locator('.skeleton-devices').count(), 0)
  assert.equal(await page.getByText('This session', { exact: true }).count(), 1)
  release()
  await page.getByRole('button', { name: 'Refresh', exact: true }).waitFor()
  await page.setViewportSize({ width: 390, height: 844 })
  gate = new Promise((resolve) => {
    release = resolve
  })
  await page.reload()
  await page.locator('.account-loading').waitFor({ state: 'attached' })
  assert.equal(
    await page
      .locator('.account-loading')
      .evaluate((el) => getComputedStyle(el).opacity),
    '0',
  )
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
    true,
  )
  release()
  await page.getByText('This session', { exact: true }).waitFor()
  assert.deepEqual(errors, [])
  console.log(
    'Passed: complete loading scaffold, parallel requests without duplicate loads, delayed skeleton fade, content fade, stable refresh, mobile skeleton layout.',
  )
} finally {
  await browser.close()
}
