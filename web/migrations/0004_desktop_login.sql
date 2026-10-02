CREATE TABLE desktop_codes (
 id TEXT PRIMARY KEY,
 session_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
 challenge TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE INDEX desktop_codes_expiry ON desktop_codes(expires_at);
