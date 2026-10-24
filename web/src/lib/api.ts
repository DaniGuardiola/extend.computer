import { contracts, type Endpoint, type Input, type Output } from './contracts'
export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
    public retryAfter?: number,
  ) {
    super(message)
  }
}
function sessionEnded(error: unknown) {
  return (
    error instanceof ApiError &&
    error.status === 401 &&
    [
      'Please sign in',
      'Session expired. Sign in again.',
      'Browser session ended. Sign in again.',
      'Credentials changed. Please sign in again.',
      'Session changed. Sign in again.',
    ].includes(error.message)
  )
}
async function transport<T>(
  path: string,
  method = 'GET',
  body?: unknown,
  options: { redirectOnSessionEnd?: boolean; signal?: AbortSignal } = {},
): Promise<T> {
  const response = await fetch('/api/v1' + path, {
    method,
    signal: options.signal,
    credentials: 'same-origin',
    headers:
      body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  }).catch((error) => {
    if (options.signal?.aborted) throw error
    throw new Error('Could not connect. Check your connection and try again.')
  })
  if (response.status === 204) return undefined as T
  const data = await response.json().catch((error) => {
    if (options.signal?.aborted) throw error
    throw new ApiError(
      response.status,
      'Could not complete the request. Please try again.',
    )
  })
  if (!response.ok) {
    const error = new ApiError(
      response.status,
      data &&
        typeof data === 'object' &&
        'error' in data &&
        typeof data.error === 'string'
        ? data.error
        : 'Please try again.',
      Number(response.headers.get('Retry-After')) || undefined,
    )
    if (sessionEnded(error) && options.redirectOnSessionEnd !== false)
      window.location.assign('/login?reason=session-ended')
    throw error
  }
  return data as T
}
export type { Account, Device, Passkey } from './contracts'
export const message = (error: unknown) => {
  if (sessionEnded(error)) return ''
  return error instanceof Error ? error.message : 'Please try again.'
}

export async function api<K extends Endpoint>(
  endpoint: K,
  ...[body, options]: Input<K> extends undefined
    ? [
        body?: Input<K>,
        options?: {
          id?: string
          signal?: AbortSignal
          redirectOnSessionEnd?: boolean
        },
      ]
    : [
        body: Input<K>,
        options?: {
          id?: string
          signal?: AbortSignal
          redirectOnSessionEnd?: boolean
        },
      ]
): Promise<Output<K>> {
  const contract = contracts[endpoint]
  const input = contract.input.safeParse(body)
  if (!input.success)
    throw new Error('Check the information you entered and try again.')
  const [method, template] = endpoint.split(' ')
  const path = template!.includes(':id')
    ? template!.replace(':id', encodeURIComponent(options?.id ?? ''))
    : template!
  if (template!.includes(':id') && !options?.id)
    throw new Error('Please try again.')
  const data = await transport<unknown>(path, method, input.data, options)
  const result = contract.output.safeParse(data)
  if (!result.success)
    throw new Error('Could not complete the request. Please try again.')
  return result.data as Output<K>
}
