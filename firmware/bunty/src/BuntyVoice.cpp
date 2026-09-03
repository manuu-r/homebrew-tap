#include "BuntyVoice.h"

#include <ArduinoJson.h>
#include <WiFi.h>
#include <algorithm>
#include <math.h>
#include <esp_heap_caps.h>
#include <esp_tls.h>
#include <string.h>

#include "BuntyVoiceCerts.h"
#include "TapInput.h"

namespace {

// Both directions are signed 16-bit mono PCM at 16 kHz, matching
// LIVE_AUDIO_SAMPLE_RATE in the gateway's wrangler.jsonc.
constexpr uint32_t kSampleRate = 16000;
// 32 ms chunks: 512 samples, 1024 bytes. Small chunks keep VAD latency low.
constexpr size_t kFrameSamples = 512;
constexpr size_t kFrameBytes = kFrameSamples * sizeof(int16_t);
// VAD starts the network connection. Keep eight seconds in PSRAM so speech is
// not lost if detection happens while the main loop is inside a dashboard TCP
// timeout and then still has to complete TLS.
constexpr size_t kCaptureRingBytes = 307200;  // 300 chunks, ~9.6 s at 16 kHz.
// Aura can return a whole sentence faster than the speaker can play it. Six
// seconds in PSRAM prevents the event task from dropping a burst of reply PCM.
constexpr size_t kPlaybackRingBytes = 196608;

constexpr int kDmaBufferCount = 6;
constexpr int kDmaBufferLength = 256;
// The DMA holds kDmaBufferCount * kDmaBufferLength frames after the last
// i2s_write returns. Acknowledging before those samples reach the amplifier
// would unmute the microphone while Bunty is still audibly talking.
constexpr uint32_t kDmaDrainMs =
    (kDmaBufferCount * kDmaBufferLength * 1000) / kSampleRate + 40;

// Device-side voice activity detection. A wake word opens the conversation;
// this fixed-threshold RMS gate then decides, per utterance, when the device
// is speaking and when it has paused. No adaptive threshold, AGC, or noise
// calibration -- a flat threshold that has proven reliable in practice.
constexpr size_t kPreRollFrames = 5;  // ~160 ms, covers the detection latency.
constexpr uint32_t kFrameDurationMs = (kFrameSamples * 1000) / kSampleRate;
// Raw PCM RMS. Speech clears this comfortably; a quiet room does not.
constexpr uint32_t kVadRmsThreshold = 430;
// Open only after sustained energy so a single click cannot trip it (~96 ms).
constexpr uint8_t kVadStartFrames = 3;
// Close the utterance after this many consecutive quiet chunks (~160 ms), then
// send input.end so Flux finalizes it.
constexpr uint16_t kVadHangoverFrames = 5;

// Root-mean-square amplitude of one chunk.
uint32_t frameLevel(const uint8_t *frame) {
  const int16_t *samples = reinterpret_cast<const int16_t *>(frame);
  uint64_t sumSquares = 0;
  for (size_t i = 0; i < kFrameSamples; ++i) {
    const int32_t value = samples[i];
    sumSquares += static_cast<uint64_t>(value) * value;
  }
  return static_cast<uint32_t>(
      sqrtf(static_cast<float>(sumSquares / kFrameSamples)));
}

// A socket can open and then stall before session.ready if the gateway or the
// Flux leg never completes. Without a ceiling the client would wait forever.
constexpr uint32_t kConnectTimeoutMs = 15000;
// How long a tap keeps local VAD live. Every completed reply pushes this
// deadline forward so a back-and-forth exchange needs only the first tap.
constexpr uint32_t kConversationWindowMs = 10000;
constexpr int32_t kSpeakingVolumePercent = 70;

bool globalCaStoreReady = false;

bool ensureGlobalCaStore() {
  if (globalCaStoreReady) return true;
  if (esp_tls_init_global_ca_store() != ESP_OK) {
    Serial.println("[voice] could not initialize the TLS trust store");
    return false;
  }
  const size_t length = strlen(kIotGatewayRootCertificates) + 1;
  if (esp_tls_set_global_ca_store(
          reinterpret_cast<const unsigned char *>(kIotGatewayRootCertificates),
          length) != ESP_OK) {
    Serial.println("[voice] could not load the gateway root certificates");
    return false;
  }
  globalCaStoreReady = true;
  return true;
}

}  // namespace

bool BuntyVoice::begin(TapInput *microphone, i2s_port_t speakerPort,
                       int bclkPin, int lrcPin, int dinPin,
                       int ampShutdownPin, const char *gatewayHost,
                       const char *deviceId, const char *gatewayToken,
                       const char *sessionId) {
  if (running_) return true;

  if (!gatewayHost || gatewayHost[0] == '\0' || !deviceId ||
      deviceId[0] == '\0' || !gatewayToken || gatewayToken[0] == '\0') {
    Serial.println("[voice] no stored gateway credential; activation deferred");
    state_ = State::Disabled;
    return false;
  }
  if (!microphone || !microphone->available()) {
    Serial.println("[voice] microphone unavailable; voice stays disabled");
    state_ = State::Disabled;
    return false;
  }
  if (!ensureGlobalCaStore()) {
    state_ = State::Disabled;
    return false;
  }

  microphone_ = microphone;
  speakerPort_ = speakerPort;
  bclkPin_ = bclkPin;
  lrcPin_ = lrcPin;
  dinPin_ = dinPin;
  ampShutdownPin_ = ampShutdownPin;

  uri_ = String("wss://") + gatewayHost + "/v1/live";
  // esp_websocket_client expects a single CRLF-terminated header block. The
  // request ID is only ever a correlation aid; the gateway derives the device
  // identity from the token alone.
  headers_ = String("Authorization: Bearer ") + gatewayToken + "\r\n" +
             "X-Request-ID: " + deviceId + ":live\r\n";
  // Scopes the gateway's short-term conversation memory to this firmware
  // build. Omitted when unset so the gateway simply runs stateless.
  if (sessionId && sessionId[0] != '\0') {
    headers_ += String("X-Bunty-Session: ") + sessionId + "\r\n";
  }

  playbackStorage_ = static_cast<uint8_t *>(heap_caps_malloc(
      kPlaybackRingBytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT));
  if (!playbackStorage_) {
    playbackStorage_ = static_cast<uint8_t *>(malloc(kPlaybackRingBytes));
  }
  if (!playbackStorage_) {
    Serial.println("[voice] could not allocate the playback ring");
    state_ = State::Disabled;
    return false;
  }
  playback_ = xStreamBufferCreateStatic(kPlaybackRingBytes, 1,
                                        playbackStorage_, &playbackControl_);
  if (!playback_) {
    Serial.println("[voice] could not create the playback stream");
    free(playbackStorage_);
    playbackStorage_ = nullptr;
    state_ = State::Disabled;
    return false;
  }

  preRoll_ = static_cast<uint8_t *>(heap_caps_malloc(
      kPreRollFrames * kFrameBytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT));
  if (!preRoll_) {
    preRoll_ = static_cast<uint8_t *>(malloc(kPreRollFrames * kFrameBytes));
  }
  if (!preRoll_) {
    Serial.println("[voice] could not allocate the pre-roll buffer");
    end();
    return false;
  }

  if (!microphone_->startCapture(kCaptureRingBytes)) {
    Serial.println("[voice] could not start microphone capture");
    end();
    return false;
  }

  running_ = true;
  state_ = WiFi.status() == WL_CONNECTED ? State::Listening : State::Offline;
  socketCleanupRequested_ = false;
  connectingSince_ = 0;

  // Both audio tasks sit on core 0 with the Wi-Fi and WebSocket stacks; the
  // Arduino loop keeps core 1 for the display and tap gestures.
  if (xTaskCreatePinnedToCore(uploadTask, "voice-up", 4096, this, 4,
                              &uploadTask_, 0) != pdPASS) {
    Serial.println("[voice] could not start the upload task");
    end();
    return false;
  }
  if (xTaskCreatePinnedToCore(playbackTask, "voice-play", 4096, this, 5,
                              &playbackTask_, 0) != pdPASS) {
    Serial.println("[voice] could not start the playback task");
    end();
    return false;
  }

  Serial.printf("[voice] transport ready for %s as %s\n", uri_.c_str(),
                deviceId);
  Serial.println("[voice] local VAD armed; cloud socket closed");
  return true;
}

void BuntyVoice::end() {
  running_ = false;
  if (client_) {
    esp_websocket_client_stop(client_);
    // stop() returns early if the client already considers itself stopped, so
    // give its task a moment to leave the TLS teardown before the transport is
    // freed underneath it.
    delay(150);
    esp_websocket_client_destroy(client_);
    client_ = nullptr;
  }
  // Let both tasks observe running_ and retire before their buffers go away.
  delay(120);
  closeSpeaker();
  if (microphone_) microphone_->stopCapture();
  if (playback_) {
    vStreamBufferDelete(playback_);
    playback_ = nullptr;
  }
  if (playbackStorage_) {
    free(playbackStorage_);
    playbackStorage_ = nullptr;
  }
  if (preRoll_) {
    free(preRoll_);
    preRoll_ = nullptr;
  }
  socketCleanupRequested_ = false;
  connectingSince_ = 0;
  state_ = State::Disabled;
}

void BuntyVoice::arm() {
  disarmAt_ = millis() + kConversationWindowMs;
  if (!armed_) {
    armed_ = true;
    resetGate();
    Serial.println("[voice] armed by tap; listening for a request");
  }
}

void BuntyVoice::service() {
  if (!running_) return;

  // Drop the arm once the window lapses, but never mid-turn.
  if (armed_ && (state_ == State::Listening || state_ == State::Offline) &&
      static_cast<int32_t>(millis() - disarmAt_) >= 0) {
    armed_ = false;
    Serial.println("[voice] conversation window closed; tap to talk again");
  }

  // Transport teardown is driven from loop(), never from the WebSocket event
  // task where esp_websocket_client_stop() would deadlock.
  if (socketCleanupRequested_) {
    socketCleanupRequested_ = false;
    state_ = State::Offline;
    if (client_) esp_websocket_client_stop(client_);
    connectingSince_ = 0;
    audioEndSeen_ = false;
    acknowledgePending_ = false;
    closeSpeaker();
    resetGate();
    if (WiFi.status() == WL_CONNECTED) {
      state_ = State::Listening;
      if (armed_) disarmAt_ = millis() + kConversationWindowMs;
      Serial.println("[voice] turn complete; still listening for a reply");
    }
    return;
  }

  if (WiFi.status() != WL_CONNECTED) {
    if (state_ != State::Offline) {
      Serial.println("[voice] Wi-Fi unavailable; suspending voice");
      if (state_ == State::Connecting || state_ == State::Uploading ||
          state_ == State::Thinking || state_ == State::Speaking ||
          state_ == State::Draining) {
        requestSocketCleanup();
      } else {
        state_ = State::Offline;
      }
    }
    return;
  }

  if (state_ == State::Offline) {
    resetGate();
    state_ = State::Listening;
    Serial.println("[voice] local VAD armed; cloud socket closed");
    return;
  }
  if (state_ != State::Connecting) return;

  if (connectingSince_ != 0) {
    if (millis() - connectingSince_ > kConnectTimeoutMs) {
      Serial.println("[voice] no session.ready within timeout; turn aborted");
      requestSocketCleanup();
    }
    return;
  }

  connectingSince_ = millis();

  if (client_) {
    // Each utterance gets a fresh WebSocket session, but the stopped handle is
    // reusable and avoids heap churn in esp-tls.
    if (esp_websocket_client_start(client_) != ESP_OK) {
      Serial.println("[voice] could not reopen the gateway WebSocket");
      requestSocketCleanup();
      return;
    }
    Serial.println("[voice] speech buffered; opening gateway socket");
    return;
  }

  esp_websocket_client_config_t config = {};
  config.uri = uri_.c_str();
  config.headers = headers_.c_str();
  config.use_global_ca_store = true;
  // An utterance owns exactly one session; retries require fresh local speech.
  config.disable_auto_reconnect = true;
  config.buffer_size = 4096;
  config.task_stack = 6144;
  config.task_prio = 5;
  config.ping_interval_sec = 10;
  config.pingpong_timeout_sec = 25;

  client_ = esp_websocket_client_init(&config);
  if (!client_) {
    Serial.println("[voice] could not initialize the WebSocket client");
    requestSocketCleanup();
    return;
  }
  esp_websocket_register_events(client_, WEBSOCKET_EVENT_ANY, websocketEvent,
                                this);
  if (esp_websocket_client_start(client_) != ESP_OK) {
    Serial.println("[voice] could not open the gateway WebSocket");
    esp_websocket_client_destroy(client_);
    client_ = nullptr;
    requestSocketCleanup();
    return;
  }
  Serial.printf("[voice] speech buffered; opening %s\n", uri_.c_str());
}

void BuntyVoice::requestSocketCleanup() {
  if (state_ == State::Disabled) return;
  socketCleanupRequested_ = true;
  state_ = State::Offline;
}

void BuntyVoice::websocketEvent(void *handler, esp_event_base_t base,
                                int32_t eventId, void *eventData) {
  auto *voice = static_cast<BuntyVoice *>(handler);
  auto *data = static_cast<esp_websocket_event_data_t *>(eventData);
  if (!voice || !voice->running_) return;

  switch (eventId) {
    case WEBSOCKET_EVENT_CONNECTED:
      voice->onConnected();
      break;
    case WEBSOCKET_EVENT_DATA:
      if (!data || data->data_len <= 0) break;
      if (data->op_code == 0x2 || data->op_code == 0x0) {
        // Binary, or a continuation of one: Aura's linear16 reply.
        voice->handleAudio(reinterpret_cast<const uint8_t *>(data->data_ptr),
                           static_cast<size_t>(data->data_len));
      } else if (data->op_code == 0x1) {
        voice->handleControl(data->data_ptr,
                             static_cast<size_t>(data->data_len));
      }
      break;
    case WEBSOCKET_EVENT_ERROR:
      // Usually TLS: a rejected certificate chain or a refused handshake.
      // Raise CORE_DEBUG_LEVEL to see esp-tls report the specific cause.
      Serial.println("[voice] websocket transport error");
      voice->onDisconnected();
      break;
    case WEBSOCKET_EVENT_DISCONNECTED:
      Serial.println("[voice] websocket disconnected");
      voice->onDisconnected();
      break;
    case WEBSOCKET_EVENT_CLOSED:
      Serial.println("[voice] websocket closed by gateway");
      voice->onDisconnected();
      break;
    default:
      break;
  }
}

void BuntyVoice::onConnected() {
  Serial.println("[voice] gateway socket open");
  // State stays Connecting until session.ready. The capture ring continues to
  // retain speech recorded during this handshake.
  state_ = State::Connecting;
}

void BuntyVoice::onDisconnected() {
  if (state_ == State::Disabled || state_ == State::Offline ||
      state_ == State::Listening || socketCleanupRequested_) {
    return;
  }
  Serial.println("[voice] gateway socket closed");
  requestSocketCleanup();
}

void BuntyVoice::handleControl(const char *payload, size_t length) {
  JsonDocument document;
  if (deserializeJson(document, payload, length) != DeserializationError::Ok) {
    return;
  }
  const char *type = document["type"] | "";
  if (!document["turn"].isNull()) turn_ = document["turn"].as<String>();
  if (state_ == State::Connecting) {
    // Anything arriving before session.ready is worth seeing; a refusal shows
    // up here as an error frame rather than a silent stall.
    Serial.printf("[voice] pre-session control: %s\n", type);
  }

  if (strcmp(type, "session.ready") == 0) {
    connectingSince_ = 0;
    transcript_ = "";
    reply_ = "";
    state_ = State::Uploading;
    Serial.println("[voice] session ready; streaming buffered speech");
    return;
  }
  if (strcmp(type, "transcript.partial") == 0 ||
      strcmp(type, "transcript.final") == 0) {
    transcript_ = document["text"] | "";
    if (strcmp(type, "transcript.final") == 0) {
      Serial.printf("[voice] heard: %s\n", transcript_.c_str());
    }
    return;
  }
  if (strcmp(type, "input.pause") == 0) {
    // Stop uploading immediately. Everything the microphone records from here
    // until input.resume is Bunty's own voice.
    state_ = State::Thinking;
    return;
  }
  if (strcmp(type, "assistant.thinking") == 0) {
    state_ = State::Thinking;
    return;
  }
  if (strcmp(type, "assistant.text") == 0) {
    reply_ = document["text"] | "";
    Serial.printf("[voice] reply: %s\n", reply_.c_str());
    return;
  }
  if (strcmp(type, "audio.start") == 0) {
    audioEndSeen_ = false;
    if (openSpeaker()) {
      acknowledgePending_ = true;
      speakingSince_ = millis();
      state_ = State::Speaking;
    }
    return;
  }
  if (strcmp(type, "audio.end") == 0) {
    // The playback task owes the acknowledgement once the ring and the DMA
    // have both emptied.
    audioEndSeen_ = true;
    return;
  }
  if (strcmp(type, "session.complete") == 0) {
    Serial.println("[voice] turn complete");
    requestSocketCleanup();
    return;
  }
  if (strcmp(type, "input.resume") == 0) {
    // Compatibility with an older gateway: this firmware uses one socket per
    // utterance, so a resume means that turn is complete and the socket ends.
    requestSocketCleanup();
    return;
  }
  if (strcmp(type, "error") == 0) {
    const char *code = document["code"] | "unknown";
    Serial.printf("[voice] gateway error %s\n", code);
    requestSocketCleanup();
    return;
  }
}

void BuntyVoice::handleAudio(const uint8_t *data, size_t length) {
  if (!playback_ || !data || length == 0) return;
  // A short timeout rather than a drop: the speaker consumes at a fixed rate,
  // so brief backpressure is normal and discarding would clip Bunty's reply.
  xStreamBufferSend(playback_, data, length, pdMS_TO_TICKS(200));
}

bool BuntyVoice::openSpeaker() {
  if (speakerOpen_) return true;

  i2s_config_t config = {};
  config.mode = static_cast<i2s_mode_t>(I2S_MODE_MASTER | I2S_MODE_TX);
  config.sample_rate = kSampleRate;
  config.bits_per_sample = I2S_BITS_PER_SAMPLE_16BIT;
  config.channel_format = I2S_CHANNEL_FMT_RIGHT_LEFT;
  config.communication_format = I2S_COMM_FORMAT_STAND_I2S;
  config.intr_alloc_flags = ESP_INTR_FLAG_LEVEL1;
  config.dma_buf_count = kDmaBufferCount;
  config.dma_buf_len = kDmaBufferLength;
  config.tx_desc_auto_clear = true;
  if (i2s_driver_install(speakerPort_, &config, 0, nullptr) != ESP_OK) {
    Serial.println("[voice] could not install the playback I2S driver");
    return false;
  }

  i2s_pin_config_t pins = {};
  pins.mck_io_num = I2S_PIN_NO_CHANGE;
  pins.bck_io_num = bclkPin_;
  pins.ws_io_num = lrcPin_;
  pins.data_out_num = dinPin_;
  pins.data_in_num = I2S_PIN_NO_CHANGE;
  if (i2s_set_pin(speakerPort_, &pins) != ESP_OK) {
    Serial.println("[voice] could not route playback I2S to the amplifier");
    i2s_driver_uninstall(speakerPort_);
    return false;
  }

  digitalWrite(ampShutdownPin_, HIGH);
  delay(10);
  speakerOpen_ = true;
  return true;
}

void BuntyVoice::closeSpeaker() {
  speechLevel_ = 0;
  if (!speakerOpen_) return;
  i2s_zero_dma_buffer(speakerPort_);
  // The amplifier idles shut down so it does not hiss between replies.
  digitalWrite(ampShutdownPin_, LOW);
  i2s_driver_uninstall(speakerPort_);
  speakerOpen_ = false;
}

void BuntyVoice::sendJson(const char *body) {
  if (!client_ || !esp_websocket_client_is_connected(client_)) return;
  esp_websocket_client_send_text(client_, body, strlen(body),
                                 pdMS_TO_TICKS(1000));
}

void BuntyVoice::sendPlaybackFinished() {
  String body = "{\"type\":\"playback.finished\"";
  if (turn_.length() > 0) {
    body += ",\"turn\":\"" + turn_ + "\"";
  }
  body += "}";
  sendJson(body.c_str());
}

bool BuntyVoice::engaged() const {
  return state_ == State::Connecting || state_ == State::Uploading ||
         state_ == State::Thinking || state_ == State::Speaking ||
         state_ == State::Draining;
}

void BuntyVoice::sendFrame(const uint8_t *frame) {
  if (!client_ || !esp_websocket_client_is_connected(client_)) return;
  esp_websocket_client_send_bin(client_,
                                reinterpret_cast<const char *>(frame),
                                static_cast<int>(kFrameBytes),
                                pdMS_TO_TICKS(1000));
}

void BuntyVoice::pushPreRoll(const uint8_t *frame) {
  if (!preRoll_) return;
  memcpy(preRoll_ + preRollNext_ * kFrameBytes, frame, kFrameBytes);
  preRollNext_ = (preRollNext_ + 1) % kPreRollFrames;
  if (preRollFrames_ < kPreRollFrames) ++preRollFrames_;
}

void BuntyVoice::flushPreRoll() {
  if (!preRoll_) return;
  // Oldest first. Until the ring has wrapped the oldest frame is at index 0;
  // afterwards it is wherever the next write would land.
  const size_t start = preRollFrames_ == kPreRollFrames ? preRollNext_ : 0;
  for (size_t i = 0; i < preRollFrames_; ++i) {
    sendFrame(preRoll_ + ((start + i) % kPreRollFrames) * kFrameBytes);
  }
  preRollFrames_ = 0;
  preRollNext_ = 0;
}

void BuntyVoice::resetGate() {
  // The noise floor deliberately survives: it describes the room, not the
  // turn, and relearning it from scratch would gate off the first words.
  gateOpen_ = false;
  preRollFrames_ = 0;
  preRollNext_ = 0;
  loudFrames_ = 0;
  quietFrames_ = 0;
}

void BuntyVoice::uploadTask(void *context) {
  auto *voice = static_cast<BuntyVoice *>(context);
  auto *frame = static_cast<uint8_t *>(malloc(kFrameBytes));
  if (!frame) {
    Serial.println("[voice] could not allocate the upload frame");
    voice->uploadTask_ = nullptr;
    vTaskDelete(nullptr);
    return;
  }

  bool vadWasArmed = false;
  bool streamStarted = false;
  while (voice->running_) {
    const State state = voice->state_;

    if (state == State::Listening && !vadWasArmed) {
      // Audio captured during the previous reply is speaker echo. Discard it
      // before locally arming for the next independent utterance.
      uint8_t scratch[512];
      while (voice->microphone_->readCapture(scratch, sizeof(scratch), 0) > 0) {
      }
      voice->resetGate();
      vadWasArmed = true;
      streamStarted = false;
    }

    if (state == State::Connecting) {
      // TapInput continues filling its large PSRAM capture ring while TLS and
      // the gateway's Flux connection open. Do not consume those samples yet.
      vadWasArmed = false;
      delay(10);
      continue;
    }

    if (state != State::Listening && state != State::Uploading) {
      vadWasArmed = false;
      streamStarted = false;
      delay(20);
      continue;
    }

    if (state == State::Uploading && !streamStarted) {
      // The pre-roll includes the frame that tripped VAD. Frames spoken during
      // the handshake follow it in TapInput's capture ring.
      voice->flushPreRoll();
      streamStarted = true;
    }

    size_t filled = 0;
    while (filled < kFrameBytes && voice->state_ == state && voice->running_) {
      const size_t received = voice->microphone_->readCapture(
          frame + filled, kFrameBytes - filled, 250);
      if (received == 0) break;
      filled += received;
    }
    if (filled < kFrameBytes || voice->state_ != state) continue;

    const uint32_t level = frameLevel(frame);
    const uint32_t threshold = kVadRmsThreshold;
    voice->vadLevel_ = level;
    voice->vadThreshold_ = threshold;

    if (state == State::Listening) {
      if (level >= threshold) {
        // Room noise is tracked for the pre-roll but only a tap opens the gate,
        // so Bunty never streams the room to the cloud unprompted.
        if (voice->armed_) {
          if (voice->loudFrames_ < UINT8_MAX) ++voice->loudFrames_;
          voice->pushPreRoll(frame);
          if (voice->loudFrames_ >= kVadStartFrames) {
            voice->gateOpen_ = true;
            voice->quietFrames_ = 0;
            // The pre-roll retains the complete multi-frame onset. Nothing is
            // sent until session.ready confirms the Flux socket exists.
            voice->connectingSince_ = 0;
            voice->state_ = State::Connecting;
            Serial.printf(
                "[voice] speech detected locally (level %u, threshold %u)\n",
                static_cast<unsigned>(level), static_cast<unsigned>(threshold));
          }
        } else {
          voice->pushPreRoll(frame);
        }
      } else {
        voice->loudFrames_ = 0;
        voice->pushPreRoll(frame);
      }
      continue;
    }

    if (level >= threshold) {
      voice->quietFrames_ = 0;
    } else if (voice->quietFrames_ < UINT16_MAX) {
      ++voice->quietFrames_;
    }

    // Include the complete pause in the PCM stream, then end it explicitly.
    voice->sendFrame(frame);
    if (voice->quietFrames_ >= kVadHangoverFrames) {
      voice->gateOpen_ = false;
      voice->sendJson("{\"type\":\"input.end\"}");
      voice->state_ = State::Thinking;
      Serial.printf("[voice] local pause detected after %u ms; stream ended\n",
                    static_cast<unsigned>(kVadHangoverFrames *
                                          kFrameDurationMs));
    }
  }

  free(frame);
  voice->uploadTask_ = nullptr;
  vTaskDelete(nullptr);
}

void BuntyVoice::playbackTask(void *context) {
  auto *voice = static_cast<BuntyVoice *>(context);
  constexpr size_t kChunkSamples = 320;
  auto *mono = static_cast<int16_t *>(malloc(kChunkSamples * sizeof(int16_t)));
  auto *stereo =
      static_cast<int16_t *>(malloc(kChunkSamples * 2 * sizeof(int16_t)));
  if (!mono || !stereo) {
    Serial.println("[voice] could not allocate the playback scratch buffers");
    free(mono);
    free(stereo);
    voice->playbackTask_ = nullptr;
    vTaskDelete(nullptr);
    return;
  }

  while (voice->running_) {
    size_t received = 0;
    if (voice->playback_) {
      received = xStreamBufferReceive(voice->playback_, mono,
                                      kChunkSamples * sizeof(int16_t),
                                      pdMS_TO_TICKS(40));
    }

    if (received > 0) {
      const size_t samples = received / sizeof(int16_t);
      uint64_t total = 0;
      for (size_t i = 0; i < samples; ++i) {
        const int32_t value = mono[i];
        total += static_cast<uint32_t>(value < 0 ? -value : value);
      }
      // Mapped against a ceiling well below full scale so conversational
      // speech uses most of the mouth's travel instead of barely opening it.
      uint32_t openness = static_cast<uint32_t>(total / samples) * 100 / 2500;
      if (openness > 100) openness = 100;
      // Smoothed, or the mouth flutters on every glottal pulse.
      voice->speechLevel_ = static_cast<uint8_t>(
          (static_cast<uint32_t>(voice->speechLevel_) * 2 + openness) / 3);

      for (size_t i = 0; i < samples; ++i) {
        // SD tied high makes the MAX98357 average both slots, so each mono
        // sample goes to left and right to keep full output level.
        const int16_t value = static_cast<int16_t>(
            static_cast<int32_t>(mono[i]) * kSpeakingVolumePercent / 100);
        stereo[i * 2] = value;
        stereo[i * 2 + 1] = value;
      }
      if (voice->speakerOpen_) {
        size_t written = 0;
        i2s_write(voice->speakerPort_, stereo, samples * 2 * sizeof(int16_t),
                  &written, portMAX_DELAY);
      }
      continue;
    }

    // The ring is empty. Only acknowledge once the gateway has said the reply
    // is complete, otherwise a momentary network gap would unmute the
    // microphone in the middle of Bunty's sentence.
    if (voice->audioEndSeen_ && voice->acknowledgePending_) {
      delay(kDmaDrainMs);
      voice->closeSpeaker();
      voice->audioEndSeen_ = false;
      voice->acknowledgePending_ = false;
      voice->state_ = State::Draining;
      voice->sendPlaybackFinished();
      // The 10 s conversation window is measured from here -- the moment the
      // reply has physically finished playing -- not from when the turn began.
      if (voice->armed_) voice->disarmAt_ = millis() + kConversationWindowMs;
      Serial.printf("[voice] turn %s played in %u ms\n",
                    voice->turn_.length() ? voice->turn_.c_str() : "?",
                    static_cast<unsigned>(millis() - voice->speakingSince_));
    }
  }

  free(mono);
  free(stereo);
  voice->playbackTask_ = nullptr;
  vTaskDelete(nullptr);
}
