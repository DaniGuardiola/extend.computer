import { DurableObject } from 'cloudflare:workers'
import { createHash, randomBytes } from 'node:crypto'
import {
  generateRegistrationOptions,
  verifyRegistrationResponse,
  generateAuthenticationOptions,
  verifyAuthenticationResponse,
  type RegistrationResponseJSON,
  type AuthenticationResponseJSON,
} from '@simplewebauthn/server'
import { hashPassword, verifyPassword } from './password'

const SESSION_SECONDS = 30 * 24 * 60 * 60
const ONLINE_SECONDS = 90
const COOKIE = 'extend_session'
const CHALLENGE_COOKIE = 'extend_challenge'
const now = () => Math.floor(Date.now() / 1000)
const token = () => randomBytes(32).toString('hex')
const digest = (value: string) =>
  createHash('sha256').update(value).digest('hex')

class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
  }
}
const fail = (status: number, message: string): never => {
  throw new ApiError(status, message)
}
const json = (data: unknown, status = 200) =>
  Response.json(data, { status, headers: { 'Cache-Control': 'no-store' } })
const empty = () =>
  new Response(null, { status: 204, headers: { 'Cache-Control': 'no-store' } })

function cookie(request: Request, name: string): string | undefined {
  return request.headers
    .get('Cookie')
    ?.split(';')
    .map((s) => s.trim())
    .find((s) => s.startsWith(name + '='))
    ?.slice(name.length + 1)
}
function setCookie(
  response: Response,
  request: Request,
  name: string,
  value: string,
  age: number,
) {
  response.headers.append(
    'Set-Cookie',
    `${name}=${value}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${age}${new URL(request.url).protocol === 'https:' ? '; Secure' : ''}`,
  )
}
function bearer(request: Request): string | undefined {
  return request.headers
    .get('Authorization')
    ?.match(/^Bearer ([a-f0-9]{64})$/)?.[1]
}
function text(value: unknown, max: number, label: string): string {
  if (
    typeof value !== 'string' ||
    value.length === 0 ||
    Buffer.byteLength(value) > max ||
    /[\x00-\x1f\x7f]/.test(value)
  )
    fail(400, `Invalid ${label}`)
  return value as string
}
function email(value: unknown): string {
  const normalized = text(value, 254, 'email address').trim().toLowerCase()
  if (
    !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalized) ||
    !/^[\x20-\x7e]+$/.test(normalized)
  )
    fail(400, 'Invalid email address')
  return normalized
}
function password(value: unknown): string {
  if (
    typeof value !== 'string' ||
    Buffer.byteLength(value) < 12 ||
    Buffer.byteLength(value) > 1024
  )
    fail(400, 'Use a password of 12 to 1024 bytes')
  return value as string
}
async function body(request: Request): Promise<Record<string, unknown>> {
  if (!request.headers.get('Content-Type')?.startsWith('application/json'))
    fail(415, 'Send JSON')
  const reader = request.body?.getReader()
  if (!reader) fail(400, 'Missing request body')
  const chunks: Uint8Array[] = []
  let size = 0
  while (true) {
    const { done, value } = await reader!.read()
    if (done) break
    size += value.byteLength
    if (size > 16 * 1024) {
      await reader!.cancel()
      fail(413, 'Request too large')
    }
    chunks.push(value)
  }
  let result: unknown
  try {
    result = JSON.parse(Buffer.concat(chunks).toString())
  } catch {
    fail(400, 'Invalid JSON')
  }
  if (!result || typeof result !== 'object' || Array.isArray(result))
    fail(400, 'Invalid request')
  return result as Record<string, unknown>
}

function credentialBase(input: Record<string, unknown>) {
  if (input.type !== 'public-key') fail(400, 'Invalid credential type')
  if (
    !input.response ||
    typeof input.response !== 'object' ||
    Array.isArray(input.response)
  )
    fail(400, 'Invalid authenticator response')
  return {
    id: text(input.id, 2048, 'credential id'),
    rawId: text(input.rawId, 2048, 'credential id'),
    type: 'public-key' as const,
    clientExtensionResults: {},
  }
}
function authentication(
  input: Record<string, unknown>,
): AuthenticationResponseJSON {
  const base = credentialBase(input)
  const response = input.response as Record<string, unknown>
  return {
    ...base,
    response: {
      clientDataJSON: text(response.clientDataJSON, 4096, 'client data'),
      authenticatorData: text(
        response.authenticatorData,
        4096,
        'authenticator data',
      ),
      signature: text(response.signature, 4096, 'signature'),
      ...(response.userHandle
        ? { userHandle: text(response.userHandle, 2048, 'user handle') }
        : {}),
    },
  }
}
function registration(
  input: Record<string, unknown>,
): RegistrationResponseJSON {
  const base = credentialBase(input)
  const response = input.response as Record<string, unknown>
  const transports = Array.isArray(response.transports)
    ? response.transports
        .filter((value): value is string => typeof value === 'string')
        .slice(0, 8)
    : []
  return {
    ...base,
    response: {
      clientDataJSON: text(response.clientDataJSON, 4096, 'client data'),
      attestationObject: text(response.attestationObject, 12000, 'attestation'),
      transports,
    },
  }
}

type Account = { id: string; email: string; password_hash: string }
type Session = { account_id: string; token_hash: string }
type Passkey = {
  id: string
  account_id: string
  public_key: string
  counter: number
  transports: string
  rp_id: string
}
type Challenge = {
  challenge: string
  account_id: string | null
  kind: string
  origin: string
}

/** Per-IP coordinated abuse limits and bounded password work. Account data lives in D1. */
export class AccountService extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env)
    this.ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS attempts (key TEXT PRIMARY KEY, count INTEGER NOT NULL, until INTEGER NOT NULL)',
    )
    this.ctx.storage.sql.exec(
      'CREATE INDEX IF NOT EXISTS attempts_expiry ON attempts(until)',
    )
  }

  // Internal binding only. No public route bypasses the checks below.
  async fetch(request: Request): Promise<Response> {
    try {
      return await this.handle(request)
    } catch (error) {
      if (error instanceof ApiError) {
        const response = json({ error: error.message }, error.status)
        if (error.status === 429) response.headers.set('Retry-After', '60')
        return response
      }
      // Do not log request bodies, credentials, or authenticator responses.
      console.error(
        JSON.stringify({
          event: 'account_api_error',
          kind: error instanceof Error ? error.name : 'unknown',
        }),
      )
      return json({ error: 'Something went wrong. Please try again.' }, 500)
    }
  }

  private limit(key: string, max: number) {
    const stamp = now()
    const sql = this.ctx.storage.sql
    sql.exec(
      'DELETE FROM attempts WHERE key IN (SELECT key FROM attempts WHERE until <= ? LIMIT 100)',
      stamp,
    )
    const rows = sql
      .exec<{ count: number }>(
        `INSERT INTO attempts VALUES (?, 1, ?) ON CONFLICT(key) DO UPDATE SET count = CASE WHEN until <= ? THEN 1 ELSE count + 1 END, until = CASE WHEN until <= ? THEN excluded.until ELSE until END RETURNING count`,
        key,
        stamp + 60,
        stamp,
        stamp,
      )
      .toArray()
    if (rows[0]!.count > max)
      fail(429, 'Too many attempts. Try again in a minute.')
  }

  private async session(request: Request): Promise<Session> {
    const raw = bearer(request) ?? cookie(request, COOKIE)
    if (!raw || !/^[a-f0-9]{64}$/.test(raw)) fail(401, 'Please sign in')
    const found = await this.env.DB.prepare(
      'SELECT account_id, token_hash FROM sessions WHERE token_hash = ? AND expires_at > ?',
    )
      .bind(digest(raw!), now())
      .first<Session>()
    if (!found) fail(401, 'Please sign in')
    return found!
  }

  private async loginResponse(
    request: Request,
    account: { id: string; email: string },
    status = 200,
  ) {
    const current = now()
    const count = await this.env.DB.prepare(
      'SELECT COUNT(*) AS count FROM sessions WHERE account_id = ? AND expires_at > ?',
    )
      .bind(account.id, current)
      .first<{ count: number }>()
    if (count!.count >= 100)
      fail(429, 'Too many sessions. Sign out of an existing session first.')
    const raw = token()
    const expires_at = current + SESSION_SECONDS
    await this.env.DB.prepare('INSERT INTO sessions VALUES (?, ?, ?, ?)')
      .bind(digest(raw), account.id, expires_at, current)
      .run()
    // Browser keeps only an HttpOnly cookie. Native callers opt in to the token response.
    const native = request.headers.get('X-Extend-Client') === 'desktop'
    const response = json(
      {
        account: { id: account.id, email: account.email },
        expires_at,
        ...(native ? { token: raw } : {}),
      },
      status,
    )
    setCookie(response, request, COOKIE, raw, SESSION_SECONDS)
    return response
  }

  private async challenge(
    request: Request,
    kind: string,
    account: string | null,
    value: string,
  ) {
    const id = token()
    await this.env.DB.batch([
      this.env.DB.prepare(
        'DELETE FROM challenges WHERE id IN (SELECT id FROM challenges WHERE expires_at <= ? LIMIT 100)',
      ).bind(now()),
      this.env.DB.prepare(
        'INSERT INTO challenges VALUES (?, ?, ?, ?, ?, ?)',
      ).bind(
        digest(id),
        value,
        account,
        kind,
        new URL(request.url).origin,
        now() + 300,
      ),
    ])
    return id
  }

  private async consumeChallenge(
    request: Request,
    kind: string,
  ): Promise<Challenge> {
    const raw = cookie(request, CHALLENGE_COOKIE)
    if (!raw || !/^[a-f0-9]{64}$/.test(raw))
      fail(401, 'Passkey request expired. Try again.')
    const found = await this.env.DB.prepare(
      'DELETE FROM challenges WHERE id = ? AND kind = ? AND expires_at > ? RETURNING challenge, account_id, kind, origin',
    )
      .bind(digest(raw!), kind, now())
      .first<Challenge>()
    if (!found || found.origin !== new URL(request.url).origin)
      fail(401, 'Passkey request expired. Try again.')
    return found!
  }

  private async handle(request: Request): Promise<Response> {
    const url = new URL(request.url)
    const path = url.pathname.replace(/^\/api/, '')
    const method = request.method
    // Cookie-authenticated writes must originate on this exact website.
    if (method !== 'GET' && !bearer(request)) {
      if (request.headers.get('Origin') !== url.origin)
        fail(403, 'Request origin is not allowed')
    }
    const ip = request.headers.get('CF-Connecting-IP') ?? 'local'
    if (method !== 'GET') this.limit('write:' + digest(ip), 120)
    if (path.startsWith('/v1/auth/') || path.startsWith('/v1/passkeys/'))
      this.limit('auth:' + digest(ip), 20)

    if (path === '/v1/server' && method === 'GET')
      return json({
        api_version: 1,
        signup_enabled: this.env.SIGNUP_ENABLED === 'true',
        heartbeat_interval_seconds: 30,
        online_timeout_seconds: ONLINE_SECONDS,
      })

    if (
      (path === '/v1/auth/signup' || path === '/v1/auth/login') &&
      method === 'POST'
    ) {
      const input = await body(request)
      const address = email(input.email)
      const secret = password(input.password)
      this.limit('email:' + digest(address), 10)
      if (path.endsWith('/signup')) {
        if (this.env.SIGNUP_ENABLED !== 'true')
          fail(403, 'Account registration is closed')
        const id = token()
        const hash = hashPassword(secret)
        const created = await this.env.DB.prepare(
          'INSERT OR IGNORE INTO accounts VALUES (?, ?, ?, ?)',
        )
          .bind(id, address, hash, now())
          .run()
        if (!created.meta.changes)
          fail(409, 'Could not create account. Try signing in instead.')
        return this.loginResponse(request, { id, email: address }, 201)
      }
      const account = await this.env.DB.prepare(
        'SELECT * FROM accounts WHERE email = ?',
      )
        .bind(address)
        .first<Account>()
      // Same costly work for nonexistent accounts, without a per-isolate mutable cache.
      const hash =
        account?.password_hash ??
        'scrypt$32768$8$3$00000000000000000000000000000000$0000000000000000000000000000000000000000000000000000000000000000'
      const valid = verifyPassword(secret, hash)
      if (!account || !valid) fail(401, 'Email or password is incorrect')
      return this.loginResponse(request, account!)
    }

    if (path === '/v1/passkeys/login/options' && method === 'POST') {
      const options = await generateAuthenticationOptions({
        rpID: url.hostname,
        userVerification: 'required',
      })
      const id = await this.challenge(request, 'login', null, options.challenge)
      const response = json(options)
      setCookie(response, request, CHALLENGE_COOKIE, id, 300)
      return response
    }
    if (path === '/v1/passkeys/login/verify' && method === 'POST') {
      const input = await body(request)
      const challenge = await this.consumeChallenge(request, 'login')
      const key = await this.env.DB.prepare(
        'SELECT * FROM passkeys WHERE id = ? AND rp_id = ?',
      )
        .bind(text(input.id, 2048, 'passkey'), url.hostname)
        .first<Passkey>()
      if (!key) fail(401, 'Could not sign in with that passkey')
      let verification
      try {
        verification = await verifyAuthenticationResponse({
          response: authentication(input),
          expectedChallenge: challenge.challenge,
          expectedOrigin: challenge.origin,
          expectedRPID: url.hostname,
          credential: {
            id: key!.id,
            publicKey: new Uint8Array(
              Buffer.from(key!.public_key, 'base64url'),
            ),
            counter: key!.counter,
          },
          requireUserVerification: true,
        })
      } catch {
        fail(401, 'Could not verify passkey')
      }
      if (!verification!.verified) fail(401, 'Could not verify passkey')
      const updated = await this.env.DB.prepare(
        'UPDATE passkeys SET counter = ? WHERE id = ? AND counter = ?',
      )
        .bind(
          verification!.authenticationInfo.newCounter,
          key!.id,
          key!.counter,
        )
        .run()
      if (!updated.meta.changes) fail(401, 'Passkey changed. Try again.')
      const account = await this.env.DB.prepare(
        'SELECT id, email FROM accounts WHERE id = ?',
      )
        .bind(key!.account_id)
        .first<Account>()
      return this.loginResponse(request, account!)
    }

    const session = await this.session(request)
    const account = session.account_id
    const db = this.env.DB

    if (path === '/v1/account' && method === 'GET') {
      return json(
        await db
          .prepare('SELECT id, email, created_at FROM accounts WHERE id = ?')
          .bind(account)
          .first(),
      )
    }
    if (path === '/v1/auth/logout' && method === 'POST') {
      await db
        .prepare('DELETE FROM sessions WHERE token_hash = ?')
        .bind(session.token_hash)
        .run()
      const response = empty()
      setCookie(response, request, COOKIE, '', 0)
      return response
    }
    if (path === '/v1/sessions/others' && method === 'DELETE') {
      await db
        .prepare(
          'DELETE FROM sessions WHERE account_id = ? AND token_hash != ?',
        )
        .bind(account, session.token_hash)
        .run()
      return empty()
    }
    if (path === '/v1/account/password' && method === 'POST') {
      const input = await body(request)
      const current = await db
        .prepare('SELECT password_hash FROM accounts WHERE id = ?')
        .bind(account)
        .first<Account>()
      if (
        !verifyPassword(
          password(input.current_password),
          current!.password_hash,
        )
      )
        fail(401, 'Current password is incorrect')
      const hash = hashPassword(password(input.password))
      await db.batch([
        db
          .prepare('UPDATE accounts SET password_hash = ? WHERE id = ?')
          .bind(hash, account),
        db
          .prepare(
            'DELETE FROM sessions WHERE account_id = ? AND token_hash != ?',
          )
          .bind(account, session.token_hash),
      ])
      return empty()
    }

    if (path === '/v1/devices' && method === 'GET') {
      const { results } = await db
        .prepare(
          `SELECT d.id, d.fingerprint, d.name, d.platform, d.created_at,
        MAX(CASE WHEN s.expires_at > ? THEN ds.last_seen END) AS last_seen
        FROM devices d LEFT JOIN device_sessions ds ON ds.device_id = d.id
        LEFT JOIN sessions s ON s.token_hash = ds.session_hash
        WHERE d.account_id = ? GROUP BY d.id ORDER BY d.created_at, d.id`,
        )
        .bind(now(), account)
        .all<{ last_seen: number | null }>()
      return json({
        devices: results.map((d) => ({
          ...d,
          online: d.last_seen !== null && d.last_seen > now() - ONLINE_SECONDS,
        })),
      })
    }
    if (path === '/v1/devices' && method === 'POST') {
      const input = await body(request)
      const publicKey = text(input.public_key, 64, 'device public key')
      if (!/^[a-fA-F0-9]{64}$/.test(publicKey) || /^0+$/.test(publicKey))
        fail(400, 'Invalid device public key')
      const fingerprint = createHash('sha256')
        .update(Buffer.from(publicKey, 'hex'))
        .digest('hex')
      const name = text(input.name, 128, 'device name').trim()
      if (
        !name ||
        !['macos', 'windows', 'linux'].includes(String(input.platform))
      )
        fail(400, 'Invalid device registration')
      const existing = await db
        .prepare(
          'SELECT id FROM devices WHERE account_id = ? AND fingerprint = ?',
        )
        .bind(account, fingerprint)
        .first<{ id: string }>()
      const id = existing?.id ?? token()
      const deviceToken = token()
      const inserted = await db.batch([
        db
          .prepare(
            `INSERT INTO devices SELECT ?, ?, ?, ?, ?, ? WHERE (SELECT COUNT(*) FROM devices WHERE account_id = ?) < 100 OR EXISTS (SELECT 1 FROM devices WHERE account_id = ? AND fingerprint = ?)
          ON CONFLICT(account_id, fingerprint) DO UPDATE SET name = excluded.name, platform = excluded.platform`,
          )
          .bind(
            id,
            account,
            fingerprint,
            name,
            input.platform,
            now(),
            account,
            account,
            fingerprint,
          ),
        db
          .prepare(
            `INSERT INTO device_sessions SELECT ?, ?, id, NULL FROM devices WHERE fingerprint = ? AND account_id = ?
          ON CONFLICT(session_hash, device_id) DO UPDATE SET token_hash = excluded.token_hash, last_seen = NULL`,
          )
          .bind(digest(deviceToken), session.token_hash, fingerprint, account),
      ])
      if (!inserted[1]!.meta.changes) fail(429, 'Device limit reached')
      const registered = await db
        .prepare(
          'SELECT id FROM devices WHERE account_id = ? AND fingerprint = ?',
        )
        .bind(account, fingerprint)
        .first<{ id: string }>()
      return json({
        id: registered!.id,
        fingerprint,
        device_token: deviceToken,
      })
    }
    const devicePath = path.match(/^\/v1\/devices\/([a-f0-9]{64})$/)
    if (devicePath && method === 'DELETE') {
      const removed = await db
        .prepare('DELETE FROM devices WHERE id = ? AND account_id = ?')
        .bind(devicePath[1], account)
        .run()
      if (!removed.meta.changes) fail(404, 'Device not found')
      return empty()
    }

    if (path === '/v1/passkeys' && method === 'GET') {
      const { results } = await db
        .prepare(
          'SELECT id, name, created_at, rp_id FROM passkeys WHERE account_id = ? ORDER BY created_at',
        )
        .bind(account)
        .all()
      return json({ passkeys: results })
    }
    if (path === '/v1/passkeys/register/options' && method === 'POST') {
      const user = await db
        .prepare('SELECT email FROM accounts WHERE id = ?')
        .bind(account)
        .first<{ email: string }>()
      const { results } = await db
        .prepare('SELECT id FROM passkeys WHERE account_id = ?')
        .bind(account)
        .all<{ id: string }>()
      if (results.length >= 10) fail(400, 'Passkey limit reached')
      const options = await generateRegistrationOptions({
        rpName: 'extend.computer',
        rpID: url.hostname,
        userName: user!.email,
        userID: new Uint8Array(Buffer.from(account, 'hex')),
        attestationType: 'none',
        authenticatorSelection: {
          residentKey: 'required',
          userVerification: 'required',
        },
        excludeCredentials: results.map((k) => ({ id: k.id })),
      })
      const id = await this.challenge(
        request,
        'register',
        account,
        options.challenge,
      )
      const response = json(options)
      setCookie(response, request, CHALLENGE_COOKIE, id, 300)
      return response
    }
    if (path === '/v1/passkeys/register/verify' && method === 'POST') {
      const input = await body(request)
      const challenge = await this.consumeChallenge(request, 'register')
      if (challenge.account_id !== account)
        fail(401, 'Passkey request belongs to another session')
      let verification
      try {
        verification = await verifyRegistrationResponse({
          response: registration(input),
          expectedChallenge: challenge.challenge,
          expectedOrigin: challenge.origin,
          expectedRPID: url.hostname,
          requireUserVerification: true,
        })
      } catch {
        fail(400, 'Could not verify passkey')
      }
      if (!verification!.verified) fail(400, 'Could not verify passkey')
      const credential = verification!.registrationInfo!.credential
      await db
        .prepare('INSERT INTO passkeys VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
        .bind(
          credential.id,
          account,
          Buffer.from(credential.publicKey).toString('base64url'),
          credential.counter,
          JSON.stringify(credential.transports ?? []),
          'Passkey',
          url.hostname,
          now(),
        )
        .run()
      return json({ success: true }, 201)
    }
    const passkeyPath = path.match(/^\/v1\/passkeys\/([^/]+)$/)
    if (passkeyPath && method === 'DELETE') {
      const removed = await db
        .prepare('DELETE FROM passkeys WHERE id = ? AND account_id = ?')
        .bind(passkeyPath[1], account)
        .run()
      if (!removed.meta.changes) fail(404, 'Passkey not found')
      return empty()
    }
    return json({ error: 'Not found' }, 404)
  }
}

export async function heartbeat(request: Request, env: Env): Promise<Response> {
  const id = new URL(request.url).pathname.match(
    /\/v1\/devices\/([a-f0-9]{64})\/heartbeat$/,
  )?.[1]
  const raw = bearer(request)
  if (!id || !raw) return json({ error: 'Invalid device credentials' }, 401)
  const result = await env.DB.prepare(
    `UPDATE device_sessions SET last_seen = ? WHERE token_hash = ? AND device_id = ?
    AND session_hash IN (SELECT token_hash FROM sessions WHERE expires_at > ?)`,
  )
    .bind(now(), digest(raw), id, now())
    .run()
  return result.meta.changes
    ? empty()
    : json({ error: 'Invalid device credentials' }, 401)
}
