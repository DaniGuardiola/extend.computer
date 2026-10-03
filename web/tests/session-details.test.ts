import assert from 'node:assert/strict'
import { test } from 'node:test'
import { sessionClient, sessionLocation } from '../src/lib/session-details.ts'
test('session labels distinguish native clients and browser platforms', () => {
  assert.deepEqual(
    sessionClient(
      'Mozilla/5.0 (Windows NT 10.0) Chrome/120 Safari/537 Edg/120',
      false,
    ),
    { platform: 'Windows', client: 'Edge' },
  )
  assert.deepEqual(
    sessionClient('Mozilla/5.0 (Macintosh; Intel Mac OS X) Safari/605', false),
    { platform: 'macOS', client: 'Safari' },
  )
  assert.deepEqual(
    sessionClient('Mozilla/5.0 (Linux; Android 15) Chrome/130', false),
    { platform: 'Android', client: 'Chrome' },
  )
  assert.deepEqual(sessionClient('', true), {
    platform: null,
    client: 'Desktop app',
  })
  assert.equal(sessionLocation(undefined), null)
  assert.equal(
    sessionLocation({ city: 'Madrid', region: 'Madrid', country: 'ES' }),
    'Madrid, Madrid, ES',
  )
  assert.equal(sessionLocation({ city: {}, country: '' }), null)
})
