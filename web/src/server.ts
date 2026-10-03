import { deviceProof } from './lib/device-keys'
import { relayRequest } from './lib/relay'
export { AccountRelay } from './lib/relay'
import start from '@tanstack/react-start/server-entry'
import { createHash } from 'node:crypto'
import { heartbeat } from './lib/account-service'
export { AccountService } from './lib/account-service'

export default {
  async scheduled(
    _event: ScheduledController,
    env: Env,
    ctx: ExecutionContext,
  ) {
    const stamp = Math.floor(Date.now() / 1000)
    ctx.waitUntil(
      env.DB.batch([
        env.DB.prepare('DELETE FROM device_proofs WHERE expires_at <= ?').bind(
          stamp,
        ),
        env.DB.prepare(
          'DELETE FROM desktop_codes WHERE id IN (SELECT id FROM desktop_codes WHERE expires_at <= ? LIMIT 1000)',
        ).bind(stamp),
        env.DB.prepare(
          'DELETE FROM sessions WHERE token_hash IN (SELECT token_hash FROM sessions WHERE expires_at <= ? LIMIT 1000)',
        ).bind(stamp),
        env.DB.prepare(
          'DELETE FROM email_tokens WHERE token_hash IN (SELECT token_hash FROM email_tokens WHERE expires_at <= ? LIMIT 1000)',
        ).bind(stamp),
        env.DB.prepare(
          'DELETE FROM challenges WHERE id IN (SELECT id FROM challenges WHERE expires_at <= ? LIMIT 1000)',
        ).bind(stamp),
      ]),
    )
  },
  async fetch(
    request: Request,
    env: Env,
    _ctx: ExecutionContext,
  ): Promise<Response> {
    const url = new URL(request.url)
    let response: Response
    if (url.pathname.startsWith('/v1/relay/')) return relayRequest(request, env)
    if (url.pathname === '/healthz') response = Response.json({ status: 'ok' })
    else if (
      url.pathname.startsWith('/api/v1/') ||
      url.pathname.startsWith('/v1/')
    ) {
      if (
        /\/devices\/[a-f0-9]{64}\/proof\/(options|verify)$/.test(url.pathname)
      ) {
        response = await deviceProof(request, env)
      } else if (
        request.method === 'POST' &&
        /\/devices\/[a-f0-9]{64}\/heartbeat$/.test(url.pathname)
      ) {
        try {
          response = await heartbeat(request, env)
        } catch {
          response = Response.json(
            { error: 'Service unavailable' },
            { status: 503 },
          )
        }
      } else {
        const ip = request.headers.get('CF-Connecting-IP') ?? 'local'
        const shard = createHash('sha256').update(ip).digest('hex')[0]!
        response = await env.ACCOUNTS.getByName('auth-' + shard).fetch(request)
      }
    } else response = await start.fetch(request)
    const headers = new Headers(response.headers)
    headers.set('X-Content-Type-Options', 'nosniff')
    headers.set('Referrer-Policy', 'strict-origin-when-cross-origin')
    headers.set('X-Frame-Options', 'DENY')
    headers.set(
      'Permissions-Policy',
      'camera=(), microphone=(), geolocation=()',
    )
    if (url.pathname !== '/' && !url.pathname.startsWith('/assets/'))
      headers.set('Cache-Control', 'no-store')
    return new Response(response.body, {
      status: response.status,
      statusText: response.statusText,
      headers,
    })
  },
}
