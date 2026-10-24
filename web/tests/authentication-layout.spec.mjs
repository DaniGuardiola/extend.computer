import { testOrigin } from './test-config.mjs'
const origin = testOrigin
import { test } from './browser-fixtures.mjs'
import assert from 'node:assert/strict'

test(
  'authentication-layout',
  { tag: '@ui' },
  async ({ newContext, newPage }) => {
    const page = await newPage()
    let credentials = [
      {
        id: 'login-key',
        name: 'Password manager',
        purpose: 'login',
        rp_id: 'localhost',
        created_at: 1791030000,
      },
      {
        id: 'factor-key',
        name: 'Backup USB key',
        purpose: 'factor',
        rp_id: 'localhost',
        created_at: 1791030000,
      },
    ]
    let factors = {
      enabled: true,
      totp: true,
      keys: 2,
      recovery_codes: 10,
      totp_available: true,
    }
    await page.route('**/api/v1/**', async (route) => {
      const path = new URL(route.request().url()).pathname
      const data = path.endsWith('/account')
        ? {
            id: 'preview',
            email: 'review@example.invalid',
            email_verified: 1,
            mfa_enabled: 1,
          }
        : path.endsWith('/devices')
          ? { devices: [] }
          : path.endsWith('/passkeys')
            ? { passkeys: credentials }
            : path.endsWith('/sessions')
              ? { sessions: [] }
              : factors
      await route.fulfill({ status: 200, json: data })
    })
    await page.goto(origin + '/account')
    await page.getByRole('heading', { name: 'Password manager' }).waitFor()
    await page.waitForLoadState('networkidle')
    const signIn = page.getByRole('region', { name: 'Sign-in methods' })
    const mfa = page.getByRole('region', { name: 'Two-factor authentication' })
    const keys = page.getByRole('region', {
      name: 'Passkeys and security keys',
    })
    assert.equal(
      await signIn.getByRole('heading', { name: 'Password manager' }).count(),
      0,
    )
    assert.equal(
      await mfa.getByRole('heading', { name: 'Backup USB key' }).count(),
      0,
    )
    assert.equal(
      await keys.getByRole('heading', { name: 'Password manager' }).count(),
      1,
    )
    assert.equal(
      await keys.getByRole('heading', { name: 'Backup USB key' }).count(),
      1,
    )
    assert.equal(
      await page.getByRole('heading', { name: 'Password manager' }).count(),
      1,
    )
    assert.equal(
      await page.getByRole('heading', { name: 'Backup USB key' }).count(),
      1,
    )
    await signIn
      .getByText('1 available for sign-in without your password.')
      .waitFor()
    await mfa.getByText('2 available to verify password sign-ins.').waitFor()
    assert.equal(
      await signIn
        .getByRole('link', { name: 'Manage passkeys' })
        .getAttribute('href'),
      '#credentials-title',
    )
    assert.equal(
      await mfa.getByRole('link', { name: 'Manage keys' }).getAttribute('href'),
      '#credentials-title',
    )
    for (const width of [320, 1024]) {
      await page.setViewportSize({ width, height: 900 })
      assert.equal(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
        true,
      )
      for (const [label, title, confirm, description] of [
        [
          'Remove passkey: Password manager',
          'Remove Password manager?',
          'Remove passkey',
          'You will no longer be able to sign in with this passkey.',
        ],
        [
          'Remove security key: Backup USB key',
          'Remove Backup USB key?',
          'Remove security key',
          'This security key will no longer verify password sign-ins.',
        ],
      ]) {
        const button = page.getByRole('button', { name: label, exact: true })
        assert.equal(await button.count(), 1)
        await button.click()
        const dialog = page.getByRole('dialog', { name: title })
        await dialog.waitFor()
        assert.ok((await dialog.textContent()).includes(description))
        assert.ok(
          (await dialog.textContent()).includes(
            'Two-factor authentication stays on',
          ),
        )
        await dialog
          .getByRole('button', { name: confirm, exact: true })
          .waitFor()
        await dialog
          .getByRole('button', { name: 'Cancel', exact: true })
          .click()
      }
    }
    for (const purpose of ['login', 'factor']) {
      credentials = [
        {
          id: 'last-key',
          name: 'Last key',
          purpose,
          rp_id: 'localhost',
          created_at: 1791030000,
        },
      ]
      factors = { ...factors, totp: false, keys: 1 }
      await page.reload()
      await page.getByRole('heading', { name: 'Last key' }).waitFor()
      await page.waitForLoadState('networkidle')
      await page
        .getByRole('button', {
          name: `Remove ${purpose === 'factor' ? 'security key' : 'passkey'}: Last key`,
          exact: true,
        })
        .click()
      const dialog = page.getByRole('dialog', { name: 'Remove Last key?' })
      await dialog.waitFor()
      assert.ok(
        (await dialog.textContent()).includes(
          'Removing it will turn off two-factor authentication and invalidate your recovery codes.',
        ),
      )
      await dialog
        .getByRole('button', {
          name: `Remove ${purpose === 'factor' ? 'key' : 'passkey'} and turn off 2FA`,
          exact: true,
        })
        .waitFor()
      await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
    }
    console.log(
      'Credentials appear once in the separate section; removal copy and last-method warnings match their purpose on mobile and desktop.',
    )
  },
)
