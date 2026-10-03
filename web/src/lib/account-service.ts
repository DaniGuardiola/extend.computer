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
import { matchTotp, newTotpSecret, sealSecret, openSecret } from './totp'
import { hashPassword, verifyPassword } from './password'
import { emailEnabled, sendAccountEmail, type EmailKind } from './email'

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

type Account = {
  id: string
  email: string
  password_hash: string
  email_verified: number
  mfa_enabled: number
  mfa_version: number
}
type Session = { account_id: string; token_hash: string }
type Passkey = {
  id: string
  account_id: string
  public_key: string
  counter: number
  transports: string
  rp_id: string
  purpose: string
}
type MfaTicket = {
  id: string
  account_id: string
  email: string
  password_version: string
  version: number
  purpose: string
  session_hash: string | null
  expires_at: number
  attempts: number
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

  // One named coordinator enforces account-wide free email budgets across all IP shards.
  async reserveEmail(): Promise<boolean> {
    const date = new Date().toISOString()
    const day = 'mail-day:' + date.slice(0, 10)
    const month = 'mail-month:' + date.slice(0, 7)
    const sql = this.ctx.storage.sql
    const stamp = now()
    sql.exec('DELETE FROM attempts WHERE until <= ?', stamp)
    for (const [key, max] of [
      [day, 90],
      [month, 2700],
    ] as const) {
      const row = sql
        .exec<{ count: number }>(
          'SELECT count FROM attempts WHERE key = ?',
          key,
        )
        .toArray()[0]
      if ((row?.count ?? 0) >= max) return false
    }
    const tomorrow =
      Math.floor(Date.parse(date.slice(0, 10) + 'T00:00:00Z') / 1000) + 86400
    const nextMonth = new Date(date)
    nextMonth.setUTCDate(1)
    nextMonth.setUTCMonth(nextMonth.getUTCMonth() + 1)
    nextMonth.setUTCHours(0, 0, 0, 0)
    for (const [key, until] of [
      [day, tomorrow],
      [month, Math.floor(nextMonth.getTime() / 1000)],
    ] as const)
      sql.exec(
        'INSERT INTO attempts VALUES (?, 1, ?) ON CONFLICT(key) DO UPDATE SET count = count + 1',
        key,
        until,
      )
    return true
  }

  private async sendEmail(account: Account, kind: EmailKind): Promise<void> {
    if (!emailEnabled(this.env))
      fail(
        503,
        'Email is not available yet. You can still sign in with a passkey.',
      )
    const allowed = await this.env.DB.prepare(
      'INSERT INTO email_cooldowns VALUES (?, ?) ON CONFLICT(account_id) DO UPDATE SET next_send = excluded.next_send WHERE next_send <= ? RETURNING account_id',
    )
      .bind(account.id, now() + 60, now())
      .first()
    if (!allowed) fail(429, 'Wait a minute before requesting another email.')
    if (!(await this.env.ACCOUNTS.getByName('mail-budget').reserveEmail()))
      fail(503, 'Email is temporarily unavailable. Try again later.')
    const raw = token()
    const id = digest(raw)
    await this.env.DB.batch([
      this.env.DB.prepare(
        'DELETE FROM email_tokens WHERE account_id = ? AND kind = ?',
      ).bind(account.id, kind),
      this.env.DB.prepare(
        'INSERT INTO email_tokens VALUES (?, ?, ?, ?, ?)',
      ).bind(id, account.id, kind, account.password_hash, now() + 1800),
    ])
    try {
      await sendAccountEmail(this.env, kind, account.email, raw)
    } catch {
      await this.env.DB.prepare('DELETE FROM email_tokens WHERE token_hash = ?')
        .bind(id)
        .run()
      fail(503, 'Email is temporarily unavailable. Try again later.')
    }
  }

  private async session(request: Request): Promise<Session> {
    const raw = bearer(request) ?? cookie(request, COOKIE)
    if (!raw || !/^[a-f0-9]{64}$/.test(raw)) fail(401, 'Please sign in')
    const found = await this.env.DB.prepare(
      'SELECT s.account_id, s.token_hash FROM sessions s JOIN accounts a ON a.id=s.account_id WHERE s.token_hash = ? AND s.expires_at > ? AND s.mfa_version=a.mfa_version',
    )
      .bind(digest(raw!), now())
      .first<Session>()
    if (!found) fail(401, 'Please sign in')
    return found!
  }

  private async mfaInfo(account: string) {
    const totp = await this.env.DB.prepare(
      'SELECT account_id FROM totp_factors WHERE account_id = ?',
    )
      .bind(account)
      .first()
    const keys = await this.env.DB.prepare(
      'SELECT COUNT(*) AS n FROM passkeys WHERE account_id = ?',
    )
      .bind(account)
      .first<{ n: number }>()
    const recovery = await this.env.DB.prepare(
      'SELECT COUNT(*) AS n FROM recovery_codes WHERE account_id = ?',
    )
      .bind(account)
      .first<{ n: number }>()
    return {
      totp: !!totp,
      keys: keys!.n,
      recovery_codes: recovery!.n,
      totp_available: !!this.env.MFA_ENCRYPTION_KEY,
    }
  }

  private async mfaPending(
    account: Account,
    purpose = 'login',
    session: string | null = null,
  ) {
    const raw = token()
    const inserted = await this.env.DB.batch([
      this.env.DB.prepare('DELETE FROM mfa_tickets WHERE expires_at <= ?').bind(
        now(),
      ),
      this.env.DB.prepare(
        'INSERT INTO mfa_tickets (id,account_id,password_version,version,purpose,session_hash,expires_at) SELECT ?,id,password_hash,mfa_version,?,?,? FROM accounts WHERE id=? AND password_hash=? AND mfa_version=? AND (SELECT COUNT(*) FROM mfa_tickets WHERE account_id=accounts.id)<20',
      ).bind(
        digest(raw),
        purpose,
        session,
        now() + 300,
        account.id,
        account.password_hash,
        account.mfa_version,
      ),
    ])
    if (!inserted[1]!.meta.changes)
      fail(
        429,
        'Too many pending sign-ins or security settings changed. Try again later.',
      )
    const info = await this.mfaInfo(account.id)
    return json({
      mfa_required: true,
      ticket: raw,
      totp: info.totp,
      keys: info.keys > 0,
      recovery: true,
    })
  }

  private async mfaTicket(raw: unknown) {
    if (typeof raw !== 'string' || !/^[a-f0-9]{64}$/.test(raw))
      fail(401, 'Verification expired. Sign in again.')
    const ticket = await this.env.DB.prepare(
      `SELECT t.*, a.email, a.mfa_enabled FROM mfa_tickets t JOIN accounts a ON a.id=t.account_id
      WHERE t.id=? AND t.expires_at>? AND t.attempts<5 AND a.password_hash=t.password_version AND a.mfa_version=t.version`,
    )
      .bind(digest(raw as string), now())
      .first<MfaTicket>()
    if (!ticket) fail(401, 'Verification expired. Sign in again.')
    return ticket!
  }

  private async mfaAttempt(ticket: MfaTicket) {
    const updated = await this.env.DB.prepare(
      'UPDATE mfa_tickets SET attempts=attempts+1 WHERE id=? AND attempts<5 AND expires_at>?',
    )
      .bind(ticket.id, now())
      .run()
    if (!updated.meta.changes) fail(401, 'Verification expired. Sign in again.')
    const attempt = await this.env.DB.prepare(
      `INSERT INTO mfa_attempts VALUES (?,1,?) ON CONFLICT(account_id) DO UPDATE SET
      count=CASE WHEN until<=? THEN 1 ELSE count+1 END, until=CASE WHEN until<=? THEN excluded.until ELSE until END RETURNING count`,
    )
      .bind(ticket.account_id, now() + 60, now(), now())
      .first<{ count: number }>()
    if (attempt!.count > 10)
      fail(429, 'Too many verification attempts. Try again in a minute.')
  }

  private async mfaCode(account: string, code: unknown) {
    if (typeof code !== 'string') fail(401, 'Invalid verification code')
    if (/^\d{6}$/.test(code as string)) {
      const factor = await this.env.DB.prepare(
        'SELECT secret,last_step FROM totp_factors WHERE account_id=?',
      )
        .bind(account)
        .first<{ secret: string; last_step: number }>()
      if (!factor || !this.env.MFA_ENCRYPTION_KEY)
        fail(401, 'Invalid verification code')
      const secret = openSecret(
        factor!.secret,
        this.env.MFA_ENCRYPTION_KEY!,
        account,
      )
      const step = matchTotp(secret, code, now(), factor!.last_step)
      if (step === null) fail(401, 'Invalid or already used verification code')
      const used = await this.env.DB.prepare(
        'UPDATE totp_factors SET last_step=? WHERE account_id=? AND secret=? AND last_step<?',
      )
        .bind(step, account, factor!.secret, step)
        .run()
      if (!used.meta.changes)
        fail(401, 'Invalid or already used verification code')
    } else {
      const normalized = (code as string).replace(/[- ]/g, '').toLowerCase()
      if (!/^[a-f0-9]{32}$/.test(normalized))
        fail(401, 'Invalid verification code')
      const used = await this.env.DB.prepare(
        'DELETE FROM recovery_codes WHERE account_id=? AND code_hash=? RETURNING code_hash',
      )
        .bind(account, digest(normalized))
        .first()
      if (!used) fail(401, 'Invalid or already used recovery code')
    }
  }

  private async mfaFinish(request: Request, ticket: MfaTicket) {
    const consumed = await this.env.DB.prepare(
      `DELETE FROM mfa_tickets WHERE id=? AND expires_at>? AND
      EXISTS(SELECT 1 FROM accounts WHERE id=? AND password_hash=? AND mfa_version=?) RETURNING id`,
    )
      .bind(
        ticket.id,
        now(),
        ticket.account_id,
        ticket.password_version,
        ticket.version,
      )
      .first()
    if (!consumed) fail(401, 'Verification expired. Sign in again.')
    if (ticket.purpose === 'manage') {
      const updated = await this.env.DB.prepare(
        `UPDATE sessions SET elevated_until=? WHERE token_hash=? AND account_id=? AND expires_at>? AND mfa_version=?
        AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND password_hash=? AND mfa_version=?)`,
      )
        .bind(
          now() + 300,
          ticket.session_hash,
          ticket.account_id,
          now(),
          ticket.version,
          ticket.account_id,
          ticket.password_version,
          ticket.version,
        )
        .run()
      if (!updated.meta.changes) fail(401, 'Session expired. Sign in again.')
      return json({ success: true })
    }
    return this.loginResponse(
      request,
      {
        id: ticket.account_id,
        email: ticket.email,
        password_hash: ticket.password_version,
      },
      200,
      undefined,
      ticket.version,
    )
  }

  private async elevated(session: Session) {
    const found = await this.env.DB.prepare(
      `SELECT a.* FROM sessions s JOIN accounts a ON a.id=s.account_id
      WHERE s.token_hash=? AND s.expires_at>? AND s.elevated_until>? AND s.mfa_version=a.mfa_version`,
    )
      .bind(session.token_hash, now(), now())
      .first<Account>()
    if (!found)
      fail(
        403,
        'Confirm your password and second factor before changing security settings.',
      )
    return found!
  }

  private async mfaChange(
    session: Session,
    enabled: boolean,
    extra: (version: number) => D1PreparedStatement[] = () => [],
    codes = true,
  ) {
    const user = await this.elevated(session)
    const version = Number.parseInt(randomBytes(6).toString('hex'), 16)
    const recovery =
      enabled && codes
        ? Array.from({ length: 10 }, () => randomBytes(16).toString('hex'))
        : []
    const guard = 'EXISTS(SELECT 1 FROM accounts WHERE id=? AND mfa_version=?)'
    const operations = [
      this.env.DB.prepare(
        `UPDATE accounts SET mfa_enabled=?,mfa_version=? WHERE id=? AND mfa_version=? AND
        EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND account_id=? AND expires_at>? AND elevated_until>? AND mfa_version=?)`,
      ).bind(
        enabled ? 1 : 0,
        version,
        user.id,
        user.mfa_version,
        session.token_hash,
        user.id,
        now(),
        now(),
        user.mfa_version,
      ),
      ...extra(version),
      this.env.DB.prepare(
        `DELETE FROM sessions WHERE account_id=? AND token_hash<>? AND ${guard}`,
      ).bind(user.id, session.token_hash, user.id, version),
      this.env.DB.prepare(
        `UPDATE sessions SET mfa_version=?,elevated_until=0 WHERE token_hash=? AND ${guard}`,
      ).bind(version, session.token_hash, user.id, version),
      this.env.DB.prepare(
        `DELETE FROM mfa_tickets WHERE account_id=? AND ${guard}`,
      ).bind(user.id, user.id, version),
      this.env.DB.prepare(
        `DELETE FROM challenges WHERE account_id=? AND ${guard}`,
      ).bind(user.id, user.id, version),
    ]
    if (codes || !enabled)
      operations.push(
        this.env.DB.prepare(
          `DELETE FROM recovery_codes WHERE account_id=? AND ${guard}`,
        ).bind(user.id, user.id, version),
      )
    for (const code of recovery)
      operations.push(
        this.env.DB.prepare(
          `INSERT INTO recovery_codes SELECT ?,? WHERE ${guard}`,
        ).bind(user.id, digest(code), user.id, version),
      )
    const result = await this.env.DB.batch(operations)
    if (!result[0]!.meta.changes)
      fail(403, 'Security settings changed. Verify again.')
    return json({
      success: true,
      recovery_codes: recovery.map((c) => c.match(/.{8}/g)!.join('-')),
    })
  }

  private async loginResponse(
    request: Request,
    account: { id: string; email: string; password_hash?: string },
    status = 200,
    passkey?: string,
    mfaVersion?: number,
    sourceSession?: string,
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
    const inserted = await this.env.DB.prepare(
      `INSERT INTO sessions (token_hash, account_id, expires_at, created_at, mfa_version) SELECT ?, id, ?, ?, mfa_version FROM accounts
      WHERE id = ? AND (? IS NULL OR password_hash = ?)
      AND ((mfa_enabled=0 AND ? IS NULL) OR mfa_version=?)
      AND (? IS NULL OR EXISTS (SELECT 1 FROM passkeys WHERE id = ? AND account_id = accounts.id))
      AND (SELECT COUNT(*) FROM sessions WHERE account_id = accounts.id AND expires_at > ?) < 100
      AND (? IS NULL OR EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND account_id=accounts.id AND expires_at>? AND mfa_version=accounts.mfa_version))`,
    )
      .bind(
        digest(raw),
        expires_at,
        current,
        account.id,
        account.password_hash ?? null,
        account.password_hash ?? null,
        mfaVersion ?? null,
        mfaVersion ?? null,
        passkey ?? null,
        passkey ?? null,
        current,
        sourceSession ?? null,
        sourceSession ?? null,
        current,
      )
      .run()
    if (!inserted.meta.changes)
      fail(401, 'Credentials changed. Please sign in again.')
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
    const raw =
      request.headers.get('X-Extend-Challenge') ??
      cookie(request, CHALLENGE_COOKIE)
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
    const db = this.env.DB
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
        desktop_browser_login: true,
        signup_enabled: this.env.SIGNUP_ENABLED === 'true',
        email_enabled: emailEnabled(this.env),
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
          'INSERT OR IGNORE INTO accounts (id, email, password_hash, created_at) VALUES (?, ?, ?, ?)',
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
      if (account!.mfa_enabled) return this.mfaPending(account!)
      return this.loginResponse(request, account!)
    }

    if (path === '/v1/passkeys/login/options' && method === 'POST') {
      const options = await generateAuthenticationOptions({
        rpID: url.hostname,
        userVerification: 'required',
      })
      const id = await this.challenge(request, 'login', null, options.challenge)
      const response = json(options)
      response.headers.set('X-Extend-Challenge', id)
      setCookie(response, request, CHALLENGE_COOKIE, id, 300)
      return response
    }
    if (path === '/v1/passkeys/login/verify' && method === 'POST') {
      const input = await body(request)
      const challenge = await this.consumeChallenge(request, 'login')
      const key = await this.env.DB.prepare(
        "SELECT * FROM passkeys WHERE id = ? AND rp_id = ? AND purpose='login'",
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
        'SELECT * FROM accounts WHERE id = ?',
      )
        .bind(key!.account_id)
        .first<Account>()
      if (account!.mfa_enabled)
        fail(
          403,
          'Two-factor authentication is enabled. Sign in with your password, then use this passkey.',
        )
      return this.loginResponse(request, account!, 200, key!.id)
    }

    if (path === '/v1/auth/desktop/exchange' && method === 'POST') {
      if (request.headers.get('X-Extend-Client') !== 'desktop')
        fail(400, 'Desktop client required')
      const input = await body(request)
      const raw = text(input.code, 64, 'authorization code')
      const verifier = text(input.verifier, 128, 'code verifier')
      if (
        !/^[a-f0-9]{64}$/.test(raw) ||
        !/^[A-Za-z0-9_-]{43,128}$/.test(verifier)
      )
        fail(400, 'Invalid desktop authorization')
      const challenge = createHash('sha256')
        .update(verifier)
        .digest('base64url')
      const found = await db
        .prepare(
          `DELETE FROM desktop_codes WHERE id=? AND challenge=? AND expires_at>? RETURNING session_hash`,
        )
        .bind(digest(raw), challenge, now())
        .first<{ session_hash: string }>()
      if (!found) fail(401, 'Desktop authorization expired. Sign in again.')
      const owner = await db
        .prepare(
          `SELECT a.* FROM accounts a JOIN sessions s ON s.account_id=a.id WHERE s.token_hash=? AND s.expires_at>? AND s.mfa_version=a.mfa_version`,
        )
        .bind(found!.session_hash, now())
        .first<Account>()
      if (!owner) fail(401, 'Browser session ended. Sign in again.')
      return this.loginResponse(
        request,
        owner!,
        200,
        undefined,
        owner!.mfa_enabled ? owner!.mfa_version : undefined,
        found!.session_hash,
      )
    }
    if (path === '/v1/auth/mfa/verify' && method === 'POST') {
      const input = await body(request)
      const ticket = await this.mfaTicket(input.ticket)
      await this.mfaAttempt(ticket)
      await this.mfaCode(ticket.account_id, input.code)
      return this.mfaFinish(request, ticket)
    }
    if (path === '/v1/auth/mfa/key/options' && method === 'POST') {
      const input = await body(request)
      const ticket = await this.mfaTicket(input.ticket)
      const { results } = await this.env.DB.prepare(
        'SELECT id,transports FROM passkeys WHERE account_id=? AND rp_id=?',
      )
        .bind(ticket.account_id, url.hostname)
        .all<{ id: string; transports: string }>()
      if (!results.length) fail(400, 'No passkey or security key registered')
      const options = await generateAuthenticationOptions({
        rpID: url.hostname,
        userVerification: 'preferred',
        allowCredentials: results.map((k) => ({ id: k.id })),
      })
      const id = await this.challenge(
        request,
        'mfa:' + ticket.id,
        ticket.account_id,
        options.challenge,
      )
      const response = json(options)
      response.headers.set('X-Extend-Challenge', id)
      setCookie(response, request, CHALLENGE_COOKIE, id, 300)
      return response
    }
    if (path === '/v1/auth/mfa/key/verify' && method === 'POST') {
      const input = await body(request)
      const ticket = await this.mfaTicket(input.ticket)
      await this.mfaAttempt(ticket)
      const challenge = await this.consumeChallenge(request, 'mfa:' + ticket.id)
      const key = await this.env.DB.prepare(
        'SELECT * FROM passkeys WHERE id=? AND account_id=? AND rp_id=?',
      )
        .bind(text(input.id, 2048, 'key'), ticket.account_id, url.hostname)
        .first<Passkey>()
      if (!key || challenge.account_id !== ticket.account_id)
        fail(401, 'Could not verify security key')
      let verified
      try {
        verified = await verifyAuthenticationResponse({
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
          requireUserVerification: false,
        })
      } catch {
        fail(401, 'Could not verify security key')
      }
      if (!verified!.verified) fail(401, 'Could not verify security key')
      const changed = await this.env.DB.prepare(
        'UPDATE passkeys SET counter=? WHERE id=? AND account_id=? AND counter=?',
      )
        .bind(
          verified!.authenticationInfo.newCounter,
          key!.id,
          ticket.account_id,
          key!.counter,
        )
        .run()
      if (!changed.meta.changes) fail(401, 'Security key changed. Try again.')
      return this.mfaFinish(request, ticket)
    }

    if (path === '/v1/auth/recovery/request' && method === 'POST') {
      if (!emailEnabled(this.env))
        fail(
          503,
          'Email recovery is not available yet. Try signing in with a passkey.',
        )
      const input = await body(request)
      const address = email(input.email)
      this.limit('recovery:' + digest(address), 3)
      const account = await this.env.DB.prepare(
        'SELECT * FROM accounts WHERE email = ? AND email_verified = 1',
      )
        .bind(address)
        .first<Account>()
      // Identical responses for unknown/unverified addresses, cooldowns, quotas and delivery errors.
      if (account)
        this.ctx.waitUntil(this.sendEmail(account, 'reset').catch(() => {}))
      return json({ success: true })
    }
    if (
      (path === '/v1/auth/email/verify' ||
        path === '/v1/auth/recovery/complete') &&
      method === 'POST'
    ) {
      const input = await body(request)
      const raw = text(input.token, 64, 'email link')
      if (!/^[a-f0-9]{64}$/.test(raw)) fail(400, 'Invalid or expired link')
      const kind = path.endsWith('/verify') ? 'verify' : 'reset'
      const replacement =
        kind === 'reset' ? hashPassword(password(input.password)) : null
      // One DELETE RETURNING atomically consumes a link, even across different DO shards.
      const link = await this.env.DB.prepare(
        'DELETE FROM email_tokens WHERE token_hash = ? AND kind = ? AND expires_at > ? RETURNING account_id, password_version',
      )
        .bind(digest(raw), kind, now())
        .first<{ account_id: string; password_version: string }>()
      if (!link) fail(400, 'Invalid or expired link. Request a new one.')
      if (kind === 'verify') {
        const changed = await this.env.DB.prepare(
          'UPDATE accounts SET email_verified = 1 WHERE id = ? AND password_hash = ?',
        )
          .bind(link!.account_id, link!.password_version)
          .run()
        if (!changed.meta.changes)
          fail(400, 'Invalid or expired link. Request a new one.')
      } else {
        const updated = await this.env.DB.batch([
          this.env.DB.prepare(
            'UPDATE accounts SET password_hash = ? WHERE id = ? AND password_hash = ?',
          ).bind(replacement, link!.account_id, link!.password_version),
          this.env.DB.prepare(
            'DELETE FROM sessions WHERE account_id = ? AND EXISTS (SELECT 1 FROM accounts WHERE id = ? AND password_hash = ?)',
          ).bind(link!.account_id, link!.account_id, replacement),
          this.env.DB.prepare(
            'DELETE FROM passkeys WHERE account_id = ? AND EXISTS (SELECT 1 FROM accounts WHERE id = ? AND password_hash = ? AND mfa_enabled=0)',
          ).bind(link!.account_id, link!.account_id, replacement),
          this.env.DB.prepare(
            'DELETE FROM email_tokens WHERE account_id = ? AND EXISTS (SELECT 1 FROM accounts WHERE id = ? AND password_hash = ?)',
          ).bind(link!.account_id, link!.account_id, replacement),
          this.env.DB.prepare(
            'DELETE FROM challenges WHERE account_id = ? AND EXISTS (SELECT 1 FROM accounts WHERE id = ? AND password_hash = ?)',
          ).bind(link!.account_id, link!.account_id, replacement),
        ])
        if (!updated[0]!.meta.changes)
          fail(400, 'Invalid or expired link. Request a new one.')
      }
      return json({ success: true })
    }

    const session = await this.session(request)
    if (path === '/v1/auth/desktop/authorize' && method === 'POST') {
      const input = await body(request)
      const challenge = text(input.challenge, 43, 'code challenge')
      if (!/^[A-Za-z0-9_-]{43}$/.test(challenge))
        fail(400, 'Invalid code challenge')
      const raw = token()
      const granted = await db.batch([
        db
          .prepare(
            'DELETE FROM desktop_codes WHERE expires_at<=? OR session_hash=?',
          )
          .bind(now(), session.token_hash),
        db
          .prepare(
            `INSERT INTO desktop_codes SELECT ?,token_hash,?,? FROM sessions s JOIN accounts a ON a.id=s.account_id WHERE s.token_hash=? AND s.expires_at>? AND s.mfa_version=a.mfa_version`,
          )
          .bind(digest(raw), challenge, now() + 120, session.token_hash, now()),
      ])
      if (!granted[1]!.meta.changes)
        fail(401, 'Browser session ended. Sign in again.')
      return json({ code: raw })
    }

    const account = session.account_id

    if (path === '/v1/mfa' && method === 'GET') {
      const user = await db
        .prepare('SELECT mfa_enabled FROM accounts WHERE id=?')
        .bind(account)
        .first<{ mfa_enabled: number }>()
      return json({
        enabled: !!user!.mfa_enabled,
        ...(await this.mfaInfo(account)),
      })
    }
    if (path === '/v1/mfa/reauth' && method === 'POST') {
      const input = await body(request)
      const user = await db
        .prepare('SELECT * FROM accounts WHERE id=?')
        .bind(account)
        .first<Account>()
      this.limit('reauth:' + account, 5)
      if (!verifyPassword(password(input.password), user!.password_hash))
        fail(401, 'Password is incorrect')
      if (user!.mfa_enabled)
        return this.mfaPending(user!, 'manage', session.token_hash)
      const changed = await db
        .prepare(
          `UPDATE sessions SET elevated_until=? WHERE token_hash=? AND expires_at>? AND mfa_version=?
        AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND password_hash=? AND mfa_version=?)`,
        )
        .bind(
          now() + 300,
          session.token_hash,
          now(),
          user!.mfa_version,
          account,
          user!.password_hash,
          user!.mfa_version,
        )
        .run()
      if (!changed.meta.changes) fail(401, 'Session changed. Sign in again.')
      return json({ success: true })
    }
    if (path === '/v1/mfa/totp/setup' && method === 'POST') {
      const user = await this.elevated(session)
      if (!this.env.MFA_ENCRYPTION_KEY)
        fail(503, 'Authenticator setup is not configured on this server')
      const secret = newTotpSecret()
      await db
        .prepare('INSERT OR REPLACE INTO totp_setup VALUES (?,?,?,?,?)')
        .bind(
          session.token_hash,
          account,
          sealSecret(secret, this.env.MFA_ENCRYPTION_KEY!, account),
          user.mfa_version,
          now() + 300,
        )
        .run()
      return json({
        secret,
        uri: `otpauth://totp/${encodeURIComponent('extend.computer:' + user.email)}?secret=${secret}&issuer=extend.computer&algorithm=SHA1&digits=6&period=30`,
      })
    }
    if (path === '/v1/mfa/totp/confirm' && method === 'POST') {
      const user = await this.elevated(session)
      const input = await body(request)
      const setup = await db
        .prepare(
          'SELECT secret FROM totp_setup WHERE session_hash=? AND account_id=? AND version=? AND expires_at>?',
        )
        .bind(session.token_hash, account, user.mfa_version, now())
        .first<{ secret: string }>()
      if (!setup || !this.env.MFA_ENCRYPTION_KEY)
        fail(400, 'Setup expired. Start again.')
      this.limit('totp-setup:' + account, 5)
      const step = matchTotp(
        openSecret(setup!.secret, this.env.MFA_ENCRYPTION_KEY!, account),
        input.code,
        now(),
      )
      if (step === null)
        fail(
          400,
          'Code is incorrect. Try the current code from your authenticator.',
        )
      return this.mfaChange(session, true, (version) => [
        db
          .prepare(
            `INSERT OR REPLACE INTO totp_factors SELECT ?,?,? WHERE EXISTS(SELECT 1 FROM accounts WHERE id=? AND mfa_version=?)`,
          )
          .bind(account, setup!.secret, step, account, version),
        db
          .prepare('DELETE FROM totp_setup WHERE session_hash=?')
          .bind(session.token_hash),
      ])
    }
    if (path === '/v1/mfa/enable' && method === 'POST') {
      await this.elevated(session)
      const info = await this.mfaInfo(account)
      if (!info.totp && !info.keys)
        fail(400, 'Add an authenticator or security key first')
      return this.mfaChange(session, true)
    }
    if (path === '/v1/mfa/recovery-codes' && method === 'POST') {
      const user = await this.elevated(session)
      if (!user.mfa_enabled) fail(400, 'Enable two-factor authentication first')
      return this.mfaChange(session, true)
    }
    if (path === '/v1/mfa/disable' && method === 'POST') {
      const user = await this.elevated(session)
      return this.mfaChange(session, false, (version) => [
        db
          .prepare(
            'DELETE FROM totp_factors WHERE account_id=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND mfa_version=?)',
          )
          .bind(account, account, version),
        db.prepare('DELETE FROM totp_setup WHERE account_id=?').bind(account),
      ])
    }
    if (path === '/v1/mfa/totp' && method === 'DELETE') {
      const user = await this.elevated(session)
      const info = await this.mfaInfo(account)
      if (user.mfa_enabled && !info.keys)
        fail(
          400,
          'Add a security key first, or turn off two-factor authentication.',
        )
      return this.mfaChange(
        session,
        !!user.mfa_enabled,
        (version) => [
          db
            .prepare(
              'DELETE FROM totp_factors WHERE account_id=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND mfa_version=?)',
            )
            .bind(account, account, version),
        ],
        false,
      )
    }

    if (path === '/v1/account' && method === 'GET') {
      const user = await db
        .prepare(
          'SELECT id, email, created_at, email_verified, mfa_enabled FROM accounts WHERE id = ?',
        )
        .bind(account)
        .first()
      return json({ ...user, email_enabled: emailEnabled(this.env) })
    }
    if (path === '/v1/auth/email/request' && method === 'POST') {
      const user = await db
        .prepare('SELECT * FROM accounts WHERE id = ?')
        .bind(account)
        .first<Account>()
      if (!user!.email_verified) await this.sendEmail(user!, 'verify')
      return json({ success: true })
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
      const owner = await db
        .prepare('SELECT mfa_enabled FROM accounts WHERE id=?')
        .bind(account)
        .first<{ mfa_enabled: number }>()
      if (owner!.mfa_enabled) await this.elevated(session)
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
      const changed = await db.batch([
        db
          .prepare(
            'UPDATE accounts SET password_hash = ? WHERE id = ? AND password_hash = ? AND EXISTS (SELECT 1 FROM sessions WHERE token_hash = ? AND account_id=accounts.id AND expires_at > ? AND mfa_version=accounts.mfa_version AND (accounts.mfa_enabled=0 OR elevated_until>?))',
          )
          .bind(
            hash,
            account,
            current!.password_hash,
            session.token_hash,
            now(),
            now(),
          ),
        db
          .prepare(
            'DELETE FROM sessions WHERE account_id = ? AND token_hash != ? AND EXISTS (SELECT 1 FROM accounts WHERE id = ? AND password_hash = ?)',
          )
          .bind(account, session.token_hash, account, hash),
      ])
      if (!changed[0]!.meta.changes)
        fail(401, 'Credentials changed. Please sign in again.')
      return empty()
    }

    if (path === '/v1/devices' && method === 'GET') {
      const { results } = await db
        .prepare(
          `SELECT d.id, d.fingerprint, d.name, d.platform, d.created_at,
        MAX(CASE WHEN s.expires_at > ? AND s.mfa_version=a.mfa_version THEN ds.key_verified ELSE 0 END) AS key_verified,
        MAX(CASE WHEN s.expires_at > ? AND s.mfa_version=a.mfa_version THEN ds.last_seen END) AS last_seen
        FROM devices d LEFT JOIN device_sessions ds ON ds.device_id = d.id
        LEFT JOIN sessions s ON s.token_hash = ds.session_hash LEFT JOIN accounts a ON a.id=s.account_id
        WHERE d.account_id = ? GROUP BY d.id ORDER BY d.created_at, d.id`,
        )
        .bind(now(), now(), account)
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
            'DELETE FROM device_proofs WHERE token_hash IN (SELECT token_hash FROM device_sessions WHERE session_hash=? AND device_id IN (SELECT id FROM devices WHERE account_id=? AND fingerprint=?))',
          )
          .bind(session.token_hash, account, fingerprint),
        db
          .prepare(
            `INSERT INTO devices (id,account_id,fingerprint,name,platform,created_at,public_key) SELECT ?, ?, ?, ?, ?, ?, ? WHERE (SELECT COUNT(*) FROM devices WHERE account_id = ?) < 100 OR EXISTS (SELECT 1 FROM devices WHERE account_id = ? AND fingerprint = ?)
          ON CONFLICT(account_id, fingerprint) DO UPDATE SET name = excluded.name, platform = excluded.platform, public_key=excluded.public_key`,
          )
          .bind(
            id,
            account,
            fingerprint,
            name,
            input.platform,
            now(),
            publicKey.toLowerCase(),
            account,
            account,
            fingerprint,
          ),
        db
          .prepare(
            `INSERT INTO device_sessions (token_hash,session_hash,device_id,last_seen) SELECT ?, ?, id, NULL FROM devices WHERE fingerprint = ? AND account_id = ?
          ON CONFLICT(session_hash, device_id) DO UPDATE SET token_hash = excluded.token_hash, last_seen = NULL, key_verified=0`,
          )
          .bind(digest(deviceToken), session.token_hash, fingerprint, account),
      ])
      if (!inserted[2]!.meta.changes) fail(429, 'Device limit reached')
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
          'SELECT id, name, created_at, rp_id, purpose FROM passkeys WHERE account_id = ? ORDER BY created_at',
        )
        .bind(account)
        .all()
      return json({ passkeys: results })
    }
    if (path === '/v1/passkeys/register/options' && method === 'POST') {
      const input = request.body ? await body(request) : {}
      const factor = input.factor === true
      const owner = await db
        .prepare('SELECT * FROM accounts WHERE id=?')
        .bind(account)
        .first<Account>()
      if (factor || owner!.mfa_enabled) await this.elevated(session)
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
          residentKey: factor ? 'preferred' : 'required',
          userVerification: factor ? 'preferred' : 'required',
        },
        excludeCredentials: results.map((k) => ({ id: k.id })),
      })
      const id = await this.challenge(
        request,
        'register:' + session.token_hash,
        account,
        JSON.stringify({
          challenge: options.challenge,
          purpose: factor ? 'factor' : 'login',
          version: owner!.mfa_version,
        }),
      )
      const response = json(options)
      response.headers.set('X-Extend-Challenge', id)
      setCookie(response, request, CHALLENGE_COOKIE, id, 300)
      return response
    }
    if (path === '/v1/passkeys/register/verify' && method === 'POST') {
      const input = await body(request)
      const challenge = await this.consumeChallenge(
        request,
        'register:' + session.token_hash,
      )
      const ceremony = JSON.parse(challenge.challenge) as {
        challenge: string
        purpose: string
        version: number
      }
      const owner = await db
        .prepare('SELECT * FROM accounts WHERE id=?')
        .bind(account)
        .first<Account>()
      if (owner!.mfa_version !== ceremony.version)
        fail(403, 'Security settings changed. Start again.')
      if (ceremony.purpose === 'factor' || owner!.mfa_enabled)
        await this.elevated(session)
      if (challenge.account_id !== account)
        fail(401, 'Passkey request belongs to another session')
      let verification
      try {
        verification = await verifyRegistrationResponse({
          response: registration(input),
          expectedChallenge: ceremony.challenge,
          expectedOrigin: challenge.origin,
          expectedRPID: url.hostname,
          requireUserVerification: ceremony.purpose === 'login',
        })
      } catch {
        fail(400, 'Could not verify passkey')
      }
      if (!verification!.verified) fail(400, 'Could not verify passkey')
      const credential = verification!.registrationInfo!.credential
      const insert = (version: number) =>
        db
          .prepare(
            `INSERT INTO passkeys (id,account_id,public_key,counter,transports,name,rp_id,created_at,purpose)
        SELECT ?,?,?,?,?,?,?,?,? WHERE EXISTS(SELECT 1 FROM accounts WHERE id=? AND mfa_version=?)
        AND EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND account_id=? AND expires_at>? AND (?=0 OR elevated_until>?))`,
          )
          .bind(
            credential.id,
            account,
            Buffer.from(credential.publicKey).toString('base64url'),
            credential.counter,
            JSON.stringify(credential.transports ?? []),
            text(
              input.name ??
                (ceremony.purpose === 'factor' ? 'Security key' : 'Passkey'),
              80,
              'key name',
            ),
            url.hostname,
            now(),
            ceremony.purpose,
            account,
            version,
            session.token_hash,
            account,
            now(),
            ceremony.purpose === 'factor' && !owner!.mfa_enabled ? 1 : 0,
            now(),
          )
      if (owner!.mfa_enabled)
        return this.mfaChange(
          session,
          true,
          (version) => [insert(version)],
          false,
        )
      const inserted = await insert(owner!.mfa_version).run()
      if (!inserted.meta.changes)
        fail(403, 'Session or security settings changed. Start again.')
      return json({ success: true }, 201)
    }
    const passkeyPath = path.match(/^\/v1\/passkeys\/([^/]+)$/)
    if (passkeyPath && method === 'DELETE') {
      const owner = await db
        .prepare('SELECT * FROM accounts WHERE id=?')
        .bind(account)
        .first<Account>()
      if (owner!.mfa_enabled) {
        await this.elevated(session)
        const info = await this.mfaInfo(account)
        const key = await db
          .prepare('SELECT id FROM passkeys WHERE id=? AND account_id=?')
          .bind(passkeyPath[1], account)
          .first()
        if (!key) fail(404, 'Passkey not found')
        if (!info.totp && info.keys <= 1)
          fail(
            400,
            'Add another factor first, or turn off two-factor authentication.',
          )
        return this.mfaChange(
          session,
          true,
          (version) => [
            db
              .prepare(
                'DELETE FROM passkeys WHERE id=? AND account_id=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND mfa_version=?)',
              )
              .bind(passkeyPath[1], account, account, version),
          ],
          false,
        )
      }
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
    AND session_hash IN (SELECT s.token_hash FROM sessions s JOIN accounts a ON a.id=s.account_id WHERE s.expires_at > ? AND s.mfa_version=a.mfa_version)`,
  )
    .bind(now(), digest(raw), id, now())
    .run()
  return result.meta.changes
    ? empty()
    : json({ error: 'Invalid device credentials' }, 401)
}
