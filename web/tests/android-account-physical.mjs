// Disposable, local-only service setup for a real tablet browser/Keystore check.
// No native app credential is requested, logged, or copied into this process.
import assert from 'node:assert/strict'
import { createHash, createPublicKey, diffieHellman, generateKeyPairSync, randomBytes, randomUUID } from 'node:crypto'
import { chmod, mkdir, readFile, rename, stat, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'

const web = fileURLToPath(new URL('..', import.meta.url))
const repository = dirname(web)
const allowedRoot = join(repository, 'native/android/.local')
const database = 'extend-android-account-physical'

export function fixturePath(input) {
  const root = resolve(input)
  assert.ok(root.startsWith(allowedRoot + '/'), 'Fixture state must be inside native/android/.local/')
  return root
}
export function fixtureConfig(root, origin, databaseId) {
  return {
    name: 'extend-android-account-physical',
    main: join(web, 'src/server.ts'),
    compatibility_date: '2026-09-01', // Match the account website's current runtime semantics.
    compatibility_flags: ['nodejs_compat'],
    workers_dev: false,
    routes: [],
    d1_databases: [{ binding: 'DB', database_name: database, database_id: databaseId, migrations_dir: join(web, 'migrations') }],
    durable_objects: { bindings: [{ name: 'ACCOUNTS', class_name: 'AccountService' }, { name: 'RELAY', class_name: 'AccountRelay' }] },
    migrations: [{ tag: 'v1', new_sqlite_classes: ['AccountService'] }, { tag: 'v2', new_sqlite_classes: ['AccountRelay'] }],
    vars: { SIGNUP_ENABLED: 'true', RELAY_ENABLED: 'false', PUBLIC_ORIGIN: origin, RESEND_API_KEY: '', EMAIL_FROM: '' },
  }
}
async function save(root, file, value) {
  const destination = join(root, file), temporary = destination + '.tmp'
  await writeFile(temporary, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 })
  await chmod(temporary, 0o600)
  await rename(temporary, destination)
}
async function load(root) {
  assert.equal((await stat(root)).mode & 0o077, 0, 'Fixture directory must be private')
  const fixture = JSON.parse(await readFile(join(root, 'fixture.json'), 'utf8'))
  const url = new URL(fixture.origin)
  assert.equal(fixture.version, 1)
  assert.equal(url.protocol, 'http:')
  assert.equal(url.hostname, '127.0.0.1')
  assert.equal(url.pathname, '/')
  assert.ok(url.port && !url.username && !url.password && !url.search && !url.hash)
  assert.equal((await stat(join(root, 'credentials.json'))).mode & 0o077, 0, 'Fixture credentials must be private')
  return { fixture, credentials: JSON.parse(await readFile(join(root, 'credentials.json'), 'utf8')) }
}
function localEnvironment(root) {
  const env = { ...process.env, EXTEND_ANDROID_ACCOUNT_FIXTURE: root, CLOUDFLARE_LOAD_DEV_VARS_FROM_DOT_ENV: 'false', CLOUDFLARE_INCLUDE_PROCESS_ENV: 'false', WRANGLER_SEND_METRICS: 'false' }
  for (const key of Object.keys(env)) {
    if (/^(CLOUDFLARE_(API|ACCOUNT|ENV)|CF_(API|ACCOUNT)|RESEND_|MFA_ENCRYPTION_KEY)/.test(key)) delete env[key]
  }
  delete env.CLOUDFLARE_VITE_WRANGLER_CONFIG_PATH
  return env
}
async function run(root, binary, args) {
  const child = spawn(process.execPath, [binary, ...args], { cwd: web, env: localEnvironment(root), stdio: 'inherit' })
  const forward = signal => child.kill(signal)
  process.once('SIGINT', forward); process.once('SIGTERM', forward)
  const code = await new Promise((accept, reject) => { child.once('error', reject); child.once('exit', (code, signal) => accept(signal ? 1 : code)) })
  process.removeListener('SIGINT', forward); process.removeListener('SIGTERM', forward)
  assert.equal(code, 0, 'Local fixture process failed')
}
async function request(origin, path, method = 'GET', payload, token) {
  const response = await fetch(origin + path, {
    method, redirect: 'error', signal: AbortSignal.timeout(12000),
    headers: { Origin: origin, 'X-Extend-Client': 'desktop', 'Content-Type': 'application/json', ...(token ? { Authorization: 'Bearer ' + token } : {}) },
    ...(payload === undefined ? {} : { body: JSON.stringify(payload) }),
  })
  // Status is enough for failures. Never print a server response containing credentials.
  assert.ok(response.ok, `${method} ${path} failed (${response.status})`)
  return response.status === 204 ? null : response.json()
}
async function preflight(origin) {
  assert.deepEqual(await request(origin, '/healthz'), { status: 'ok' })
  const server = await request(origin, '/v1/server')
  assert.equal(server.desktop_browser_login, true)
  assert.equal(server.signup_enabled, true)
  assert.equal(server.email_enabled, false)
  return server
}
// Raw JWK d is the same 32-byte X25519 seed consumed by Identity::from_secret.
// Persist only this newly generated fixture key, never an app identity.
export function disposablePeerKey() {
  const { publicKey, privateKey } = generateKeyPairSync('x25519')
  const public_key = Buffer.from(publicKey.export({ format: 'jwk' }).x, 'base64url').toString('hex')
  const seed = Buffer.from(privateKey.export({ format: 'jwk' }).d, 'base64url')
  assert.equal(seed.length, 32)
  const peer_seed = seed.toString('hex')
  seed.fill(0)
  return { privateKey, public_key, peer_seed, peer_fingerprint: createHash('sha256').update(Buffer.from(public_key, 'hex')).digest('hex') }
}
async function seed(root, fixture, credentials) {
  await preflight(fixture.origin)
  assert.ok(!credentials.observer_token, 'Fixture was already seeded')
  const signup = await request(fixture.origin, '/v1/auth/signup', 'POST', { email: credentials.email, password: credentials.password })
  credentials.account_id = signup.account.id
  credentials.observer_token = signup.token
  // Save immediately so a failed peer proof can be cleaned up without losing ownership.
  await save(root, 'credentials.json', credentials)
  const { privateKey, public_key, peer_seed, peer_fingerprint } = disposablePeerKey()
  Object.assign(credentials, { peer_seed, peer_fingerprint, peer_seed_kind: 'disposable-account-fixture-v1' })
  await save(root, 'credentials.json', credentials)
  const peer = await request(fixture.origin, '/v1/devices', 'POST', { name: 'Account fixture peer', platform: 'macos', public_key }, credentials.observer_token)
  assert.equal(peer.fingerprint, peer_fingerprint, 'Registered disposable peer identity changed')
  credentials.peer_id = peer.id; credentials.peer_token = peer.device_token
  await save(root, 'credentials.json', credentials)
  const options = await request(fixture.origin, `/v1/devices/${peer.id}/proof/options`, 'POST', {}, peer.device_token)
  const remote = createPublicKey({ key: { kty: 'OKP', crv: 'X25519', x: Buffer.from(options.server_key, 'hex').toString('base64url') }, format: 'jwk' })
  const shared = diffieHellman({ privateKey, publicKey: remote })
  const proof = createHash('sha256').update('extend.computer/device-proof/v1\0').update(shared).update(Buffer.from(options.challenge, 'hex')).digest('hex')
  shared.fill(0)
  await request(fixture.origin, `/v1/devices/${peer.id}/proof/verify`, 'POST', { challenge: options.challenge, proof }, peer.device_token)
  await request(fixture.origin, `/v1/devices/${peer.id}/heartbeat`, 'POST', {}, peer.device_token)
  credentials.peer_id = peer.id; credentials.peer_token = peer.device_token
  await save(root, 'credentials.json', credentials)
  console.log(JSON.stringify({ seeded: true, email: credentials.email, peer_id: peer.id, peer_fingerprint: peer.fingerprint, key_verified: true }))
}
async function main() {
  const [command, directory, argument] = process.argv.slice(2)
  assert.ok(command && directory, 'Usage: node tests/android-account-physical.mjs prepare|migrate|serve|seed|status|revoke <state-directory> [port|device-id]')
  const root = fixturePath(directory)
  if (command === 'prepare') {
    const port = Number(argument || 3046)
    assert.ok(Number.isInteger(port) && port >= 1024 && port <= 65535)
    await mkdir(root, { mode: 0o700 }) // Existing state is never overwritten.
    await chmod(root, 0o700)
    const fixture = { version: 1, origin: `http://127.0.0.1:${port}`, prepared_at: new Date().toISOString() }
    await save(root, 'fixture.json', fixture)
    await save(root, 'wrangler.json', fixtureConfig(root, fixture.origin, randomUUID()))
    await save(root, 'credentials.json', { email: `android-physical-${randomBytes(6).toString('hex')}@example.invalid`, password: randomBytes(24).toString('base64url') })
    // This config's directory controls local secret lookup; no production secrets are inherited.
    await writeFile(join(root, '.dev.vars'), 'RESEND_API_KEY=""\nEMAIL_FROM=""\n', { mode: 0o600 })
    console.log(JSON.stringify({ prepared: true, origin: fixture.origin, credentials_file: join(root, 'credentials.json'), state: join(root, 'state') }))
    return
  }
  const { fixture, credentials } = await load(root)
  if (command === 'migrate') return run(root, join(web, 'node_modules/wrangler/bin/wrangler.js'), ['d1', 'migrations', 'apply', database, '--config', join(root, 'wrangler.json'), '--local', '--persist-to', join(root, 'state')])
  if (command === 'serve') return run(root, join(web, 'node_modules/vite/bin/vite.js'), ['--config', 'tests/android-account-physical.vite.config.ts'])
  if (command === 'seed') return seed(root, fixture, credentials)
  await preflight(fixture.origin)
  assert.ok(credentials.observer_token, 'Seed the fixture first')
  if (command === 'status') {
    const directory = await request(fixture.origin, '/v1/devices', 'GET', undefined, credentials.observer_token)
    const devices = directory.devices.map(({ id, fingerprint, name, platform, key_verified, online, last_seen }) => ({ id, fingerprint, name, platform, key_verified: !!key_verified, online, last_seen }))
    console.log(JSON.stringify({ email: credentials.email, devices }, null, 2))
  } else if (command === 'revoke') {
    assert.match(argument || '', /^[a-f0-9]{64}$/)
    await request(fixture.origin, `/v1/devices/${argument}`, 'DELETE', undefined, credentials.observer_token)
    console.log(JSON.stringify({ revoked: argument }))
  } else throw new Error('Unknown fixture command')
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1 })
}
