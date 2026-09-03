#pragma once

#include <Arduino.h>

class Display;

// Owns every chunky cyan robo-eye emotion drawn below Bunty's persistent 32 px
// status bar. Callers serialize access to the shared Flow32 Display before
// invoking these methods; the class owns frame selection, timing, and clipped
// partial presentation.
class BuntyAnimations {
 public:
  static constexpr uint32_t wakeDurationMs() { return 1540; }

  explicit BuntyAnimations(Display &display) : display_(display) {}

  // The first speaking frame can be composed into a larger full-screen frame.
  void beginSpeaking(uint32_t now, bool present = true);
  void serviceSpeaking(uint32_t now);

  // A stroked vector mouth below the eyes. `openness` is 0-100 and is meant to
  // track live speech loudness. Only the band the mouth can occupy is redrawn,
  // so it composes over an existing speaking frame without disturbing the eyes.
  void drawSpeakingMouth(uint8_t openness);

  void beginSleeping(uint32_t now);
  void serviceSleeping(uint32_t now);

  void beginWaking(uint32_t now);
  // Returns false once the wake animation has completed.
  bool serviceWaking(uint32_t now);

 private:
  void drawSpeakingFrame();
  void drawSleepFrame();
  void drawWakeFrame();

  Display &display_;
  uint32_t lastFrameAt_ = 0;
  uint32_t animationStartedAt_ = 0;
  uint8_t frame_ = 0;
};
