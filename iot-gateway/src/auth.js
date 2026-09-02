const TOKEN_PREFIX = "iot_";
const MAX_TOKEN_LENGTH = 128;

const DEVICE_LOOKUP_SQL = `
  SELECT id, enabled, max_input_chars, max_output_tokens
  FROM devices
  WHERE token_hash = ?1 OR pending_token_hash = ?1
  LIMIT 1
`;

/**
 * Extract a constrained bearer token without logging or returning it in errors.
 *
 * @param {string | null} authorization
 * @returns {string | null}
 */
export function parseBearerToken(authorization) {
  if (!authorization) return null;

  const match = /^Bearer ([A-Za-z0-9_-]+)$/.exec(authorization);
  if (!match) return null;

  const token = match[1];
  if (!token.startsWith(TOKEN_PREFIX) || token.length > MAX_TOKEN_LENGTH) {
    return null;
  }

  return token;
}

/**
 * @param {string} value
 * @returns {Promise<string>}
 */
export async function sha256Hex(value) {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest("SHA-256", bytes);

  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

/**
 * Authenticate an IoT device by looking up the SHA-256 hash of its random token.
 * D1 stores only the hash, never the bearer token itself.
 *
 * @param {Request} request
 * @param {{ prepare: Function }} database
 * @returns {Promise<null | {
 *   id: string,
 *   maxInputChars: number,
 *   maxOutputTokens: number
 * }>}
 */
export async function authenticateDevice(request, database) {
  const token = parseBearerToken(request.headers.get("authorization"));
  if (!token) return null;

  const tokenHash = await sha256Hex(token);
  const row = await database
    .prepare(DEVICE_LOOKUP_SQL)
    .bind(tokenHash)
    .first();

  if (!row || Number(row.enabled) !== 1) return null;

  return {
    id: String(row.id),
    maxInputChars: Number(row.max_input_chars),
    maxOutputTokens: Number(row.max_output_tokens),
  };
}
