import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  completeScore,
  latencyIndices,
  median,
} from '../src/lib/benchmark-scoring'

test('latency index gives equal weight to matched repeats, not event counts', () => {
  const rows = [
    { product: 'a', scene: 'motion', repeat: 1, median: 10, p95: 20 },
    { product: 'b', scene: 'motion', repeat: 1, median: 20, p95: 40 },
    { product: 'a', scene: 'motion', repeat: 2, median: 30, p95: 40 },
    { product: 'b', scene: 'motion', repeat: 2, median: 15, p95: 20 },
  ]
  const scores = latencyIndices(rows, ['a', 'b'])!
  assert.ok(Math.abs(scores.a! - scores.b!) < 1e-9)
  assert.ok(Math.abs(scores.a! - 100 * Math.sqrt(0.5)) < 1e-9)
  assert.equal(latencyIndices(rows.slice(0, 3), ['a', 'b']), null)
  assert.equal(latencyIndices([...rows, rows[0]!], ['a', 'b']), null)
  assert.equal(
    latencyIndices([{ ...rows[0]!, median: 0 }, ...rows.slice(1)], ['a', 'b']),
    null,
  )
})

test('incomplete categories never produce a reweighted overall score', () => {
  assert.equal(
    completeScore([
      { score: 100, weight: 1 },
      { score: null, weight: 1 },
    ]),
    null,
  )
  assert.equal(
    completeScore([
      { score: 100, weight: 1 },
      { score: 50, weight: 1 },
    ]),
    75,
  )
  assert.equal(completeScore([]), null)
  assert.equal(median([3, 1, 2, 4]), 2.5)
})
