import {
  createHmac,
  randomBytes,
  timingSafeEqual,
  createCipheriv,
  createDecipheriv,
} from 'node:crypto'
const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
export function base32(bytes: Uint8Array): string {
  let bits = 0,
    value = 0,
    result = ''
  for (const byte of bytes) {
    value = (value << 8) | byte
    bits += 8
    while (bits >= 5) {
      bits -= 5
      result += alphabet[(value >>> bits) & 31]
    }
  }
  if (bits) result += alphabet[(value << (5 - bits)) & 31]
  return result
}
export function decode32(secret: string): Buffer {
  let bits = 0,
    value = 0
  const bytes: number[] = []
  for (const char of secret) {
    const n = alphabet.indexOf(char)
    if (n < 0) throw new Error('Invalid secret')
    value = (value << 5) | n
    bits += 5
    if (bits >= 8) {
      bits -= 8
      bytes.push((value >>> bits) & 255)
    }
  }
  return Buffer.from(bytes)
}
export function totp(secret: string, step: number, digits = 6): string {
  const counter = Buffer.alloc(8)
  counter.writeBigUInt64BE(BigInt(step))
  const hash = createHmac('sha1', decode32(secret)).update(counter).digest()
  const offset = hash[19]! & 15
  return ((hash.readUInt32BE(offset) & 0x7fffffff) % 10 ** digits)
    .toString()
    .padStart(digits, '0')
}
export function matchTotp(
  secret: string,
  code: unknown,
  seconds: number,
  lastStep = -1,
): number | null {
  if (typeof code !== 'string' || !/^\d{6}$/.test(code)) return null
  const current = Math.floor(seconds / 30)
  for (const step of [current, current - 1, current + 1])
    if (
      step > lastStep &&
      step >= 0 &&
      timingSafeEqual(Buffer.from(totp(secret, step)), Buffer.from(code))
    )
      return step
  return null
}
export const newTotpSecret = () => base32(randomBytes(20))
export function sealSecret(
  secret: string,
  key: string,
  account: string,
): string {
  if (!/^[a-f0-9]{64}$/.test(key)) throw new Error('Invalid MFA encryption key')
  const nonce = randomBytes(12)
  const cipher = createCipheriv('aes-256-gcm', Buffer.from(key, 'hex'), nonce)
  cipher.setAAD(Buffer.from(account))
  const encrypted = Buffer.concat([
    cipher.update(secret, 'utf8'),
    cipher.final(),
  ])
  return Buffer.concat([nonce, cipher.getAuthTag(), encrypted]).toString(
    'base64url',
  )
}
export function openSecret(
  value: string,
  key: string,
  account: string,
): string {
  if (!/^[a-f0-9]{64}$/.test(key)) throw new Error('Invalid MFA encryption key')
  const bytes = Buffer.from(value, 'base64url')
  const cipher = createDecipheriv(
    'aes-256-gcm',
    Buffer.from(key, 'hex'),
    bytes.subarray(0, 12),
  )
  cipher.setAAD(Buffer.from(account))
  cipher.setAuthTag(bytes.subarray(12, 28))
  return Buffer.concat([
    cipher.update(bytes.subarray(28)),
    cipher.final(),
  ]).toString('utf8')
}
declare global {
  interface Env {
    MFA_ENCRYPTION_KEY: string
  }
}
