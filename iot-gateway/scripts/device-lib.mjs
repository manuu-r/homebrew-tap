import { createHash, randomBytes } from "node:crypto";

const DEVICE_ID_PATTERN = /^[A-Za-z0-9._:-]{1,64}$/;

export function validateDeviceId(deviceId) {
  if (!DEVICE_ID_PATTERN.test(deviceId || "")) {
    throw new Error(
      "Device ID must be 1-64 letters, digits, dots, underscores, colons, or hyphens.",
    );
  }
  return deviceId;
}

export function generateDeviceToken() {
  return `iot_${randomBytes(32).toString("base64url")}`;
}

export function hashDeviceToken(token) {
  return createHash("sha256").update(token, "utf8").digest("hex");
}

export function provisionSql(deviceId, tokenHash) {
  validateDeviceId(deviceId);
  validateTokenHash(tokenHash);

  return [
    "INSERT INTO devices (id, token_hash, pending_token_hash, enabled, updated_at)",
    `VALUES ('${deviceId}', '${tokenHash}', NULL, 1, CURRENT_TIMESTAMP)`,
    "ON CONFLICT(id) DO UPDATE SET",
    "token_hash = excluded.token_hash,",
    "pending_token_hash = NULL,",
    "enabled = 1,",
    "updated_at = CURRENT_TIMESTAMP;",
  ].join(" ");
}

export function stageSql(deviceId, tokenHash) {
  validateDeviceId(deviceId);
  validateTokenHash(tokenHash);

  return [
    "INSERT INTO devices (id, token_hash, pending_token_hash, enabled, updated_at)",
    `VALUES ('${deviceId}', '${tokenHash}', '${tokenHash}', 1, CURRENT_TIMESTAMP)`,
    "ON CONFLICT(id) DO UPDATE SET",
    "pending_token_hash = excluded.pending_token_hash,",
    "enabled = 1,",
    "updated_at = CURRENT_TIMESTAMP;",
  ].join(" ");
}

export function activateSql(deviceId) {
  validateDeviceId(deviceId);
  return [
    "UPDATE devices",
    "SET token_hash = pending_token_hash,",
    "pending_token_hash = NULL,",
    "updated_at = CURRENT_TIMESTAMP",
    `WHERE id = '${deviceId}' AND pending_token_hash IS NOT NULL;`,
  ].join(" ");
}

export function enabledSql(deviceId, enabled) {
  validateDeviceId(deviceId);
  return `UPDATE devices SET enabled = ${enabled ? 1 : 0}, updated_at = CURRENT_TIMESTAMP WHERE id = '${deviceId}';`;
}

export function listSql() {
  return "SELECT id, enabled, pending_token_hash IS NOT NULL AS token_staged, max_input_chars, max_output_tokens, created_at, updated_at FROM devices ORDER BY id;";
}

function validateTokenHash(tokenHash) {
  if (!/^[a-f0-9]{64}$/.test(tokenHash)) {
    throw new Error("Token hash must be a lowercase SHA-256 hex digest.");
  }
}
