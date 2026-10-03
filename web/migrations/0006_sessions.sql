ALTER TABLE sessions ADD COLUMN id TEXT;
ALTER TABLE sessions ADD COLUMN last_access_at INTEGER;
ALTER TABLE sessions ADD COLUMN platform TEXT;
ALTER TABLE sessions ADD COLUMN client TEXT;
ALTER TABLE sessions ADD COLUMN location TEXT;
UPDATE sessions SET id=lower(hex(randomblob(16))), last_access_at=created_at;
CREATE UNIQUE INDEX sessions_id ON sessions(id);
