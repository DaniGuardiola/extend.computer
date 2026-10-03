export function sessionClient(userAgent: string, native: boolean) {
  const ua = userAgent.slice(0, 1024)
  const platform = /Android/i.test(ua)
    ? 'Android'
    : /iPhone|iPad/i.test(ua)
      ? 'iOS'
      : /Windows/i.test(ua)
        ? 'Windows'
        : /Macintosh|Mac OS X/i.test(ua)
          ? 'macOS'
          : /Linux/i.test(ua)
            ? 'Linux'
            : null
  const client = native
    ? 'Desktop app'
    : /Edg\//.test(ua)
      ? 'Edge'
      : /Firefox\//.test(ua)
        ? 'Firefox'
        : /Chrome\//.test(ua)
          ? 'Chrome'
          : /Safari\//.test(ua)
            ? 'Safari'
            : 'Browser'
  return { platform, client }
}

export function sessionLocation(cf: unknown): string | null {
  if (!cf || typeof cf !== 'object') return null
  const values = cf as Record<string, unknown>
  const parts = [values.city, values.region, values.country].filter(
    (value): value is string =>
      typeof value === 'string' && value.length > 0 && value.length < 100,
  )
  return parts.length ? parts.join(', ') : null
}
