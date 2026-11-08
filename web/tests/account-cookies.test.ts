import test from 'node:test'
import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'
import { call, site, startAccountService } from './account-service-harness'

const password = 'disposable-cookie-password'
const hex = () => randomBytes(32).toString('hex')

test('session cookies are Secure except on plain-HTTP local development', async () => {
  const harness = await startAccountService()
  for (const [base, secure] of [
    [site, true],
    ['http://extend.computer', true],
    ['http://localhost:3001', false],
    ['http://127.0.0.1:3001', false],
  ] as const) {
    const signup = await call(harness, '/auth/signup', {
      method: 'POST',
      data: { email: `cookie-${hex().slice(0, 8)}@example.invalid`, password },
      origin: base,
      base,
    })
    assert.equal(signup.status, 201, base)
    assert.match(
      signup.cookie!,
      /^extend_session=[a-f0-9]{64}; Path=\/; HttpOnly; SameSite=Lax; Max-Age=2592000/,
    )
    assert.equal(signup.cookie!.endsWith('; Secure'), secure, base)
    const options = await call(harness, '/passkeys/login/options', {
      method: 'POST',
      origin: base,
      base,
    })
    assert.equal(options.cookie!.endsWith('; Secure'), secure, base)
  }
})
