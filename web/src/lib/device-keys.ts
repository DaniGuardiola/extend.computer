import { createHash, randomBytes, timingSafeEqual } from 'node:crypto'

const now = () => Math.floor(Date.now() / 1000)
const digest = (raw: string) => createHash('sha256').update(raw).digest('hex')
const reply = (error: string, status: number) =>
  Response.json({ error }, { status })

export async function deviceProof(
  request: Request,
  env: Env,
): Promise<Response> {
  const match = new URL(request.url).pathname.match(
    /\/v1\/devices\/([a-f0-9]{64})\/proof\/(options|verify)$/,
  )
  const raw = request.headers
    .get('Authorization')
    ?.match(/^Bearer ([a-f0-9]{64})$/)?.[1]
  if (!match || !raw || request.method !== 'POST')
    return reply('Invalid device credentials', 401)
  const credential = digest(raw)
  const key = await env.DB.prepare(
    `SELECT d.public_key FROM devices d JOIN device_sessions ds ON ds.device_id=d.id JOIN sessions s ON s.token_hash=ds.session_hash JOIN accounts a ON a.id=s.account_id WHERE ds.token_hash=? AND d.id=? AND s.expires_at>? AND s.mfa_version=a.mfa_version`,
  )
    .bind(credential, match[1], now())
    .first<{ public_key: string | null }>()
  if (!key?.public_key) return reply('Invalid device credentials', 401)
  if (match[2] === 'options') {
    const { publicKey, privateKey } = (await crypto.subtle.generateKey(
      { name: 'X25519' },
      true,
      ['deriveBits'],
    )) as CryptoKeyPair
    const remote = await crypto.subtle.importKey(
      'raw',
      Buffer.from(key.public_key, 'hex'),
      { name: 'X25519' },
      false,
      [],
    )
    let shared: Buffer
    try {
      shared = Buffer.from(
        await crypto.subtle.deriveBits(
          { name: 'X25519', public: remote },
          privateKey,
          256,
        ),
      )
    } catch {
      return reply('Invalid device key', 400)
    }
    if (shared.every((n) => n === 0)) return reply('Invalid device key', 400)
    const challenge = randomBytes(32).toString('hex')
    const expected = createHash('sha256')
      .update('extend.computer/device-proof/v1\0')
      .update(shared)
      .update(Buffer.from(challenge, 'hex'))
      .digest('hex')
    await env.DB.prepare(
      'INSERT INTO device_proofs VALUES (?, ?, ?, ?) ON CONFLICT(token_hash) DO UPDATE SET challenge=excluded.challenge,expected=excluded.expected,expires_at=excluded.expires_at',
    )
      .bind(credential, challenge, expected, now() + 120)
      .run()
    return Response.json({
      server_key: Buffer.from(
        await crypto.subtle.exportKey('raw', publicKey),
      ).toString('hex'),
      challenge,
    })
  }
  const reader = request.body?.getReader()
  let payload = ''
  if (!reader) return reply('Invalid device proof', 400)
  const decoder = new TextDecoder()
  let length = 0
  while (true) {
    const chunk = await reader.read()
    if (chunk.done) break
    length += chunk.value.byteLength
    if (length > 1024) {
      await reader.cancel()
      return reply('Invalid device proof', 400)
    }
    payload += decoder.decode(chunk.value, { stream: true })
  }
  let input: { challenge?: unknown; proof?: unknown }
  try {
    input = JSON.parse(payload)
  } catch {
    return reply('Invalid device proof', 400)
  }
  if (!input || typeof input !== 'object')
    return reply('Invalid device proof', 400)
  if (
    typeof input.challenge !== 'string' ||
    !/^[a-f0-9]{64}$/.test(input.challenge) ||
    typeof input.proof !== 'string' ||
    !/^[a-f0-9]{64}$/.test(input.proof)
  )
    return reply('Invalid device proof', 400)
  // DELETE RETURNING is atomic across requests: replay can never succeed.
  const found = await env.DB.prepare(
    'DELETE FROM device_proofs WHERE token_hash=? RETURNING challenge,expected,expires_at',
  )
    .bind(credential)
    .first<{ challenge: string; expected: string; expires_at: number }>()
  if (
    !found ||
    found.expires_at <= now() ||
    found.challenge !== input.challenge ||
    !timingSafeEqual(
      Buffer.from(found.expected, 'hex'),
      Buffer.from(input.proof, 'hex'),
    )
  )
    return reply('Invalid device proof', 401)
  const result = await env.DB.prepare(
    `UPDATE device_sessions SET key_verified=1 WHERE token_hash=? AND device_id=? AND session_hash IN (SELECT s.token_hash FROM sessions s JOIN accounts a ON a.id=s.account_id WHERE s.expires_at>? AND s.mfa_version=a.mfa_version)`,
  )
    .bind(credential, match[1], now())
    .run()
  return result.meta.changes
    ? new Response(null, { status: 204 })
    : reply('Invalid device credentials', 401)
}
