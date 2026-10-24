import { api } from '../src/lib/api'

// Compile-only checks: changing endpoint types must not silently allow invalid
// callers or restore caller-defined response assertions.
function verifyContracts() {
  api('POST /auth/login', { email: 'a@example.invalid', password: 'password' })
  api('GET /account').then((account) => account.mfa_enabled)
  // @ts-expect-error Unknown endpoint
  api('GET /unknown')
  // @ts-expect-error Login requires a password
  api('POST /auth/login', { email: 'a@example.invalid' })
  // @ts-expect-error Usage setting requires a boolean
  api('PATCH /passkeys/:id', { sign_in_enabled: 'yes' }, { id: 'key' })
  // @ts-expect-error A GET endpoint has no request body
  api('GET /account', { password: 'password' })
  // @ts-expect-error Response is inferred from the endpoint
  api<{ invented: string }>('GET /account')
}
void verifyContracts
