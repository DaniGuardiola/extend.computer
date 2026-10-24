import { chromium } from 'playwright'
import { PNG } from 'pngjs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'
import { previewEmails, previewHtml } from '../scripts/email-fixtures'

const update = process.argv.includes('--update')
const baseline = fileURLToPath(new URL('./email-snapshots/', import.meta.url))
const results = fileURLToPath(
  new URL('../test-results/email-visual/', import.meta.url),
)
await mkdir(update ? baseline : results, { recursive: true })
const browser = await chromium.launch({ headless: true })
let failures = 0
try {
  const page = await browser.newPage({ deviceScaleFactor: 1, locale: 'en-US' })
  // Templates use system fonts and an embedded local logo; no network needed.
  await page.route('**/*', (route) => route.abort())
  for (const message of await previewEmails()) {
    for (const theme of ['light', 'dark', 'fallback'] as const) {
      for (const width of [375, 640]) {
        const name = `${message.kind}-${theme}-${width}.png`
        await page.setViewportSize({ width, height: 1000 })
        await page.setContent(previewHtml(message.html, theme), {
          waitUntil: 'load',
        })
        await page.evaluate(() => document.fonts.ready)
        const shot = await page.screenshot({ fullPage: true })
        if (update) {
          await writeFile(join(baseline, name), shot)
          console.log(`Updated ${name}`)
          continue
        }
        let expected: Buffer
        try {
          expected = await readFile(join(baseline, name))
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
          await writeFile(join(results, name), shot)
          failures++
          console.error(
            `${name}: missing baseline (use npm run test:email:visual:update after reviewing)`,
          )
          continue
        }
        const before = PNG.sync.read(expected)
        const after = PNG.sync.read(shot)
        if (before.width !== after.width || before.height !== after.height) {
          failures++
          await writeFile(join(results, name), shot)
          console.error(
            `${name}: dimensions changed ${before.width}×${before.height} → ${after.width}×${after.height}`,
          )
          continue
        }
        const diff = new PNG({ width: after.width, height: after.height })
        let changed = 0
        for (let i = 0; i < after.data.length; i += 4) {
          const mismatch = !before.data
            .subarray(i, i + 4)
            .equals(after.data.subarray(i, i + 4))
          if (mismatch) changed++
          diff.data[i] = mismatch ? 255 : after.data[i]
          diff.data[i + 1] = mismatch ? 0 : after.data[i + 1]
          diff.data[i + 2] = mismatch ? 0 : after.data[i + 2]
          diff.data[i + 3] = 255
        }
        console.log(`${name}: ${changed} changed pixels`)
        if (changed) {
          failures++
          await writeFile(join(results, name), shot)
          await writeFile(
            join(results, name.replace('.png', '.diff.png')),
            PNG.sync.write(diff),
          )
        }
      }
    }
  }
  console.log(`Chromium ${browser.version()} on ${process.platform}`)
} finally {
  await browser.close()
}
if (failures) {
  console.error(
    `${failures} visual checks failed. Actual/diff images: ${results}`,
  )
  process.exitCode = 1
}
