import { test } from 'node:test'
import assert from 'node:assert/strict'
import { hashPassword, verifyPassword } from '../src/lib/password.ts'
test('password verification rejects wrong credentials and damaged hashes', () => {
  const hash = hashPassword('a sufficiently long password')
  assert.equal(verifyPassword('a sufficiently long password', hash), true)
  assert.equal(verifyPassword('wrong long password', hash), false)
  assert.equal(verifyPassword('a sufficiently long password', 'broken'), false)
  assert.equal(
    verifyPassword(
      'a sufficiently long password',
      hash.replace('32768', '999999999'),
    ),
    false,
  )
})
test('random salts produce distinct password hashes', () => {
  assert.notEqual(
    hashPassword('another long password'),
    hashPassword('another long password'),
  )
})
