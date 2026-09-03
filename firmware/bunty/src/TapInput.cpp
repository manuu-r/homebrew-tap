#include "TapInput.h"

#include <algorithm>
#include <esp_heap_caps.h>

namespace {

constexpr uint32_t kSampleRate = 16000;
constexpr uint32_t kMinimumTapLevel = 24000;
constexpr uint32_t kNoiseMultiplier = 7;
constexpr uint32_t kRefractoryMs = 150;
constexpr uint32_t kQuietToRearmMs = 70;
// The MEMS microphone's useful speech occupies only a small part of the
// signed 24-bit slot. After converting to linear16, restore 18 dB of level for
// VAD and STT; saturating arithmetic protects close taps and loud speech.
constexpr int32_t kVoiceCaptureGain = 8;
// Roughly 150 ms of audio is enough to tell the live microphone slot from the
// silent one without delaying the first spoken word noticeably.
constexpr uint16_t kSlotProbeReads = 50;

bool deadlinePassed(uint32_t now, uint32_t deadline) {
  return static_cast<int32_t>(now - deadline) >= 0;
}

}  // namespace

bool TapInput::begin(i2s_port_t port, int bclkPin, int wsPin, int dataPin) {
  port_ = port;
  i2s_config_t config = {};
  config.mode = static_cast<i2s_mode_t>(I2S_MODE_MASTER | I2S_MODE_RX);
  config.sample_rate = kSampleRate;
  config.bits_per_sample = I2S_BITS_PER_SAMPLE_32BIT;
  // A board may strap the microphone to either slot. Reading both lets the
  // level detector use whichever contains samples without a firmware option.
  config.channel_format = I2S_CHANNEL_FMT_RIGHT_LEFT;
  config.communication_format = I2S_COMM_FORMAT_STAND_I2S;
  config.intr_alloc_flags = ESP_INTR_FLAG_LEVEL1;
  config.dma_buf_count = 6;
  config.dma_buf_len = 64;
  config.use_apll = false;
  config.tx_desc_auto_clear = false;
  config.fixed_mclk = 0;
  if (i2s_driver_install(port_, &config, 0, nullptr) != ESP_OK) {
    Serial.println("[tap] could not install microphone I2S driver");
    return false;
  }

  i2s_pin_config_t pins = {};
  pins.mck_io_num = I2S_PIN_NO_CHANGE;
  pins.bck_io_num = bclkPin;
  pins.ws_io_num = wsPin;
  pins.data_out_num = I2S_PIN_NO_CHANGE;
  pins.data_in_num = dataPin;
  if (i2s_set_pin(port_, &pins) != ESP_OK) {
    Serial.println("[tap] could not route microphone I2S pins");
    i2s_driver_uninstall(port_);
    return false;
  }

  tapQueue_ = xQueueCreate(8, sizeof(TapEvent));
  if (!tapQueue_) {
    Serial.println("[tap] could not create detector queue");
    i2s_driver_uninstall(port_);
    return false;
  }
  available_ = true;
  reset();
  if (xTaskCreatePinnedToCore(detectorTask, "tap-detector", 3072, this, 2,
                              nullptr, 1) != pdPASS) {
    Serial.println("[tap] could not start detector task");
    available_ = false;
    vQueueDelete(tapQueue_);
    tapQueue_ = nullptr;
    i2s_driver_uninstall(port_);
    return false;
  }
  Serial.println("[tap] microphone detector ready");
  return true;
}

uint32_t TapInput::readLevel() {
  if (!available_) return 0;

  int32_t samples[96];
  size_t bytesRead = 0;
  // Capture cannot afford dropped samples, so the read blocks and the DMA
  // paces the detector loop. Tap-only operation keeps the original
  // poll-and-sleep cadence so wake and reset thresholds behave as before.
  const TickType_t wait = capturing_ ? pdMS_TO_TICKS(20) : 0;
  if (i2s_read(port_, samples, sizeof(samples), &bytesRead, wait) != ESP_OK ||
      bytesRead == 0) {
    return 0;
  }

  uint32_t peak = 0;
  const size_t count = bytesRead / sizeof(samples[0]);
  int16_t mono[sizeof(samples) / sizeof(samples[0]) / 2];
  size_t monoCount = 0;
  const bool capturing = capturing_;

  for (size_t i = 0; i < count; ++i) {
    // Common I2S MEMS microphones deliver a signed 24-bit sample left-aligned
    // in this 32-bit slot. Shifting keeps the adaptive thresholds readable.
    const int32_t sample = samples[i] >> 8;
    // Shifting the signed 24-bit sample makes negation safe for every value.
    const uint32_t magnitude =
        static_cast<uint32_t>(sample < 0 ? -sample : sample);
    peak = std::max(peak, magnitude);

    if (!capturing) continue;

    // I2S_CHANNEL_FMT_RIGHT_LEFT interleaves two slots per frame. While the
    // active slot is still unknown, total each one's energy instead of
    // uploading what may be a channel of silence.
    if (captureSlot_ < 0) {
      slotEnergy_[i & 1] += magnitude;
    } else if (static_cast<int8_t>(i & 1) == captureSlot_) {
      // 24-bit down to the linear16 the gateway expects.
      int32_t voiceSample = (sample >> 8) * kVoiceCaptureGain;
      voiceSample = std::max<int32_t>(INT16_MIN,
                                      std::min<int32_t>(INT16_MAX, voiceSample));
      mono[monoCount++] = static_cast<int16_t>(voiceSample);
    }
  }

  if (capturing && captureSlot_ < 0 && ++slotProbeReads_ >= kSlotProbeReads) {
    captureSlot_ = slotEnergy_[1] > slotEnergy_[0] ? 1 : 0;
    Serial.printf("[tap] microphone on I2S slot %d\n",
                  static_cast<int>(captureSlot_));
  }

  if (capturing && captureStream_ && monoCount > 0) {
    // Dropping on a full buffer is deliberate: a stalled uploader must not
    // block the detector task and stall tap detection with it.
    xStreamBufferSend(captureStream_, mono, monoCount * sizeof(mono[0]), 0);
  }
  return peak;
}

bool TapInput::startCapture(size_t ringBytes) {
  if (!available_ || capturing_) return capturing_;

  // PSRAM keeps the ring away from the internal heap the display framebuffer,
  // Wi-Fi, and the TLS session are already competing for.
  captureStorage_ = static_cast<uint8_t *>(
      heap_caps_malloc(ringBytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT));
  if (!captureStorage_) {
    captureStorage_ = static_cast<uint8_t *>(malloc(ringBytes));
  }
  if (!captureStorage_) {
    Serial.println("[tap] could not allocate the capture ring");
    return false;
  }

  captureStream_ = xStreamBufferCreateStatic(ringBytes, 1, captureStorage_,
                                             &captureControl_);
  if (!captureStream_) {
    Serial.println("[tap] could not create the capture stream");
    free(captureStorage_);
    captureStorage_ = nullptr;
    return false;
  }

  captureSlot_ = -1;
  slotEnergy_[0] = 0;
  slotEnergy_[1] = 0;
  slotProbeReads_ = 0;
  capturing_ = true;
  Serial.printf("[tap] microphone capture started (%u byte ring)\n",
                static_cast<unsigned>(ringBytes));
  return true;
}

void TapInput::stopCapture() {
  if (!capturing_) return;
  capturing_ = false;
  // The detector task may be inside readLevel(); let it finish before the
  // stream and its storage go away.
  delay(30);
  if (captureStream_) {
    vStreamBufferDelete(captureStream_);
    captureStream_ = nullptr;
  }
  if (captureStorage_) {
    free(captureStorage_);
    captureStorage_ = nullptr;
  }
  Serial.println("[tap] microphone capture stopped");
}

size_t TapInput::readCapture(void *destination, size_t bytes, uint32_t waitMs) {
  if (!capturing_ || !captureStream_) return 0;
  return xStreamBufferReceive(captureStream_, destination, bytes,
                              pdMS_TO_TICKS(waitMs));
}

bool TapInput::detectTap(TapEvent *event) {
  const uint32_t level = readLevel();
  if (level == 0) return false;

  const uint32_t now = millis();
  const uint32_t threshold =
      std::max(kMinimumTapLevel, noiseFloor_ * kNoiseMultiplier);

  // Only quiet samples influence the baseline; otherwise a knock would raise
  // its own threshold and make the next tap hard to detect.
  if (level < threshold) {
    noiseFloor_ = (noiseFloor_ * 31 + level) / 32;
  }

  if (level < threshold / 2) {
    if (quietSince_ == 0) quietSince_ = now;
    if (now - quietSince_ >= kQuietToRearmMs) armed_ = true;
  } else {
    quietSince_ = 0;
  }

  if (!armed_ || !deadlinePassed(now, suppressedUntil_) ||
      now - lastTapAt_ < kRefractoryMs || level < threshold) {
    return false;
  }

  armed_ = false;
  lastTapAt_ = now;
  Serial.printf("[tap] impulse %u (floor %u)\n",
                static_cast<unsigned>(level),
                static_cast<unsigned>(noiseFloor_));
  if (event) {
    event->occurredAt = now;
    event->level = level;
    event->noiseFloor = noiseFloor_;
  }
  return true;
}

void TapInput::detectorTask(void *context) {
  auto *input = static_cast<TapInput *>(context);
  while (input->available_) {
    TapEvent event;
    if (input->detectTap(&event)) {
      xQueueSend(input->tapQueue_, &event, 0);
    }
    // The blocking read inside detectTap() paces the loop while capturing;
    // sleeping here as well would drop microphone samples.
    if (!input->capturing_) delay(4);
  }
  vTaskDelete(nullptr);
}

bool TapInput::poll(TapEvent *event) {
  if (!available_ || !tapQueue_) return false;
  TapEvent received;
  if (xQueueReceive(tapQueue_, &received, 0) != pdTRUE) return false;
  if (event) *event = received;
  return true;
}

TapDecision TapInput::waitForPairingDecision(uint32_t timeoutMs,
                                             uint32_t doubleTapWindowMs) {
  if (!available_) return TapDecision::TimedOut;

  reset();
  const uint32_t startedAt = millis();
  uint32_t firstTapAt = 0;
  while (true) {
    const uint32_t now = millis();
    if (now - startedAt >= timeoutMs) return TapDecision::TimedOut;
    if (poll()) {
      if (firstTapAt == 0) {
        firstTapAt = now;
      } else if (now - firstTapAt <= doubleTapWindowMs) {
        return TapDecision::Cancel;
      }
    }
    if (firstTapAt != 0 && now - firstTapAt > doubleTapWindowMs) {
      return TapDecision::Confirm;
    }
    delay(8);
  }
}

void TapInput::reset() {
  if (!available_) return;
  if (tapQueue_) {
    TapEvent discarded;
    while (xQueueReceive(tapQueue_, &discarded, 0) == pdTRUE) {
    }
  }
  suppressFor(250);
}

void TapInput::suppressFor(uint32_t milliseconds) {
  suppressedUntil_ = millis() + milliseconds;
}
