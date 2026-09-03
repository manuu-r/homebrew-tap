#pragma once

#include <Arduino.h>
#include <driver/i2s.h>
#include <freertos/FreeRTOS.h>
#include <freertos/queue.h>
#include <freertos/stream_buffer.h>

enum class TapDecision {
  Confirm,
  Cancel,
  TimedOut,
};

// Keep the signal evidence with each detection. Pairing only needs the event,
// while destructive runtime gestures can additionally demand a deliberate,
// enclosure-level knock instead of accepting every microphone impulse.
struct TapEvent {
  uint32_t occurredAt = 0;
  uint32_t level = 0;
  uint32_t noiseFloor = 0;
};

// Reads the digital MEMS microphone on the second I2S controller and turns
// short, high-energy impulses into debounced tap events. The detector tracks
// the room's noise floor, requires the signal to settle before rearming, and
// applies a refractory period so one physical knock is not counted twice.
class TapInput {
 public:
  bool begin(i2s_port_t port, int bclkPin, int wsPin, int dataPin);
  bool available() const { return available_; }

  // Non-blocking; returns true once for each detected physical tap.
  bool poll(TapEvent *event = nullptr);

  // One tap confirms after the double-tap window closes. A second tap inside
  // that window cancels immediately.
  TapDecision waitForPairingDecision(uint32_t timeoutMs = 30000,
                                    uint32_t doubleTapWindowMs = 700);

  void reset();
  void suppressFor(uint32_t milliseconds);

  // Voice capture shares the microphone with tap detection rather than
  // claiming the I2S port for itself. The detector task stays the single
  // reader and additionally publishes 16-bit mono PCM for the gateway
  // uploader, so taps keep working while Bunty is listening.
  bool startCapture(size_t ringBytes);
  void stopCapture();
  bool capturing() const { return capturing_; }

  // Blocks up to waitMs for microphone PCM. Returns the bytes written to dst.
  size_t readCapture(void *destination, size_t bytes, uint32_t waitMs);

 private:
  static void detectorTask(void *context);
  bool detectTap(TapEvent *event);
  uint32_t readLevel();

  i2s_port_t port_ = I2S_NUM_MAX;
  bool available_ = false;
  bool armed_ = false;
  QueueHandle_t tapQueue_ = nullptr;
  uint32_t noiseFloor_ = 2500;
  uint32_t lastTapAt_ = 0;
  uint32_t quietSince_ = 0;
  volatile uint32_t suppressedUntil_ = 0;

  StreamBufferHandle_t captureStream_ = nullptr;
  uint8_t *captureStorage_ = nullptr;
  StaticStreamBuffer_t captureControl_ = {};
  volatile bool capturing_ = false;
  // One of the two I2S slots carries the microphone and the other is silent.
  // -1 means the active slot has not been measured yet.
  int8_t captureSlot_ = -1;
  uint64_t slotEnergy_[2] = {0, 0};
  uint16_t slotProbeReads_ = 0;
};
