import {
  installAccountFixture,
  reachAccountState,
} from './account-fixtures.mjs'
import { testOrigin as origin } from './test-config.mjs'
import assert from 'node:assert/strict'
const cases = [
  ...[
    '',
    'devices',
    'passkeys',
    'sessions',
    'security-off',
    'loading',
    'load-error',
    'refresh-error',
    'refresh-offline',
    'email-unverified',
    'email-unverified-limit',
    'email-verified',
    'code-setup',
    'recovery-codes',
    'remove-key',
    'remove-last-key',
    'remove-last-passkey',
    'security-key',
    'security-code-key',
  ].map((state) => ({
    name: 'account-' + (state || 'default'),
    url: '/account',
    state,
  })),
  ...[
    'login',
    'signup',
    'recover',
    'email',
    'desktop/connected',
    'page-error-preview',
    'page-not-found-preview',
  ].map((url) => ({
    name: url.replaceAll('/', '-'),
    url: url === 'page-error-preview' ? '/missing-page' : '/' + url,
    errorPage: url === 'page-error-preview',
  })),
  ...['reset', 'reset-done'].map((state) => ({
    name: 'email-' + state,
    url: '/email#reset=' + '0'.repeat(64),
    state,
  })),
  { name: 'recover-sent', url: '/recover', state: 'sent' },
  {
    name: 'login-error',
    url: '/login',
    action: async (page) => {
      await page
        .getByLabel('Email', { exact: true })
        .fill('review@example.invalid')
      await page
        .getByLabel('Password', { exact: true })
        .fill('incorrect-password')
      await page.getByRole('button', { name: 'Sign in', exact: true }).click()
      await page.getByRole('alert').waitFor()
    },
  },
  {
    name: 'login-focus',
    url: '/login',
    action: (page) => page.getByLabel('Email', { exact: true }).focus(),
  },
  {
    name: 'login-hover',
    url: '/login',
    action: (page) =>
      page.getByRole('button', { name: 'Sign in', exact: true }).hover(),
  },
  {
    name: 'add-key',
    url: '/account',
    action: async (page) => {
      await page.getByRole('button', { name: 'Add key', exact: true }).click()
      await page.getByRole('dialog').waitFor()
    },
  },
  {
    name: 'sign-out',
    url: '/account',
    state: 'sessions',
    action: async (page) => {
      await page
        .getByRole('button', { name: 'Sign out this session', exact: true })
        .click()
      await page.getByRole('dialog').waitFor()
    },
  },
  {
    name: 'desktop-connect',
    url:
      '/desktop/connect?port=54321&state=' +
      '0'.repeat(64) +
      '&challenge=' +
      'A'.repeat(43),
  },
  {
    name: 'desktop-connect-guest',
    url:
      '/desktop/connect?port=54321&state=' +
      '0'.repeat(64) +
      '&challenge=' +
      'A'.repeat(43),
    guest: true,
  },
]
cases.push(
  { name: 'landing-control', url: '/' },
  { name: 'login-ended', url: '/login?reason=session-ended' },
  { name: 'login-pending', url: '/login', state: 'signing-in' },
  { name: 'signup-pending', url: '/signup', state: 'creating-account' },
  {
    name: 'password-dialog',
    url: '/account',
    action: async (page) => {
      await page.getByRole('button', { name: 'Change', exact: true }).click()
      await page.getByRole('dialog').waitFor()
    },
  },
  {
    name: 'device-tooltip',
    url: '/account',
    state: 'devices',
    action: async (page) => {
      await page.locator('.device-detail [tabindex]').first().hover()
      await page.getByRole('tooltip').waitFor()
    },
  },
  {
    name: 'setup-key',
    url: '/account',
    state: 'code-setup',
    action: async (page) => {
      await page
        .getByRole('button', {
          name: 'Can’t scan? Show setup key',
          exact: true,
        })
        .click()
    },
  },
  {
    name: 'setup-confirm',
    url: '/account',
    state: 'code-setup',
    action: async (page) => {
      await page.getByRole('button', { name: 'Next', exact: true }).click()
    },
  },
  {
    name: 'recovery-copied',
    url: '/account',
    state: 'recovery-codes',
    action: async (page) => {
      await page
        .context()
        .grantPermissions(['clipboard-read', 'clipboard-write'])
      await page
        .getByRole('button', { name: 'Copy codes', exact: true })
        .click()
      await page.getByRole('button', { name: 'Copied!', exact: true }).waitFor()
    },
  },
  {
    name: 'recovery-downloaded',
    url: '/account',
    state: 'recovery-codes',
    action: async (page) => {
      await page
        .getByRole('button', { name: 'Download codes', exact: true })
        .click()
      await page
        .getByRole('button', { name: 'Downloaded!', exact: true })
        .waitFor()
    },
  },
  {
    name: 'email-sent',
    url: '/account',
    state: 'email-unverified',
    action: async (page) => {
      await page
        .getByRole('button', { name: 'Send verification email', exact: true })
        .click()
    },
  },
)
const widths = [390, 768, 1440]
export const jobs = cases.flatMap((c) =>
  widths.flatMap((width) =>
    ['light', 'dark'].map((theme) => ({ ...c, width, theme })),
  ),
)
// Capture motion variants separately, with animation held at its first frame.
for (const url of ['/page-error-preview', '/page-not-found-preview'])
  for (const theme of ['light', 'dark'])
    jobs.push({
      name: url.slice(1) + '-motion',
      url: url === '/page-error-preview' ? '/missing-page' : url,
      width: 768,
      theme,
      motion: true,
    })
if (process.env.EXTEND_VISUAL_FILTER) {
  const selected = jobs.filter((job) =>
    new RegExp(process.env.EXTEND_VISUAL_FILTER).test(job.name),
  )
  jobs.splice(0, jobs.length, ...selected)
}

export async function captureVisual(page, job) {
  await page.clock.setFixedTime(new Date('2026-10-05T10:00:00Z'))
  await page.addInitScript(
    (theme) => localStorage.setItem('extend-theme', theme),
    job.theme,
  )
  const guest =
    job.guest ||
    (!job.url.startsWith('/account') && !job.name.startsWith('desktop-connect'))
  await installAccountFixture(page, job.state, {
    guest,
    pending: job.state === 'signing-in' || job.state === 'creating-account',
  })
  if (job.state === 'email-verified')
    await page.addInitScript(() =>
      sessionStorage.setItem('extend-email-verified', '1'),
    )
  // Decode the GIF into its matching still asset without changing image geometry.
  await page.route('**/media/*.gif', async (route) => {
    const source = route.request().url().replace('.gif', '-still.png')
    const response = await route.fetch({ url: source })
    return route.fulfill({ response })
  })
  await page.goto(origin + job.url)
  // The SSR form is visible before React attaches submit handlers. ThemePicker
  // applies the saved preference in an effect, so this proves hydration before
  // interacting without adding preview/test markers to production code.
  if (job.url !== '/' && job.state !== 'loading')
    await page.waitForFunction(
      (theme) =>
        document.querySelector('.theme-picker select')?.value === theme,
      job.theme,
    )
  if (job.state !== 'loading') await page.waitForLoadState('networkidle')
  await page.evaluate(() => document.fonts.ready)
  assert.ok(
    await page.evaluate(
      () =>
        document.fonts.check('15px "Inter Variable"') &&
        document.fonts.check('15px "Space Grotesk Variable"'),
    ),
    'App fonts failed to load',
  )
  await page.locator('main#main').waitFor()
  assert.equal(
    await page.locator('vite-error-overlay').count(),
    0,
    'Vite error overlay',
  )
  if (job.errorPage || job.name === 'page-error-preview-motion') {
    await page.evaluate(async () =>
      (await import('/tests/render-page-error.tsx')).renderPageError(),
    )
    await page.getByRole('heading', { name: 'Something went wrong.' }).waitFor()
  }
  await reachAccountState(page, job.state ?? '')
  if (
    ['sent', 'reset-done', 'signing-in', 'creating-account'].includes(job.state)
  ) {
    if (
      job.state === 'sent' ||
      job.state.endsWith('ing-in') ||
      job.state === 'creating-account'
    )
      await page
        .getByLabel('Email', { exact: true })
        .fill('review@example.invalid')
    if (
      job.state === 'reset-done' ||
      job.state === 'signing-in' ||
      job.state === 'creating-account'
    )
      await page
        .getByLabel(job.state === 'reset-done' ? 'New password' : 'Password', {
          exact: true,
        })
        .fill('test-password-long')
    await page
      .locator('form button[type="submit"], form button:not([type])')
      .last()
      .click()
    if (job.state === 'sent')
      await page.getByRole('heading', { name: 'Check your inbox' }).waitFor()
    if (job.state === 'reset-done')
      await page.getByRole('heading', { name: 'Password reset' }).waitFor()
  }
  if (job.action) await job.action(page)
  const frozenStyles =
    '*,*::before,*::after { animation: none !important; transition: none !important; caret-color: transparent !important; }'
  if (job.state === 'loading')
    await page.evaluate((content) => {
      // addStyleTag waits on a page event handler, which is disabled in this
      // deliberate SSR-only capture. DevTools evaluation still works.
      const style = document.createElement('style')
      style.textContent = content
      document.head.append(style)
    }, frozenStyles)
  else await page.addStyleTag({ content: frozenStyles })
  await page.evaluate(
    () =>
      document.activeElement?.matches('input') ||
      document.activeElement?.blur(),
  )
  if (job.name === 'login-focus')
    await page.getByLabel('Email', { exact: true }).focus()
  await page.evaluate(() => window.scrollTo(0, 0))
  await page.waitForTimeout(50)
  return page.screenshot({ fullPage: true, animations: 'disabled' })
}
