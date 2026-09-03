// Short-term conversation memory for the live voice endpoint.
//
// One narrow job: given a device and its per-boot session id, return the
// recent turns as Claude `messages`, and append each completed turn. Storage
// is D1 so history survives the reconnect the gateway forces every
// LIVE_MAX_SESSION_SECONDS. A session id the device has not used before
// retires that device's older rows, which is how a reflash starts clean.
//
// Every call is best-effort. A D1 error must never break a voice turn, so
// failures are swallowed and the turn proceeds without memory.

const SESSION_ID_PATTERN = /^[A-Za-z0-9]{8,64}$/;

/** @param {string | null | undefined} value */
export function normalizeSessionId(value) {
  if (typeof value !== "string") return null;
  return SESSION_ID_PATTERN.test(value) ? value : null;
}

/**
 * @param {{ DEVICES_DB?: any }} env
 * @param {string} deviceId
 * @param {string} sessionId
 */
export async function retireOldSessions(env, deviceId, sessionId) {
  if (!env?.DEVICES_DB || !sessionId) return;
  try {
    await env.DEVICES_DB.prepare(
      "DELETE FROM conversation_turns WHERE device_id = ?1 AND session_id != ?2",
    )
      .bind(deviceId, sessionId)
      .run();
  } catch {
    // A stale row is harmless; it is scoped to a session id no live device
    // will present again.
  }
}

/**
 * Recent turns as Claude messages, oldest first, user/assistant interleaved.
 *
 * @param {{ DEVICES_DB?: any }} env
 * @param {string} deviceId
 * @param {string} sessionId
 * @param {number} maxTurns One turn is a user message plus its assistant reply.
 * @returns {Promise<Array<{ role: "user" | "assistant", content: string }>>}
 */
export async function loadHistory(env, deviceId, sessionId, maxTurns) {
  if (!env?.DEVICES_DB || !sessionId || maxTurns < 1) return [];
  try {
    const result = await env.DEVICES_DB.prepare(
      `SELECT turn_index, role, content
         FROM conversation_turns
        WHERE device_id = ?1 AND session_id = ?2
        ORDER BY turn_index DESC, role ASC
        LIMIT ?3`,
    )
      .bind(deviceId, sessionId, maxTurns * 2)
      .all();

    const rows = result?.results ?? [];
    // Group by turn and emit only complete user+assistant pairs, oldest first.
    // Claude requires strictly alternating roles, so a half-turn left by the
    // LIMIT (or a partially written turn) is dropped rather than passed on.
    const byTurn = new Map();
    for (const row of rows) {
      const key = Number(row.turn_index);
      const turn = byTurn.get(key) || {};
      if (row.role === "assistant") turn.assistant = String(row.content ?? "");
      else turn.user = String(row.content ?? "");
      byTurn.set(key, turn);
    }

    const messages = [];
    for (const key of [...byTurn.keys()].sort((a, b) => a - b)) {
      const turn = byTurn.get(key);
      if (!turn.user || !turn.assistant) continue;
      messages.push({ role: "user", content: turn.user });
      messages.push({ role: "assistant", content: turn.assistant });
    }
    return messages;
  } catch {
    return [];
  }
}

/**
 * Append one completed turn, then trim the session to its most recent turns.
 *
 * @param {{ DEVICES_DB?: any }} env
 * @param {string} deviceId
 * @param {string} sessionId
 * @param {number} turnIndex Monotonic within a session; used only for ordering.
 * @param {string} userText
 * @param {string} assistantText
 * @param {number} keepTurns
 */
export async function appendTurn(
  env,
  deviceId,
  sessionId,
  turnIndex,
  userText,
  assistantText,
  keepTurns,
) {
  if (!env?.DEVICES_DB || !sessionId) return;
  if (!userText || !assistantText) return;

  const index = Number.isFinite(turnIndex) ? Math.trunc(turnIndex) : 0;
  try {
    await env.DEVICES_DB.batch([
      env.DEVICES_DB.prepare(
        `INSERT INTO conversation_turns
           (device_id, session_id, turn_index, role, content)
         VALUES (?1, ?2, ?3, 'user', ?4)
         ON CONFLICT (device_id, session_id, turn_index, role)
         DO UPDATE SET content = excluded.content`,
      ).bind(deviceId, sessionId, index, userText),
      env.DEVICES_DB.prepare(
        `INSERT INTO conversation_turns
           (device_id, session_id, turn_index, role, content)
         VALUES (?1, ?2, ?3, 'assistant', ?4)
         ON CONFLICT (device_id, session_id, turn_index, role)
         DO UPDATE SET content = excluded.content`,
      ).bind(deviceId, sessionId, index, assistantText),
      // Keep the newest keepTurns turns; delete anything older by turn_index.
      env.DEVICES_DB.prepare(
        `DELETE FROM conversation_turns
          WHERE device_id = ?1 AND session_id = ?2
            AND turn_index <= (
              SELECT MAX(turn_index) - ?3
                FROM conversation_turns
               WHERE device_id = ?1 AND session_id = ?2
            )`,
      ).bind(deviceId, sessionId, Math.max(1, keepTurns)),
    ]);
  } catch {
    // Losing a turn only shortens memory; the conversation still works.
  }
}
