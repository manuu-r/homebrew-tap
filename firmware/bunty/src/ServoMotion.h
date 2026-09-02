#pragma once

#include <Arduino.h>

// Small, non-blocking hobby-servo driver. It owns one LEDC channel and glides
// to a target angle along an eased profile so display, tap, and network work
// keep running while Bunty changes posture. Position is a pure function of
// wall-clock time, so a stalled loop() resumes the glide smoothly instead of
// snapping several degrees at once.
class ServoMotion {
 public:
  bool begin(uint8_t pin, uint8_t channel, uint8_t initialAngle);
  void moveTo(float angle);
  void service(uint32_t now);
  bool moving() const { return moving_; }

 private:
  static constexpr uint32_t kFrequencyHz = 50;
  // ESP32-S3 LEDC timers top out at 14 bits. Requesting 16 bits makes
  // ledcSetup() fail and leaves the SG90 signal pin silent.
  static constexpr uint8_t kResolutionBits = 14;
  static constexpr uint16_t kMinimumPulseUs = 500;
  static constexpr uint16_t kMaximumPulseUs = 2500;
  // 50 Hz sub-degree position updates: each move is a continuous glide rather
  // than a stack of visible one-degree jumps.
  static constexpr uint32_t kUpdateIntervalMs = 20;
  static constexpr float kDegreesPerSecond = 20.0f;
  static constexpr float kMinimumMoveMs = 220.0f;

  void writeAngle(float angle);

  uint8_t channel_ = 0;
  float currentAngle_ = 90.0f;
  float startAngle_ = 90.0f;
  float targetAngle_ = 90.0f;
  uint32_t moveStartMs_ = 0;
  uint32_t moveDurationMs_ = 0;
  uint32_t lastUpdateMs_ = 0;
  bool moving_ = false;
  bool ready_ = false;
};
