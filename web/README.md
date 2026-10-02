# Official website and account service

TanStack Start, React, Tailwind CSS, and Ariakit. The landing page, password/passkey authentication, device directory, and session controls share one Cloudflare Worker. Account/device APIs are also available under `/v1` for native clients.

## Develop

Use Node 24 or later:

```sh
cd web
npm ci
npm run db:local
npm run dev
```

Open `http://localhost:3001` for passkeys. Raw IP addresses are not valid WebAuthn relying-party domains. Local data stays in `.wrangler/state` and never touches the production database.

```sh
npm run check
npm test
npm run build
npm run test:api # requires a running local server
```

`tests/passkey-browser.mjs` exercises signup, passkey registration/login/removal, and the confirmation dialog against an isolated Chromium with a virtual authenticator. Supply `EXTEND_TEST_CDP` and optionally `EXTEND_TEST_URL`. Test accounts use `example.invalid`; these scripts do not send email.

## Deploy

```sh
npm run deploy
```

Wrangler applies D1 migrations, uploads the built Worker and assets, and attaches the configured domain. Before deploying a separate installation, create its own D1 database, replace the database/account IDs and domain in `wrangler.jsonc`, and run `npm run types`. Never reuse the official database for another installation or an untrusted preview.

The Rust server in [`../server`](../server/README.md) remains the standalone SQLite/Docker option. Its passwords use Argon2id; the Cloudflare implementation uses scrypt inside SQLite-backed Durable Objects. Their databases and account credentials are separate. The Rust server supports passkey registration, discoverable login, listing, and removal with an explicitly configured HTTPS origin. It remains a bearer-token API without the website's browser-session UI.

## Hosting and authentication

All configured Cloudflare products have free tiers: Workers/static assets, D1, SQLite-backed Durable Objects, and Workers Builds. No paid plan is required or enabled. Free quota exhaustion can make requests fail; it does not automatically upgrade the account. Follow Cloudflare's current [Workers limits](https://developers.cloudflare.com/workers/platform/limits/), [D1 pricing](https://developers.cloudflare.com/d1/platform/pricing/), [Durable Object pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/), and [Builds pricing](https://developers.cloudflare.com/workers/ci-cd/builds/limits-and-pricing/).

Slow password hashing cannot reliably fit the free Worker's 10ms CPU budget. The Worker delegates account work to 16 IP-sharded Durable Objects with a larger request CPU budget. Passwords use scrypt N=32768, r=8, p=3, random salts, and constant-time verification. Account data lives in D1; Durable Object SQLite coordinates bounded per-IP/per-email abuse limits. Native device heartbeat requests go directly to D1.

Browser sessions use Secure HttpOnly SameSite=Lax cookies. Cookie-authenticated writes require the exact Origin. Native signup/login requests send `X-Extend-Client: desktop` plus an Origin matching the server URL to receive an account bearer token. Session/device tokens are random 256-bit values stored only as SHA-256 hashes. Device tokens can only heartbeat their own device. Account tokens cannot substitute for device tokens. Sign-out, device removal, password changes, and other-session revocation invalidate the relevant credentials.

Passkeys require user verification, use one-time five-minute challenges bound to the exact origin/account, and validate signatures and counters with SimpleWebAuthn. Passkeys are scoped to their hostname: register them on the final domain rather than a temporary workers.dev address. Adding a passkey currently starts with password signup. Email verification and recovery use the optional email configuration below. Without it, keep a password or a second passkey available.

Daily cleanup removes bounded batches of expired sessions/challenges. Device presence expires after 90 seconds without a heartbeat. Request bodies are capped at 16 KiB; session/device/passkey counts are bounded. Signups can be closed with `SIGNUP_ENABLED=false`.

## Device connection boundary

The directory is account-private and supports registration, presence, and removal. Desktop sign-in, device registration, presence, account device listing, and sign-out are integrated. The app can connect already-paired local devices from the account directory; new devices still require local pairing. Automatic account-based pairing and internet connections are not implemented. Local account-free pairing still works. A public-key registration is a claim, not proof of possession. Before enabling account-based control, verify key possession and bind discovery to the engine's pinned encrypted peer handshake and local permissions. Internet connections also need rendezvous/NAT traversal and an encrypted relay fallback.

## Transactional email

Cloudflare's free plan cannot send to arbitrary users. The optional Resend adapter works from the existing free Worker without upgrading Cloudflare. Keep Resend on its free tier with overages disabled. Sending remains disabled until both `RESEND_API_KEY` (a Worker secret) and `EMAIL_FROM` (a verified sender) exist. `PUBLIC_ORIGIN` must be the installation's exact HTTPS origin; localhost HTTP is accepted for development. Never put the API key in source or build variables.

The official installation's sender is `extend.computer <accounts@mail.extend.computer>`, with the Resend domain in Ireland. Verify the domain's DKIM and sending records, keep CNAME records DNS-only, and leave click/open tracking disabled for authentication links. Create a sending-only API key restricted to `mail.extend.computer` and store it as `RESEND_API_KEY` on the Worker. Self-hosted installations should use their own sender, domain, and key.

```sh
npx wrangler secret put RESEND_API_KEY
# Add EMAIL_FROM to wrangler.jsonc vars after verifying its domain in Resend.
npm run deploy
```

Signup requests a verification email when sending is enabled; the account dashboard can resend. Recovery accepts verified accounts only, returns the same response for unknown accounts and delivery failures, and sends in the background to avoid exposing account existence through provider latency. Links expire after 30 minutes, are stored as SHA-256 hashes, and are consumed once with DELETE RETURNING. They are bound to the account's password version so a password change invalidates old links. Tokens use URL fragments, are cleared from history on arrival, and require a form POST; mail scanners cannot consume them with GET requests.

A password reset signs out every login, invalidates device credentials, and clears other email links and registration challenges. Enabled two-factor authentication and its factors remain required; email recovery cannot bypass them. Passkeys are removed only when two-factor authentication is off. Account/device records remain. Password changes and session creation use conditional database writes so a concurrent recovery cannot reissue a session from stale credentials.

Email requests have a global per-account one-minute cooldown. A single Durable Object enforces an installation-wide budget of 90 sends per UTC day and 2,700 per calendar month, including failures, below Resend's free limits. These limits never enable paid overages. `tests/email-api.mjs` uses local-only seeded link fixtures to verify expiry, replay, password-version binding, reset, and credential revocation without sending email. `tests/email.test.ts` verifies the payload and mocked provider transport.

The official sender was activated on October 2, 2026. Live verification and recovery were tested using Resend's designated `delivered+...@resend.dev` address: both messages reached the provider's simulated delivery event, both links worked once, and recovery revoked the old session and password. The disposable account was removed afterward. A separate verification email also reached the user’s real inbox, confirmed on October 2, 2026.

## Two-factor authentication

The account dashboard supports six-digit TOTP authenticator codes, passkeys, USB/NFC security keys (including touch-only keys), and ten single-use recovery codes. Enabling 2FA requires a fresh password check and proof of the enrolled factor. Save recovery codes when shown; only their hashes are stored. With 2FA enabled, sign in with your password followed by one enrolled factor or recovery code. Direct passwordless passkey login is disabled for these accounts.

Security changes require a fresh password and, when enabled, a second factor. Changes revoke other sessions and invalidate pending challenges. TOTP codes cannot be replayed. MFA tickets expire after five minutes, have bounded attempts, and cannot access accounts or devices.

Before deploying TOTP, generate a separate random 32-byte key encoded as 64 hexadecimal characters and store it as the Worker secret `MFA_ENCRYPTION_KEY` (`npx wrangler secret put MFA_ENCRYPTION_KEY`). Back up this key securely alongside the database: TOTP secrets use AES-256-GCM with account-bound authenticated encryption. Replacing the key without migrating encrypted secrets prevents existing authenticators from working. Passkey-only 2FA does not require this key.

The standalone server supports the same factors and ticket flow; see its README for configuration and API shapes. `EXTEND_TEST_URL=http://localhost:3006 node tests/mfa-browser.mjs` verifies the UI in isolated Chromium; `tests/mfa-api.mjs` verifies enforcement, replay protection, and session revocation against local D1. Unit tests cover RFC 6238 vectors and encryption tampering.

## Desktop browser sign-in

The app offers direct password login with TOTP/recovery codes and browser login for passkeys/security keys. Browser login opens `/desktop/connect` with an ephemeral loopback port, random state, and SHA-256 PKCE challenge. The user explicitly approves the signed-in account. The server issues a two-minute code bound to that browser session and challenge. `/v1/auth/desktop/exchange` consumes it only with the correct verifier, checks the source session, and issues a separate native session. Codes are stored as hashes and cannot be replayed. Account/session tokens never enter callback URLs or the desktop webview.

`tests/desktop-login.mjs` verifies the browser handoff, verifier binding, replay, and browser-session revocation using an isolated Chromium and local callback listener. Device registration uses the app's existing X25519 identity. Account presence is a directory hint; it never grants input-control permission or replaces pinned local pairing.
