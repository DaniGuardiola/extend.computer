import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  accountEmail,
  emailEnabled,
  sendAccountEmail,
} from '../src/lib/email.ts'
const token = 'a'.repeat(64)
const env = {
  RESEND_API_KEY: 'test-key',
  EMAIL_FROM: 'extend <accounts@example.com>',
  PUBLIC_ORIGIN: 'https://example.com',
}
test('email links keep tokens in fragments and explain recovery revocation', () => {
  const email = accountEmail(
    'reset',
    'user@example.com',
    token,
    env.PUBLIC_ORIGIN,
  )
  assert.ok(email.text.includes('https://example.com/email#reset=' + token))
  assert.ok(email.text.includes('Two-factor authentication stays enabled'))
  assert.ok(email.html.includes('expires in 30 minutes'))
  assert.throws(() =>
    accountEmail('verify', 'user@example.com', token, 'http://example.com'),
  )
  assert.throws(() =>
    accountEmail(
      'verify',
      'user@example.com',
      token,
      'https://example.com/path',
    ),
  )
  assert.equal(emailEnabled({}), false)
})
test('mail transport uses idempotency and fails without exposing provider errors', async () => {
  let sent = 0
  await sendAccountEmail(
    env,
    'verify',
    'user@example.com',
    token,
    async (url, options) => {
      sent++
      assert.equal(url, 'https://api.resend.com/emails')
      assert.equal(
        (options!.headers as Record<string, string>)['Idempotency-Key'],
        token,
      )
      assert.deepEqual(JSON.parse(options!.body as string).to, [
        'user@example.com',
      ])
      return new Response('{}', { status: 200 })
    },
  )
  assert.equal(sent, 1)
  await assert.rejects(
    sendAccountEmail(
      env,
      'reset',
      'user@example.com',
      token,
      async () => new Response('provider secret', { status: 429 }),
    ),
    { message: 'Email delivery unavailable' },
  )
  await assert.rejects(
    sendAccountEmail({}, 'reset', 'user@example.com', token),
    { message: 'Email is not configured' },
  )
})
