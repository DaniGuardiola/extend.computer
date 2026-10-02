ALTER TABLE accounts ADD COLUMN email_verified INTEGER NOT NULL DEFAULT 0;
CREATE TABLE email_tokens (
  token_hash TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK(kind IN ('verify', 'reset')),
  password_version TEXT NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX email_tokens_expiry ON email_tokens(expires_at);
CREATE TABLE email_cooldowns (
  account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
  next_send INTEGER NOT NULL
);
