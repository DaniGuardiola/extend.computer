import { randomBytes, randomInt, scryptSync } from 'node:crypto'

// A batch shares a random salt so checking any of its codes needs one slow hash.
// Separate accounts and replacement batches always get fresh salts.
export function hashRecoveryCode(code: string, salt: string): string {
  if (!/^\d{8}$/.test(code) || !/^[a-f0-9]{32}$/.test(salt))
    throw new Error('Invalid recovery code hash input')
  const hash = scryptSync(code, salt, 32, {
    N: 32768,
    r: 8,
    p: 3,
    maxmem: 64 * 1024 * 1024,
  }).toString('hex')
  return `recovery-scrypt$${salt}$${hash}`
}

export function recoverySalt(encoded: string): string | undefined {
  return /^recovery-scrypt\$([a-f0-9]{32})\$[a-f0-9]{64}$/.exec(encoded)?.[1]
}

export function newRecoveryCodes() {
  const salt = randomBytes(16).toString('hex')
  const codes = new Set<string>()
  while (codes.size < 10)
    codes.add(randomInt(100_000_000).toString().padStart(8, '0'))
  return [...codes].map((code) => ({
    display: `${code.slice(0, 4)} ${code.slice(4)}`,
    hash: hashRecoveryCode(code, salt),
  }))
}
