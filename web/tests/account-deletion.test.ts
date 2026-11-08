import test from 'node:test'
import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'
import {
  call,
  digest,
  site,
  startAccountService,
  type Harness,
} from './account-service-harness'
import { newRecoveryCodes } from '../src/lib/recovery-codes'

const password = 'disposable-deletion-password'
const hex = () => randomBytes(32).toString('hex')

// A browser session (cookie) and an app session (bearer token).
async function signUp(harness: Harness, email: string) {
  const browser = await call(harness, '/auth/signup', {
    method: 'POST',
    data: { email, password },
  })
  assert.equal(browser.status, 201)
  const cookie = /^extend_session=([a-f0-9]{64});/.exec(browser.cookie!)![1]!
  // Apps opt in to a bearer token instead of the cookie.
  const login = await harness.service.fetch(
    new Request(site + '/v1/auth/login', {
      method: 'POST',
      headers: {
        Origin: site,
        'Content-Type': 'application/json',
        'X-Extend-Client': 'desktop',
      },
      body: JSON.stringify({ email, password }),
    }),
  )
  const token = ((await login.json()) as { token: string }).token
  const device = await call(harness, '/devices', {
    method: 'POST',
    data: { public_key: hex(), name: 'Studio Mac', platform: 'macos' },
    token,
  })
  assert.equal(device.status, 200)
  return {
    id: browser.data.account.id as string,
    cookie,
    token,
    device: device.data.id as string,
    deviceToken: device.data.device_token as string,
  }
}

// Fill every other account table so the deletion check covers all of them.
function seed(harness: Harness, account: Awaited<ReturnType<typeof signUp>>) {
  const session = digest(account.token)
  const later = Math.floor(Date.now() / 1000) + 300
  const rows: [string, unknown[]][] = [
    [
      'INSERT INTO passkeys (id,account_id,public_key,counter,transports,name,rp_id,created_at,purpose) VALUES (?,?,?,0,?,?,?,0,?)',
      [hex(), account.id, hex(), '[]', 'Passkey', 'extend.computer', 'login'],
    ],
    [
      'INSERT INTO challenges VALUES (?,?,?,?,?,?)',
      [hex(), 'challenge', account.id, 'register:' + session, site, later],
    ],
    [
      'INSERT INTO email_tokens VALUES (?,?,?,?,?)',
      [hex(), account.id, 'verify', 'version', later],
    ],
    ['INSERT INTO email_cooldowns VALUES (?,?)', [account.id, later]],
    ['INSERT INTO totp_factors VALUES (?,?,0)', [account.id, 'sealed']],
    [
      'INSERT INTO totp_setup VALUES (?,?,?,0,?)',
      [session, account.id, 'sealed', later],
    ],
    ['INSERT INTO recovery_codes VALUES (?,?)', [account.id, hex()]],
    [
      'INSERT INTO mfa_tickets (id,account_id,password_version,version,purpose,session_hash,expires_at) VALUES (?,?,?,0,?,?,?)',
      [hex(), account.id, 'version', 'manage', session, later],
    ],
    ['INSERT INTO mfa_attempts VALUES (?,1,?)', [account.id, later]],
    [
      'INSERT INTO desktop_codes VALUES (?,?,?,?)',
      [hex(), session, 'challenge', later],
    ],
    [
      'INSERT INTO device_proofs VALUES (?,?,?,?)',
      [digest(account.deviceToken), 'challenge', 'expected', later],
    ],
  ]
  for (const [sql, values] of rows)
    harness.db.prepare(sql).run(...(values as string[]))
}

// Tables with a row that mentions one of the account's identifiers.
function tablesMentioning(harness: Harness, ids: string[]) {
  const tables = harness.db
    .prepare(
      "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .all()
    .map((row) => row.name as string)
  return {
    tables,
    mentioning: tables.filter((table) =>
      harness.db
        .prepare(`SELECT * FROM ${table}`)
        .all()
        .some((row) => ids.some((id) => JSON.stringify(row).includes(id))),
    ),
  }
}
const identifiers = (account: Awaited<ReturnType<typeof signUp>>) => [
  account.id,
  digest(account.cookie),
  digest(account.token),
  account.device,
  digest(account.deviceToken),
]

test('account deletion needs a fresh security check and the account email, then removes every related row', async () => {
  const harness = await startAccountService()
  const email = `delete-${hex().slice(0, 8)}@example.invalid`
  const doomed = await signUp(harness, email)
  const kept = await signUp(
    harness,
    `keep-${hex().slice(0, 8)}@example.invalid`,
  )
  seed(harness, doomed)
  seed(harness, kept)
  await harness.env.ACCOUNTS.getByName('mail-budget').reserveAccountEmail(
    doomed.id,
  )
  // Every table holds rows of both accounts before deletion.
  const before = tablesMentioning(harness, identifiers(doomed))
  assert.deepEqual(before.mentioning, before.tables)
  assert.deepEqual(
    tablesMentioning(harness, identifiers(kept)).mentioning,
    before.tables,
  )

  const remove = (options: Parameters<typeof call>[2] = {}) =>
    call(harness, '/account', {
      method: 'DELETE',
      cookie: doomed.cookie,
      data: { email },
      ...options,
    })
  const exists = () =>
    !!harness.db.prepare('SELECT 1 FROM accounts WHERE id=?').get(doomed.id)

  // Signed out requests are refused before anything else.
  assert.equal((await remove({ cookie: undefined })).status, 401)
  // Signed in is not enough: deletion needs a security check first.
  assert.equal((await remove()).status, 403)
  assert.equal(
    (await remove({ token: doomed.token, cookie: undefined })).status,
    403,
  )
  // Browser writes must come from the site itself.
  assert.equal(
    (await remove({ origin: 'https://attacker.example' })).status,
    403,
  )
  assert.equal((await remove({ origin: null })).status, 403)
  assert.equal(
    (
      await call(harness, '/mfa/reauth', {
        method: 'POST',
        cookie: doomed.cookie,
        data: { password: 'wrong-password-for-account' },
      })
    ).status,
    401,
  )
  assert.equal((await remove()).status, 403)
  assert.ok(exists())

  const reauth = await call(harness, '/mfa/reauth', {
    method: 'POST',
    cookie: doomed.cookie,
    data: { password },
  })
  assert.deepEqual(reauth.data, { success: true })
  // The check belongs to the browser session, not to the account's other sessions.
  assert.equal(
    (await remove({ token: doomed.token, cookie: undefined })).status,
    403,
  )
  // The typed confirmation must match the account email.
  for (const data of [{ email: 'someone@example.invalid' }, { email: '' }, {}])
    assert.equal((await remove({ data })).status, 400)
  assert.equal((await remove({ data: undefined })).status, 415)
  assert.ok(exists())

  const deleted = await remove({ data: { email: `  ${email.toUpperCase()} ` } })
  assert.equal(deleted.status, 204)
  assert.equal(
    deleted.cookie,
    'extend_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Secure',
  )
  await harness.settle()

  // Nothing of the deleted account remains; the other account is untouched.
  assert.deepEqual(
    tablesMentioning(harness, identifiers(doomed)).mentioning,
    [],
  )
  assert.deepEqual(
    tablesMentioning(harness, identifiers(kept)).mentioning,
    before.tables,
  )
  for (const name of ['auth-0', 'mail-budget'])
    assert.deepEqual(
      harness
        .storage(name)
        .prepare('SELECT key FROM attempts')
        .all()
        .filter((row) => String(row.key).includes(doomed.id)),
      [],
    )

  // Every session and app credential of the deleted account stops working.
  assert.equal(
    (await call(harness, '/account', { cookie: doomed.cookie })).status,
    401,
  )
  assert.equal(
    (await call(harness, '/account', { token: doomed.token })).status,
    401,
  )
  const beat = (account: typeof doomed) =>
    harness.heartbeat(
      new Request(`${site}/v1/devices/${account.device}/heartbeat`, {
        method: 'POST',
        headers: { Authorization: 'Bearer ' + account.deviceToken },
      }),
    )
  assert.equal((await beat(doomed)).status, 401)
  assert.equal((await beat(kept)).status, 204)
  assert.equal(
    (await call(harness, '/account', { token: kept.token })).status,
    200,
  )
  // The address is free again.
  assert.equal(
    (
      await call(harness, '/auth/signup', {
        method: 'POST',
        data: { email, password },
      })
    ).status,
    201,
  )
})

test('with two-factor authentication on, a password alone cannot delete the account', async () => {
  const harness = await startAccountService()
  const email = `delete-mfa-${hex().slice(0, 8)}@example.invalid`
  const account = await signUp(harness, email)
  const [code] = newRecoveryCodes()
  harness.db
    .prepare('UPDATE accounts SET mfa_enabled=1 WHERE id=?')
    .run(account.id)
  harness.db
    .prepare('INSERT INTO recovery_codes VALUES (?,?)')
    .run(account.id, code!.hash)
  const remove = () =>
    call(harness, '/account', {
      method: 'DELETE',
      cookie: account.cookie,
      data: { email },
    })
  const reauth = await call(harness, '/mfa/reauth', {
    method: 'POST',
    cookie: account.cookie,
    data: { password },
  })
  assert.equal(reauth.data.mfa_required, true)
  assert.equal((await remove()).status, 403)
  const verified = await call(harness, '/auth/mfa/verify', {
    method: 'POST',
    cookie: account.cookie,
    data: { ticket: reauth.data.ticket, code: code!.display },
  })
  assert.deepEqual(verified.data, { success: true })
  assert.equal((await remove()).status, 204)
  assert.equal(
    harness.db.prepare('SELECT 1 FROM accounts WHERE id=?').get(account.id),
    undefined,
  )
})
