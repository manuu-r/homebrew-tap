-- Short-term conversation memory for the live voice endpoint.
--
-- Rows are scoped by (device_id, session_id). The device mints session_id once
-- per boot from esp_random(), so it survives the ~5-minute reconnect the
-- gateway forces at LIVE_MAX_SESSION_SECONDS but is replaced on any reflash or
-- power cycle. A new session_id for a device retires that device's earlier
-- rows, so history never leaks across firmware builds.

CREATE TABLE IF NOT EXISTS conversation_turns (
  device_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  turn_index INTEGER NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
  content TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (device_id, session_id, turn_index, role)
);

CREATE INDEX IF NOT EXISTS conversation_turns_recent_idx
  ON conversation_turns (device_id, session_id, turn_index);
