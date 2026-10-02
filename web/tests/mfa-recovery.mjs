// Local-only seeded link fixtures: no production data or real email delivery.
import assert from 'node:assert/strict'
import { randomBytes, createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'
const url = process.env.EXTEND_TEST_URL ?? 'http://localhost:3001'
assert.ok(
  ['localhost', '127.0.0.1'].includes(new URL(url).hostname),
  'This fixture test is local-only',
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
      env: {
        ...process.env,
        WRANGLER_LOG_PATH: '/tmp/extend-email-test-db.log',
      },
    },
  )
}
const hash = (s) => createHash('sha256').update(s).digest('hex')
const request = async (path, data, token) => {
  const response = await fetch(url + '/api/v1' + path, {
    method: data === undefined ? 'GET' : 'POST',
    headers: {
      'Content-Type': 'application/json',
      Origin: url,
      'X-Extend-Client': 'desktop',
      ...(token ? { Authorization: 'Bearer ' + token } : {}),
    },
    body: data === undefined ? undefined : JSON.stringify(data),
  })
  return {
    status: response.status,
    data: await response.json().catch(() => null),
  }
}
const password = 'disposable-email-test-password'
const email = 'email-flow-' + Date.now() + '@example.invalid'
const signup = await request('/auth/signup', { email, password })
assert.equal(signup.status, 201)
const id = signup.data.account.id
const session = signup.data.token
const code = randomBytes(16).toString('hex')
const raw = randomBytes(32).toString('hex')
try {
  await sql(
    `UPDATE accounts SET mfa_enabled=1,email_verified=1 WHERE id='${id}'; INSERT INTO recovery_codes VALUES ('${id}','${hash(code)}'); INSERT INTO passkeys (id,account_id,public_key,counter,transports,name,rp_id,created_at,purpose) VALUES ('mfa-fixture-${id}','${id}','fixture',0,'[]','Fixture','localhost',unixepoch(),'factor'); INSERT INTO email_tokens SELECT '${hash(raw)}',id,'reset',password_hash,unixepoch()+1800 FROM accounts WHERE id='${id}';`,
  )
  const pending = await request('/auth/login', { email, password })
  assert.equal(pending.data.mfa_required, true)
  assert.equal(
    (
      await request('/auth/recovery/complete', {
        token: raw,
        password: 'recovered-disposable-password',
      })
    ).status,
    200,
  )
  assert.equal((await request('/account', undefined, session)).status, 401)
  assert.equal(
    (await request('/auth/mfa/verify', { ticket: pending.data.ticket, code }))
      .status,
    401,
  )
  const login = await request('/auth/login', {
    email,
    password: 'recovered-disposable-password',
  })
  assert.equal(login.data.mfa_required, true)
  assert.equal(login.data.token, undefined)
  const verified = await request('/auth/mfa/verify', {
    ticket: login.data.ticket,
    code,
  })
  assert.equal(verified.status, 200)
  const keys = await request('/passkeys', undefined, verified.data.token)
  assert.equal(keys.data.passkeys.length, 1)
  console.log(
    JSON.stringify({
      emailRecoveryKeepsMfa: true,
      keysPreserved: true,
      oldTicketRevoked: true,
      passwordAloneBlocked: true,
    }),
  )
} finally {
  await sql(`DELETE FROM accounts WHERE id='${id}';`)
}
