import { PNG } from 'pngjs'
import fs from 'node:fs/promises'
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'

// Preserve the existing strict comparison: dimensions must match and no pixel
// may exceed 8/255 in any RGB channel. No percentage-of-image allowance.
export function comparePixels(before, after) {
  assert.equal(after.width, before.width, 'Screenshot width changed')
  assert.equal(after.height, before.height, 'Screenshot height changed')
  const diff = new PNG({ width: after.width, height: after.height })
  let changedPixels = 0
  let belowTolerancePixels = 0
  for (let i = 0; i < after.data.length; i += 4) {
    const delta = Math.max(
      ...[0, 1, 2].map((c) => Math.abs(before.data[i + c] - after.data[i + c])),
    )
    if (delta > 8) {
      changedPixels++
      diff.data.set([255, 0, 100, 255], i)
    } else {
      if (delta) belowTolerancePixels++
      diff.data.set(
        [after.data[i], after.data[i + 1], after.data[i + 2], 80],
        i,
      )
    }
  }
  return { changedPixels, belowTolerancePixels, diff }
}

export async function compareSnapshot(testInfo, name, shot) {
  const expected = fileURLToPath(
    new URL(`./account-snapshots/${name}`, import.meta.url),
  )
  if (testInfo.config.updateSnapshots === 'all') {
    await fs.writeFile(expected, shot)
    return
  }
  // Missing baselines fail; updates are always an explicit separate command.
  const baseline = await fs.readFile(expected)
  try {
    const result = comparePixels(PNG.sync.read(baseline), PNG.sync.read(shot))
    await testInfo.attach('pixel-comparison', {
      body: JSON.stringify({
        changedPixels: result.changedPixels,
        belowTolerancePixels: result.belowTolerancePixels,
        channelTolerance: 8,
      }),
      contentType: 'application/json',
    })
    if (result.changedPixels)
      await testInfo.attach('diff', {
        body: PNG.sync.write(result.diff),
        contentType: 'image/png',
      })
    assert.equal(
      result.changedPixels,
      0,
      'Screenshot pixels changed beyond 8/255',
    )
  } catch (error) {
    await testInfo.attach('expected', {
      body: baseline,
      contentType: 'image/png',
    })
    await testInfo.attach('actual', { body: shot, contentType: 'image/png' })
    throw error
  }
}
