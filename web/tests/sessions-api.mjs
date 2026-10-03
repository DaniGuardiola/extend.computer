import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'

const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3006'
assert.ok(['localhost', '127.0.0.1'].includes(new URL(origin).hostname))
async function request(path, method = 'GET', data, token) {
  const response = await fetch(origin + '/api/v1' + path, {
    method,
    headers: {
      Origin: origin,
      'Content-Type': 'application/json',
      'X-Extend-Client': 'desktop',
      'User-Agent':
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/130.0.0.0',
      'X-Extend-Session-Location': encodeURIComponent('Spoofed city'),
      ...(token ? { Authorization: 'Bearer ' + token } : {}),
    },
    body: data === undefined ? undefined : JSON.stringify(data),
  })
  return {
    status: response.status,
    data: response.status === 204 ? null : await response.json(),
    cookie: response.headers.get('set-cookie'),
  }
}
const password = 'disposable-session-test-password'
const email = `sessions-${Date.now()}@example.invalid`
const a = await request('/auth/signup', 'POST', { email, password })
const b = await request('/auth/login', 'POST', { email, password })
const other = await request('/auth/signup', 'POST', {
  email: `other-${email}`,
  password,
})
assert.equal(a.status, 201)
assert.equal(b.status, 200)
assert.equal(other.status, 201)
const list = await request('/sessions', 'GET', undefined, a.data.token)
assert.equal(list.status, 200)
assert.equal(list.data.sessions.length, 2)
const current = list.data.sessions.find((s) => s.current)
const second = list.data.sessions.find((s) => !s.current)
for (const session of list.data.sessions) {
  assert.match(session.id, /^[a-f0-9]{32}$/)
  assert.equal(session.platform, 'macOS')
  assert.equal(session.client, 'Desktop app')
  assert.equal(session.location, null)
  assert.ok(session.last_access_at >= session.created_at)
  assert.equal(session.token_hash, undefined)
  assert.equal(session.token, undefined)
}
assert.equal((await request('/sessions', 'GET')).status, 401)
assert.equal(
  (
    await request(
      '/sessions/' + second.id,
      'DELETE',
      undefined,
      other.data.token,
    )
  ).status,
  404,
)
const device = await request(
  '/devices',
  'POST',
  {
    name: 'Session test Mac',
    platform: 'macos',
    public_key: randomBytes(32).toString('hex'),
  },
  b.data.token,
)
assert.equal(device.status, 200)
assert.equal(
  (await request('/sessions/' + second.id, 'DELETE', undefined, a.data.token))
    .status,
  204,
)
assert.equal(
  (await request('/account', 'GET', undefined, b.data.token)).status,
  401,
)
assert.equal(
  (
    await request(
      '/devices/' + device.data.id + '/heartbeat',
      'POST',
      {},
      device.data.device_token,
    )
  ).status,
  401,
)
assert.equal(
  (await request('/sessions', 'GET', undefined, a.data.token)).data.sessions
    .length,
  1,
)
const c = await request('/auth/login', 'POST', { email, password })
const own = await request(
  '/sessions/' + current.id,
  'DELETE',
  undefined,
  a.data.token,
)
assert.equal(own.status, 204)
assert.match(own.cookie, /Max-Age=0/)
assert.equal(
  (await request('/account', 'GET', undefined, a.data.token)).status,
  401,
)
const all = await request('/sessions', 'DELETE', undefined, c.data.token)
assert.equal(all.status, 204)
assert.match(all.cookie, /Max-Age=0/)
assert.equal(
  (await request('/account', 'GET', undefined, c.data.token)).status,
  401,
)
assert.equal(
  (await request('/account', 'GET', undefined, other.data.token)).status,
  200,
)
console.log(
  'PASS: session metadata, privacy, location spoofing, individual/all revocation, device revocation',
)
