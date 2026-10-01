CREATE TABLE accounts (
  id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE sessions (
  token_hash TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL
);
CREATE INDEX sessions_account ON sessions(account_id);
CREATE INDEX sessions_expiry ON sessions(expires_at);
CREATE TABLE devices (
  id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  fingerprint TEXT NOT NULL, name TEXT NOT NULL, platform TEXT NOT NULL, created_at INTEGER NOT NULL,
  UNIQUE(account_id, fingerprint)
);
CREATE TABLE device_sessions (
  token_hash TEXT PRIMARY KEY, session_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
  device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE, last_seen INTEGER,
  UNIQUE(session_hash, device_id)
);
CREATE INDEX device_sessions_device ON device_sessions(device_id);
CREATE TABLE passkeys (
  id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  public_key TEXT NOT NULL, counter INTEGER NOT NULL, transports TEXT NOT NULL, name TEXT NOT NULL,
  rp_id TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE INDEX passkeys_account ON passkeys(account_id);
CREATE TABLE challenges (
  id TEXT PRIMARY KEY, challenge TEXT NOT NULL, account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, origin TEXT NOT NULL, expires_at INTEGER NOT NULL
);
CREATE INDEX challenges_expiry ON challenges(expires_at);
