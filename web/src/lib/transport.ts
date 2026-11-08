// Plain HTTP is only for local development, matching the hosts the desktop and
// mobile apps accept without HTTPS. Every other host must use HTTPS.
const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '[::1]'])
// One year. Not includeSubDomains: not every name in the zone serves HTTPS.
export const HSTS = 'max-age=31536000'

export function isLocalHost(url: URL): boolean {
  return LOCAL_HOSTS.has(url.hostname)
}

/** Cookies stay HTTPS-only everywhere except plain-HTTP local development. */
export function cookieHeader(
  url: URL,
  name: string,
  value: string,
  age: number,
): string {
  const secure = url.protocol === 'https:' || !isLocalHost(url)
  return `${name}=${value}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${age}${secure ? '; Secure' : ''}`
}

/** Sends plain-HTTP requests for any non-local host to the same HTTPS URL. */
export function httpsRedirect(request: Request): Response | null {
  const url = new URL(request.url)
  if (url.protocol !== 'http:' || isLocalHost(url)) return null
  url.protocol = 'https:'
  url.port = ''
  // 308 keeps the method and body for API clients; 301 is the widely cached
  // form for page loads.
  const status =
    request.method === 'GET' || request.method === 'HEAD' ? 301 : 308
  return new Response(null, { status, headers: { Location: url.href } })
}

export function setSecurityHeaders(url: URL, headers: Headers) {
  headers.set('X-Content-Type-Options', 'nosniff')
  headers.set('Referrer-Policy', 'strict-origin-when-cross-origin')
  headers.set('X-Frame-Options', 'DENY')
  headers.set('Permissions-Policy', 'camera=(), microphone=(), geolocation=()')
  if (url.protocol === 'https:' && !isLocalHost(url))
    headers.set('Strict-Transport-Security', HSTS)
}
