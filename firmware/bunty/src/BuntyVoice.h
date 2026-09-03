#pragma once

#include <Arduino.h>
#include <driver/i2s.h>
#include <esp_websocket_client.h>
#include <freertos/FreeRTOS.h>
#include <freertos/stream_buffer.h>

class TapInput;

#ifndef IOT_GATEWAY_HOST
#define IOT_GATEWAY_HOST "bunty.underdogthinkers.com"
#endif

// Runs voice activity detection locally, opens the IoT Gateway WebSocket only
// for an utterance, streams that utterance, and plays the spoken reply.
//
// The gateway pipeline is half-duplex by design: the device sends `input.end`
// when its VAD observes the final pause, Flux is forced to finalize that audio,
// Claude answers, and Aura speaks. The gateway closes the turn only after
// Bunty acknowledges that the speaker and DMA buffers have drained.
//
// The microphone is not claimed directly. TapInput owns the capture I2S port
// and remains its only reader, publishing PCM for this class, so tap gestures
// keep working while voice is armed.
class BuntyVoice {
 public:
  enum class State {
    Disabled,   // No credential, or voice was never started.
    Offline,    // Wi-Fi is unavailable, or a socket is being cleaned up.
    Listening,  // Local VAD is armed; there is no cloud connection.
    Connecting, // Speech found; buffering while the socket opens.
    Uploading,  // Streaming the active utterance to Flux.
    Thinking,   // Turn finalized; Flux, Claude, or Aura is working.
    Speaking,   // Receiving and playing PCM.
    Draining,   // Playback finished locally, acknowledgement pending.
  };

  bool begin(TapInput *microphone, i2s_port_t speakerPort, int bclkPin,
             int lrcPin, int dinPin, int ampShutdownPin,
             const char *gatewayHost, const char *deviceId,
             const char *gatewayToken, const char *sessionId);
  void end();

  // Called from loop(). Opens a turn-scoped socket after local speech is found
  // and performs transport cleanup; the audio paths run on their own tasks.
  void service();

  // Arm local VAD for a short conversation window. Until armed, room noise is
  // ignored and no cloud socket is ever opened, so a tap is required to talk.
  // Each completed reply extends the window; it lapses after silence.
  void arm();
  bool armed() const { return armed_; }

  State state() const { return state_; }
  bool active() const {
    return state_ != State::Disabled && state_ != State::Offline;
  }
  // True while the amplifier is driving Bunty's reply, for speaking eyes.
  bool speaking() const { return state_ == State::Speaking; }

  // Smoothed loudness of the audio currently reaching the amplifier, 0-100.
  // Drives the mouth so it opens on Bunty's actual speech rather than a timer.
  uint8_t speechLevel() const { return speechLevel_; }
  uint32_t inputLevel() const { return vadLevel_; }
  uint32_t inputThreshold() const { return vadThreshold_; }

  // True while a turn is genuinely under way: speech is being heard, or the
  // gateway is working on or delivering a reply. Idle listening is excluded so
  // a quiet room can still let the display sleep normally.
  bool engaged() const;

  // Latest transcript and reply text, for on-screen diagnostics. Both are
  // empty until the gateway sends one.
  const String &transcript() const { return transcript_; }
  const String &reply() const { return reply_; }

 private:
  static void websocketEvent(void *handler, esp_event_base_t base,
                             int32_t eventId, void *eventData);
  static void uploadTask(void *context);
  static void playbackTask(void *context);

  void handleControl(const char *payload, size_t length);
  void handleAudio(const uint8_t *data, size_t length);
  void onConnected();
  void onDisconnected();

  bool openSpeaker();
  void closeSpeaker();
  void sendFrame(const uint8_t *frame);
  void pushPreRoll(const uint8_t *frame);
  void flushPreRoll();
  void resetGate();
  void sendJson(const char *body);
  void sendPlaybackFinished();
  void requestSocketCleanup();

  TapInput *microphone_ = nullptr;
  esp_websocket_client_handle_t client_ = nullptr;

  i2s_port_t speakerPort_ = I2S_NUM_MAX;
  int bclkPin_ = -1;
  int lrcPin_ = -1;
  int dinPin_ = -1;
  int ampShutdownPin_ = -1;
  bool speakerOpen_ = false;

  StreamBufferHandle_t playback_ = nullptr;
  uint8_t *playbackStorage_ = nullptr;
  StaticStreamBuffer_t playbackControl_ = {};

  TaskHandle_t uploadTask_ = nullptr;
  TaskHandle_t playbackTask_ = nullptr;
  volatile bool running_ = false;

  volatile State state_ = State::Disabled;
  volatile bool socketCleanupRequested_ = false;
  volatile bool audioEndSeen_ = false;
  volatile bool acknowledgePending_ = false;
  String turn_;
  String transcript_;
  String reply_;
  String headers_;
  String uri_;

  uint32_t connectingSince_ = 0;
  uint32_t speakingSince_ = 0;

  // Tap-to-talk. armed_ gates every escalation from Listening to Connecting;
  // disarmAt_ is the millis() deadline after which the window lapses.
  volatile bool armed_ = false;
  volatile uint32_t disarmAt_ = 0;

  // Device-side voice activity detection. Frames recorded before the gate
  // opens are kept so Flux still hears the start of the first word.
  uint8_t *preRoll_ = nullptr;
  size_t preRollFrames_ = 0;
  size_t preRollNext_ = 0;
  bool gateOpen_ = false;
  uint8_t loudFrames_ = 0;
  uint16_t quietFrames_ = 0;
  volatile uint32_t vadLevel_ = 0;
  volatile uint32_t vadThreshold_ = 0;
  volatile uint8_t speechLevel_ = 0;
};
