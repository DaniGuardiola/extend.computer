import test from 'node:test'
import assert from 'node:assert/strict'
import { relayEnabled, relayDisabled, relayStatus } from '../src/lib/relay-policy'

test('relay requires explicit opt-in and publishes uncached availability', async () => {
  for (const value of [undefined, 'false', 'TRUE', '1', '']) {
    assert.equal(relayEnabled({ RELAY_ENABLED: value }), false)
    const response = relayStatus({ RELAY_ENABLED: value })
    assert.equal(response.headers.get('Cache-Control'), 'no-store')
    assert.deepEqual(await response.json(), { relay_enabled: false })
  }
  assert.equal(relayEnabled({ RELAY_ENABLED: 'true' }), true)
})

test('disabled attempts return a machine-readable response', async () => {
  const response = relayDisabled()
  assert.equal(response.status, 503)
  assert.deepEqual(await response.json(), {
    code: 'relay_disabled', error: 'Relay is disabled', relay_enabled: false,
  })
})
