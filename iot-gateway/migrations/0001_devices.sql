CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  pending_token_hash TEXT,
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  max_input_chars INTEGER NOT NULL DEFAULT 2000 CHECK (max_input_chars BETWEEN 1 AND 8000),
  max_output_tokens INTEGER NOT NULL DEFAULT 256 CHECK (max_output_tokens BETWEEN 1 AND 4096),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX IF NOT EXISTS devices_token_hash_idx
  ON devices (token_hash);

CREATE UNIQUE INDEX IF NOT EXISTS devices_pending_token_hash_idx
  ON devices (pending_token_hash)
  WHERE pending_token_hash IS NOT NULL;
