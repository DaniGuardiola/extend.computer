import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash, createPrivateKey, createPublicKey } from 'node:crypto'
import { disposablePeerKey, fixtureConfig, fixturePath } from './android-account-physical.mjs'

const repository = fileURLToPath(new URL('../..', import.meta.url))
test('fixture paths cannot select regular app state or leave ignored native test state', () => {
  const root = join(repository, 'native/android/.local/account-fixture-test')
  assert.equal(fixturePath(root), root)
  for (const path of [repository, '/tmp/account-test', join(repository, 'web/.wrangler/state'), join(root, '../../../../../../tmp/account')]) {
    assert.throws(() => fixturePath(path))
  }
})
test('local fixture config owns distinct storage, excludes cloud and disables external mail', async () => {
  const root = fixturePath(join(repository, 'native/android/.local/account-fixture-test'))
  const config = fixtureConfig(root, 'http://127.0.0.1:3046', '00000000-0000-4000-8000-000000000001')
  assert.equal(config.account_id, undefined)
  assert.equal(config.workers_dev, false)
  assert.deepEqual(config.routes, [])
  assert.equal(config.d1_databases.length, 1)
  assert.equal(config.d1_databases[0].remote, undefined)
  assert.equal(config.d1_databases[0].database_name, 'extend-android-account-physical')
  assert.notEqual(config.d1_databases[0].database_id, 'aa7b3297-a30e-4238-8bb2-9ae446e0b5f4')
  assert.deepEqual(config.vars, { SIGNUP_ENABLED: 'true', RELAY_ENABLED: 'false', PUBLIC_ORIGIN: 'http://127.0.0.1:3046', RESEND_API_KEY: '', EMAIL_FROM: '' })
  const vite = await readFile(join(dirname(fileURLToPath(import.meta.url)), 'android-account-physical.vite.config.ts'), 'utf8')
  assert.match(vite, /remoteBindings: false/)
  assert.match(vite, /inspectorPort: false/)
  assert.match(vite, /tunnel: false/)
  assert.match(vite, /persistState: \{ path: join\(root, 'state'\) \}/)
})

test('generated disposable seed restores exactly the registered X25519 public identity', () => {
  const peer = disposablePeerKey()
  const restored = createPrivateKey({ format: 'jwk', key: {
    kty: 'OKP', crv: 'X25519', x: Buffer.from(peer.public_key, 'hex').toString('base64url'),
    d: Buffer.from(peer.peer_seed, 'hex').toString('base64url'),
  } })
  const publicBytes = Buffer.from(createPublicKey(restored).export({ format: 'jwk' }).x, 'base64url')
  assert.equal(publicBytes.toString('hex'), peer.public_key)
  assert.equal(createHash('sha256').update(publicBytes).digest('hex'), peer.peer_fingerprint)
  assert.equal(peer.peer_seed.length, 64)
  assert.equal(peer.privateKey.type, 'private')
})
