import assert from 'node:assert/strict'
import { randomBytes, createHmac } from 'node:crypto'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3002'
assert.ok(
  ['localhost', '127.0.0.1'].includes(new URL(origin).hostname),
  'This test is local-only',
)
const root = fileURLToPath(new URL('..', import.meta.url))
const command = promisify(execFile)
const sql = async (statement) => {
  await command(
    process.execPath,
    [
      root + 'node_modules/wrangler/bin/wrangler.js',
      'd1',
      'execute',
      'extend-computer',
      '--local',
      '--command',
      statement,
    ],
    {
      cwd: root,
      env: { ...process.env, WRANGLER_LOG_PATH: '/tmp/extend-mfa-test-db.log' },
    },
  )
}
const request = async (path, data, token) => {
  const response = await fetch(origin + '/api/v1' + path, {
    method: data === undefined ? 'GET' : 'POST',
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
    body: await response.json().catch(() => null),
    cookie: response.headers.get('set-cookie'),
  }
}
const password = randomBytes(24).toString('hex')
const email = 'mfa-' + Date.now() + '@example.invalid'
const signup = await request('/auth/signup', { email, password })
assert.equal(signup.status, 201)
const session = signup.body.token
const account = signup.body.account.id
function otp(secret) {
  let value = 0,
    bits = 0,
    bytes = []
  for (const c of secret) {
    value = (value << 5) | 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.indexOf(c)
    bits += 5
    if (bits >= 8) {
      bits -= 8
      bytes.push((value >>> bits) & 255)
    }
  }
  const counter = Buffer.alloc(8)
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000)))
  const hash = createHmac('sha1', Buffer.from(bytes)).update(counter).digest()
  return ((hash.readUInt32BE(hash[19] & 15) & 0x7fffffff) % 1e6)
    .toString()
    .padStart(6, '0')
}
try {
  const other = await request('/auth/login', { email, password })
  assert.equal(other.status, 200)
  assert.equal((await request('/mfa/totp/setup', {}, session)).status, 403)
  assert.equal(
    (await request('/mfa/reauth', { password }, session)).status,
    200,
  )
  const setup = await request('/mfa/totp/setup', {}, session)
  assert.equal(setup.status, 200)
  const confirmed = await request(
    '/mfa/totp/confirm',
    { code: otp(setup.body.secret) },
    session,
  )
  assert.equal(confirmed.status, 200)
  assert.equal(confirmed.body.recovery_codes.length, 10)
  assert.equal(
    (await request('/account', undefined, other.body.token)).status,
    401,
  )
  assert.equal((await request('/mfa/disable', {}, session)).status, 403)
  const login = await request('/auth/login', { email, password })
  assert.equal(login.status, 200)
  assert.equal(login.body.mfa_required, true)
  assert.equal(login.body.token, undefined)
  assert.equal(login.cookie, null)
  assert.equal(
    (await request('/account', undefined, login.body.ticket)).status,
    401,
  )
  assert.equal(
    (
      await request('/auth/mfa/verify', {
        ticket: login.body.ticket,
        code: otp(setup.body.secret),
      })
    ).status,
    401,
  )
  const recovery = confirmed.body.recovery_codes[0]
  const verified = await request('/auth/mfa/verify', {
    ticket: login.body.ticket,
    code: recovery,
  })
  assert.equal(verified.status, 200)
  assert.ok(verified.body.token)
  assert.equal(
    (
      await request('/auth/mfa/verify', {
        ticket: login.body.ticket,
        code: recovery,
      })
    ).status,
    401,
  )
  const repeat = await request('/auth/login', { email, password })
  assert.equal(
    (
      await request('/auth/mfa/verify', {
        ticket: repeat.body.ticket,
        code: recovery,
      })
    ).status,
    401,
  )
  // Authenticated password+second-factor reauthentication is required to disable.
  const pending = await request(
    '/mfa/reauth',
    { password },
    verified.body.token,
  )
  assert.equal(pending.body.mfa_required, true)
  assert.equal(
    (
      await request('/auth/mfa/verify', {
        ticket: pending.body.ticket,
        code: confirmed.body.recovery_codes[1],
      })
    ).status,
    200,
  )
  assert.equal(
    (await request('/mfa/disable', {}, verified.body.token)).status,
    200,
  )
  assert.equal((await request('/account', undefined, session)).status, 401)
  assert.equal(
    (
      await request('/auth/mfa/verify', {
        ticket: repeat.body.ticket,
        code: confirmed.body.recovery_codes[2],
      })
    ).status,
    401,
  )
  assert.ok((await request('/auth/login', { email, password })).body.token)
  console.log(
    JSON.stringify({
      passed: [
        'no session before MFA',
        'setup reauthentication',
        'TOTP replay rejected',
        'single-use recovery',
        'ticket replay rejected',
        'security change revokes sessions and pending logins',
        'fresh MFA required to disable',
      ],
    }),
  )
} finally {
  await sql(`DELETE FROM accounts WHERE id='${account}';`)
}
