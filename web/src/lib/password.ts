import { randomBytes, scryptSync, timingSafeEqual } from 'node:crypto'

// OWASP scrypt profile: N=2^15, r=8, p=3. Runs in the Durable Object CPU budget.
const parameters = { N: 32768, r: 8, p: 3, maxmem: 64 * 1024 * 1024 }

export function hashPassword(password: string): string {
  const salt = randomBytes(16).toString('hex')
  const hash = scryptSync(password, salt, 32, parameters).toString('hex')
  return `scrypt$32768$8$3$${salt}$${hash}`
}

export function verifyPassword(password: string, encoded: string): boolean {
  const [algorithm, n, r, p, salt, hash, extra] = encoded.split('$')
  if (
    algorithm !== 'scrypt' ||
    n !== '32768' ||
    r !== '8' ||
    p !== '3' ||
    extra ||
    !/^[a-f0-9]{32}$/.test(salt ?? '') ||
    !/^[a-f0-9]{64}$/.test(hash ?? '')
  )
    return false
  return timingSafeEqual(
    scryptSync(password, salt!, 32, parameters),
    Buffer.from(hash!, 'hex'),
  )
}
