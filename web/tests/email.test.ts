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
test('email links keep tokens in fragments and explain recovery revocation', async () => {
  const email = await accountEmail(
    'reset',
    'user@example.com',
    token,
    env.PUBLIC_ORIGIN,
  )
  assert.ok(email.text.includes('https://example.com/email#reset=' + token))
  assert.ok(email.text.includes('Two-factor authentication stays enabled'))
  assert.ok(email.text.includes('Your passkeys and security keys are kept'))
  assert.ok(!email.text.includes('Passkeys are removed'))
  assert.ok(email.html.includes('expires in 30 minutes'))
  await assert.rejects(() =>
    accountEmail('verify', 'user@example.com', token, 'http://example.com'),
  )
  await assert.rejects(() =>
    accountEmail(
      'verify',
      'user@example.com',
      token,
      'https://example.com/path',
    ),
  )
  assert.equal(emailEnabled({}), false)
})
test('both templates render HTML and plain text with the same action and fallback link', async () => {
  for (const kind of ['verify', 'reset'] as const) {
    const email = await accountEmail(
      kind,
      'user@example.com',
      token,
      env.PUBLIC_ORIGIN,
    )
    const link = `${env.PUBLIC_ORIGIN}/email#${kind}=${token}`
    assert.ok(email.html.includes('<html'))
    assert.ok(email.html.includes(`href="${link}"`))
    assert.ok(email.html.includes('Button not working?'))
    assert.ok(email.text.includes(link))
    assert.ok(email.text.includes('expires in 30 minutes'))
    assert.ok(!email.text.includes('<table'))
    assert.ok(email.html.length < 100_000, 'Stay below Gmail clipping size')
    assert.deepEqual(email.to, ['user@example.com'])
  }
  await assert.rejects(() =>
    accountEmail('verify', 'user@example.com', 'invalid', env.PUBLIC_ORIGIN),
  )
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
