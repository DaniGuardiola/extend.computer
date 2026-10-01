import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'
const origin = process.env.EXTEND_TEST_URL ?? 'http://localhost:3001'
const address = origin + '/api/v1'
const password = 'disposable-api-test-password'
const suffix = Date.now()
async function request(path, method = 'GET', data, credential, extra = {}) {
  const response = await fetch(address + path, {
    method,
    headers: {
      'Content-Type': 'application/json',
      Origin: origin,
      'X-Extend-Client': 'desktop',
      ...(credential ? { Authorization: 'Bearer ' + credential } : {}),
      ...extra,
    },
    body: data !== undefined ? JSON.stringify(data) : undefined,
  })
  return {
    status: response.status,
    body: response.status === 204 ? null : await response.json(),
  }
}
const a = await request('/auth/signup', 'POST', {
  email: `api-a-${suffix}@example.invalid`,
  password,
})
const b = await request('/auth/signup', 'POST', {
  email: `api-b-${suffix}@example.invalid`,
  password,
})
assert.equal(a.status, 201)
assert.equal(b.status, 201)
const key = randomBytes(32).toString('hex')
const device = await request(
  '/devices',
  'POST',
  { name: 'API test Mac', platform: 'macos', public_key: key },
  a.body.token,
)
assert.equal(device.status, 200)
assert.equal(
  (await request('/devices', 'GET', undefined, b.body.token)).body.devices
    .length,
  0,
)
assert.equal(
  (
    await request(
      '/devices/' + device.body.id,
      'DELETE',
      undefined,
      b.body.token,
    )
  ).status,
  404,
)
assert.equal(
  (
    await request(
      '/devices/' + device.body.id + '/heartbeat',
      'POST',
      undefined,
      a.body.token,
    )
  ).status,
  401,
)
assert.equal(
  (
    await request(
      '/devices/' + device.body.id + '/heartbeat',
      'POST',
      undefined,
      device.body.device_token,
    )
  ).status,
  204,
)
assert.equal(
  (await request('/devices', 'GET', undefined, a.body.token)).body.devices[0]
    .online,
  true,
)
const second = await request('/auth/login', 'POST', {
  email: a.body.account.email,
  password,
})
const again = await request(
  '/devices',
  'POST',
  { name: 'Updated Mac', platform: 'macos', public_key: key },
  second.body.token,
)
assert.equal(again.body.id, device.body.id)
assert.equal(
  (await request('/sessions/others', 'DELETE', undefined, a.body.token)).status,
  204,
)
assert.equal(
  (await request('/account', 'GET', undefined, second.body.token)).status,
  401,
)
assert.equal(
  (
    await request(
      '/devices/' + device.body.id + '/heartbeat',
      'POST',
      undefined,
      again.body.device_token,
    )
  ).status,
  401,
)
assert.equal(
  (
    await request(
      '/devices/' + device.body.id,
      'DELETE',
      undefined,
      a.body.token,
    )
  ).status,
  204,
)
assert.equal(
  (
    await request(
      '/devices/' + device.body.id + '/heartbeat',
      'POST',
      undefined,
      device.body.device_token,
    )
  ).status,
  401,
)
assert.equal(
  (
    await request(
      '/auth/login',
      'POST',
      { email: a.body.account.email, password },
      undefined,
      { Origin: 'https://attacker.invalid' },
    )
  ).status,
  403,
)
assert.equal(
  (
    await request('/auth/login', 'POST', {
      email: a.body.account.email,
      password: 'wrong-password-value',
    })
  ).status,
  401,
)
assert.equal(
  (await request('/auth/logout', 'POST', undefined, a.body.token)).status,
  204,
)
assert.equal(
  (await request('/account', 'GET', undefined, a.body.token)).status,
  401,
)
console.log(
  JSON.stringify({
    passed: [
      'signup/login',
      'account isolation',
      'device credential separation',
      'heartbeat presence',
      'stable device identity',
      'session revocation',
      'device revocation',
      'CSRF origin rejection',
      'wrong password rejection',
      'logout invalidation',
    ],
    test_accounts: [a.body.account.email, b.body.account.email],
  }),
)
