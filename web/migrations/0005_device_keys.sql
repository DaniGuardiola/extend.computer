ALTER TABLE devices ADD COLUMN public_key TEXT;
ALTER TABLE device_sessions ADD COLUMN key_verified INTEGER NOT NULL DEFAULT 0;
CREATE TABLE device_proofs (
    token_hash TEXT PRIMARY KEY REFERENCES device_sessions(token_hash) ON DELETE CASCADE,
    challenge TEXT NOT NULL,
    expected TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
