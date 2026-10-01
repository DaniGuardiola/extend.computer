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

The Rust server in [`../server`](../server/README.md) remains the standalone SQLite/Docker option. Its passwords use Argon2id; the Cloudflare implementation uses scrypt inside SQLite-backed Durable Objects. Their databases and account credentials are separate. The Rust server does not currently implement the website's cookie/passkey/session-management extensions.

## Hosting and authentication

All configured Cloudflare products have free tiers: Workers/static assets, D1, SQLite-backed Durable Objects, and Workers Builds. No paid plan is required or enabled. Free quota exhaustion can make requests fail; it does not automatically upgrade the account. Follow Cloudflare's current [Workers limits](https://developers.cloudflare.com/workers/platform/limits/), [D1 pricing](https://developers.cloudflare.com/d1/platform/pricing/), [Durable Object pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/), and [Builds pricing](https://developers.cloudflare.com/workers/ci-cd/builds/limits-and-pricing/).

Slow password hashing cannot reliably fit the free Worker's 10ms CPU budget. The Worker delegates account work to 16 IP-sharded Durable Objects with a larger request CPU budget. Passwords use scrypt N=32768, r=8, p=3, random salts, and constant-time verification. Account data lives in D1; Durable Object SQLite coordinates bounded per-IP/per-email abuse limits. Native device heartbeat requests go directly to D1.

Browser sessions use Secure HttpOnly SameSite=Lax cookies. Cookie-authenticated writes require the exact Origin. Native signup/login requests send `X-Extend-Client: desktop` plus an Origin matching the server URL to receive an account bearer token. Session/device tokens are random 256-bit values stored only as SHA-256 hashes. Device tokens can only heartbeat their own device. Account tokens cannot substitute for device tokens. Sign-out, device removal, password changes, and other-session revocation invalidate the relevant credentials.

Passkeys require user verification, use one-time five-minute challenges bound to the exact origin/account, and validate signatures and counters with SimpleWebAuthn. Passkeys are scoped to their hostname: register them on the final domain rather than a temporary workers.dev address. Adding a passkey currently starts with password signup. Email verification and email-based password recovery are not implemented; keep a password or a second passkey available.

Daily cleanup removes bounded batches of expired sessions/challenges. Device presence expires after 90 seconds without a heartbeat. Request bodies are capped at 16 KiB; session/device/passkey counts are bounded. Signups can be closed with `SIGNUP_ENABLED=false`.

## Device connection boundary

The directory is account-private and supports registration, presence, and removal. Desktop account sign-in and automatic connections are not integrated yet. Local account-free pairing still works. A public-key registration is a claim, not proof of possession. Before enabling account-based control, verify key possession and bind discovery to the engine's pinned encrypted peer handshake and local permissions. Internet connections also need rendezvous/NAT traversal and an encrypted relay fallback.
