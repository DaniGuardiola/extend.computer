// Deterministic API fixtures for browser tests. Never imported by the app.
export const passkeys = (now) => [
  {
    id: 'dev-preview-password-manager',
    name: '1Password',
    created_at: now - 86400 * 7,
    rp_id: 'localhost',
    purpose: 'login',
  },
  {
    id: 'dev-preview-touch-id',
    name: 'MacBook Pro',
    created_at: now - 86400,
    rp_id: 'localhost',
    purpose: 'login',
  },
  {
    id: 'dev-preview-u2f-key',
    name: 'Backup USB security key',
    created_at: now - 86400 * 30,
    rp_id: 'localhost',
    purpose: 'factor',
  },
]

export const sessions = (now) => [
  {
    id: 'dev-preview-current-browser',
    client: 'Chrome',
    platform: 'macOS',
    location: null,
    current: 1,
    created_at: now - 86400,
    last_access_at: now,
  },
  {
    id: 'dev-preview-phone-browser',
    client: 'Safari',
    platform: 'iOS',
    location: 'Madrid, Spain',
    current: 0,
    created_at: now - 86400 * 5,
    last_access_at: now - 3600,
  },
  {
    id: 'dev-preview-other-browser',
    client: 'Firefox',
    platform: 'Windows',
    location: 'Barcelona, Spain',
    current: 0,
    created_at: now - 86400 * 25,
    last_access_at: now - 86400 * 18,
  },
  {
    id: 'dev-preview-desktop-app',
    device_names: ['Studio Mac'],
    client: 'Desktop app',
    platform: 'macOS',
    location: 'Madrid, Spain',
    current: 0,
    created_at: now - 86400 * 7,
    last_access_at: now - 60,
  },
  {
    id: 'dev-preview-second-desktop-app',
    device_names: ['Travel MacBook'],
    client: 'Desktop app',
    platform: 'macOS',
    location: 'Barcelona, Spain',
    current: 0,
    created_at: now - 86400 * 3,
    last_access_at: now - 7200,
  },
]

export const devices = (now) => [
  {
    id: 'dev-preview-laptop',
    name: 'MacBook Pro',
    platform: 'macos',
    fingerprint:
      '8e42c7b91af0356d2cb8e49f617a0d935cf26e8b9401a7d63e5f092c4ab17860',
    online: true,
    last_seen: now,
  },
  {
    id: 'dev-preview-desktop',
    name: 'Home desktop',
    platform: 'windows',
    fingerprint:
      '3b7f09a2d861c4e05fa923b17d68e4c290f51b7ad3068ce49a12f075de83b6c4',
    online: false,
    last_seen: now - 86400,
  },
]

export async function installAccountFixture(
  page,
  state = '',
  { guest = false, pending = '' } = {},
) {
  const now = Math.floor(new Date('2026-10-05T10:00:00Z').getTime() / 1000)
  let keys = state === 'passkeys' ? passkeys(now) : []
  if (
    state.startsWith('remove-') ||
    state === 'security-key' ||
    state === 'security-code-key'
  )
    keys = [
      {
        id: 'test-key',
        name: state === 'remove-last-passkey' ? '1Password' : 'Security key',
        created_at: now,
        rp_id: 'localhost',
        purpose: state === 'remove-last-passkey' ? 'login' : 'factor',
      },
    ]
  const factors = {
    enabled:
      state === 'passkeys' ||
      state.startsWith('remove-') ||
      state === 'recovery-codes' ||
      state === 'security-key' ||
      state === 'security-code-key',
    totp: [
      'passkeys',
      'remove-key',
      'recovery-codes',
      'security-code-key',
    ].includes(state),
    keys: keys.length,
    recovery_codes: state === 'passkeys' || state === 'recovery-codes' ? 10 : 0,
    totp_available: true,
  }
  let deviceRequests = 0
  await page.route('**/api/v1/**', async (route) => {
    const p = new URL(route.request().url()).pathname
    if (p.endsWith('/account')) {
      if (state === 'loading') return // Held until browser context closes.
      if (state === 'load-error')
        return route.fulfill({
          status: 503,
          json: {
            error: 'Could not load your account. Please try refreshing.',
          },
        })
      if (guest)
        return route.fulfill({ status: 401, json: { error: 'Please sign in' } })
      return route.fulfill({
        json: {
          id: 'test',
          email: 'review@example.invalid',
          email_verified: state.startsWith('email-unverified') ? 0 : 1,
          email_enabled: true,
          mfa_enabled: Number(factors.enabled),
        },
      })
    }
    if (p.endsWith('/devices')) {
      if (++deviceRequests > 1 && state === 'refresh-offline')
        return route.abort('failed')
      if (deviceRequests > 1 && state === 'refresh-error')
        return route.fulfill({
          status: 503,
          json: { error: 'Could not refresh your devices. Please try again.' },
        })
      return route.fulfill({
        json: { devices: state === 'devices' ? devices(now) : [] },
      })
    }
    if (p.endsWith('/passkeys'))
      return route.fulfill({ json: { passkeys: keys } })
    if (p.endsWith('/sessions'))
      return route.fulfill({
        json: { sessions: state === 'sessions' ? sessions(now) : [] },
      })
    if (p.endsWith('/mfa')) return route.fulfill({ json: factors })
    if (p.endsWith('/mfa/reauth'))
      return route.fulfill({
        json:
          state === 'security-key' || state === 'security-code-key'
            ? {
                mfa_required: true,
                ticket: 'test',
                totp: state === 'security-code-key',
                keys: true,
                recovery: true,
              }
            : { success: true },
      })
    if (p.endsWith('/mfa/totp/setup'))
      return route.fulfill({
        json: {
          secret: 'GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ',
          uri: 'otpauth://totp/extend.computer:preview?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=extend.computer&algorithm=SHA1&digits=6&period=30',
        },
      })
    if (p.endsWith('/mfa/recovery-codes'))
      return route.fulfill({
        json: {
          recovery_codes: Array.from(
            { length: 10 },
            (_, i) => `1234${String(i + 1).padStart(4, '0')}`,
          ),
        },
      })
    if (p.endsWith('/auth/email/request'))
      return route.fulfill({
        json: { retry_after: state === 'email-unverified-limit' ? 3360 : 60 },
      })
    if (/\/auth\/(login|signup)$/.test(p)) {
      if (pending) return
      return route.fulfill({
        status: 401,
        json: { error: 'Incorrect email or password.' },
      })
    }
    if (/\/passkeys\/[^/]+$/.test(p) && route.request().method() === 'PATCH') {
      const body = route.request().postDataJSON()
      keys = keys.map((k) =>
        p.endsWith('/' + k.id)
          ? { ...k, purpose: body.sign_in_enabled ? 'login' : 'login-disabled' }
          : k,
      )
    }
    return route.fulfill({ json: { success: true } })
  })
}

export async function reachAccountState(page, state) {
  if (state === 'refresh-error' || state === 'refresh-offline') {
    await page.getByRole('button', { name: 'Refresh', exact: true }).click()
    await page.getByRole('alert').waitFor()
  } else if (
    state.startsWith('remove-') ||
    state === 'security-key' ||
    state === 'security-code-key'
  ) {
    await page
      .getByRole('button', {
        name:
          state === 'remove-last-passkey'
            ? 'Remove passkey: 1Password'
            : 'Remove security key: Security key',
        exact: true,
      })
      .click()
  } else if (
    state === 'code-setup' ||
    state === 'recovery-codes' ||
    (state.startsWith('security-') && state !== 'security-off')
  ) {
    const name =
      state === 'code-setup'
        ? 'Set up'
        : state === 'recovery-codes'
          ? 'Generate new codes'
          : 'Add key'
    await page.getByRole('button', { name, exact: true }).click()
    const dialog = page.getByRole('dialog', { name: 'Confirm security change' })
    await dialog.getByLabel('Current password').fill('test-password')
    await dialog.getByRole('button', { name: 'Continue', exact: true }).click()
    if (state === 'code-setup')
      await page.getByRole('button', { name: 'Next', exact: true }).waitFor()
    else if (state === 'recovery-codes')
      await page
        .getByRole('button', { name: 'Copy codes', exact: true })
        .waitFor()
    else
      await dialog
        .getByRole('button', { name: 'Continue', exact: true })
        .waitFor({ state: 'hidden' })
  }
}
