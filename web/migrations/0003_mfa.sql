ALTER TABLE accounts ADD COLUMN mfa_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE accounts ADD COLUMN mfa_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN mfa_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN elevated_until INTEGER NOT NULL DEFAULT 0;
ALTER TABLE passkeys ADD COLUMN purpose TEXT NOT NULL DEFAULT 'login';
CREATE TABLE totp_factors (
 account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
 secret TEXT NOT NULL, last_step INTEGER NOT NULL DEFAULT -1
);
CREATE TABLE totp_setup (
 session_hash TEXT PRIMARY KEY REFERENCES sessions(token_hash) ON DELETE CASCADE,
 account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
 secret TEXT NOT NULL, version INTEGER NOT NULL, expires_at INTEGER NOT NULL
);
CREATE TABLE recovery_codes (
 account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
 code_hash TEXT NOT NULL, PRIMARY KEY(account_id, code_hash)
);
CREATE TABLE mfa_tickets (
 id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
 password_version TEXT NOT NULL, version INTEGER NOT NULL, purpose TEXT NOT NULL,
 session_hash TEXT REFERENCES sessions(token_hash) ON DELETE CASCADE,
 expires_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX mfa_tickets_expiry ON mfa_tickets(expires_at);
CREATE TABLE mfa_attempts (
 account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
 count INTEGER NOT NULL, until INTEGER NOT NULL
);
