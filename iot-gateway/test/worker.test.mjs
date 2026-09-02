import assert from "node:assert/strict";
import test from "node:test";

import worker from "../src/index.js";
import { hashDeviceToken } from "../scripts/device-lib.mjs";

const validToken = "iot_worker_test_token";
const validHash = hashDeviceToken(validToken);

function makeEnv(overrides = {}) {
  const calls = [];

  const env = {
    AI_GATEWAY_ID: "iot-test",
    AI_MODEL: "anthropic/test-model",
    STT_MODEL: "@cf/deepgram/flux",
    TTS_MODEL: "@cf/deepgram/aura-2-en",
    TTS_SPEAKER: "aries",
    AI_GATEWAY_LOGS: "false",
    DEFAULT_MAX_OUTPUT_TOKENS: "64",
    GLOBAL_MAX_OUTPUT_TOKENS: "128",
    VOICE_MAX_OUTPUT_TOKENS: "48",
    MAX_REQUEST_BYTES: "2048",
    MAX_TRANSCRIPT_CHARS: "180",
    MAX_TTS_CHARS: "160",
    LIVE_AUDIO_SAMPLE_RATE: "16000",
    LIVE_MAX_SESSION_SECONDS: "300",
    LIVE_MAX_AUDIO_SECONDS: "240",
    LIVE_MAX_FRAME_BYTES: "4096",
    LIVE_MAX_TURNS: "8",
    LIVE_PLAYBACK_ACK_TIMEOUT_MS: "30000",
    STT_EOT_THRESHOLD: "0.7",
    STT_EOT_TIMEOUT_MS: "1200",
    SYSTEM_PROMPT: "Fixed trusted system prompt.",
    VOICE_SYSTEM_PROMPT: "Fixed trusted voice prompt.",
    DEVICES_DB: {
      prepare() {
        return {
          bind(hash) {
            return {
              async first() {
                if (hash !== validHash) return null;
                return {
                  id: "device-01",
                  enabled: 1,
                  max_input_chars: 200,
                  max_output_tokens: 100,
                };
              },
            };
          },
        };
      },
    },
    DEVICE_RATE_LIMITER: {
      async limit() {
        return { success: true };
      },
    },
    DEVICE_SESSION_RATE_LIMITER: {
      async limit() {
        return { success: true };
      },
    },
    AI: {
      async run(...args) {
        calls.push(args);
        return { content: [{ type: "text", text: "ok" }] };
      },
    },
    ...overrides,
  };

  return { env, calls };
}

function inferenceRequest(token, body) {
  return new Request("https://gateway.example/v1/infer", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
}

function liveRequest(token, requestId = "live:42") {
  return new Request("https://gateway.example/v1/live", {
    method: "GET",
    headers: {
      Authorization: `Bearer ${token}`,
      Upgrade: "websocket",
      "X-Request-ID": requestId,
    },
  });
}

test("the Worker rejects an unknown device before calling AI", async () => {
  const { env, calls } = makeEnv();
  const response = await worker.fetch(
    inferenceRequest("iot_wrong_token", { input: "Hello" }),
    env,
  );

  assert.equal(response.status, 401);
  assert.equal(calls.length, 0);
});

test("the Worker supplies trusted model, prompt, metadata, and caps", async () => {
  const { env, calls } = makeEnv();
  const response = await worker.fetch(
    inferenceRequest(validToken, {
      input: "Explain the sensor reading",
      context: { humidity: 77 },
      max_output_tokens: 90,
      request_id: "sensor:42",
    }),
    env,
  );

  assert.equal(response.status, 200);
  assert.equal(calls.length, 1);

  const [model, input, options] = calls[0];
  assert.equal(model, "anthropic/test-model");
  assert.equal(input.system, "Fixed trusted system prompt.");
  assert.equal(input.max_tokens, 90);
  assert.match(input.messages[0].content, /"humidity":77/);
  assert.deepEqual(options.gateway.metadata, {
    device_id: "device-01",
    request_id: "sensor:42",
  });

  const body = await response.json();
  assert.equal(body.request_id, "sensor:42");
});

test("the Worker applies rate limiting before reading the body", async () => {
  const { env, calls } = makeEnv({
    DEVICE_RATE_LIMITER: {
      async limit() {
        return { success: false };
      },
    },
  });

  const response = await worker.fetch(
    inferenceRequest(validToken, { input: "Hello" }),
    env,
  );

  assert.equal(response.status, 429);
  assert.equal(response.headers.get("retry-after"), "60");
  assert.equal(calls.length, 0);
});

test("the live route requires a WebSocket upgrade", async () => {
  const { env, calls } = makeEnv();
  const response = await worker.fetch(
    new Request("https://gateway.example/v1/live"),
    env,
  );

  assert.equal(response.status, 426);
  assert.equal(response.headers.get("upgrade"), "websocket");
  assert.equal(calls.length, 0);
});

test("the live route authenticates before starting Flux", async () => {
  const { env, calls } = makeEnv();
  const response = await worker.fetch(liveRequest("iot_wrong_token"), env);

  assert.equal(response.status, 401);
  assert.equal(calls.length, 0);
});

test("the live route rate-limits session openings before starting Flux", async () => {
  const { env, calls } = makeEnv({
    DEVICE_SESSION_RATE_LIMITER: {
      async limit() {
        return { success: false };
      },
    },
  });
  const response = await worker.fetch(liveRequest(validToken), env);

  assert.equal(response.status, 429);
  assert.equal(response.headers.get("retry-after"), "60");
  assert.equal(calls.length, 0);
});

test("the live route opens Flux with raw linear16 turn detection settings", async () => {
  const { env, calls } = makeEnv();
  const response = await worker.fetch(liveRequest(validToken), env);

  // The default AI mock intentionally does not return a WebSocket. Reaching
  // this error proves that authentication/configuration reached Flux setup.
  assert.equal(response.status, 502);
  assert.equal(calls.length, 1);

  const [model, input, options] = calls[0];
  assert.equal(model, "@cf/deepgram/flux");
  assert.deepEqual(input, {
    encoding: "linear16",
    sample_rate: "16000",
    eot_threshold: "0.7",
    eot_timeout_ms: "1200",
  });
  assert.equal(options.websocket, true);
  assert.deepEqual(options.gateway.metadata, {
    device_id: "device-01",
    request_id: "live:42",
    pipeline_stage: "stt",
  });
});
