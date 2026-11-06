import assert from 'node:assert/strict'
import { test } from 'node:test'
import { isDevicePlatform } from '../src/lib/device-platform.ts'

test('native registration accepts mobile and desktop platforms', () => {
  for (const platform of ['android', 'ios', 'macos', 'windows', 'linux'])
    assert.equal(isDevicePlatform(platform), true, platform)
})
test('native registration rejects unknown, case-changed and coerced platform values', () => {
  for (const value of [
    'iOS',
    'unknown',
    'Android',
    '',
    null,
    undefined,
    1,
    ['android'],
    {},
  ])
    assert.equal(isDevicePlatform(value), false)
})
