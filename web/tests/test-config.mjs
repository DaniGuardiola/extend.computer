import { randomInt } from 'node:crypto'
import assert from 'node:assert/strict'

const target = new URL(process.env.EXTEND_TEST_URL ?? 'http://localhost:3001')
assert.ok(
  ['localhost', '127.0.0.1'].includes(target.hostname),
  'Tests require a local server',
)
assert.equal(target.pathname, '/', 'EXTEND_TEST_URL must be a server origin')
export const testOrigin = target.origin

// Separate local integration clients so unrelated suites do not share IP budgets.
export function createTestHeaders() {
  return {
    'CF-Connecting-IP': `10.${randomInt(1, 255)}.${randomInt(1, 255)}.${randomInt(1, 255)}`,
  }
}
export const testHeaders = createTestHeaders()
