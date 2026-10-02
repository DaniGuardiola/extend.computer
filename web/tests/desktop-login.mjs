// Local protocol + real browser test. No real emails or user browser access.
import { chromium } from 'playwright'
import { randomBytes, createHash } from 'node:crypto'
import { createServer } from 'node:http'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3010'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
const browser = await chromium.launch({ headless: true })
const context = await browser.newContext()
const server = createServer()
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
let id
const request = async (path, body, token) => {
  const r = await context.request.post(origin + '/v1' + path, {
    headers: {
      Origin: origin,
      'X-Extend-Client': 'desktop',
      ...(token ? { Authorization: 'Bearer ' + token } : {}),
    },
    data: body,
  })
  return { status: r.status(), data: await r.json().catch(() => null) }
}
try {
  const email = 'desktop-flow-' + Date.now() + '@example.invalid'
  const password = randomBytes(24).toString('hex')
  const signup = await request('/auth/signup', {
    email,
    password,
  })
  assert.equal(signup.status, 201)
  id = signup.data.account.id
  const verifier = randomBytes(32).toString('base64url')
  const challenge = createHash('sha256').update(verifier).digest('base64url')
  const state = randomBytes(32).toString('hex')
  const callback = new Promise((resolve) =>
    server.once('request', (req, res) => {
      res.end('Return to desktop')
      resolve(new URL(req.url, 'http://127.0.0.1'))
    }),
  )
  await context.clearCookies()
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto(
    origin +
      `/desktop/connect?port=${server.address().port}&state=${state}&challenge=${challenge}`,
  )
  await page.waitForLoadState('networkidle')
  await page.getByRole('textbox', {name:'Email',exact:true}).fill(email)
  await page.getByLabel('Password',{exact:true}).fill(password)
  await page.getByRole('button',{name:'Sign in',exact:true}).click()
  await page.getByText(`Signed in as ${email}.`, { exact: true }).waitFor()
  await page
    .getByRole('button', { name: 'Continue to desktop', exact: true })
    .click()
  const returned = await callback
  assert.equal(returned.searchParams.get('state'), state)
  const code = returned.searchParams.get('code')
  assert.equal(
    (
      await request('/auth/desktop/exchange', {
        code,
        verifier: randomBytes(32).toString('base64url'),
      })
    ).status,
    401,
  )
  const exchange = await request('/auth/desktop/exchange', { code, verifier })
  assert.equal(exchange.status, 200)
  assert.ok(exchange.data.token)
  assert.equal(exchange.data.account.id, id)
  assert.equal(
    (await request('/auth/desktop/exchange', { code, verifier })).status,
    401,
  )
  const grant = await request(
    '/auth/desktop/authorize',
    { challenge },
    signup.data.token,
  )
  assert.equal(grant.status, 200)
  assert.equal(
    (await request('/auth/logout', {}, signup.data.token)).status,
    204,
  )
  assert.equal(
    (
      await request('/auth/desktop/exchange', {
        code: grant.data.code,
        verifier,
      })
    ).status,
    401,
  )
  assert.deepEqual(errors, [])
  console.log(
    JSON.stringify({
      desktopBrowserLogin: true,
      pkceRequired: true,
      codeSingleUse: true,
      sourceSessionRevocation: true,
      noBrowserErrors: true,
    }),
  )
} finally {
  if (id)
    await promisify(execFile)(
      process.execPath,
      [
        'node_modules/wrangler/bin/wrangler.js',
        'd1',
        'execute',
        'extend-computer',
        '--local',
        '--command',
        `DELETE FROM accounts WHERE id='${id}';`,
      ],
      {
        cwd: fileURLToPath(new URL('..', import.meta.url)),
        env: {
          ...process.env,
          WRANGLER_LOG_PATH: '/tmp/extend-desktop-test-db.log',
        },
      },
    )
  await context.close()
  await browser.close()
  await new Promise((resolve) => server.close(resolve))
}
