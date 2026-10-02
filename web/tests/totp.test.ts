import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  base32,
  totp,
  matchTotp,
  sealSecret,
  openSecret,
} from '../src/lib/totp.ts'
const secret = base32(Buffer.from('12345678901234567890'))
test('RFC 6238 SHA-1 vectors, including timestamps after 2038', () => {
  for (const [seconds, expected] of [
    [59, '94287082'],
    [1111111109, '07081804'],
    [1111111111, '14050471'],
    [1234567890, '89005924'],
    [2000000000, '69279037'],
    [20000000000, '65353130'],
  ] as const)
    assert.equal(totp(secret, Math.floor(seconds / 30), 8), expected)
})
test('clock skew is bounded and a previously accepted step cannot replay', () => {
  const code = totp(secret, 100)
  assert.equal(matchTotp(secret, code, 3000), 100)
  assert.equal(matchTotp(secret, code, 3030), 100)
  assert.equal(matchTotp(secret, code, 3060), null)
  assert.equal(matchTotp(secret, code, 3000, 100), null)
  assert.equal(matchTotp(secret, '00000', 3000), null)
})
test('TOTP ciphertext is randomized, account-bound and rejects tampering', () => {
  const key = '12'.repeat(32)
  const a = sealSecret(secret, key, 'account-a')
  const b = sealSecret(secret, key, 'account-a')
  assert.notEqual(a, b)
  assert.equal(openSecret(a, key, 'account-a'), secret)
  assert.throws(() => openSecret(a, key, 'account-b'))
  assert.throws(() => openSecret(a, '13'.repeat(32), 'account-a'))
  const changed = Buffer.from(a, 'base64url')
  changed[30] ^= 1
  assert.throws(() =>
    openSecret(changed.toString('base64url'), key, 'account-a'),
  )
})
