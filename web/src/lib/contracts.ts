import { z } from 'zod'
import type {
  PublicKeyCredentialCreationOptionsJSON,
  PublicKeyCredentialRequestOptionsJSON,
  RegistrationResponseJSON,
  AuthenticationResponseJSON,
} from '@simplewebauthn/browser'

const flag = z.union([z.literal(0), z.literal(1)])
export const accountSchema = z.object({
  id: z.string(),
  email: z.string(),
  email_verified: flag,
  mfa_enabled: flag,
  created_at: z.number().optional(),
  email_enabled: z.boolean().optional(),
})
export const deviceSchema = z.object({
  id: z.string(),
  name: z.string(),
  platform: z.string(),
  fingerprint: z.string(),
  online: z.boolean(),
  last_seen: z.number().nullable().optional(),
})
export const passkeySchema = z.object({
  id: z.string(),
  name: z.string(),
  created_at: z.number(),
  rp_id: z.string().optional(),
  purpose: z.enum(['login', 'login-disabled', 'factor']),
})
export const mfaSchema = z.object({
  enabled: z.boolean(),
  totp: z.boolean(),
  keys: z.number().int().nonnegative(),
  recovery_codes: z.number().int().nonnegative(),
  totp_available: z.boolean().optional(),
})
export const sessionSchema = z.object({
  id: z.string(),
  created_at: z.number(),
  last_access_at: z.number(),
  platform: z.string().nullable(),
  client: z.string().nullable(),
  location: z.string().nullable(),
  current: flag,
  device_names: z.array(z.string()).optional(),
})
export const pendingSchema = z.object({
  mfa_required: z.literal(true),
  ticket: z.string(),
  totp: z.boolean(),
  keys: z.boolean(),
  recovery: z.boolean(),
})
export type Account = z.infer<typeof accountSchema>
export type Device = z.infer<typeof deviceSchema>
export type Passkey = z.infer<typeof passkeySchema>
export type MfaInfo = z.infer<typeof mfaSchema>
export type AccountSession = z.infer<typeof sessionSchema>
export type MfaPending = z.infer<typeof pendingSchema>

const credential = z.custom<
  RegistrationResponseJSON | AuthenticationResponseJSON
>(
  (value) =>
    z
      .looseObject({
        id: z.string(),
        rawId: z.string(),
        type: z.literal('public-key'),
        response: z.looseObject({ clientDataJSON: z.string() }),
      })
      .safeParse(value).success,
)
const requestOptions = z.looseObject({
  challenge: z.string(),
  rpId: z.string().optional(),
  allowCredentials: z
    .array(z.looseObject({ id: z.string(), type: z.literal('public-key') }))
    .optional(),
})
const creationOptions = z.looseObject({
  challenge: z.string(),
  rp: z.object({ name: z.string(), id: z.string().optional() }),
  user: z.object({ id: z.string(), name: z.string(), displayName: z.string() }),
  pubKeyCredParams: z.array(
    z.object({ type: z.literal('public-key'), alg: z.number().int() }),
  ),
})
// Browser WebAuthn performs the remaining protocol-specific validation.
const authentication = z.custom<PublicKeyCredentialRequestOptionsJSON>(
  (value) => requestOptions.safeParse(value).success,
)
const registration = z.custom<PublicKeyCredentialCreationOptionsJSON>(
  (value) => creationOptions.safeParse(value).success,
)
const success = z.object({ success: z.boolean() })
const recovery = z.object({ recovery_codes: z.array(z.string()).optional() })
const noBody = z.undefined()
const empty = z.unknown().transform(() => undefined)
const signin = z.union([
  pendingSchema,
  z.object({ account: z.looseObject({ id: z.string() }) }),
])
const password = z.string().min(1)
const signOut = z.object({ sign_out_others: z.boolean() })
const endpoint = <I extends z.ZodType, O extends z.ZodType>(
  input: I,
  output: O,
) => ({ input, output })

export const contracts = {
  'GET /account': endpoint(noBody, accountSchema),
  'GET /devices': endpoint(
    noBody,
    z.object({ devices: z.array(deviceSchema) }),
  ),
  'GET /passkeys': endpoint(
    noBody,
    z.object({ passkeys: z.array(passkeySchema) }),
  ),
  'GET /mfa': endpoint(noBody, mfaSchema),
  'GET /sessions': endpoint(
    noBody,
    z.object({ sessions: z.array(sessionSchema) }),
  ),
  'GET /server': endpoint(noBody, z.object({ email_enabled: z.boolean() })),
  'POST /auth/login': endpoint(
    z.object({ email: z.string(), password }),
    signin,
  ),
  'POST /auth/signup': endpoint(
    z.object({ email: z.string(), password }),
    signin,
  ),
  'POST /auth/desktop/authorize': endpoint(
    z.object({ challenge: z.string() }),
    z.object({ code: z.string() }),
  ),
  'POST /auth/logout': endpoint(noBody, empty),
  'POST /auth/email/request': endpoint(
    noBody,
    z.object({ retry_after: z.number().int().nonnegative() }),
  ),
  'POST /auth/email/verify': endpoint(z.object({ token: z.string() }), empty),
  'POST /auth/recovery/request': endpoint(
    z.object({ email: z.string() }),
    empty,
  ),
  'POST /auth/recovery/complete': endpoint(
    z.object({ token: z.string(), password }),
    empty,
  ),
  'POST /mfa/reauth': endpoint(
    z.object({ password }),
    z.union([pendingSchema, success]),
  ),
  'POST /auth/mfa/verify': endpoint(
    z.object({ ticket: z.string(), code: z.string() }),
    empty,
  ),
  'POST /auth/mfa/key/options': endpoint(
    z.object({ ticket: z.string() }),
    authentication,
  ),
  'POST /auth/mfa/key/verify': endpoint(
    z.intersection(credential, z.object({ ticket: z.string() })),
    empty,
  ),
  'POST /passkeys/login/options': endpoint(noBody, authentication),
  'POST /passkeys/login/verify': endpoint(credential, empty),
  'POST /passkeys/register/options': endpoint(
    z.object({ auto: z.literal(true) }),
    registration,
  ),
  'POST /passkeys/register/verify': endpoint(credential, recovery),
  'PATCH /passkeys/:id': endpoint(
    z.object({ sign_in_enabled: z.boolean() }),
    empty,
  ),
  'DELETE /passkeys/:id': endpoint(signOut, empty),
  'DELETE /devices/:id': endpoint(noBody, empty),
  'DELETE /sessions/:id': endpoint(noBody, empty),
  'DELETE /sessions': endpoint(noBody, empty),
  'POST /account/password': endpoint(
    signOut.extend({ current_password: password, password }),
    empty,
  ),
  'POST /mfa/totp/setup': endpoint(
    noBody,
    z.object({ secret: z.string(), uri: z.string() }),
  ),
  'POST /mfa/totp/confirm': endpoint(
    signOut.extend({ code: z.string() }),
    recovery,
  ),
  'DELETE /mfa/totp': endpoint(signOut, empty),
  'POST /mfa/enable': endpoint(noBody, recovery),
  'POST /mfa/recovery-codes': endpoint(noBody, recovery),
} as const
export type Endpoint = keyof typeof contracts
export type Input<K extends Endpoint> = z.input<(typeof contracts)[K]['input']>
export type Output<K extends Endpoint> = z.output<
  (typeof contracts)[K]['output']
>
