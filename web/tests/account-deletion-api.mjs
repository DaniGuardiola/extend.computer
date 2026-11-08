import { testOrigin } from './test-config.mjs'
// Local-only: deletes a disposable account in the local D1 database and checks
// that no row of it remains, using the real Workers runtime and D1 cascades.
import assert from 'node:assert/strict'
import { randomBytes, createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'
const origin = testOrigin
assert.ok(
  ['localhost', '127.0.0.1'].includes(new URL(origin).hostname),
  'This test is local-only',
)
const root = fileURLToPath(new URL('..', import.meta.url))
const command = promisify(execFile)
const query = async (statement) => {
  const { stdout } = await command(
    process.execPath,
    [
      root + 'node_modules/wrangler/bin/wrangler.js',
      'd1',
      'execute',
      'extend-computer',
      '--local',
      '--json',
      '--command',
      statement,
    ],
    {
      cwd: root,
      env: {
        ...process.env,
        WRANGLER_LOG_PATH: '/tmp/extend-deletion-test-db.log',
      },
    },
  )
  return JSON.parse(stdout)[0].results
}
const hash = (value) => createHash('sha256').update(value).digest('hex')
async function request(path, method = 'GET', data, token) {
  const response = await fetch(origin + path, {
    method,
    headers: {
      Origin: origin,
      'Content-Type': 'application/json',
      'X-Extend-Client': 'desktop',
      ...(token ? { Authorization: 'Bearer ' + token } : {}),
    },
    body: data === undefined ? undefined : JSON.stringify(data),
  })
  return {
    status: response.status,
    cookie: response.headers.get('set-cookie'),
    hsts: response.headers.get('strict-transport-security'),
    data: response.status === 204 ? null : await response.json(),
  }
}
const password = 'disposable-deletion-test-password'
const email = `deletion-${Date.now()}@example.invalid`
const first = await request('/v1/auth/signup', 'POST', { email, password })
assert.equal(first.status, 201)
// Local development stays on plain HTTP: no Secure cookie and no HSTS.
assert.doesNotMatch(first.cookie, /Secure/)
assert.equal(first.hsts, null)
const id = first.data.account.id
const second = await request('/v1/auth/login', 'POST', { email, password })
const device = await request(
  '/v1/devices',
  'POST',
  {
    name: 'Deletion test Mac',
    platform: 'macos',
    public_key: randomBytes(32).toString('hex'),
  },
  second.data.token,
)
assert.equal(device.status, 200)
const tokens = [first.data.token, second.data.token]
const remove = () =>
  request('/v1/account', 'DELETE', { email }, first.data.token)
assert.equal((await remove()).status, 403)
assert.equal(
  (await request('/v1/mfa/reauth', 'POST', { password }, first.data.token))
    .status,
  200,
)
assert.equal(
  (
    await request(
      '/v1/account',
      'DELETE',
      { email: 'x@example.invalid' },
      first.data.token,
    )
  ).status,
  400,
)
assert.equal((await remove()).status, 204)
for (const token of tokens)
  assert.equal(
    (await request('/v1/account', 'GET', undefined, token)).status,
    401,
  )
assert.equal(
  (
    await request(
      `/v1/devices/${device.data.id}/heartbeat`,
      'POST',
      {},
      device.data.device_token,
    )
  ).status,
  401,
)
const sessions = tokens
  .map(hash)
  .map((value) => `'${value}'`)
  .join(',')
const rows = await query(`SELECT
  (SELECT COUNT(*) FROM accounts WHERE id='${id}') +
  (SELECT COUNT(*) FROM sessions WHERE account_id='${id}' OR token_hash IN (${sessions})) +
  (SELECT COUNT(*) FROM devices WHERE id='${device.data.id}') +
  (SELECT COUNT(*) FROM device_sessions WHERE device_id='${device.data.id}' OR token_hash='${hash(device.data.device_token)}')
  AS remaining`)
assert.equal(rows[0].remaining, 0)
console.log(
  'Passed: deletion refused without a fresh check or the right email; account, sessions, device and device credentials removed.',
)
