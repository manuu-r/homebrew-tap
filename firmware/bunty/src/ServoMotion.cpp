#include "ServoMotion.h"

bool ServoMotion::begin(uint8_t pin, uint8_t channel, uint8_t initialAngle) {
  channel_ = channel;
  const float start =
      initialAngle > 180 ? 180.0f : static_cast<float>(initialAngle);
  currentAngle_ = start;
  startAngle_ = start;
  targetAngle_ = start;
  if (ledcSetup(channel_, kFrequencyHz, kResolutionBits) <= 0) return false;
  ledcAttachPin(pin, channel_);
  ready_ = true;
  moving_ = false;
  lastUpdateMs_ = millis();
  writeAngle(currentAngle_);
  return true;
}

void ServoMotion::moveTo(float angle) {
  if (angle < 0.0f) angle = 0.0f;
  if (angle > 180.0f) angle = 180.0f;

  const float pending = angle > targetAngle_ ? angle - targetAngle_
                                             : targetAngle_ - angle;
  if (!moving_ && pending < 0.1f) return;

  startAngle_ = currentAngle_;
  targetAngle_ = angle;
  const float distance = targetAngle_ > startAngle_
                             ? targetAngle_ - startAngle_
                             : startAngle_ - targetAngle_;
  float durationMs = distance / kDegreesPerSecond * 1000.0f;
  if (durationMs < kMinimumMoveMs) durationMs = kMinimumMoveMs;
  moveDurationMs_ = static_cast<uint32_t>(durationMs);
  moveStartMs_ = millis();
  lastUpdateMs_ = moveStartMs_;
  moving_ = true;
}

void ServoMotion::service(uint32_t now) {
  if (!ready_ || !moving_ || now - lastUpdateMs_ < kUpdateIntervalMs) return;
  lastUpdateMs_ = now;

  float t = moveDurationMs_ == 0
                ? 1.0f
                : static_cast<float>(now - moveStartMs_) / moveDurationMs_;
  if (t >= 1.0f) {
    t = 1.0f;
    moving_ = false;
  }
  // Smoothstep easing: velocity is zero at both ends, so starts, stops, and
  // direction reversals are gentle instead of snapping.
  const float eased = t * t * (3.0f - 2.0f * t);
  currentAngle_ = startAngle_ + (targetAngle_ - startAngle_) * eased;
  writeAngle(currentAngle_);
}

void ServoMotion::writeAngle(float angle) {
  const float pulseUs =
      kMinimumPulseUs +
      (static_cast<float>(kMaximumPulseUs - kMinimumPulseUs) * angle) / 180.0f;
  constexpr float kPeriodUs = 1000000.0f / kFrequencyHz;
  constexpr uint32_t kMaximumDuty = (1UL << kResolutionBits) - 1;
  const uint32_t duty =
      static_cast<uint32_t>(pulseUs * kMaximumDuty / kPeriodUs + 0.5f);
  ledcWrite(channel_, duty);
}
