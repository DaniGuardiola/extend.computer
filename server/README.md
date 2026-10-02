# Account server

This is the standalone service for independent installations. The official website uses Cloudflare Workers with a separate database and matching account/device API. It stores accounts and an account-private device directory. The desktop app does not use it yet; local pairing and control continue to work without an account.

## Run

```sh
EXTEND_ORIGIN=http://localhost:8080 EXTEND_SIGNUP_ENABLED=true cargo run --manifest-path server/Cargo.toml
```

The default address is `127.0.0.1:8080`. Set `EXTEND_BIND` to change it and `EXTEND_DATABASE` to choose the SQLite file. Signup is disabled unless `EXTEND_SIGNUP_ENABLED=true`; disable it again after creating accounts on a private installation. Existing accounts can still log in.

For Docker, from this directory:

```sh
EXTEND_SIGNUP_ENABLED=true docker compose up --build -d
```

Compose binds to loopback and saves SQLite in a named volume. Put an HTTPS reverse proxy in front before using credentials over a network. The image runs as an unprivileged user. Back up SQLite with its online backup API or stop the service before copying the database; copying an active WAL database file alone can lose recent changes. Keep the volume when upgrading.

## API v1

Requests and responses use JSON. Times are Unix seconds. Account and device tokens use `Authorization: Bearer <token>`. Tokens are returned only when created and must be stored in the desktop credential store, scoped to the server URL. Responses use `Cache-Control: no-store`.

| Method | Path | Credential | Behavior |
| --- | --- | --- | --- |
| GET | `/healthz` | None | Process health |
| GET | `/v1/server` | None | API version, signup policy, heartbeat timing |
| POST | `/v1/auth/signup` | None | Create account and login session |
| POST | `/v1/auth/login` | None | Create login session |
| GET | `/v1/account` | Account token | Current account ID and email |
| POST | `/v1/auth/logout` | Account token | Revoke this login and its device credentials |
| GET | `/v1/devices` | Account token | This account's registered devices and presence |
| POST | `/v1/devices` | Account token | Register or update device; issue device token |
| POST | `/v1/devices/{id}/heartbeat` | Device token | Refresh this device's presence |
| DELETE | `/v1/devices/{id}` | Account token | Remove device and revoke all its device credentials |

Signup and login accept `{"email":"you@example.com","password":"your long password"}`. Passwords must be 12–1024 bytes. Email addresses are trimmed and normalized to ASCII lowercase; email ownership is not verified in this first slice.

Device registration accepts `{"public_key":"<64 hex characters>","name":"My Mac","platform":"macos"}`. Use the engine's existing 32-byte X25519 public key. The fingerprint is SHA-256 of its decoded bytes, matching the local engine. Supported platforms are `macos`, `windows`, and `linux`.

Registration returns `id`, `fingerprint`, and `device_token`. Re-registering the same key on the same account keeps the device ID and replaces the heartbeat token for that login session. A different login gets its own heartbeat credential. Send heartbeats every 30 seconds; a device becomes offline after 90 seconds without a heartbeat. Registration alone does not mark it online. Logging out invalidates that login's heartbeat credentials immediately; other logged-in sessions remain valid. Device registration persists while offline. Expired login sessions and their device credentials are removed on the next database operation.

## Passkeys

Set `EXTEND_ORIGIN` to the exact external HTTPS origin (for example `https://accounts.example.com`). Local development permits `http://localhost:8080`. Paths, credentials, insecure remote origins, subdomains, and alternate ports are not accepted. This setting is explicit; Host and proxy headers cannot choose a relying party. Without it, password accounts still work and passkey ceremonies return 503.

| Method | Path | Credential | Behavior |
| --- | --- | --- | --- |
| POST | `/v1/passkeys/register/options` | Account token | Browser registration options |
| POST | `/v1/passkeys/register/verify` | Same account token/session | Verify browser registration response |
| POST | `/v1/passkeys/login/options` | None | Discoverable authentication options |
| POST | `/v1/passkeys/login/verify` | None | Verify assertion and issue account token |
| GET | `/v1/passkeys` | Account token | List this account's passkeys |
| DELETE | `/v1/passkeys/{id}` | Account token | Remove a passkey |

Ceremony POSTs require `Origin: <EXTEND_ORIGIN>`. Options match SimpleWebAuthn's flat JSON shapes. Send the browser's credential response as JSON to the verification endpoint. Options responses set an HttpOnly challenge cookie; native clients can instead return the response's `X-Extend-Challenge` header. Registration is bound to the exact account session that started it. Signup with a password first, then add up to 10 passkeys. Successful passkey login returns the same bearer-session shape as password login.

`webauthn-rs` verifies origin, relying party, challenge, signature, user presence and user verification. Resident credentials are requested for passwordless account selection. Counters are checked and updated under the database lock. Keys persist in SQLite; challenges stay only in bounded server memory, expire after five minutes, and are consumed once. Restarting invalidates in-flight ceremonies while enrolled keys survive. Keys are installation-specific; the Cloudflare and Rust databases remain separate. This is an API port, not a bundled standalone website.

The browser integration test uses the website's Playwright development dependency and an isolated Chromium with a virtual authenticator:

```sh
EXTEND_TEST_URL=http://localhost:8081 EXTEND_TEST_CDP=<isolated-browser-websocket> node server/tests/passkey-browser.mjs
```

## Trust and hosting

Passwords use Argon2id. Random 256-bit account and device tokens are stored as SHA-256 hashes. Login sessions expire after 30 days. Device tokens can only heartbeat their own device; they cannot read the directory or manage accounts. Every directory operation is scoped to an authenticated account. Database queries run outside the async executor. Password hashing has a two-worker bound; login/signup requests have a 20-per-minute socket-IP limit and a 16 KiB body limit. Accounts have limits of 100 active sessions and 100 registered devices.

Forwarded IP headers are ignored. Behind a reverse proxy, the application rate limit is shared by requests from that proxy. Configure per-client limits at the proxy before public hosting. SQLite supports one server instance with a local persistent disk; multiple replicas and distributed rate limits need a later storage change. Email verification, account recovery, richer session management, and distributed abuse controls remain separate work for the standalone server.

The registry records a public-key claim; it does not prove possession of that device's private key. Account membership and presence must not silently become permission to control a computer. No private device keys, trust grants, input events, or screen contents are uploaded. Directory removal currently revokes server access only; it does not terminate an existing local control session or remove local pairing.

## Desktop integration next

Add server selection with the official URL as default and a custom HTTPS URL for self-hosting. Account credentials and device records belong to that server; switching servers must not reuse credentials. Keep account-free local operation available.

After login, register the local engine identity, heartbeat while signed in, and display the account's other devices. Before enabling connection from those entries, implement device-key possession verification and bind account discovery to the engine's pinned encrypted peer handshake and local control permissions. Internet connections also need rendezvous, NAT traversal, and encrypted relay fallback. This server does not publish connection addresses or implement those routes yet.

## Verify

```sh
cargo test --manifest-path server/Cargo.toml
cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings
```
