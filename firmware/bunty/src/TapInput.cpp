#include "TapInput.h"

#include <algorithm>
#include <limits>

namespace {

constexpr uint32_t kSampleRate = 16000;
constexpr uint32_t kMinimumTapLevel = 24000;
constexpr uint32_t kNoiseMultiplier = 7;
constexpr uint32_t kRefractoryMs = 150;
constexpr uint32_t kQuietToRearmMs = 70;

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
  if (i2s_read(port_, samples, sizeof(samples), &bytesRead, 0) != ESP_OK ||
      bytesRead == 0) {
    return 0;
  }

  uint32_t peak = 0;
  const size_t count = bytesRead / sizeof(samples[0]);
  for (size_t i = 0; i < count; ++i) {
    // Common I2S MEMS microphones deliver a signed 24-bit sample left-aligned
    // in this 32-bit slot. Shifting keeps the adaptive thresholds readable.
    const int32_t sample = samples[i] >> 8;
    const uint32_t magnitude = sample == std::numeric_limits<int32_t>::min()
                                   ? std::numeric_limits<uint32_t>::max()
                                   : static_cast<uint32_t>(abs(sample));
    peak = std::max(peak, magnitude);
  }
  return peak;
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
    delay(4);
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
  while (millis() - startedAt < timeoutMs) {
    if (poll()) {
      if (firstTapAt == 0) {
        firstTapAt = millis();
      } else if (millis() - firstTapAt <= doubleTapWindowMs) {
        return TapDecision::Cancel;
      }
    }
    if (firstTapAt != 0 && millis() - firstTapAt > doubleTapWindowMs) {
      return TapDecision::Confirm;
    }
    delay(8);
  }
  return TapDecision::TimedOut;
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
