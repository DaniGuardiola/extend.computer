# Custom account servers

[← Releases and updates](releases.md) · [Index](../README.md) · [Index →](../README.md)

An account is optional. Pairing, keyboard and mouse sharing, and display
sharing all work without one. Signing in adds two things: the devices in your
account trust each other without pairing codes, and each app can list your
devices and show which are online. The apps use the official server at
`https://extend.computer` unless you choose another one.

A custom account server is a server you run yourself. Accounts, device names,
public device keys and presence then stay on your own machine, and nobody else
can add devices to your account.

## What the server does

- Stores accounts (email and password hash, optional two-factor
  authentication) and each account's device directory: device name, platform
  and public key.
- Checks that each device holds the private key it registered, using a one-use
  challenge. Private keys never leave the device.
- Tracks presence: signed-in apps send a heartbeat every 30 seconds, and a
  device shows as offline after 90 seconds without one.
- Optionally relays keyboard and mouse connections between devices in the same
  account when they can't reach each other directly. The relay is off by
  default. It forwards the devices' own end-to-end encrypted connection and
  can't read input. Display sessions never use it.

Connections themselves go directly between devices over the local network or a
USB cable. The server is not involved in screen or input data unless the relay
is on and the devices can only reach each other through it.

> [!IMPORTANT]
> Only use an account server you trust. The server decides which devices belong
> to your account, and devices in your account connect to each other without an
> approval prompt while **Allow connections** is on. A server operator can add
> a device to your account.

## Choose a server

| Option | What you get | Guide |
| --- | --- | --- |
| Standalone server ([`server/`](../../server/)) | One Rust binary with SQLite, or a Docker container. An account API without a website. | This page and the [server README](../../server/README.md) |
| A separate deployment of the website ([`web/`](../../web/)) | The same Cloudflare Worker as the official service: sign-up, browser sign-in, passkeys, email verification and recovery, account settings. Needs a Cloudflare account and its own database. | [Website deploy](../../web/README.md#deploy) |

The rest of this page covers the standalone server.

## Run the standalone server

You need Docker with Compose, a domain name, and an HTTPS reverse proxy. From
`server/` in a checkout of this repository:

```sh
export EXTEND_ORIGIN=https://accounts.example.com
export EXTEND_MFA_ENCRYPTION_KEY=$(openssl rand -hex 32)
EXTEND_SIGNUP_ENABLED=true docker compose up --build -d
```

[`compose.yaml`](../../server/compose.yaml) builds the
[`Dockerfile`](../../server/Dockerfile), runs the server as an unprivileged
user, publishes it on `127.0.0.1:8080` only, and keeps the SQLite database in
the `accounts` volume. It passes these settings through:

| Variable | Default | Meaning |
| --- | --- | --- |
| `EXTEND_ORIGIN` | `http://localhost:8080` | The exact public HTTPS origin, without a path. Passkey requests need it. |
| `EXTEND_SIGNUP_ENABLED` | `false` | Allows new accounts. Turn it off again once your accounts exist. |
| `EXTEND_MFA_ENCRYPTION_KEY` | Empty | 64 hexadecimal characters. Needed before anyone sets up an authenticator app. Keep it stable and back it up with the database. |
| `EXTEND_RELAY_ENABLED` | `false` | Turns on the keyboard and mouse relay. |

Without Docker, build and run the binary directly:

```sh
EXTEND_ORIGIN=https://accounts.example.com EXTEND_SIGNUP_ENABLED=true \
  cargo run --release --manifest-path server/Cargo.toml
```

It listens on `127.0.0.1:8080` (set `EXTEND_BIND` to change it) and stores data
in `extend.sqlite3` in the working directory (set `EXTEND_DATABASE`).

### HTTPS and the reverse proxy

The apps only accept HTTPS servers, except `localhost` for development. Put a
reverse proxy with a certificate in front of port 8080, and let it forward
WebSocket upgrades for `/v1/relay/*` if you turn on the relay. With
[Caddy](https://caddyserver.com/), which handles certificates and WebSockets by
itself, this is the whole configuration:

```
accounts.example.com {
	reverse_proxy 127.0.0.1:8080
}
```

The server ignores forwarded IP headers, so its sign-in limit (20 attempts a
minute per address) applies to all traffic from the proxy together. Add
per-client rate limits at the proxy before opening the server to the internet.

### Create accounts

The standalone server has no website, so create accounts through its API while
sign-up is on:

```sh
curl https://accounts.example.com/v1/auth/signup \
  -H 'Content-Type: application/json' \
  -d '{"email":"you@example.com","password":"at least twelve characters"}'
```

Then set `EXTEND_SIGNUP_ENABLED=false` and restart. Existing accounts can still
sign in. The response contains a session token for that account; it expires
after 30 days without use.

### Back up and upgrade

Back up the database with SQLite's online backup (for example
`sqlite3 extend.sqlite3 ".backup backup.sqlite3"`), or stop the server before
copying the file. Copying a live database file alone can lose recent changes.
Keep the `EXTEND_MFA_ENCRYPTION_KEY` with the backup.

To upgrade, update your checkout and run `docker compose up --build -d` again.
Keep the volume.

## Point the apps at your server

**Desktop.** Click **Log in** (or open **Settings**), check **Use a custom
account server**, and enter the server's address, such as
`accounts.example.com`. A bare host name means HTTPS. Then sign in with your
email and password. The app remembers the server; to go back to the official
one, sign out and uncheck the option.

**Phones and tablets.** On the account screen, tap **Use another server**,
enter the server's address and tap **Save server**, then sign in.

Sign in to the same server on every device you want to connect. Signed-in
devices keep their login in the system credential store, scoped to that server.

## What works with the standalone server

| Works | Doesn't work yet |
| --- | --- |
| Signing in with email and password in the apps | **Create an account** and **Manage account & security** in the apps: they open a website, which this server doesn't have |
| Authenticator app codes and recovery codes at sign-in, once two-factor authentication is set up through the API | Browser sign-in and passkey sign-in in the apps (passkeys work through the API with a website of your own) |
| Devices in the account connecting without pairing codes | Email verification and password recovery |
| Device list, presence, removing devices, signing out | Changing the email or password, deleting an account, exporting data |
| The keyboard and mouse relay, when turned on | Display sessions over the internet: they need the local network or a USB cable |
| One server with a local SQLite disk | Several replicas behind a load balancer |

Accounts on different servers are separate. Moving to another server means
creating the account there and signing in again on each device. Devices you
paired directly stay paired whichever server you use, or with none.

For the full API and security details, see the
[server README](../../server/README.md).

---

[← Releases and updates](releases.md) · [Index](../README.md) · [Index →](../README.md)
