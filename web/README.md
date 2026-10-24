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

`npm run test:browser` runs the isolated browser UI suites with mocked API responses, including account data refresh, dialogs, validation, themes, and app handoff. `npm run test:browser:integration` runs the real local backend suites; those create disposable `example.invalid` accounts and some require the built standalone Rust server or desktop callback tooling. Neither command uses the user's browser.

Playwright Test manages a development server on `http://localhost:3026` by default. Run `npm run db:local` once before backend integration tests. Set `EXTEND_TEST_URL` to reuse another local server (for example, `EXTEND_TEST_URL=http://localhost:3016 npm run test:browser`). The target must be localhost or 127.0.0.1; use localhost for WebAuthn. Standalone API scripts still default to port 3001. Browser suites use isolated Chromium contexts and virtual authenticators; no personal browser or CDP setup is needed. Run browser suites sequentially so reports and traces cannot overwrite each other. Failed checks retain traces under `test-results/playwright`; `npm run test:browser:report` opens the HTML report.

## Deploy

The official Worker is connected to `DaniGuardiola/extend.computer` through
Cloudflare Workers Builds. Pushes to `main` build and deploy automatically from
the `web` directory using Node 24.19.0. The build command is `npm run build`;
the deploy command is `npm run db:remote && npx wrangler deploy`, so required D1
migrations run before the Worker update. Preview builds are disabled. Deployment
credentials are managed by Cloudflare; no GitHub deployment secret is required.

Manual deployment remains available:

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

Sessions expire after 30 days without activity. Authenticated account requests renew browser cookies and session expiry; device heartbeats renew their linked session. The sessions list flags 14 or more days of inactivity. Expired sessions cannot be renewed. Daily cleanup removes bounded batches of expired sessions/challenges. Device presence expires after 90 seconds without a heartbeat. Request bodies are capped at 16 KiB; session/device/passkey counts are bounded. Signups can be closed with `SIGNUP_ENABLED=false`.

The account dashboard lists active sessions with an opaque public ID, client/platform, sign-in time, last activity, and approximate Cloudflare location when available. No raw session credentials or IP addresses are exposed by the session list. Local geolocation is omitted because the development runtime simulates it. Account requests update activity at most once per minute; device heartbeats also count toward activity. Older sessions acquire client details on their next account request. Sign out one session or all sessions, including the current browser; removing a session also revokes its linked device credentials. Apply migration `0006_sessions.sql` before running this version. Local checks: `node tests/sessions-api.mjs` and `npx playwright test tests/sessions.spec.mjs` with `EXTEND_TEST_URL` pointing to the development server.

## Device connection boundary

The directory is account-private and supports registration, presence, and removal. Desktop sign-in, device registration, presence, account device listing, and sign-out are integrated. The app can connect already-paired local devices from the account directory; new devices still require local pairing. Automatic account-based pairing and internet connections are not implemented. Local account-free pairing still works. A public-key registration is a claim, not proof of possession. Before enabling account-based control, verify key possession and bind discovery to the engine's pinned encrypted peer handshake and local permissions. Internet connections also need rendezvous/NAT traversal and an encrypted relay fallback.

## Transactional email

Emails are authored as typed React Email components in `src/emails/AccountEmail.tsx`. The shared layout includes the brand header, action button, expiry notice, and fallback link; verification and recovery supply their own copy. `src/lib/email.ts` validates links, renders HTML, derives plain text from the same HTML, and sends both through Resend.

Author colors, spacing, and typography with the React Email `Tailwind` wrapper and `pixelBasedPreset`. Its named theme colors live beside the template and match the brand palette. Rendering converts utility classes to inline styles; table components still provide email-compatible layout. Keep explicit responsive and dark-mode rules for progressive enhancement.

Light colors remain inline defaults. An optional `prefers-color-scheme: dark` stylesheet adds the dark palette in supporting clients; clients that ignore stylesheets keep the light layout. The preview theme selector includes light, dark, and an unsupported-client simulation with stylesheet rules removed. These browser previews do not simulate Gmail or Outlook's automatic color inversion; real inbox rendering still needs checking.

Run `npm run email:preview` and open `http://127.0.0.1:3002` to review both templates at desktop and mobile widths, including their plain-text versions. Edit the component and reload the page. This local preview uses dummy tokens and never sends email. To export a standalone preview, run `npx tsx scripts/email-preview.ts --output /tmp/extend-email-preview`.

Preview and visual checks share fixtures in `scripts/email-fixtures.ts`. Run `npm run test:email:visual` to compare both emails at 375px and 640px in light, dark, and unsupported-client modes against the 12 tracked PNGs in `tests/email-snapshots/`. Every changed pixel or missing baseline fails the command; failures write actual and diff images to the ignored `test-results/email-visual/` directory. No running server or email provider is required. Install the browser once with `npx playwright install chromium` if needed.

For an intentional design change, review the preview, then run `npm run test:email:visual:update` and review the changed PNGs. Baselines never update during a normal check. These baselines were captured on macOS; use the same OS and Playwright Chromium version for exact comparisons, since system fonts and rendering can differ. Email visual checks are an explicit local command, separate from `npm test` and account visual checks.

Cloudflare's free plan cannot send to arbitrary users. The optional Resend adapter works from the existing free Worker without upgrading Cloudflare. Keep Resend on its free tier with overages disabled. Sending remains disabled until both `RESEND_API_KEY` (a Worker secret) and `EMAIL_FROM` (a verified sender) exist. `PUBLIC_ORIGIN` must be the installation's exact HTTPS origin; localhost HTTP is accepted for development. Never put the API key in source or build variables.

The official installation's sender is `extend.computer <accounts@mail.extend.computer>`, with the Resend domain in Ireland. Verify the domain's DKIM and sending records, keep CNAME records DNS-only, and leave click/open tracking disabled for authentication links. Create a sending-only API key restricted to `mail.extend.computer` and store it as `RESEND_API_KEY` on the Worker. Self-hosted installations should use their own sender, domain, and key.

```sh
npx wrangler secret put RESEND_API_KEY
# Add EMAIL_FROM to wrangler.jsonc vars after verifying its domain in Resend.
npm run deploy
```

Signup requests a verification email when sending is enabled; the account dashboard can resend. Recovery accepts verified accounts only, returns the same response for unknown accounts and delivery failures, and sends in the background to avoid exposing account existence through provider latency. Links expire after 30 minutes, are stored as SHA-256 hashes, and are consumed once with DELETE RETURNING. They are bound to the account's password version so a password change invalidates old links. Tokens use URL fragments, are cleared from history on arrival, and require a form POST; mail scanners cannot consume them with GET requests.

A password reset signs out every login, invalidates device credentials, and clears other email links and registration challenges. Enabled two-factor authentication and its factors remain required; email recovery cannot bypass them. Passkeys are preserved regardless of whether two-factor authentication is enabled. Account/device records remain. Password changes and session creation use conditional database writes so a concurrent recovery cannot reissue a session from stale credentials.

Email requests have a global per-account one-minute cooldown. A single Durable Object enforces an installation-wide budget of 90 sends per UTC day and 2,700 per calendar month, including failures, below Resend's free limits. These limits never enable paid overages. `tests/email-api.mjs` uses local-only seeded link fixtures to verify expiry, replay, password-version binding, reset, and credential revocation without sending email. `tests/email.test.ts` verifies the payload and mocked provider transport.

Live verification and recovery can be exercised with Resend's designated `delivered+...@resend.dev` address: both messages reach the provider's simulated delivery event, both links work once, and recovery revokes the old session and password.

## Two-factor authentication

The account dashboard uses one **Add key** flow. After a fresh security check, `{ "auto": true }` registration requests discoverable credentials and user verification when available, without requiring either. A key supports passwordless sign-in only when registration verifies user verification and the browser reports a discoverable credential; otherwise it remains a second factor. Passwordless authentication always requires a verified assertion. Adding any key atomically enables two-factor authentication and issues recovery codes when it was off. Existing recovery codes remain valid when adding another key. Turning off passwordless sign-in retains second-factor protection; removing the last method turns it off. Older `{ "factor": true }` and default passwordless registration requests remain supported and also require fresh verification.

Removing the last enrolled verification method automatically disables 2FA and
invalidates recovery codes. The dashboard warns before this happens. Removing
one method while others remain keeps 2FA enabled.

The account dashboard supports six-digit TOTP authenticator codes, passkeys, USB/NFC security keys (including touch-only keys), and ten single-use eight-digit recovery codes. Enabling 2FA requires a fresh password check and proof of the enrolled factor. Save recovery codes when shown; only their hashes are stored. With 2FA enabled, sign in with your password followed by one enrolled factor or recovery code. Passkeys with fingerprint, face, or PIN verification also support direct sign-in with 2FA enabled. Touch-only security keys remain second factors after a password.

Security changes require a fresh password and, when enabled, a second factor. Adding factors and regenerating recovery codes preserve signed-in sessions. The dashboard offers an unchecked “Sign out other sessions” option when removing or replacing a factor, disabling 2FA, or changing a password. Selecting it revokes other sessions; leaving it unchecked preserves them. These hosted API endpoints accept a boolean `sign_out_others`; callers that omit it retain their previous behavior. Email password recovery still revokes all sessions. Security changes invalidate pending challenges and clear recent security verification. TOTP codes cannot be replayed. MFA tickets expire after five minutes, have bounded attempts, and cannot access accounts or devices.

Before deploying TOTP, generate a separate random 32-byte key encoded as 64 hexadecimal characters and store it as the Worker secret `MFA_ENCRYPTION_KEY` (`npx wrangler secret put MFA_ENCRYPTION_KEY`). Back up this key securely alongside the database: TOTP secrets use AES-256-GCM with account-bound authenticated encryption. Replacing the key without migrating encrypted secrets prevents existing authenticators from working. Passkey-only 2FA does not require this key.

The standalone server supports the same factors and ticket flow; see its README for configuration and API shapes. `npx playwright test tests/mfa.spec.mjs` verifies the UI in isolated Chromium; `tests/mfa-api.mjs` verifies enforcement, replay protection, and session revocation against local D1. `tests/passkey-mfa.spec.mjs` verifies direct passkey sign-in with 2FA, the password second-factor gate, and rejection of assertions without user verification. Unit tests cover RFC 6238 vectors and encryption tampering.

## Desktop browser sign-in

The app offers direct password login with TOTP/recovery codes and browser login for passkeys/security keys. Browser login opens `/desktop/connect` with an ephemeral loopback port, random state, and SHA-256 PKCE challenge. The user explicitly approves the signed-in account. The server issues a two-minute code bound to that browser session and challenge. `/v1/auth/desktop/exchange` consumes it only with the correct verifier, checks the source session, and issues a separate native session. Codes are stored as hashes and cannot be replayed. Account/session tokens never enter callback URLs or the desktop webview. After the desktop saves the session, it restores and focuses its main window. The loopback callback redirects to `/desktop/connected#close` with no-referrer/no-store headers and no login parameters. The completion page removes the close marker, attempts to close the tab, and retains a branded success view when the browser disallows closing.

`tests/desktop-login.spec.mjs` verifies the browser handoff, verifier binding, replay, and browser-session revocation using an isolated Chromium and local callback listener. Device registration uses the app's existing X25519 identity. Account presence is a directory hint; it never grants input-control permission or replaces pinned local pairing.

`EXTEND_TEST_URL=http://localhost:3006 node tests/session-choice-api.mjs` checks session preservation and explicit revocation on sensitive hosted API changes; repeat with `EXTEND_TEST_SIGN_OUT=true` for the disabling-2FA revocation case. `tests/session-choice.spec.mjs` checks the dashboard option and its unchecked default.

## Account UI styles and visual checks

Account, authentication, recovery, app handoff and error pages use Tailwind. Layout utilities live in the components; shared controls and contextual responsive rules use `@apply` recipes in `src/styles.css`. Theme colors are exposed as `app-*` tokens, so light/dark/system selection applies to both utilities and recipes. Custom CSS remains for motion, SVG masks, browser selection behavior and the landing page.

`tests/account-visual.spec.mjs` captures 286 views: 390, 768 and 1440px widths, both themes, account states, authentication/recovery pages, app handoff, dialogs, tooltips, focus/hover, copy/download feedback and reduced-motion illustrations. State fixtures live only in `tests/account-fixtures.mjs`; dialogs and pending forms are reached through real interactions, without application preview switches. It uses isolated browser contexts and mocked API responses; it never changes a real account or sends email. Dates, locale, fonts, transitions and GIF frames are held stable. Behavioral browser tests separately check interaction and animation lifecycle.

On Linux arm64 in the pinned container below, Playwright starts and stops the development server automatically:

```sh
npm run test:visual
# Only after reviewing an intentional design change:
npm run test:visual:baseline
```

Baselines are tracked in `tests/account-snapshots`; normal checks never create or update them. Review changed PNGs before accepting an intentional design update. `EXTEND_VISUAL_FILTER` selects case names using a regular expression. CI runs the suite once on Ubuntu arm64 in the pinned Playwright Noble container, matching the browser, fonts and architecture used for capture. Account application code is platform-independent; only the visual comparison environment is fixed.

On macOS or another platform, use the same Linux environment from the `web` directory. Give the Docker VM 8 GiB of memory for the development server and parallel browsers:

```sh
docker build --platform linux/arm64 -f Dockerfile.playwright -t extend-web-tests .
docker run --rm --init --ipc=host extend-web-tests
```

For an intentional baseline update, capture inside the container and copy the reviewed PNGs back before rebuilding the image to verify them:

```sh
docker run --name extend-web-baselines --init --ipc=host extend-web-tests npm run test:visual:baseline
docker cp extend-web-baselines:/workspace/tests/account-snapshots/. tests/account-snapshots/
docker rm -v extend-web-baselines
```

The Docker build excludes local secrets, dependencies and generated artifacts. It uses its own dependencies and development server; no personal browser or running local application is involved. Ordinary unit/UI tests still run directly on macOS. CI installs the repository's Node version; the local image supplies a compatible Node 24 version.

The Playwright HTML report includes pixel counts and expected/actual/diff attachments for failures. Any size change or pixel channel difference greater than 8/255 fails. Smaller differences are recorded separately without assuming their cause. This tolerance permits tiny color differences, without allowing a percentage of the page to change unnoticed.

Chromium uses software rendering and grayscale text in tests to keep the paint path consistent across Docker and CI. Clipboard feedback captures grant clipboard permission and wait for the success label; download captures also wait for their success label.

Account data uses TanStack Query with one QueryClient per router/SSR request. Account, devices, credentials, MFA, sessions, and server settings have shared query definitions in `src/lib/query.ts`. Browser-only queries preserve the existing SSR loading shell. Data stays fresh for 30 seconds; retries, focus refetch and reconnect refetch are disabled to preserve explicit retry and offline feedback. Mutations invalidate only affected resources. Sign-in, sign-out, account switching and password recovery clear the session cache; passwords, tickets and issued codes are reset from mutation state immediately after each request. No cache is persisted to storage.

`src/lib/contracts.ts` defines typed request/response schemas for the browser API. The transport validates both sides and reports malformed responses without exposing their contents. Domain mutation hooks in `src/lib/mutations.ts` own browser prompts, requests and affected-query invalidation; pending state covers the whole workflow, including refresh.

### Benchmark results page

The local `/benchmarks` route presents the reviewed snapshot in
`src/lib/benchmarks-data.json`. Run `npm run dev` and open
`http://127.0.0.1:3001/benchmarks`. The summary contains sanitized measurements,
hardware/build context, scope limitations and report hashes; it contains no raw
recordings, device addresses, credentials, private paths or third-party adapters.
Keep private instrumentation and research tooling outside this website checkout.

The responsiveness index is relative to the products in the selected matched
scene/repeat set, with equal weights for median and p95 latency. It is explicitly
provisional when comparison context is incomplete. Overall scores remain absent
until all required categories and comparison gates have valid evidence. Missing
categories must never be silently removed or reweighted. Update the snapshot
only from reviewed reports, retaining original build labels, units, method,
repeat count, limitations and provenance. The data download includes the selected
scene and scoring version so its displayed index can be reproduced.

Scoring checks: `node --import tsx --test tests/benchmark-scoring.test.ts`.
