import test from 'node:test'
import assert from 'node:assert/strict'
import { PNG } from 'pngjs'
// @ts-ignore JavaScript test utility; runtime behavior is tested below.
import { comparePixels, compareSnapshot } from './visual-comparison.mjs'

function pixel(red: number) {
  const png = new PNG({ width: 1, height: 1 })
  png.data.set([red, 0, 0, 255])
  return png
}
test('visual comparison permits only channel differences up to 8 and no changed-pixel allowance', () => {
  assert.equal(comparePixels(pixel(0), pixel(8)).changedPixels, 0)
  assert.equal(comparePixels(pixel(0), pixel(8)).belowTolerancePixels, 1)
  assert.equal(comparePixels(pixel(0), pixel(9)).changedPixels, 1)
  assert.throws(
    () => comparePixels(pixel(0), new PNG({ width: 2, height: 1 })),
    /width changed/,
  )
})
test('missing visual baselines fail instead of silently creating them', async () => {
  await assert.rejects(
    compareSnapshot(
      { config: { updateSnapshots: 'none' } },
      'missing-baseline.png',
      PNG.sync.write(pixel(0)),
    ),
    { code: 'ENOENT' },
  )
})
