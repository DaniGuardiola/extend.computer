import { testOrigin } from './test-config.mjs'
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

const origin = testOrigin
test('account-refresh', { tag: '@ui' }, async ({ newContext, newPage }) => {
  const page = await newPage()
  let hold
  let factors = { enabled: false, totp: false, keys: 0, recovery_codes: 0 }
  await page.route('**/api/v1/**', async (route) => {
    if (hold) await hold
    const path = new URL(route.request().url()).pathname
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
          ? {
              passkeys: factors.keys
                ? [
                    {
                      id: 'key',
                      name: 'Test key',
                      purpose: 'factor',
                      rp_id: 'localhost',
                      created_at: 1791030000,
                    },
                  ]
                : [],
            }
          : path.endsWith('/sessions')
            ? {
                sessions: [
                  {
                    id: 'current',
                    current: 1,
                    client: 'Chrome',
                    platform: 'macOS',
                    location: 'London, GB',
                    created_at: 1791030000,
                    last_access_at: 1791030000,
                  },
                ],
              }
            : factors
    await route.fulfill({ status: 200, json: data })
  })
  await page.goto(origin + '/account')
  await page.getByRole('heading', { name: 'No devices registered' }).waitFor()
  await page.waitForLoadState('networkidle')
  const signOut = page.getByRole('button', { name: 'Sign out', exact: true })
  const addPasskey = page.getByRole('button', { name: 'Add key', exact: true })
  const refresh = page.locator('.account-refresh')
  const before = await addPasskey.boundingBox()
  let release
  hold = new Promise((resolve) => {
    release = resolve
  })
  const startedAt = Date.now()
  const idleWidth = (await refresh.boundingBox()).width
  await refresh.click()
  assert.equal(await refresh.getAttribute('aria-busy'), 'true')
  assert.equal(await signOut.isEnabled(), true)
  assert.equal(await addPasskey.isEnabled(), true)
  assert.deepEqual(await addPasskey.boundingBox(), before)
  assert.equal((await refresh.textContent()).trim(), 'Refreshing…')
  release()
  hold = undefined
  await page.waitForFunction(() =>
    document.querySelector('[aria-busy="false"]'),
  )
  assert.ok(Date.now() - startedAt >= 2400)
  const confirmedAt = Date.now()
  assert.ok((await refresh.boundingBox()).width > idleWidth)
  assert.equal(await refresh.locator('svg.lucide-check').count(), 1)
  assert.equal((await refresh.textContent()).trim(), 'Refreshed!')
  assert.equal(await refresh.isDisabled(), true)
  await page.getByRole('button', { name: 'Refresh', exact: true }).waitFor()
  assert.ok(Date.now() - confirmedAt >= 3500)
  assert.equal(await refresh.isEnabled(), true)
  assert.deepEqual(await addPasskey.boundingBox(), before)
  for (const width of [320, 630, 768, 900, 1024]) {
    await page.setViewportSize({ width, height: 988 })
    assert.equal(await page.locator('.account-email').isVisible(), true)
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
    )
    const brand = await page.locator('.site-header .brand').boundingBox()
    const email = await page.locator('.account-email').boundingBox()
    const logout = await signOut.boundingBox()
    assert.ok(
      email.y >= brand.y + brand.height || email.x >= brand.x + brand.width,
    )
    assert.ok(logout.x >= brand.x + brand.width)
    assert.equal(
      await signOut.evaluate((el) => getComputedStyle(el).whiteSpace),
      'nowrap',
    )
  }
  await page.setViewportSize({ width: 889, height: 1314 })
  await page.keyboard.press('Tab')
  await page.locator('.session-location').focus()
  await page.getByRole('tooltip').waitFor()
  assert.match(
    await page.getByRole('tooltip').textContent(),
    /Estimated from this session’s IP address/,
  )
  await page.getByRole('button', { name: 'Sign out all', exact: true }).click()
  await page.getByRole('dialog', { name: 'Sign out all sessions?' }).waitFor()
  await page.getByRole('button', { name: 'Cancel', exact: true }).click()
  await page
    .getByRole('button', { name: 'Sign out this session', exact: true })
    .click()
  await page.getByRole('dialog', { name: 'Sign out this session?' }).waitFor()
  await page.getByRole('button', { name: 'Cancel', exact: true }).click()
  assert.equal(
    await page
      .getByRole('region', { name: 'Passkeys and security keys' })
      .getByRole('button', { name: 'Add key', exact: true })
      .count(),
    1,
  )
  for (const [totp, keys, description] of [
    [true, 0, 'Password sign-ins require a one-time code.'],
    [false, 1, 'Password sign-ins require a security key.'],
    [true, 1, 'Password sign-ins require a one-time code or a security key.'],
  ]) {
    factors = { enabled: true, totp, keys, recovery_codes: 10 }
    await page.reload()
    await page.getByText(description, { exact: true }).waitFor()
    assert.equal(
      await page.getByRole('button', { name: 'Turn off', exact: true }).count(),
      0,
    )
    if (totp && !keys) {
      await page.getByRole('button', { name: 'On', exact: true }).focus()
      await page.getByRole('tooltip').waitFor()
      assert.match(
        await page.getByRole('tooltip').textContent(),
        /Remove all verification methods/,
      )
      await page.getByRole('button', { name: 'Remove', exact: true }).click()
      await page.getByText(/This is your last verification method/).waitFor()
      await page.getByRole('button', { name: 'Cancel', exact: true }).click()
    }
    if (!totp && keys) {
      await page
        .getByRole('button', {
          name: 'Remove security key: Test key',
          exact: true,
        })
        .click()
      await page.getByText(/This is your last verification method/).waitFor()
      await page.getByRole('button', { name: 'Cancel', exact: true }).click()
    }
  }
  console.log(
    'Refresh timing, dynamic width, confirmation checkmark, and surrounding layout passed.',
  )
})
