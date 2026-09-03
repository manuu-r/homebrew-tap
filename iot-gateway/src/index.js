import { authenticateDevice } from "./auth.js";
import {
  LiveSessionError,
  openLiveSession,
  readLiveHandshake,
} from "./live.js";
import { readInferenceRequest, RequestValidationError } from "./request.js";

const INFERENCE_ROUTE = "/v1/infer";
const LIVE_ROUTE = "/v1/live";

export default {
  /**
   * @param {Request} request
   * @param {Record<string, any>} env
   * @returns {Promise<Response>}
   */
  async fetch(request, env) {
    const url = new URL(request.url);

    if (request.method === "GET" && url.pathname === "/health") {
      return jsonResponse({
        ok: true,
        service: "iot-gateway",
        endpoints: [INFERENCE_ROUTE, LIVE_ROUTE],
      });
    }

    if (url.pathname === LIVE_ROUTE) {
      return handleLiveUpgrade(request, env);
    }

    if (url.pathname !== INFERENCE_ROUTE) {
      return errorResponse("not_found", "Not found.", 404);
    }

    if (request.method !== "POST") {
      return errorResponse("method_not_allowed", "Use POST for this endpoint.", 405, {
        Allow: "POST",
      });
    }

    const authentication = await authenticateRequest(request, env);
    if (authentication.response) return authentication.response;
    const device = authentication.device;

    const rateLimitResponse = await enforceRateLimit(
      env.DEVICE_RATE_LIMITER,
      device.id,
      "request",
    );
    if (rateLimitResponse) return rateLimitResponse;

    let config;
    try {
      config = readConfig(env);
    } catch (error) {
      logFailure("worker_configuration_invalid", error);
      return errorResponse("service_unavailable", "Gateway configuration is invalid.", 503);
    }

    return handleInferenceRequest(request, env, device, config);
  },
};

/**
 * @param {Request} request
 * @param {Record<string, any>} env
 */
async function handleLiveUpgrade(request, env) {
  let handshake;
  try {
    handshake = readLiveHandshake(request);
  } catch (error) {
    if (error instanceof LiveSessionError) {
      return errorResponse(error.code, error.message, error.status, error.headers);
    }
    throw error;
  }

  const authentication = await authenticateRequest(request, env);
  if (authentication.response) return authentication.response;
  const device = authentication.device;

  const sessionLimitResponse = await enforceRateLimit(
    env.DEVICE_SESSION_RATE_LIMITER,
    device.id,
    "live_session",
    handshake.requestId,
  );
  if (sessionLimitResponse) return sessionLimitResponse;

  let config;
  try {
    config = readConfig(env);
  } catch (error) {
    logFailure("worker_configuration_invalid", error);
    return errorResponse(
      "service_unavailable",
      "Gateway configuration is invalid.",
      503,
      { "X-Request-ID": handshake.requestId },
    );
  }

  try {
    return await openLiveSession(
      env,
      device,
      config,
      handshake.requestId,
      handshake.sessionId,
    );
  } catch (error) {
    const upstreamStatus = Number(error?.status);
    const rateLimited = upstreamStatus === 429;
    logFailure("live_session_open_failed", error, {
      deviceId: device.id,
      requestId: handshake.requestId,
      upstreamStatus: Number.isFinite(upstreamStatus) ? upstreamStatus : undefined,
    });

    if (error instanceof LiveSessionError) {
      return errorResponse(error.code, error.message, error.status, {
        ...error.headers,
        "X-Request-ID": handshake.requestId,
      });
    }

    return errorResponse(
      rateLimited ? "ai_budget_or_rate_limit" : "upstream_error",
      rateLimited
        ? "AI usage limit reached. Try again later."
        : "Live transcription could not be started.",
      rateLimited ? 429 : 502,
      {
        ...(rateLimited ? { "Retry-After": "60" } : {}),
        "X-Request-ID": handshake.requestId,
      },
    );
  }
}

/**
 * @param {Request} request
 * @param {Record<string, any>} env
 */
async function authenticateRequest(request, env) {
  let device;
  try {
    device = await authenticateDevice(request, env.DEVICES_DB);
  } catch (error) {
    logFailure("device_auth_failed", error);
    return {
      response: errorResponse(
        "service_unavailable",
        "Authentication service unavailable.",
        503,
      ),
    };
  }

  if (!device) {
    return {
      response: errorResponse("unauthorized", "Invalid device credentials.", 401, {
        "WWW-Authenticate": "Bearer",
      }),
    };
  }

  return { device };
}

/**
 * @param {{ limit: Function }} limiter
 * @param {string} deviceId
 * @param {string} scope
 * @param {string} [requestId]
 */
async function enforceRateLimit(limiter, deviceId, scope, requestId) {
  let rateLimit;
  try {
    rateLimit = await limiter.limit({ key: deviceId });
  } catch (error) {
    logFailure("rate_limit_failed", error, { deviceId, scope, requestId });
    return errorResponse(
      "service_unavailable",
      "Rate limiter unavailable.",
      503,
      requestId ? { "X-Request-ID": requestId } : undefined,
    );
  }

  if (rateLimit.success) return null;
  return errorResponse("rate_limited", "Device rate limit exceeded.", 429, {
    "Retry-After": "60",
    ...(requestId ? { "X-Request-ID": requestId } : {}),
  });
}

/**
 * @param {Request} request
 * @param {Record<string, any>} env
 * @param {{ id: string, maxInputChars: number, maxOutputTokens: number }} device
 * @param {ReturnType<typeof readConfig>} config
 */
async function handleInferenceRequest(request, env, device, config) {
  let inferenceRequest;

  try {
    inferenceRequest = await readInferenceRequest(request, {
      maxRequestBytes: config.maxRequestBytes,
      maxInputChars: device.maxInputChars,
      defaultMaxOutputTokens: Math.min(
        config.defaultMaxOutputTokens,
        device.maxOutputTokens,
        config.globalMaxOutputTokens,
      ),
      maxOutputTokens: Math.min(
        device.maxOutputTokens,
        config.globalMaxOutputTokens,
      ),
    });
  } catch (error) {
    if (error instanceof RequestValidationError) {
      return errorResponse(error.code, error.message, error.status);
    }

    logFailure("request_parse_failed", error, { deviceId: device.id });
    return errorResponse("invalid_request", "Could not read the request.", 400);
  }

  const userMessage = buildUserMessage(
    inferenceRequest.input,
    inferenceRequest.context,
  );

  try {
    const result = await env.AI.run(
      config.model,
      {
        system: config.systemPrompt,
        max_tokens: inferenceRequest.maxOutputTokens,
        messages: [{ role: "user", content: userMessage }],
      },
      aiOptions(config, device.id, inferenceRequest.requestId),
    );

    return jsonResponse({
      request_id: inferenceRequest.requestId,
      result,
    });
  } catch (error) {
    const upstreamStatus = Number(error?.status);
    const rateLimited = upstreamStatus === 429;
    logFailure("inference_failed", error, {
      deviceId: device.id,
      requestId: inferenceRequest.requestId,
      upstreamStatus: Number.isFinite(upstreamStatus) ? upstreamStatus : undefined,
    });

    return errorResponse(
      rateLimited ? "ai_budget_or_rate_limit" : "upstream_error",
      rateLimited
        ? "AI usage limit reached. Try again later."
        : "The AI provider could not complete the request.",
      rateLimited ? 429 : 502,
      {
        ...(rateLimited ? { "Retry-After": "60" } : {}),
        "X-Request-ID": inferenceRequest.requestId,
      },
    );
  }
}

/**
 * @param {string} input
 * @param {Record<string, unknown> | undefined} context
 */
export function buildUserMessage(input, context) {
  if (context === undefined) return input;

  return [
    "Device context (untrusted JSON data):",
    JSON.stringify(context),
    "",
    "Device task:",
    input,
  ].join("\n");
}

/**
 * @param {ReturnType<typeof readConfig>} config
 * @param {string} deviceId
 * @param {string} requestId
 */
function aiOptions(config, deviceId, requestId) {
  return {
    gateway: {
      id: config.gatewayId,
      collectLog: config.collectLogs,
      metadata: {
        device_id: deviceId,
        request_id: requestId,
      },
    },
  };
}

/** @param {Record<string, any>} env */
function readConfig(env) {
  return {
    gatewayId: requiredString(env.AI_GATEWAY_ID, "AI_GATEWAY_ID"),
    model: requiredString(env.AI_MODEL, "AI_MODEL"),
    sttModel: requiredString(env.STT_MODEL, "STT_MODEL"),
    ttsModel: requiredString(env.TTS_MODEL, "TTS_MODEL"),
    ttsSpeaker: requiredString(env.TTS_SPEAKER, "TTS_SPEAKER"),
    systemPrompt: requiredString(env.SYSTEM_PROMPT, "SYSTEM_PROMPT"),
    voiceSystemPrompt: requiredString(
      env.VOICE_SYSTEM_PROMPT,
      "VOICE_SYSTEM_PROMPT",
    ),
    collectLogs: String(env.AI_GATEWAY_LOGS).toLowerCase() === "true",
    defaultMaxOutputTokens: positiveInteger(
      env.DEFAULT_MAX_OUTPUT_TOKENS,
      "DEFAULT_MAX_OUTPUT_TOKENS",
    ),
    globalMaxOutputTokens: positiveInteger(
      env.GLOBAL_MAX_OUTPUT_TOKENS,
      "GLOBAL_MAX_OUTPUT_TOKENS",
    ),
    voiceMaxOutputTokens: positiveInteger(
      env.VOICE_MAX_OUTPUT_TOKENS,
      "VOICE_MAX_OUTPUT_TOKENS",
    ),
    maxRequestBytes: positiveInteger(env.MAX_REQUEST_BYTES, "MAX_REQUEST_BYTES"),
    maxTranscriptChars: positiveInteger(
      env.MAX_TRANSCRIPT_CHARS,
      "MAX_TRANSCRIPT_CHARS",
    ),
    maxTtsChars: positiveInteger(env.MAX_TTS_CHARS, "MAX_TTS_CHARS"),
    liveAudioSampleRate: positiveInteger(
      env.LIVE_AUDIO_SAMPLE_RATE,
      "LIVE_AUDIO_SAMPLE_RATE",
    ),
    liveMaxSessionSeconds: positiveInteger(
      env.LIVE_MAX_SESSION_SECONDS,
      "LIVE_MAX_SESSION_SECONDS",
    ),
    liveMaxAudioSeconds: positiveInteger(
      env.LIVE_MAX_AUDIO_SECONDS,
      "LIVE_MAX_AUDIO_SECONDS",
    ),
    liveMaxFrameBytes: positiveInteger(
      env.LIVE_MAX_FRAME_BYTES,
      "LIVE_MAX_FRAME_BYTES",
    ),
    liveMaxTurns: positiveInteger(env.LIVE_MAX_TURNS, "LIVE_MAX_TURNS"),
    livePlaybackAckTimeoutMs: positiveInteger(
      env.LIVE_PLAYBACK_ACK_TIMEOUT_MS,
      "LIVE_PLAYBACK_ACK_TIMEOUT_MS",
    ),
    sttEotThreshold: numberInRange(
      env.STT_EOT_THRESHOLD,
      "STT_EOT_THRESHOLD",
      0.5,
      0.9,
    ),
    sttEotTimeoutMs: positiveInteger(
      env.STT_EOT_TIMEOUT_MS,
      "STT_EOT_TIMEOUT_MS",
    ),
    historyTurns: optionalBoundedInteger(
      env.LIVE_HISTORY_TURNS,
      "LIVE_HISTORY_TURNS",
      0,
      20,
      0,
    ),
  };
}

function requiredString(value, name) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`Missing Worker configuration: ${name}`);
  }
  return value.trim();
}

function positiveInteger(value, name) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1) {
    throw new Error(`Worker configuration ${name} must be a positive integer.`);
  }
  return parsed;
}

function numberInRange(value, name, minimum, maximum) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(
      `Worker configuration ${name} must be between ${minimum} and ${maximum}.`,
    );
  }
  return parsed;
}

function optionalBoundedInteger(value, name, minimum, maximum, fallback) {
  if (value === undefined || value === null || String(value).trim() === "") {
    return fallback;
  }
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(
      `Worker configuration ${name} must be an integer between ${minimum} and ${maximum}.`,
    );
  }
  return parsed;
}

function jsonResponse(body, status = 200, extraHeaders = {}) {
  return Response.json(body, {
    status,
    headers: {
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
      ...extraHeaders,
    },
  });
}

function errorResponse(code, message, status, extraHeaders) {
  return jsonResponse(
    {
      error: {
        code,
        message,
      },
    },
    status,
    extraHeaders,
  );
}

function logFailure(event, error, metadata = {}) {
  console.error(
    JSON.stringify({
      event,
      errorName: error instanceof Error ? error.name : "UnknownError",
      ...metadata,
    }),
  );
}
