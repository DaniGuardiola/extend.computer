export type EmailKind = 'verify' | 'reset'
export type EmailConfig = {
  RESEND_API_KEY?: string
  EMAIL_FROM?: string
  PUBLIC_ORIGIN?: string
}

declare global {
  interface Env extends EmailConfig {}
}

export function emailEnabled(env: EmailConfig): boolean {
  return !!(env.RESEND_API_KEY && env.EMAIL_FROM && env.PUBLIC_ORIGIN)
}

export function accountEmail(
  kind: EmailKind,
  to: string,
  token: string,
  origin: string,
) {
  const url = new URL(origin)
  if (
    (url.protocol !== 'https:' &&
      !(url.protocol === 'http:' && url.hostname === 'localhost')) ||
    url.pathname !== '/' ||
    url.search ||
    url.hash ||
    url.username ||
    url.password
  )
    throw new Error('Invalid email origin')
  if (!/^[a-f0-9]{64}$/.test(token)) throw new Error('Invalid email token')
  const link = `${url.origin}/email#${kind}=${token}`
  const title = kind === 'verify' ? 'Verify your email' : 'Reset your password'
  const description =
    kind === 'verify'
      ? 'Confirm this address belongs to you.'
      : 'Choose a new password for your extend.computer account. Existing sessions and passkeys will be revoked.'
  return {
    to: [to],
    subject: `${title} · extend.computer`,
    text: `${title}\n\n${description}\n\n${link}\n\nThis link expires in 30 minutes and works once. If you didn’t request this, ignore this email.`,
    html: `<div style="font-family:system-ui,sans-serif;max-width:520px;margin:40px auto;color:#182014"><p style="font-weight:600">extend.computer</p><h1>${title}</h1><p>${description}</p><p><a href="${link}" style="display:inline-block;padding:14px 22px;background:#d5f58c;color:#182014;border-radius:6px;text-decoration:none">${title}</a></p><p style="color:#66705f;font-size:13px">This link expires in 30 minutes and works once. If you didn’t request this, ignore this email.</p></div>`,
  }
}

export async function sendAccountEmail(
  env: EmailConfig,
  kind: EmailKind,
  email: string,
  token: string,
  transport: typeof fetch = fetch,
): Promise<void> {
  if (!emailEnabled(env)) throw new Error('Email is not configured')
  const response = await transport('https://api.resend.com/emails', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${env.RESEND_API_KEY}`,
      'Content-Type': 'application/json',
      'Idempotency-Key': token,
    },
    body: JSON.stringify({
      from: env.EMAIL_FROM,
      ...accountEmail(kind, email, token, env.PUBLIC_ORIGIN!),
    }),
    signal: AbortSignal.timeout(10_000),
  })
  if (!response.ok) throw new Error('Email delivery unavailable')
}
