import {
  appendTurn,
  loadHistory,
  normalizeSessionId,
  retireOldSessions,
} from "./history.js";

const REQUEST_ID_PATTERN = /^[A-Za-z0-9._:-]{1,64}$/;
const WEBSOCKET_OPEN = 1;

export class LiveSessionError extends Error {
  /**
   * @param {string} code
   * @param {string} message
   * @param {number} status
   * @param {Record<string, string>} [headers]
   */
  constructor(code, message, status, headers = {}) {
    super(message);
    this.name = "LiveSessionError";
    this.code = code;
    this.status = status;
    this.headers = headers;
  }
}

/** @param {Request} request */
export function readLiveHandshake(request) {
  if (request.method !== "GET") {
    throw new LiveSessionError(
      "method_not_allowed",
      "Use a WebSocket GET request for this endpoint.",
      405,
      { Allow: "GET" },
    );
  }

  if ((request.headers.get("upgrade") || "").toLowerCase() !== "websocket") {
    throw new LiveSessionError(
      "websocket_required",
      "Upgrade this request to a WebSocket connection.",
      426,
      { Upgrade: "websocket" },
    );
  }

  const suppliedRequestId = request.headers.get("x-request-id");
  if (
    suppliedRequestId !== null &&
    !REQUEST_ID_PATTERN.test(suppliedRequestId)
  ) {
    throw new LiveSessionError(
      "invalid_request_id",
      "X-Request-ID must be 1-64 letters, digits, dots, underscores, colons, or hyphens.",
      400,
    );
  }

  return {
    requestId: suppliedRequestId || crypto.randomUUID(),
    // Per-boot conversation scope. Absent (curl, tests) simply means no
    // memory; a malformed value is ignored rather than rejected so a firmware
    // bug degrades to statelessness instead of a dead endpoint.
    sessionId: normalizeSessionId(request.headers.get("x-bunty-session")),
  };
}

/**
 * Open the upstream Flux socket, attach the bidirectional session, and return
 * the downstream WebSocket upgrade response.
 *
 * @param {Record<string, any>} env
 * @param {{ id: string, maxInputChars: number, maxOutputTokens: number }} device
 * @param {Record<string, any>} config
 * @param {string} requestId
 */
export async function openLiveSession(
  env,
  device,
  config,
  requestId,
  sessionId = null,
) {
  const upstream = await openFluxSocket(env, device.id, config, requestId);

  let client;
  let downstream;
  try {
    const pair = new WebSocketPair();
    [client, downstream] = Object.values(pair);
  } catch (error) {
    safeClose(upstream, 1011, "Could not initialize session");
    throw error;
  }

  const session = new LiveSession({
    env,
    device,
    config,
    requestId,
    sessionId,
    downstream,
    upstream,
  });
  session.start();

  return new Response(null, {
    status: 101,
    headers: {
      "Cache-Control": "no-store",
      "X-Request-ID": requestId,
    },
    webSocket: client,
  });
}

/**
 * @param {Record<string, any>} env
 * @param {string} deviceId
 * @param {Record<string, any>} config
 * @param {string} requestId
 */
export async function openFluxSocket(env, deviceId, config, requestId) {
  const response = await env.AI.run(
    config.sttModel,
    {
      encoding: "linear16",
      sample_rate: String(config.liveAudioSampleRate),
      eot_threshold: String(config.sttEotThreshold),
      eot_timeout_ms: String(config.sttEotTimeoutMs),
    },
    {
      websocket: true,
      ...aiOptions(config, deviceId, requestId, "stt"),
    },
  );

  const socket = response?.webSocket;
  if (!socket || typeof socket.accept !== "function") {
    throw new LiveSessionError(
      "upstream_error",
      "The live transcription service did not open a WebSocket.",
      502,
    );
  }

  return socket;
}

export class LiveSession {
  /**
   * @param {{
   *   env: Record<string, any>,
   *   device: { id: string, maxInputChars: number, maxOutputTokens: number },
   *   config: Record<string, any>,
   *   requestId: string,
   *   downstream: WebSocket,
   *   upstream: WebSocket
   * }} options
   */
  constructor(options) {
    this.env = options.env;
    this.device = options.device;
    this.config = options.config;
    this.requestId = options.requestId;
    this.sessionId = options.sessionId || null;
    this.downstream = options.downstream;
    this.upstream = options.upstream;

    this.audioBytes = 0;
    this.turnCount = 0;
    this.inputEnded = false;
    this.transcriptionComplete = false;
    this.responding = false;
    this.stopped = false;
    this.latestTranscripts = new Map();
    this.completedTranscripts = new Map();
    this.processedTurns = new Set();
    this.turnQueue = Promise.resolve();
    this.clientMessageQueue = Promise.resolve();
    this.expirationTimer = undefined;
    this.playbackTimer = undefined;
    this.awaitingPlaybackTurn = null;
  }

  start() {
    if (this.sessionId) {
      // Fire-and-forget: a new boot id clears the previous build's history.
      retireOldSessions(this.env, this.device.id, this.sessionId);
    }
    this.downstream.addEventListener("message", (event) => {
      // Blob conversion is asynchronous in the Workers runtime. Serialize all
      // device messages so `input.end` can never overtake the last PCM frame.
      const data = event.data;
      this.clientMessageQueue = this.clientMessageQueue
        .then(() => this.handleClientMessage({ data }))
        .catch((error) => {
          this.logFailure("device_message_failed", error);
          this.sendJson({
            type: "error",
            code: "invalid_message",
            message: "The device message could not be processed.",
          });
          this.stop(true, 1003, "Invalid device message");
        });
    });
    this.downstream.addEventListener("close", () => {
      this.stop(false, 1000, "Device disconnected");
    });
    this.downstream.addEventListener("error", () => {
      this.stop(false, 1011, "Device WebSocket failed");
    });

    this.upstream.addEventListener("message", (event) => {
      this.handleUpstreamMessage(event);
    });
    this.upstream.addEventListener("close", () => {
      if (this.stopped || this.transcriptionComplete) return;
      if (this.inputEnded) {
        // Cloudflare's Flux transport can close after the ordered
        // ForceEndTurn/CloseStream pair without surfacing EndOfTurn to the
        // Worker. Deepgram defines the final Update before CloseStream as the
        // usable transcript in that case, so preserve it instead of turning a
        // successfully decoded utterance into `no_speech`.
        const latest = Array.from(this.latestTranscripts.entries()).at(-1);
        this.finalizeTranscription(latest?.[0] || "0", latest?.[1] || "");
        return;
      }
      this.sendJson({
        type: "error",
        code: "stt_disconnected",
        message: "Live transcription disconnected.",
      });
      this.completeSession("0");
    });
    this.upstream.addEventListener("error", () => {
      if (!this.stopped && !this.transcriptionComplete) {
        this.sendJson({
          type: "error",
          code: "stt_error",
          message: "Live transcription failed.",
        });
        this.stop(true, 1011, "Transcription failed");
      }
    });

    this.downstream.accept();
    this.upstream.accept();

    this.expirationTimer = setTimeout(() => {
      this.sendJson({
        type: "error",
        code: "session_expired",
        message: "The live session reached its duration limit.",
      });
      this.stop(true, 1000, "Session duration limit reached");
    }, this.config.liveMaxSessionSeconds * 1000);
    this.expirationTimer?.unref?.();

    this.sendJson({
      type: "session.ready",
      request_id: this.requestId,
      mode: "device_vad",
      input_audio: {
        encoding: "linear16",
        sample_rate: this.config.liveAudioSampleRate,
        channels: 1,
      },
      output_audio: {
        encoding: "linear16",
        sample_rate: this.config.liveAudioSampleRate,
        channels: 1,
      },
    });
  }

  /** @param {{ data: unknown }} event */
  async handleClientMessage(event) {
    if (this.stopped) return;

    if (typeof event.data === "string") {
      this.handleControlMessage(event.data);
      return;
    }

    let audio = binaryView(event.data);
    if (!audio && event.data && typeof event.data.arrayBuffer === "function") {
      audio = new Uint8Array(await event.data.arrayBuffer());
    }
    if (!audio) {
      this.sendJson({
        type: "error",
        code: "invalid_message",
        message: "Send control messages as JSON and audio as binary PCM.",
      });
      return;
    }

    if (audio.byteLength === 0) return;
    if (this.inputEnded) return;
    if (audio.byteLength > this.config.liveMaxFrameBytes) {
      this.sendJson({
        type: "error",
        code: "audio_frame_too_large",
        message: `Audio frames must be at most ${this.config.liveMaxFrameBytes} bytes.`,
      });
      return;
    }
    if (audio.byteLength % 2 !== 0) {
      this.sendJson({
        type: "error",
        code: "invalid_audio_frame",
        message: "16-bit PCM frames must contain an even number of bytes.",
      });
      return;
    }

    if (this.responding) {
      // Half-duplex mode deliberately drops microphone frames while the
      // speaker is active so Bunty does not transcribe its own response.
      return;
    }

    this.audioBytes += audio.byteLength;
    const maxAudioBytes =
      this.config.liveMaxAudioSeconds *
      this.config.liveAudioSampleRate *
      2;
    if (this.audioBytes > maxAudioBytes) {
      this.sendJson({
        type: "error",
        code: "audio_limit_reached",
        message: "The session reached its live audio limit.",
      });
      this.stop(true, 1009, "Audio limit reached");
      return;
    }

    if (!isOpen(this.upstream)) {
      this.stop(true, 1011, "Transcription is unavailable");
      return;
    }

    try {
      this.upstream.send(audio);
    } catch (error) {
      this.logFailure("stt_audio_send_failed", error);
      this.stop(true, 1011, "Could not stream microphone audio");
    }
  }

  /** @param {string} raw */
  handleControlMessage(raw) {
    let message;
    try {
      message = JSON.parse(raw);
    } catch {
      this.sendJson({
        type: "error",
        code: "invalid_control_message",
        message: "Control messages must be valid JSON.",
      });
      return;
    }

    if (!message || typeof message !== "object") {
      this.sendJson({
        type: "error",
        code: "invalid_control_message",
        message: "Control messages must be JSON objects.",
      });
      return;
    }

    if (message.type === "ping") {
      this.sendJson({ type: "pong" });
      return;
    }
    if (message.type === "input.end") {
      if (this.inputEnded) return;
      if (this.audioBytes === 0) {
        this.sendJson({
          type: "error",
          code: "empty_audio",
          message: "The utterance contained no audio.",
        });
        this.completeSession("0");
        return;
      }
      if (!isOpen(this.upstream)) {
        this.sendJson({
          type: "error",
          code: "stt_disconnected",
          message: "Transcription ended before the utterance was complete.",
        });
        this.completeSession("0");
        return;
      }

      this.inputEnded = true;
      try {
        // Bunty owns endpointing. Deepgram documents this exact ordering for
        // external VAD: finalize the active turn, then drain and close STT.
        this.upstream.send(JSON.stringify({ type: "ForceEndTurn" }));
        this.upstream.send(JSON.stringify({ type: "CloseStream" }));
        this.sendJson({ type: "input.ended" });
      } catch (error) {
        this.logFailure("stt_finalize_failed", error);
        this.sendJson({
          type: "error",
          code: "stt_finalize_failed",
          message: "The transcription stream could not be finalized.",
        });
        this.stop(true, 1011, "Could not finalize transcription");
      }
      return;
    }
    if (message.type === "session.close") {
      this.stop(true, 1000, "Device ended session");
      return;
    }
    if (message.type === "playback.finished") {
      const turn = String(message.turn ?? "");
      if (this.awaitingPlaybackTurn === turn) {
        this.finishPlayback(turn);
      } else {
        this.sendJson({
          type: "error",
          code: "unexpected_playback_ack",
          message: "No matching audio playback is awaiting confirmation.",
        });
      }
      return;
    }

    this.sendJson({
      type: "error",
      code: "unsupported_control_message",
      message:
        "Supported controls are ping, input.end, playback.finished, and session.close.",
    });
  }

  /** @param {{ data: unknown }} event */
  handleUpstreamMessage(event) {
    if (this.stopped) return;

    const raw = textView(event.data);
    if (raw === null) {
      this.logFailure(
        "stt_protocol_error",
        new TypeError("Flux returned a non-text control message."),
      );
      return;
    }

    let message;
    try {
      message = JSON.parse(raw);
    } catch (error) {
      this.logFailure("stt_protocol_error", error);
      return;
    }

    const eventType = String(message?.event || message?.type || "");
    const turnIndex = normalizeTurnIndex(message?.turn_index, this.turnCount);
    const transcript =
      typeof message?.transcript === "string" ? message.transcript.trim() : "";

    if (transcript) this.latestTranscripts.set(turnIndex, transcript);

    if (eventType === "StartOfTurn") {
      this.sendJson({ type: "speech.started", turn: turnIndex });
      return;
    }

    if (eventType === "Update") {
      if (transcript) {
        this.sendJson({
          type: "transcript.partial",
          turn: turnIndex,
          text: transcript,
        });
      }
      return;
    }

    if (eventType === "EagerEndOfTurn") {
      this.sendJson({ type: "speech.maybe_finished", turn: turnIndex });
      return;
    }

    if (eventType === "TurnResumed") {
      this.sendJson({ type: "speech.resumed", turn: turnIndex });
      return;
    }

    if (eventType !== "EndOfTurn") return;

    const finalTranscript =
      transcript || this.latestTranscripts.get(turnIndex) || "";
    this.latestTranscripts.delete(turnIndex);

    if (!this.inputEnded) {
      // Flux may see an EndOfTurn inside a longer device-owned utterance (for
      // example, a natural pause between sentences). Keep that segment, but
      // do not call Claude until Bunty's VAD sends input.end.
      if (finalTranscript) {
        this.completedTranscripts.set(turnIndex, finalTranscript);
      }
      return;
    }

    if (isOpen(this.upstream)) {
      try {
        this.upstream.send(JSON.stringify({ type: "CloseStream" }));
      } catch {
        // CloseStream may already be draining after the device's input.end.
      }
    }
    this.finalizeTranscription(turnIndex, finalTranscript);
  }

  /**
   * Commit the last live transcript exactly once, whether Flux supplied a
   * normal EndOfTurn or closed after flushing its final Update.
   *
   * @param {string} turnIndex
   * @param {string} transcript
   */
  finalizeTranscription(turnIndex, transcript) {
    if (this.stopped || this.transcriptionComplete) return;
    const currentTranscript = transcript.trim();
    if (currentTranscript) {
      this.completedTranscripts.set(turnIndex, currentTranscript);
    }
    const finalTranscript = Array.from(this.completedTranscripts.values())
      .join(" ")
      .trim();
    this.latestTranscripts.clear();
    this.completedTranscripts.clear();
    this.inputEnded = true;
    this.transcriptionComplete = true;
    if (!finalTranscript) {
      this.sendJson({
        type: "error",
        code: "no_speech",
        message: "No speech could be transcribed.",
      });
      this.completeSession(turnIndex);
      return;
    }
    if (this.processedTurns.has(turnIndex)) return;

    this.processedTurns.add(turnIndex);
    this.enqueueTurn(turnIndex, finalTranscript);
  }

  /**
   * @param {string} turnIndex
   * @param {string} transcript
   */
  enqueueTurn(turnIndex, transcript) {
    if (this.responding || this.stopped) return;

    this.turnCount += 1;
    if (this.turnCount > this.config.liveMaxTurns) {
      this.sendJson({
        type: "error",
        code: "turn_limit_reached",
        message: "The session reached its conversation-turn limit.",
      });
      this.stop(true, 1000, "Turn limit reached");
      return;
    }

    this.responding = true;
    this.sendJson({ type: "transcript.final", turn: turnIndex, text: transcript });
    this.sendJson({ type: "input.pause", turn: turnIndex });

    this.turnQueue = this.turnQueue
      .then(() => this.processTurn(turnIndex, transcript))
      .catch((error) => {
        this.logFailure("live_turn_unhandled_error", error, { turn: turnIndex });
        this.sendJson({
          type: "error",
          code: "turn_failed",
          message: "The live voice turn could not be completed.",
          turn: turnIndex,
        });
      })
      .finally(() => {
        if (this.awaitingPlaybackTurn === null) this.completeSession(turnIndex);
      });
  }

  /**
   * @param {string} turnIndex
   * @param {string} transcript
   */
  async processTurn(turnIndex, transcript) {
    const transcriptLimit = Math.min(
      this.config.maxTranscriptChars,
      this.device.maxInputChars,
    );
    if (transcript.length > transcriptLimit) {
      this.sendJson({
        type: "error",
        code: "transcript_too_large",
        message: `Transcribed speech must be at most ${transcriptLimit} characters.`,
        turn: turnIndex,
      });
      return;
    }

    let rateLimit;
    try {
      rateLimit = await this.env.DEVICE_RATE_LIMITER.limit({
        key: this.device.id,
      });
    } catch (error) {
      this.logFailure("rate_limit_failed", error, { turn: turnIndex });
      this.sendJson({
        type: "error",
        code: "service_unavailable",
        message: "The rate limiter is unavailable.",
        turn: turnIndex,
      });
      return;
    }

    if (!rateLimit.success) {
      this.sendJson({
        type: "error",
        code: "rate_limited",
        message: "Device rate limit exceeded. Try again later.",
        retry_after: 60,
        turn: turnIndex,
      });
      return;
    }

    this.sendJson({ type: "assistant.thinking", turn: turnIndex });

    const history = this.sessionId
      ? await loadHistory(
          this.env,
          this.device.id,
          this.sessionId,
          this.config.historyTurns,
        )
      : [];

    // Stream Claude's reply and speak it a sentence at a time, so the opening
    // words play while the rest is still being generated. A one-sentence reply
    // still produces exactly one TTS call.
    const messages = [...history, { role: "user", content: transcript }];
    const maxTokens = Math.min(
      this.config.voiceMaxOutputTokens,
      this.device.maxOutputTokens,
      this.config.globalMaxOutputTokens,
    );

    let modelResult;
    try {
      modelResult = await this.env.AI.run(
        this.config.model,
        {
          system: this.config.voiceSystemPrompt,
          max_tokens: maxTokens,
          messages,
          stream: true,
        },
        aiOptions(this.config, this.device.id, this.requestId, "llm", turnIndex),
      );
    } catch (error) {
      this.sendStageError("llm", error, turnIndex);
      return;
    }

    const speech = new SpeechEmitter(this, turnIndex);

    try {
      for await (const delta of iterModelText(modelResult)) {
        if (this.stopped || !isOpen(this.downstream)) break;
        speech.push(delta);
        let sentence;
        while ((sentence = speech.takeSentence()) !== null) {
          await speech.speak(sentence);
        }
      }
    } catch (error) {
      // A stream that fails after audio has started keeps whatever played;
      // one that fails before is retried once as a plain non-streaming call.
      this.logFailure(
        speech.audioStarted ? "llm_stream_interrupted" : "llm_stream_failed",
        error,
        { turn: turnIndex },
      );
    }

    if (!speech.fullText.trim() && !speech.audioStarted) {
      let fallback;
      try {
        fallback = await this.env.AI.run(
          this.config.model,
          {
            system: this.config.voiceSystemPrompt,
            max_tokens: maxTokens,
            messages,
          },
          aiOptions(this.config, this.device.id, this.requestId, "llm", turnIndex),
        );
      } catch (error) {
        this.sendStageError("llm", error, turnIndex);
        return;
      }
      speech.push(extractAssistantText(fallback));
    }

    if (!this.stopped) {
      try {
        await speech.flushRemainder();
      } catch (error) {
        if (!speech.audioStarted) {
          this.sendStageError("tts", error, turnIndex);
          return;
        }
        this.logFailure("tts_failed", error, { turn: turnIndex });
      }
    }

    const spoken = speech.fullText.replace(/\s+/g, " ").trim();
    if (!spoken && !speech.audioStarted) {
      this.sendJson({
        type: "error",
        code: "empty_model_response",
        message: "Claude returned no text to speak.",
        turn: turnIndex,
      });
      return;
    }

    if (this.sessionId && spoken) {
      // Not awaited: memory is best-effort and must not delay the turn.
      appendTurn(
        this.env,
        this.device.id,
        this.sessionId,
        this.turnCount,
        transcript,
        spoken,
        this.config.historyTurns,
      );
    }

    speech.end();
  }

  /**
   * Forward one TTS response's PCM to the device. Unlike a full turn it emits
   * no audio.start / audio.end and does not touch the playback acknowledgement;
   * SpeechEmitter owns those so several sentences share one audio stream.
   *
   * @param {unknown} output
   * @returns {Promise<number>} bytes forwarded
   */
  async pipeTtsAudio(output) {
    let body = output;
    let contentType = "application/octet-stream";

    if (output instanceof Response) {
      if (!output.ok) {
        const error = new Error("TTS returned a non-success response.");
        error.status = output.status;
        throw error;
      }
      body = output.body;
      contentType = output.headers.get("content-type") || contentType;
    }

    if (contentType.toLowerCase().includes("json")) {
      throw new TypeError("TTS returned JSON instead of raw audio.");
    }

    let bytes = 0;

    if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) {
      if (!this.stopped && isOpen(this.downstream)) {
        const chunk = binaryView(body);
        bytes += chunk.byteLength;
        this.downstream.send(chunk);
      }
      return bytes;
    }

    if (body && typeof body.getReader === "function") {
      const reader = body.getReader();
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        if (this.stopped || !isOpen(this.downstream)) {
          await reader.cancel("device disconnected");
          return bytes;
        }

        const chunk = binaryView(value);
        if (!chunk) {
          await reader.cancel("invalid TTS audio chunk");
          throw new TypeError("TTS returned a non-binary audio chunk.");
        }
        if (chunk.byteLength > 0) {
          bytes += chunk.byteLength;
          this.downstream.send(chunk);
        }
      }
      return bytes;
    }

    throw new TypeError("TTS did not return a readable audio stream.");
  }

  /** @param {string} turnIndex */
  awaitPlayback(turnIndex) {
    this.awaitingPlaybackTurn = turnIndex;
    if (this.playbackTimer !== undefined) clearTimeout(this.playbackTimer);
    this.playbackTimer = setTimeout(() => {
      if (this.awaitingPlaybackTurn !== turnIndex || this.stopped) return;
      this.sendJson({
        type: "error",
        code: "playback_ack_timeout",
        message: "Audio playback was not acknowledged; the turn is closing.",
        turn: turnIndex,
      });
      this.finishPlayback(turnIndex);
    }, this.config.livePlaybackAckTimeoutMs);
    this.playbackTimer?.unref?.();
  }

  /** @param {string} turnIndex */
  finishPlayback(turnIndex) {
    if (this.awaitingPlaybackTurn !== turnIndex) return;
    this.awaitingPlaybackTurn = null;
    if (this.playbackTimer !== undefined) {
      clearTimeout(this.playbackTimer);
      this.playbackTimer = undefined;
    }
    this.completeSession(turnIndex);
  }

  /** @param {string} turnIndex */
  completeSession(turnIndex) {
    if (this.stopped) return;
    this.responding = false;
    this.sendJson({ type: "session.complete", turn: turnIndex });
    this.stop(true, 1000, "Voice turn complete");
  }

  /** @returns {Promise<void>} */
  whenIdle() {
    return this.turnQueue;
  }

  /**
   * @param {"llm" | "tts"} stage
   * @param {unknown} error
   * @param {string} turnIndex
   */
  sendStageError(stage, error, turnIndex) {
    const upstreamStatus = Number(error?.status);
    const rateLimited = upstreamStatus === 429;
    this.logFailure(`${stage}_failed`, error, {
      turn: turnIndex,
      upstreamStatus: Number.isFinite(upstreamStatus) ? upstreamStatus : undefined,
    });
    this.sendJson({
      type: "error",
      code: rateLimited ? "ai_budget_or_rate_limit" : `${stage}_failed`,
      message: rateLimited
        ? "AI usage limit reached. Try again later."
        : stage === "llm"
          ? "Claude could not answer the transcribed speech."
          : "Speech generation could not be completed.",
      ...(rateLimited ? { retry_after: 60 } : {}),
      turn: turnIndex,
    });
  }

  /** @param {Record<string, unknown>} message */
  sendJson(message) {
    if (!isOpen(this.downstream)) return false;
    try {
      this.downstream.send(JSON.stringify(message));
      return true;
    } catch (error) {
      this.logFailure("device_send_failed", error);
      return false;
    }
  }

  /**
   * @param {boolean} closeDownstream
   * @param {number} code
   * @param {string} reason
   */
  stop(closeDownstream, code, reason) {
    if (this.stopped) return;
    this.stopped = true;
    if (this.expirationTimer !== undefined) {
      clearTimeout(this.expirationTimer);
      this.expirationTimer = undefined;
    }
    if (this.playbackTimer !== undefined) {
      clearTimeout(this.playbackTimer);
      this.playbackTimer = undefined;
    }
    safeClose(this.upstream, code, reason);
    if (closeDownstream) safeClose(this.downstream, code, reason);
  }

  /**
   * @param {string} event
   * @param {unknown} error
   * @param {Record<string, unknown>} [metadata]
   */
  logFailure(event, error, metadata = {}) {
    console.error(
      JSON.stringify({
        event,
        errorName: error instanceof Error ? error.name : "UnknownError",
        deviceId: this.device.id,
        requestId: this.requestId,
        ...metadata,
      }),
    );
  }
}

// Turns a stream of model text deltas into spoken sentences. It buffers the
// tail until a sentence boundary (or a long unpunctuated run), sends each
// sentence to TTS as its own request, and streams that audio on the single
// shared audio.start / audio.end pair the device expects.
class SpeechEmitter {
  /**
   * @param {LiveSession} session
   * @param {string} turnIndex
   */
  constructor(session, turnIndex) {
    this.session = session;
    this.turnIndex = turnIndex;
    this.buffer = "";
    this.fullText = "";
    this.spokenChars = 0;
    this.totalBytes = 0;
    this.audioStarted = false;
  }

  /** @param {string} text */
  push(text) {
    if (!text) return;
    this.buffer += text;
    this.fullText += text;
  }

  /** @returns {string | null} the next complete sentence, or null */
  takeSentence() {
    const buffer = this.buffer;
    let cut = -1;
    for (let i = 0; i < buffer.length - 1; i += 1) {
      const character = buffer[i];
      if (
        (character === "." || character === "!" || character === "?") &&
        /\s/.test(buffer[i + 1])
      ) {
        cut = i + 1;
      }
    }
    // A long run with no sentence punctuation still has to be broken up, or the
    // first audio would wait for the whole reply. Split on a word boundary.
    if (cut === -1 && buffer.length >= 220) {
      const space = buffer.lastIndexOf(" ", 200);
      if (space >= 40) cut = space + 1;
    }
    if (cut === -1) return null;
    const sentence = buffer.slice(0, cut);
    this.buffer = buffer.slice(cut);
    return sentence;
  }

  /** Speak whatever text is still buffered after the stream ends. */
  async flushRemainder() {
    const tail = this.buffer;
    this.buffer = "";
    if (tail.trim()) await this.speak(tail);
  }

  /** @param {string} chunk */
  async speak(chunk) {
    const session = this.session;
    if (session.stopped || !isOpen(session.downstream)) return;

    const config = session.config;
    const text = chunk.replace(/\s+/g, " ").trim();
    if (!text) return;
    if (this.spokenChars >= config.maxTtsChars) return;

    const room = config.maxTtsChars - this.spokenChars;
    const speakText = text.length > room ? text.slice(0, room) : text;
    this.spokenChars += speakText.length;

    const ttsOutput = await session.env.AI.run(
      config.ttsModel,
      {
        text: speakText,
        speaker: config.ttsSpeaker,
        encoding: "linear16",
        container: "none",
        sample_rate: config.liveAudioSampleRate,
      },
      {
        returnRawResponse: true,
        ...aiOptions(
          config,
          session.device.id,
          session.requestId,
          "tts",
          this.turnIndex,
        ),
      },
    );

    if (!this.audioStarted) {
      session.sendJson({
        type: "audio.start",
        turn: this.turnIndex,
        encoding: "linear16",
        sample_rate: config.liveAudioSampleRate,
        channels: 1,
      });
      this.audioStarted = true;
    }

    this.totalBytes += await session.pipeTtsAudio(ttsOutput);

    // Cumulative text so the device screen tracks what has been spoken so far.
    session.sendJson({
      type: "assistant.text",
      turn: this.turnIndex,
      text: this.fullText.replace(/\s+/g, " ").trim(),
    });
  }

  /** Close the shared audio stream and hand off to the playback ack. */
  end() {
    const session = this.session;
    if (session.stopped || !this.audioStarted) return;
    session.awaitPlayback(this.turnIndex);
    session.sendJson({
      type: "audio.end",
      turn: this.turnIndex,
      bytes: this.totalBytes,
    });
  }
}

/**
 * Yield text pieces from a model result whether it is an SSE stream, a Response
 * wrapping one, or a already-complete object. Unrecognized event shapes yield
 * nothing so the caller can fall back to a non-streaming request.
 *
 * @param {unknown} result
 * @returns {AsyncGenerator<string>}
 */
async function* iterModelText(result) {
  let stream = null;
  if (result instanceof Response) stream = result.body;
  else if (result && typeof result.getReader === "function") stream = result;

  if (!stream || typeof stream.getReader !== "function") {
    const text = extractAssistantText(result);
    if (text) yield text;
    return;
  }

  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer +=
        typeof value === "string" ? value : decoder.decode(value, { stream: true });

      let newlineIndex;
      while ((newlineIndex = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, newlineIndex).trim();
        buffer = buffer.slice(newlineIndex + 1);
        if (!line.startsWith("data:")) continue;
        const payload = line.slice(5).trim();
        if (!payload || payload === "[DONE]") continue;
        let parsed;
        try {
          parsed = JSON.parse(payload);
        } catch {
          continue;
        }
        const piece = pickStreamDelta(parsed);
        if (piece) yield piece;
      }
    }
  } finally {
    try {
      reader.releaseLock();
    } catch {
      // Already released, or the stream errored; nothing to do.
    }
  }
}

/** @param {any} object */
function pickStreamDelta(object) {
  if (!object || typeof object !== "object") return "";
  // Workers AI native models.
  if (typeof object.response === "string") return object.response;
  // Anthropic content_block_delta.
  if (object.delta && typeof object.delta.text === "string") {
    return object.delta.text;
  }
  // OpenAI-compatible chunk shape.
  const choice = Array.isArray(object.choices) ? object.choices[0] : null;
  if (choice && choice.delta && typeof choice.delta.content === "string") {
    return choice.delta.content;
  }
  // A whole message object delivered as one event.
  if (Array.isArray(object.content)) {
    const text = object.content
      .filter((block) => block && typeof block.text === "string")
      .map((block) => block.text)
      .join("");
    if (text) return text;
  }
  return "";
}

/** @param {unknown} result */
export function extractAssistantText(result) {
  if (typeof result === "string") return result.trim();
  if (!result || typeof result !== "object") return "";

  if (Array.isArray(result.content)) {
    const text = result.content
      .filter((block) => block && typeof block.text === "string")
      .map((block) => block.text)
      .join("\n")
      .trim();
    if (text) return text;
  }

  for (const field of ["response", "output_text", "text"]) {
    if (typeof result[field] === "string" && result[field].trim()) {
      return result[field].trim();
    }
  }

  if (result.result && typeof result.result === "object") {
    return extractAssistantText(result.result);
  }

  return "";
}

/**
 * @param {string} text
 * @param {number} maxChars
 */
export function normalizeSpeechText(text, maxChars) {
  const singleLine = text.replace(/\s+/g, " ").trim();
  if (singleLine.length <= maxChars) return singleLine;

  const candidate = singleLine.slice(0, Math.max(1, maxChars - 1));
  const lastWhitespace = candidate.lastIndexOf(" ");
  const minimumUsefulBoundary = Math.floor(candidate.length * 0.75);
  const trimmed =
    lastWhitespace >= minimumUsefulBoundary
      ? candidate.slice(0, lastWhitespace)
      : candidate;
  return `${trimmed.trimEnd()}…`;
}

/**
 * @param {Record<string, any>} config
 * @param {string} deviceId
 * @param {string} requestId
 * @param {"stt" | "llm" | "tts"} stage
 * @param {string} [turn]
 */
function aiOptions(config, deviceId, requestId, stage, turn) {
  const metadata = {
    device_id: deviceId,
    request_id: requestId,
    pipeline_stage: stage,
  };
  if (turn !== undefined) metadata.turn = String(turn);

  return {
    gateway: {
      id: config.gatewayId,
      collectLog: config.collectLogs,
      metadata,
    },
  };
}

/** @param {unknown} value */
function binaryView(value) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  return null;
}

/** @param {unknown} value */
function textView(value) {
  if (typeof value === "string") return value;
  const bytes = binaryView(value);
  return bytes ? new TextDecoder().decode(bytes) : null;
}

/**
 * @param {unknown} value
 * @param {number} fallback
 */
function normalizeTurnIndex(value, fallback) {
  if (Number.isInteger(value) && value >= 0) return String(value);
  if (typeof value === "string" && /^[0-9]{1,12}$/.test(value)) return value;
  return String(fallback);
}

/** @param {WebSocket} socket */
function isOpen(socket) {
  return socket?.readyState === WEBSOCKET_OPEN;
}

/**
 * @param {WebSocket} socket
 * @param {number} code
 * @param {string} reason
 */
function safeClose(socket, code, reason) {
  if (!socket || socket.readyState >= 2) return;
  try {
    socket.close(code, reason.slice(0, 120));
  } catch {
    // The other side may have already closed between the state check and call.
  }
}
