import test from 'node:test'
import assert from 'node:assert/strict'
import { api, ApiError } from '../src/lib/api'
import {
  accountSchema,
  passkeySchema,
  deviceSchema,
} from '../src/lib/contracts'

test('required account protection flags are validated; optional metadata is compatible', () => {
  const account = {
    id: 'a',
    email: 'a@example.invalid',
    email_verified: 1,
    mfa_enabled: 0,
  }
  assert.equal(accountSchema.safeParse(account).success, true)
  assert.equal(
    accountSchema.safeParse({ ...account, mfa_enabled: undefined }).success,
    false,
  )
  assert.equal(
    accountSchema.safeParse({ ...account, email_verified: true }).success,
    false,
  )
  for (const purpose of ['login', 'login-disabled', 'factor']) {
    assert.equal(
      passkeySchema.safeParse({
        id: 'key',
        name: 'Key',
        created_at: 1,
        purpose,
      }).success,
      true,
    )
  }
  assert.equal(
    passkeySchema.safeParse({
      id: 'key',
      name: 'Key',
      created_at: 1,
      purpose: 'unknown',
    }).success,
    false,
  )
})

test('API rejects malformed requests before transport and malformed responses without leaking payloads', async (t) => {
  let requests = 0
  t.mock.method(globalThis, 'fetch', async () => {
    requests++
    return Response.json({
      id: 'secret-account-data',
      email: 'private@example.invalid',
    })
  })
  await assert.rejects(
    api('POST /auth/login', { email: 'a@example.invalid', password: '' }),
    {
      message: 'Check the information you entered and try again.',
    },
  )
  assert.equal(requests, 0)
  await assert.rejects(api('GET /account'), {
    message: 'Could not complete the request. Please try again.',
  })
  assert.equal(requests, 1)
})

test('API encodes opaque IDs, preserves cancellation and accepts empty successful responses', async (t) => {
  const controller = new AbortController()
  t.mock.method(globalThis, 'fetch', async (url, options) => {
    assert.equal(url, '/api/v1/devices/a%2Fb%20%3F')
    assert.equal(options?.signal, controller.signal)
    assert.equal(options?.method, 'DELETE')
    return new Response(null, { status: 204 })
  })
  await api('DELETE /devices/:id', undefined, {
    id: 'a/b ?',
    signal: controller.signal,
  })
  await assert.rejects(api('DELETE /devices/:id'), /Please try again/)
})

test('API preserves server errors and retry timing', async (t) => {
  t.mock.method(globalThis, 'fetch', async () =>
    Response.json(
      { error: 'Please wait before trying again.' },
      { status: 429, headers: { 'Retry-After': '60' } },
    ),
  )
  await assert.rejects(api('POST /auth/email/request'), (error: unknown) => {
    assert.ok(error instanceof ApiError)
    assert.equal(error.status, 429)
    assert.equal(error.retryAfter, 60)
    return true
  })
})

test('Android device directory responses retain their platform', () => {
  const device = deviceSchema.parse({
    id: 'device',
    name: 'Android tablet',
    platform: 'android',
    fingerprint: 'a'.repeat(64),
    online: false,
  })
  assert.equal(device.platform, 'android')
})
