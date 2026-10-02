export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
  }
}
export async function api<T>(
  path: string,
  method = 'GET',
  body?: unknown,
): Promise<T> {
  const response = await fetch('/api/v1' + path, {
    method,
    credentials: 'same-origin',
    headers:
      body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
  if (response.status === 204) return undefined as T
  const data = await response.json()
  if (!response.ok)
    throw new ApiError(
      response.status,
      data &&
        typeof data === 'object' &&
        'error' in data &&
        typeof data.error === 'string'
        ? data.error
        : 'Please try again.',
    )
  return data as T
}
export type Account = {
  id: string
  email: string
  created_at: number
  email_verified: number
  email_enabled: boolean
  mfa_enabled: number
}
export type Device = {
  id: string
  name: string
  platform: string
  fingerprint: string
  online: boolean
  last_seen: number | null
}
export type Passkey = {
  id: string
  name: string
  created_at: number
  rp_id: string
  purpose: string
}
export const message = (error: unknown) =>
  error instanceof Error ? error.message : 'Please try again.'
