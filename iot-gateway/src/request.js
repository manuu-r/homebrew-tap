const ALLOWED_FIELDS = new Set([
  "input",
  "context",
  "max_output_tokens",
  "request_id",
]);

const REQUEST_ID_PATTERN = /^[A-Za-z0-9._:-]{1,64}$/;

export class RequestValidationError extends Error {
  /**
   * @param {string} code
   * @param {string} message
   * @param {number} [status]
   */
  constructor(code, message, status = 400) {
    super(message);
    this.name = "RequestValidationError";
    this.code = code;
    this.status = status;
  }
}

/**
 * Read and validate the intentionally small public request schema.
 * Model, system prompt, tools, and provider headers are never accepted from a device.
 *
 * @param {Request} request
 * @param {{
 *   maxRequestBytes: number,
 *   maxInputChars: number,
 *   defaultMaxOutputTokens: number,
 *   maxOutputTokens: number
 * }} limits
 */
export async function readInferenceRequest(request, limits) {
  const contentType = request.headers.get("content-type") || "";
  if (!contentType.toLowerCase().startsWith("application/json")) {
    throw new RequestValidationError(
      "unsupported_media_type",
      "Content-Type must be application/json.",
      415,
    );
  }

  const declaredLength = Number(request.headers.get("content-length"));
  if (Number.isFinite(declaredLength) && declaredLength > limits.maxRequestBytes) {
    throw new RequestValidationError(
      "request_too_large",
      "Request body is too large.",
      413,
    );
  }

  const rawBody = await readBodyWithLimit(request, limits.maxRequestBytes);

  let body;
  try {
    body = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(rawBody));
  } catch {
    throw new RequestValidationError("invalid_json", "Request body is not valid JSON.");
  }

  if (!isPlainObject(body)) {
    throw new RequestValidationError("invalid_body", "Request body must be a JSON object.");
  }

  const unknownFields = Object.keys(body).filter((key) => !ALLOWED_FIELDS.has(key));
  if (unknownFields.length > 0) {
    throw new RequestValidationError(
      "unknown_fields",
      `Unknown request field${unknownFields.length === 1 ? "" : "s"}: ${unknownFields.join(", ")}.`,
    );
  }

  if (typeof body.input !== "string" || body.input.trim().length === 0) {
    throw new RequestValidationError(
      "invalid_input",
      "input must be a non-empty string.",
    );
  }

  if (body.input.length > limits.maxInputChars) {
    throw new RequestValidationError(
      "input_too_large",
      `input must be at most ${limits.maxInputChars} characters.`,
      413,
    );
  }

  if (body.context !== undefined && !isPlainObject(body.context)) {
    throw new RequestValidationError(
      "invalid_context",
      "context must be a JSON object when provided.",
    );
  }

  let maxOutputTokens = limits.defaultMaxOutputTokens;
  if (body.max_output_tokens !== undefined) {
    if (!Number.isInteger(body.max_output_tokens) || body.max_output_tokens < 1) {
      throw new RequestValidationError(
        "invalid_max_output_tokens",
        "max_output_tokens must be a positive integer.",
      );
    }
    maxOutputTokens = body.max_output_tokens;
  }

  if (maxOutputTokens > limits.maxOutputTokens) {
    throw new RequestValidationError(
      "max_output_tokens_too_large",
      `max_output_tokens must be at most ${limits.maxOutputTokens}.`,
    );
  }

  let requestId = crypto.randomUUID();
  if (body.request_id !== undefined) {
    if (
      typeof body.request_id !== "string" ||
      !REQUEST_ID_PATTERN.test(body.request_id)
    ) {
      throw new RequestValidationError(
        "invalid_request_id",
        "request_id must be 1-64 letters, digits, dots, underscores, colons, or hyphens.",
      );
    }
    requestId = body.request_id;
  }

  return {
    input: body.input.trim(),
    context: body.context,
    maxOutputTokens,
    requestId,
  };
}

/**
 * Read a possibly chunked request without first buffering an attacker-controlled body.
 *
 * @param {Request} request
 * @param {number} maxBytes
 * @returns {Promise<Uint8Array>}
 */
async function readBodyWithLimit(request, maxBytes) {
  if (!request.body) return new Uint8Array();

  const reader = request.body.getReader();
  const chunks = [];
  let totalBytes = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    totalBytes += value.byteLength;
    if (totalBytes > maxBytes) {
      await reader.cancel("request body exceeds configured limit");
      throw new RequestValidationError(
        "request_too_large",
        "Request body is too large.",
        413,
      );
    }

    chunks.push(value);
  }

  const body = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }

  return body;
}

/**
 * @param {unknown} value
 * @returns {value is Record<string, unknown>}
 */
function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }

  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}
