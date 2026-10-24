import { createElement } from 'react'
import { render, toPlainText } from 'react-email'
import {
  AccountEmail,
  accountEmailCopy,
  type EmailKind,
} from '../emails/AccountEmail'

export type { EmailKind } from '../emails/AccountEmail'
export type EmailConfig = {
  RESEND_API_KEY?: string
  EMAIL_FROM?: string
  PUBLIC_ORIGIN?: string
}

declare global {
  interface Env {
    RESEND_API_KEY?: string
  }
}

export function emailEnabled(env: EmailConfig): boolean {
  return !!(env.RESEND_API_KEY && env.EMAIL_FROM && env.PUBLIC_ORIGIN)
}

export async function accountEmail(
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
  const html = await render(
    createElement(AccountEmail, { kind, link, origin: url.origin }),
  )
  return {
    to: [to],
    subject: `${accountEmailCopy[kind].title} · extend.computer`,
    text: toPlainText(html),
    html,
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
      ...(await accountEmail(kind, email, token, env.PUBLIC_ORIGIN!)),
    }),
    signal: AbortSignal.timeout(10_000),
  })
  if (!response.ok) throw new Error('Email delivery unavailable')
}
