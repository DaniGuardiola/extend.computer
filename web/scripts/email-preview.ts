import { createServer } from 'node:http'
import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { previewEmails, previewHtml } from './email-fixtures'

const escape = (value: string) =>
  value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
const messages = await previewEmails()
const html = `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>extend.computer email previews</title><style>
body{font-family:system-ui,sans-serif;background:#f3f5ef;color:#20271e;margin:32px}main{max-width:1240px;margin:auto}h1{font-size:24px}p{color:#596451}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:24px}section{min-width:0}iframe{width:100%;height:900px;border:1px solid #dce2d5;border-radius:8px;background:white}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:white;padding:20px;border-radius:8px;font-size:13px}details{margin-top:16px}select{font:inherit;padding:6px}body[data-width="mobile"] iframe{width:375px;max-width:100%}@media(max-width:600px){body{margin:16px}}
</style><main><h1>Email previews</h1><p>Current templates, dummy tokens. No email sent. Links disabled inside previews.</p><p>Dark preview shows authored styles; mail apps may apply their own color changes. Unsupported preview removes stylesheet rules, keeping inline defaults.</p><label>Viewport <select onchange="document.body.dataset.width=this.value"><option value="desktop">Desktop</option><option value="mobile">Mobile (375px)</option></select></label> <label>Theme <select onchange="document.querySelectorAll('iframe').forEach(frame => frame.srcdoc=frame.dataset[this.value])"><option value="light">Light</option><option value="dark">Dark</option><option value="fallback">Unsupported client</option></select></label><div class="grid">${messages.map((message) => `<section><h2>${escape(message.subject)}</h2><iframe sandbox title="${escape(message.subject)}" srcdoc="${escape(previewHtml(message.html, 'light'))}" data-light="${escape(previewHtml(message.html, 'light'))}" data-dark="${escape(previewHtml(message.html, 'dark'))}" data-fallback="${escape(previewHtml(message.html, 'fallback'))}"></iframe><details><summary>Plain text</summary><pre>${escape(message.text)}</pre></details></section>`).join('')}</div></main></html>`

const outputIndex = process.argv.indexOf('--output')
if (outputIndex !== -1) {
  const directory = process.argv[outputIndex + 1]
  if (!directory) throw new Error('--output requires a directory')
  await mkdir(directory, { recursive: true })
  await writeFile(join(directory, 'index.html'), html)
  for (const message of messages)
    await writeFile(join(directory, `${message.kind}.html`), message.html)
  console.log(join(directory, 'index.html'))
} else {
  createServer((_request, response) => {
    response.writeHead(200, {
      'Content-Type': 'text/html; charset=utf-8',
      'Cache-Control': 'no-store',
    })
    response.end(html)
  }).listen(3002, '127.0.0.1', () =>
    console.log('Email previews: http://127.0.0.1:3002 (reload after editing)'),
  )
}
