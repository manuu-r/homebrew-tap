import assert from "node:assert/strict";
import test from "node:test";

import {
  LiveSession,
  LiveSessionError,
  normalizeSpeechText,
  openFluxSocket,
  readLiveHandshake,
} from "../src/live.js";

class FakeSocket {
  constructor() {
    this.readyState = 0;
    this.sent = [];
    this.listeners = new Map();
    this.closed = null;
  }

  accept() {
    this.readyState = 1;
  }

  addEventListener(type, listener) {
    const listeners = this.listeners.get(type) || [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  send(value) {
    if (this.readyState !== 1) throw new Error("socket is not open");
    if (ArrayBuffer.isView(value)) {
      this.sent.push(new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice());
      return;
    }
    if (value instanceof ArrayBuffer) {
      this.sent.push(new Uint8Array(value).slice());
      return;
    }
    this.sent.push(value);
  }

  close(code, reason) {
    if (this.readyState >= 2) return;
    this.readyState = 3;
    this.closed = { code, reason };
    for (const listener of this.listeners.get("close") || []) {
      listener({ code, reason });
    }
  }
}

function makeConfig(overrides = {}) {
  return {
    gatewayId: "iot-test",
    model: "anthropic/test-haiku",
    sttModel: "@cf/deepgram/flux",
    ttsModel: "@cf/deepgram/aura-2-en",
    ttsSpeaker: "aries",
    voiceSystemPrompt: "Reply in one line.",
    collectLogs: false,
    voiceMaxOutputTokens: 48,
    globalMaxOutputTokens: 128,
    maxTranscriptChars: 200,
    maxTtsChars: 160,
    liveAudioSampleRate: 16000,
    liveMaxSessionSeconds: 300,
    liveMaxAudioSeconds: 240,
    liveMaxFrameBytes: 4096,
    liveMaxTurns: 8,
    livePlaybackAckTimeoutMs: 30000,
    sttEotThreshold: 0.7,
    sttEotTimeoutMs: 1200,
    ...overrides,
  };
}

function makeSession({ aiRun, rateLimit, config } = {}) {
  const calls = [];
  const downstream = new FakeSocket();
  const upstream = new FakeSocket();
  const env = {
    DEVICE_RATE_LIMITER: {
      async limit(...args) {
        return rateLimit ? rateLimit(...args) : { success: true };
      },
    },
    AI: {
      async run(...args) {
        calls.push(args);
        return aiRun(...args, calls.length);
      },
    },
  };
  const session = new LiveSession({
    env,
    device: {
      id: "device-01",
      maxInputChars: 180,
      maxOutputTokens: 100,
    },
    config: makeConfig(config),
    requestId: "live:42",
    downstream,
    upstream,
  });
  session.start();
  return { session, downstream, upstream, calls };
}

function jsonMessages(socket) {
  return socket.sent
    .filter((message) => typeof message === "string")
    .map((message) => JSON.parse(message));
}

test("readLiveHandshake accepts only a WebSocket GET with a safe request ID", () => {
  const parsed = readLiveHandshake(
    new Request("https://gateway.example/v1/live", {
      headers: {
        Upgrade: "websocket",
        "X-Request-ID": "bunty:live:1",
      },
    }),
  );
  assert.deepEqual(parsed, { requestId: "bunty:live:1" });

  assert.throws(
    () => readLiveHandshake(new Request("https://gateway.example/v1/live")),
    (error) =>
      error instanceof LiveSessionError && error.code === "websocket_required",
  );
});

test("openFluxSocket uses the real-time Flux WebSocket schema", async () => {
  const calls = [];
  const socket = new FakeSocket();
  const returned = await openFluxSocket(
    {
      AI: {
        async run(...args) {
          calls.push(args);
          return { webSocket: socket };
        },
      },
    },
    "device-01",
    makeConfig(),
    "live:42",
  );

  assert.equal(returned, socket);
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], "@cf/deepgram/flux");
  assert.deepEqual(calls[0][1], {
    encoding: "linear16",
    sample_rate: "16000",
    eot_threshold: "0.7",
    eot_timeout_ms: "1200",
  });
  assert.equal(calls[0][2].websocket, true);
  assert.equal(calls[0][2].gateway.metadata.pipeline_stage, "stt");
});

test("LiveSession forwards bounded raw PCM and partial transcripts", () => {
  const { session, downstream, upstream } = makeSession({
    aiRun: async () => {
      throw new Error("AI should not run for a partial transcript");
    },
  });

  const pcm = new Uint8Array([1, 2, 3, 4]);
  session.handleClientMessage({ data: pcm.buffer });
  assert.deepEqual(upstream.sent, [pcm]);

  session.handleUpstreamMessage({
    data: JSON.stringify({
      event: "Update",
      turn_index: 0,
      transcript: "hello Bunty",
    }),
  });
  assert.ok(
    jsonMessages(downstream).some(
      (message) =>
        message.type === "transcript.partial" && message.text === "hello Bunty",
    ),
  );

  session.stop(true, 1000, "test complete");
});

test("LiveSession preserves Blob audio ordering and lets device VAD end input", async () => {
  const { session, downstream, upstream } = makeSession({
    aiRun: async () => {
      throw new Error("AI should not run before a final transcript");
    },
  });

  await session.handleClientMessage({
    data: new Blob([new Uint8Array([1, 2, 3, 4])]),
  });
  await session.handleClientMessage({
    data: JSON.stringify({ type: "input.end" }),
  });

  assert.deepEqual(upstream.sent, [
    new Uint8Array([1, 2, 3, 4]),
    JSON.stringify({ type: "ForceEndTurn" }),
    JSON.stringify({ type: "CloseStream" }),
  ]);
  assert.ok(
    jsonMessages(downstream).some((message) => message.type === "input.ended"),
  );
  session.stop(true, 1000, "test complete");
});

test("LiveSession preserves the last Update when Flux closes during finalization", async () => {
  const { session, downstream, upstream, calls } = makeSession({
    aiRun: async (_model, _input, _options, callNumber) => {
      if (callNumber === 1) {
        return { content: [{ type: "text", text: "The voice path works." }] };
      }
      return new Response(new Uint8Array([10, 20, 30, 40]), {
        headers: { "Content-Type": "audio/l16" },
      });
    },
  });

  session.handleUpstreamMessage({
    data: JSON.stringify({
      event: "Update",
      turn_index: 0,
      transcript: "Test",
    }),
  });
  session.handleUpstreamMessage({
    data: JSON.stringify({
      event: "EndOfTurn",
      turn_index: 0,
      transcript: "Test",
    }),
  });
  assert.equal(calls.length, 0, "Flux cannot outrun the device VAD");

  session.handleUpstreamMessage({
    data: JSON.stringify({
      event: "Update",
      turn_index: 1,
      transcript: "the voice path",
    }),
  });
  await session.handleClientMessage({
    data: new Uint8Array([1, 2, 3, 4]),
  });
  await session.handleClientMessage({
    data: JSON.stringify({ type: "input.end" }),
  });

  upstream.close(1000, "Flux stream drained");
  await session.whenIdle();

  assert.equal(calls.length, 2);
  assert.equal(calls[0][1].messages[0].content, "Test the voice path");
  const controls = jsonMessages(downstream);
  assert.ok(
    controls.some(
      (message) =>
        message.type === "transcript.final" &&
        message.text === "Test the voice path",
    ),
  );
  assert.ok(!controls.some((message) => message.code === "no_speech"));

  session.handleClientMessage({
    data: JSON.stringify({ type: "playback.finished", turn: "0" }),
  });
});

test("LiveSession runs final speech through Haiku and streams Aura Aries PCM", async () => {
  const { session, downstream, calls } = makeSession({
    aiRun: async (_model, _input, _options, callNumber) => {
      if (callNumber === 1) {
        return {
          content: [{ type: "text", text: "  It is\n twenty-four degrees.  " }],
        };
      }
      return new Response(new Uint8Array([10, 20, 30, 40]), {
        headers: { "Content-Type": "audio/l16" },
      });
    },
  });

  await session.handleClientMessage({
    data: new Uint8Array([1, 2, 3, 4]),
  });
  await session.handleClientMessage({
    data: JSON.stringify({ type: "input.end" }),
  });
  session.handleUpstreamMessage({
    data: JSON.stringify({
      event: "EndOfTurn",
      turn_index: 3,
      transcript: "What is the temperature?",
    }),
  });
  await session.whenIdle();

  assert.equal(calls.length, 2);
  const [model, modelInput, modelOptions] = calls[0];
  assert.equal(model, "anthropic/test-haiku");
  assert.equal(modelInput.system, "Reply in one line.");
  assert.equal(modelInput.max_tokens, 48);
  assert.equal(modelInput.messages[0].content, "What is the temperature?");
  assert.deepEqual(modelOptions.gateway.metadata, {
    device_id: "device-01",
    request_id: "live:42",
    pipeline_stage: "llm",
    turn: "3",
  });

  const [ttsModel, ttsInput, ttsOptions] = calls[1];
  assert.equal(ttsModel, "@cf/deepgram/aura-2-en");
  assert.deepEqual(ttsInput, {
    text: "It is twenty-four degrees.",
    speaker: "aries",
    encoding: "linear16",
    container: "none",
    sample_rate: 16000,
  });
  assert.equal(ttsOptions.returnRawResponse, true);
  assert.equal(ttsOptions.gateway.metadata.pipeline_stage, "tts");

  const controls = jsonMessages(downstream);
  const controlTypes = controls.map((message) => message.type);
  assert.ok(controlTypes.includes("transcript.final"));
  assert.ok(controlTypes.includes("input.pause"));
  assert.ok(controlTypes.includes("assistant.thinking"));
  assert.ok(
    controls.some(
      (message) =>
        message.type === "assistant.text" &&
        message.text === "It is twenty-four degrees.",
    ),
  );
  assert.ok(controlTypes.includes("audio.start"));
  assert.ok(controlTypes.includes("audio.end"));
  assert.ok(!controlTypes.includes("input.resume"));
  assert.deepEqual(
    downstream.sent.filter((message) => message instanceof Uint8Array),
    [new Uint8Array([10, 20, 30, 40])],
  );

  session.handleClientMessage({
    data: JSON.stringify({ type: "playback.finished", turn: "3" }),
  });
  assert.ok(
    jsonMessages(downstream).some(
      (message) => message.type === "session.complete" && message.turn === "3",
    ),
  );
  assert.deepEqual(downstream.closed, {
    code: 1000,
    reason: "Voice turn complete",
  });

  session.stop(true, 1000, "test complete");
});

test("LiveSession enforces per-turn rate limits without calling AI", async () => {
  const { session, downstream, calls } = makeSession({
    rateLimit: async () => ({ success: false }),
    aiRun: async () => {
      throw new Error("AI should not run when rate limited");
    },
  });

  await session.handleClientMessage({
    data: new Uint8Array([1, 2, 3, 4]),
  });
  await session.handleClientMessage({
    data: JSON.stringify({ type: "input.end" }),
  });
  session.handleUpstreamMessage({
    data: JSON.stringify({
      event: "EndOfTurn",
      turn_index: 0,
      transcript: "Hello",
    }),
  });
  await session.whenIdle();

  assert.equal(calls.length, 0);
  const controls = jsonMessages(downstream);
  assert.ok(
    controls.some(
      (message) => message.type === "error" && message.code === "rate_limited",
    ),
  );
  assert.ok(controls.some((message) => message.type === "session.complete"));
  session.stop(true, 1000, "test complete");
});

test("LiveSession rejects oversized frames before forwarding them", () => {
  const { session, downstream, upstream } = makeSession({
    config: { liveMaxFrameBytes: 4 },
    aiRun: async () => {
      throw new Error("AI should not run");
    },
  });
  session.handleClientMessage({ data: new Uint8Array(6).buffer });

  assert.equal(upstream.sent.length, 0);
  assert.ok(
    jsonMessages(downstream).some(
      (message) =>
        message.type === "error" && message.code === "audio_frame_too_large",
    ),
  );
  session.stop(true, 1000, "test complete");
});

test("normalizeSpeechText produces a bounded single-line TTS string", () => {
  assert.equal(
    normalizeSpeechText("  first line\nsecond   line ", 80),
    "first line second line",
  );
  assert.equal(normalizeSpeechText("one two three four", 14), "one two three…");
});
