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
const email = 'session-choice-' + Date.now() + '@example.invalid'
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
  let other = await request('/auth/login', { email, password })
  const setup = await request('/mfa/reauth', { password }, session)
  assert.equal(setup.status, 200)
  const initial = await request('/mfa/totp/setup', {}, session)
  let enabled = await request(
    '/mfa/totp/confirm',
    { code: otp(initial.body.secret) },
    session,
  )
  assert.equal(enabled.status, 200)
  async function elevate(codes) {
    const proof = await request('/mfa/reauth', { password }, session)
    const verified = await request(
      '/auth/mfa/verify',
      { ticket: proof.body.ticket, code: codes[0] },
      session,
    )
    assert.equal(verified.status, 200, JSON.stringify({ proof, verified }))
  }
  for (const signOut of [false, true]) {
    await elevate(enabled.body.recovery_codes)
    const replacement = await request('/mfa/totp/setup', {}, session)
    enabled = await request(
      '/mfa/totp/confirm',
      { code: otp(replacement.body.secret), sign_out_others: signOut },
      session,
    )
    assert.equal(enabled.status, 200)
    assert.equal(
      (await request('/account', undefined, other.body.token)).status,
      signOut ? 401 : 200,
    )
    assert.equal((await request('/account', undefined, session)).status, 200)
  }
  for (const signOut of [process.env.EXTEND_TEST_SIGN_OUT === 'true']) {
    const pending = await request('/auth/login', { email, password })
    other = await request('/auth/mfa/verify', {
      ticket: pending.body.ticket,
      code: enabled.body.recovery_codes[1],
    })
    assert.equal(other.status, 200)
    await elevate(enabled.body.recovery_codes)
    const disabled = await request(
      '/mfa/disable',
      { sign_out_others: signOut },
      session,
    )
    assert.equal(disabled.status, 200)
    assert.equal(
      (await request('/account', undefined, other.body.token)).status,
      signOut ? 401 : 200,
    )
    assert.equal((await request('/account', undefined, session)).status, 200)
  }

  let currentPassword = password
  for (const signOut of [false, true]) {
    other = await request('/auth/login', { email, password: currentPassword })
    const nextPassword = randomBytes(24).toString('hex')
    assert.equal(
      (
        await request(
          '/account/password',
          {
            current_password: currentPassword,
            password: nextPassword,
            sign_out_others: signOut,
          },
          session,
        )
      ).status,
      204,
    )
    assert.equal(
      (await request('/account', undefined, other.body.token)).status,
      signOut ? 401 : 200,
    )
    assert.equal((await request('/account', undefined, session)).status, 200)
    currentPassword = nextPassword
  }
  console.log(
    'Passed: optional session revocation for authenticator replacement, disabling MFA, and password changes; current session preserved.',
  )
} finally {
  await sql(`DELETE FROM accounts WHERE id='${account}';`)
}
