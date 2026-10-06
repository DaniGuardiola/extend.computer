// Relay remains available in the implementation, but requires explicit opt-in.
export function relayEnabled(env: object): boolean {
  return 'RELAY_ENABLED' in env && env.RELAY_ENABLED === 'true'
}
export function relayDisabled(): Response {
  return Response.json(
    { code: 'relay_disabled', error: 'Relay is disabled', relay_enabled: false },
    { status: 503, headers: { 'Cache-Control': 'no-store', 'X-Extend-Relay-Enabled': 'false' } },
  )
}
export function relayStatus(env: object): Response {
  return Response.json({ relay_enabled: relayEnabled(env) }, {
    headers: { 'Cache-Control': 'no-store' },
  })
}
