import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  hashRecoveryCode,
  newRecoveryCodes,
  recoverySalt,
} from '../src/lib/recovery-codes.ts'

test('backup codes are unique eight-digit numbers with salted slow hashes', () => {
  const codes = newRecoveryCodes()
  assert.equal(codes.length, 10)
  assert.equal(new Set(codes.map((code) => code.display)).size, 10)
  const salt = recoverySalt(codes[0]!.hash)!
  assert.match(salt, /^[a-f0-9]{32}$/)
  for (const code of codes) {
    assert.match(code.display, /^\d{4} \d{4}$/)
    assert.equal(recoverySalt(code.hash), salt)
    assert.equal(
      hashRecoveryCode(code.display.replace(' ', ''), salt),
      code.hash,
    )
  }
  assert.notEqual(
    hashRecoveryCode('00001234', salt),
    hashRecoveryCode('00001235', salt),
  )
  assert.notEqual(
    hashRecoveryCode('00001234', salt),
    hashRecoveryCode('00001234', 'ff'.repeat(16)),
  )
  assert.equal(recoverySalt('broken'), undefined)
  assert.throws(() => hashRecoveryCode('123456', salt))
})
