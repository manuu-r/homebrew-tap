#pragma once

#include <stdint.h>

struct DisplayPanel;

/**
 * Application-owned panel backend. Implement this to use TFT_eSPI, LovyanGFX,
 * an e-paper driver, or any controller other than the optional ST77xx adapter.
 * The pixel format is RGB565. `pixels` points at the source rectangle's first
 * pixel, `stride` is its row stride in pixels, and x/y are panel coordinates.
 */
class DisplayTransport {
public:
  virtual ~DisplayTransport() = default;
  virtual bool begin(const DisplayPanel &panel) = 0;
  virtual void end() {}
  virtual void setBacklight(bool /*on*/) {}
  virtual void present(const uint16_t *pixels, int16_t x, int16_t y,
                       int16_t width, int16_t height, int16_t stride) = 0;
};
