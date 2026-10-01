# Account server

The same server supports the official extend.computer service and independent installations. It stores accounts and an account-private device directory. The desktop app does not use it yet; local pairing and control continue to work without an account.

## Run

```sh
EXTEND_SIGNUP_ENABLED=true cargo run --manifest-path server/Cargo.toml
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

## Trust and hosting

Passwords use Argon2id. Random 256-bit account and device tokens are stored as SHA-256 hashes. Login sessions expire after 30 days. Device tokens can only heartbeat their own device; they cannot read the directory or manage accounts. Every directory operation is scoped to an authenticated account. Database queries run outside the async executor. Password hashing has a two-worker bound; login/signup requests have a 20-per-minute socket-IP limit and a 16 KiB body limit. Accounts have limits of 100 active sessions and 100 registered devices.

Forwarded IP headers are ignored. Behind a reverse proxy, the application rate limit is shared by requests from that proxy. Configure per-client limits at the proxy before public hosting. SQLite supports one server instance with a local persistent disk; multiple replicas and distributed rate limits need a later storage change. Signup, email verification, account recovery, session management, abuse controls, and deployment operations need more work before opening an official public service.

The registry records a public-key claim; it does not prove possession of that device's private key. Account membership and presence must not silently become permission to control a computer. No private device keys, trust grants, input events, or screen contents are uploaded. Directory removal currently revokes server access only; it does not terminate an existing local control session or remove local pairing.

## Desktop integration next

Add server selection with the official URL as default and a custom HTTPS URL for self-hosting. Account credentials and device records belong to that server; switching servers must not reuse credentials. Keep account-free local operation available.

After login, register the local engine identity, heartbeat while signed in, and display the account's other devices. Before enabling connection from those entries, implement device-key possession verification and bind account discovery to the engine's pinned encrypted peer handshake and local control permissions. Internet connections also need rendezvous, NAT traversal, and encrypted relay fallback. This server does not publish connection addresses or implement those routes yet.

## Verify

```sh
cargo test --manifest-path server/Cargo.toml
cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings
```
