#pragma once

#include <stdint.h>

/**
 * Physical / wiring profile for a TFT module.
 * width × height = framebuffer, UI coordinates, and SPI present size (1:1).
 *
 * Concrete board and panel presets live in the app, not the library.
 */
struct DisplayPanel {
  const char *id = "display";
  int16_t width = 0;
  int16_t height = 0;
  int16_t cornerRadius = 0;
  bool preferPsram = true;

  /**
   * UI density vs design px (padding, gap, radius, fonts). Default 1.0.
   * AA text picks the nearest baked font — no bitmap stretch.
   */
  float uiScale = 1.0f;
};
