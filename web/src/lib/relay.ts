//! Relay only opaque Noise transport bytes; authorization stays account/device scoped.
import { relayEnabled, relayDisabled, relayStatus } from './relay-policy'
import { DurableObject } from 'cloudflare:workers'
import { createHash, randomBytes } from 'node:crypto'

type DeviceAuth = {
  account_id: string
  device_id: string
  fingerprint: string
  token_hash: string
}
type Attachment = DeviceAuth & {
  role: 'control' | 'sender' | 'receiver'
  channel: string
  peer: string
  created: number
  kind: 'presence' | 'control'
}
const now = () => Math.floor(Date.now() / 1000)
const error = (message: string, status: number) =>
  Response.json({ error: message }, { status })

export async function relayAuth(
  env: Env,
  tokenHash: string,
  device: string,
): Promise<DeviceAuth | null> {
  return env.DB.prepare(
    `SELECT d.account_id,d.id AS device_id,d.fingerprint,ds.token_hash FROM device_sessions ds JOIN devices d ON d.id=ds.device_id JOIN sessions s ON s.token_hash=ds.session_hash JOIN accounts a ON a.id=s.account_id WHERE ds.token_hash=? AND d.id=? AND ds.key_verified=1 AND s.expires_at>? AND s.mfa_version=a.mfa_version`,
  )
    .bind(tokenHash, device, now())
    .first<DeviceAuth>()
}
export async function relayRequest(
  request: Request,
  env: Env,
): Promise<Response> {
  const url = new URL(request.url)
  if (url.pathname === '/v1/relay/status') return relayStatus(env)
  if (!relayEnabled(env)) return relayDisabled()
  const raw = request.headers
    .get('Authorization')
    ?.match(/^Bearer ([a-f0-9]{64})$/)?.[1]
  const device = url.searchParams.get('device') ?? ''
  if (!raw || !/^[a-f0-9]{64}$/.test(device))
    return error('Invalid device credentials', 401)
  const auth = await relayAuth(
    env,
    createHash('sha256').update(raw).digest('hex'),
    device,
  )
  if (!auth) return error('Invalid device credentials', 401)
  return env.RELAY.getByName(auth.account_id).fetch(request)
}

export class AccountRelay extends DurableObject<Env> {
  private checked = new Map<string, number>()
  private traffic = new Map<
    string,
    { at: number; bytes: number; frames: number }
  >()
  private sockets(channel?: string) {
    return this.ctx.getWebSockets(channel)
  }
  private attachment(ws: WebSocket): Attachment {
    return ws.deserializeAttachment() as Attachment
  }
  private closeChannel(
    ws: WebSocket,
    code = 1000,
    reason = 'Connection closed',
  ) {
    const a = this.attachment(ws)
    for (const socket of a.role === 'control'
      ? [ws]
      : this.sockets(a.channel)) {
      try {
        socket.close(code, reason)
      } catch {
        /* Already closed. */
      }
    }
  }
  private async valid(ws: WebSocket, force = false): Promise<boolean> {
    if (!relayEnabled(this.env)) {
      this.closeChannel(ws, 1001, 'Relay disabled')
      return false
    }
    const a = this.attachment(ws)
    const checked = this.checked.get(a.token_hash) ?? 0
    if (!force && checked > now() - 5) return true
    const valid = await relayAuth(this.env, a.token_hash, a.device_id)
    if (
      !valid ||
      valid.account_id !== a.account_id ||
      valid.fingerprint !== a.fingerprint
    ) {
      this.closeChannel(ws, 1008, 'Sign in again')
      return false
    }
    this.checked.set(a.token_hash, now())
    return true
  }
  async fetch(request: Request): Promise<Response> {
    if (!relayEnabled(this.env)) return relayDisabled()
    const url = new URL(request.url)
    const raw = request.headers
      .get('Authorization')
      ?.match(/^Bearer ([a-f0-9]{64})$/)?.[1]
    if (request.headers.get('Upgrade')?.toLowerCase() !== 'websocket' || !raw)
      return error('WebSocket required', 400)
    const auth = await relayAuth(
      this.env,
      createHash('sha256').update(raw).digest('hex'),
      url.searchParams.get('device') ?? '',
    )
    if (!auth) return error('Invalid device credentials', 401)
    // Defense in depth: this object never mixes accounts, even after hibernation.
    if (
      this.sockets().some(
        (ws) => this.attachment(ws).account_id !== auth.account_id,
      )
    )
      return error('Account mismatch', 403)
    const action = url.pathname.split('/').at(-1)
    let role: Attachment['role']
    let channel = ''
    let peer = ''
    let kind: Attachment['kind'] = 'presence'
    let sender: WebSocket | undefined
    let receiverControl: WebSocket | undefined
    if (action === 'connect') {
      role = 'control'
      for (const ws of this.sockets().filter(
        (ws) =>
          this.attachment(ws).role === 'control' &&
          this.attachment(ws).device_id === auth.device_id,
      ))
        this.closeChannel(ws, 1000, 'Reconnected')
      if (
        this.sockets().filter((ws) => this.attachment(ws).role === 'control')
          .length >= 100
      )
        return error('Device limit reached', 429)
    } else if (action === 'tunnel') {
      role = 'sender'
      peer = url.searchParams.get('peer') ?? ''
      const requestedKind = url.searchParams.get('kind')
      if (
        !/^[a-f0-9]{64}$/.test(peer) ||
        peer === auth.device_id ||
        !['presence', 'control'].includes(requestedKind ?? '')
      )
        return error('Invalid relay target', 400)
      kind = requestedKind as Attachment['kind']
      receiverControl = this.sockets().find((ws) => {
        const a = this.attachment(ws)
        return (
          a.role === 'control' &&
          a.device_id === peer &&
          a.account_id === auth.account_id
        )
      })
      if (!receiverControl || !(await this.valid(receiverControl, true)))
        return error('Device offline', 404)
      if (
        this.sockets().filter((ws) => this.attachment(ws).role === 'sender')
          .length >= 4
      )
        return error('Connection limit reached', 429)
      channel = randomBytes(32).toString('hex')
    } else if (action === 'accept') {
      role = 'receiver'
      channel = url.searchParams.get('channel') ?? ''
      if (!/^[a-f0-9]{64}$/.test(channel)) return error('Invalid channel', 400)
      sender = this.sockets(channel).find(
        (ws) => this.attachment(ws).role === 'sender',
      )
      if (
        !sender ||
        this.sockets(channel).length !== 1 ||
        !(await this.valid(sender, true))
      )
        return error('Channel unavailable', 404)
      const a = this.attachment(sender)
      if (
        a.peer !== auth.device_id ||
        a.account_id !== auth.account_id ||
        a.created < now() - 30
      )
        return error('Channel unavailable', 404)
      peer = a.device_id
      kind = a.kind
    } else return error('Not found', 404)
    const [client, server] = Object.values(new WebSocketPair()) as [
      WebSocket,
      WebSocket,
    ]
    const attachment: Attachment = {
      ...auth,
      role,
      channel,
      peer,
      kind,
      created: now(),
    }
    this.ctx.acceptWebSocket(server, [role === 'control' ? 'control' : channel])
    server.serializeAttachment(attachment)
    this.checked.set(auth.token_hash, now())
    if (receiverControl)
      receiverControl.send(
        JSON.stringify({
          type: 'offer',
          channel,
          peer: auth.fingerprint,
          kind,
        }),
      )
    if (sender) {
      sender.send('{"type":"ready"}')
      server.send('{"type":"ready"}')
    }
    if (role === 'control') server.send('{"type":"ready"}')
    await this.ctx.storage.setAlarm(Date.now() + 30_000)
    return new Response(null, { status: 101, webSocket: client })
  }
  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer) {
    if (!(await this.valid(ws))) return
    const a = this.attachment(ws)
    if (a.role === 'control') {
      if (message === 'ping') {
        ws.send('pong')
        return
      }
      this.closeChannel(ws, 1008, 'Invalid message')
      return
    }
    if (
      !(message instanceof ArrayBuffer) ||
      message.byteLength > 65_536 ||
      !message.byteLength
    ) {
      this.closeChannel(ws, 1008, 'Invalid frame')
      return
    }
    // Bound both bandwidth and tiny-frame floods per account, not per socket.
    const bucket = this.traffic.get('account') ?? {
      at: now(),
      bytes: 0,
      frames: 0,
    }
    if (bucket.at !== now()) {
      bucket.at = now()
      bucket.bytes = 0
      bucket.frames = 0
    }
    bucket.bytes += message.byteLength
    bucket.frames++
    this.traffic.set('account', bucket)
    if (bucket.bytes > 2 * 1024 * 1024 || bucket.frames > 2000) {
      this.closeChannel(ws, 1008, 'Relay limit reached')
      return
    }
    const other = this.sockets(a.channel).find((socket) => socket !== ws)
    if (!other || !(await this.valid(other))) {
      this.closeChannel(ws, 1008, 'Peer unavailable')
      return
    }
    try {
      other.send(message)
    } catch {
      this.closeChannel(ws, 1011, 'Peer unavailable')
    }
  }
  webSocketClose(ws: WebSocket) {
    this.closeChannel(ws)
  }
  webSocketError(ws: WebSocket) {
    this.closeChannel(ws, 1011, 'Connection failed')
  }
  async alarm() {
    for (const ws of this.sockets()) {
      const a = this.attachment(ws)
      if (
        a.role !== 'control' &&
        (a.created < now() - 3600 ||
          (this.sockets(a.channel).length < 2 && a.created < now() - 30))
      ) {
        this.closeChannel(ws, 1000, 'Connection expired')
        continue
      }
      await this.valid(ws, true)
    }
    if (this.sockets().length)
      await this.ctx.storage.setAlarm(Date.now() + 30_000)
  }
}

declare global {
  interface Env {
    RELAY: DurableObjectNamespace<AccountRelay>
  }
}
