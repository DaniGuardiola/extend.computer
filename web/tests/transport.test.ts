import test from 'node:test'
import assert from 'node:assert/strict'
import {
  cookieHeader,
  httpsRedirect,
  setSecurityHeaders,
} from '../src/lib/transport'

const local = [
  'http://localhost:3001/api/v1/auth/login',
  'http://127.0.0.1:3001/api/v1/auth/login',
  'http://[::1]:3001/api/v1/auth/login',
]
const hosted = [
  'http://extend.computer/api/v1/auth/login',
  'http://extend-computer.example.workers.dev/login',
  'http://accounts.example.com/v1/auth/login',
  // Only exact loopback names are development hosts.
  'http://127.0.0.2/login',
  'http://localhost.example.com/login',
  'http://192.168.1.20:3001/login',
]

test('cookies are Secure everywhere except plain-HTTP local development', () => {
  for (const url of [
    'https://extend.computer/api/v1/auth/login',
    'https://localhost:3001/api/v1/auth/login',
    ...hosted,
  ])
    assert.equal(
      cookieHeader(new URL(url), 'extend_session', 'abc', 60),
      'extend_session=abc; Path=/; HttpOnly; SameSite=Lax; Max-Age=60; Secure',
      url,
    )
  for (const url of local)
    assert.equal(
      cookieHeader(new URL(url), 'extend_challenge', '', 0),
      'extend_challenge=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0',
      url,
    )
})

test('plain HTTP outside local development redirects to the same HTTPS URL', () => {
  const page = httpsRedirect(
    new Request('http://extend.computer/login?reason=session-ended'),
  )
  assert.equal(page?.status, 301)
  assert.equal(
    page?.headers.get('Location'),
    'https://extend.computer/login?reason=session-ended',
  )
  assert.equal(
    httpsRedirect(new Request('http://extend.computer/', { method: 'HEAD' }))
      ?.status,
    301,
  )
  const write = httpsRedirect(
    new Request('http://extend.computer/api/v1/auth/login', {
      method: 'POST',
      body: '{}',
    }),
  )
  assert.equal(write?.status, 308)
  assert.equal(
    write?.headers.get('Location'),
    'https://extend.computer/api/v1/auth/login',
  )
  assert.equal(
    httpsRedirect(
      new Request('http://extend.computer:8080/account'),
    )?.headers.get('Location'),
    'https://extend.computer/account',
  )
  for (const url of hosted)
    assert.match(
      httpsRedirect(new Request(url))?.headers.get('Location') ?? '',
      /^https:\/\//,
      url,
    )
  for (const url of [...local, 'https://extend.computer/login'])
    assert.equal(httpsRedirect(new Request(url)), null, url)
})

test('HTTPS responses get HSTS for one year without subdomains or preload', () => {
  const headers = new Headers()
  setSecurityHeaders(new URL('https://extend.computer/account'), headers)
  assert.equal(headers.get('Strict-Transport-Security'), 'max-age=31536000')
  assert.equal(headers.get('X-Content-Type-Options'), 'nosniff')
  assert.equal(
    headers.get('Referrer-Policy'),
    'strict-origin-when-cross-origin',
  )
  assert.equal(headers.get('X-Frame-Options'), 'DENY')
  assert.equal(
    headers.get('Permissions-Policy'),
    'camera=(), microphone=(), geolocation=()',
  )
  // Never pin local development hosts, even when they use HTTPS.
  for (const url of [...local, 'https://localhost:3001/account']) {
    const development = new Headers()
    setSecurityHeaders(new URL(url), development)
    assert.equal(development.get('Strict-Transport-Security'), null, url)
    assert.equal(development.get('X-Content-Type-Options'), 'nosniff')
  }
})
