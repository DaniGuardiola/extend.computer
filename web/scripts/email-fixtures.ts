import { readFile } from 'node:fs/promises'
import { accountEmail } from '../src/lib/email'

export async function previewEmails() {
  // Embed the local asset only in previews, before it has been deployed.
  const logo = `data:image/png;base64,${(await readFile(new URL('../public/email-logo.png', import.meta.url))).toString('base64')}`
  const messages = await Promise.all(
    (['verify', 'reset'] as const).map(async (kind) => ({
      kind,
      ...(await accountEmail(
        kind,
        'preview@example.invalid',
        'a'.repeat(64),
        'https://extend.computer',
      )),
    })),
  )
  for (const message of messages) {
    message.html = message.html.replace(
      'https://extend.computer/email-logo.png',
      logo,
    )
  }
  return messages
}

// Force only the dark media query for browser previews; sent HTML stays adaptive.
export const previewHtml = (
  html: string,
  theme: 'light' | 'dark' | 'fallback',
) =>
  theme === 'fallback'
    ? html.replace(/<style>[\s\S]*?<\/style>/g, '')
    : html.replace(
        '@media (prefers-color-scheme: dark)',
        theme === 'dark' ? '@media all' : '@media not all',
      )
