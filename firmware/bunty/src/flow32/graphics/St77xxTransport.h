#pragma once

#include "flow32/graphics/DisplayTransport.h"

#include <Adafruit_ST7735.h>
#include <Adafruit_ST7789.h>
#include <SPI.h>

#ifndef FLOW32_ENABLE_ST7735
#define FLOW32_ENABLE_ST7735 1
#endif

enum class PanelChip : uint8_t { ST7789, ST7735 };

/** Board/panel policy for the optional Adafruit ST77xx adapter. */
struct St77xxConfig {
  PanelChip chip = PanelChip::ST7789;
  SPIClass *spi = nullptr;
  int pinCs = -1;
  int pinDc = -1;
  int pinRst = -1;
  int pinBacklight = -1;
  int pinMiso = -1;
  int pinMosi = -1;
  int pinSclk = -1;
  int16_t gramWidth = 0;
  int16_t gramHeight = 0;
  int16_t panelYOffset = 0;
  uint8_t rotation = 0;
  uint8_t st7735Init = INITR_18BLACKTAB;
  uint32_t spiHz = 40000000;
  bool backlightActiveHigh = true;
};

/** Synchronous Adafruit ST7735/ST7789 transport. */
class St77xxTransport final : public DisplayTransport {
public:
  explicit St77xxTransport(const St77xxConfig &config) : config_(config) {}
  ~St77xxTransport() override { end(); }

  bool begin(const DisplayPanel &panel) override;
  void end() override;
  void setBacklight(bool enabled) override;
  void present(const uint16_t *pixels, int16_t x, int16_t y, int16_t width,
               int16_t height, int16_t stride) override;

  const St77xxConfig &config() const { return config_; }

private:
  St77xxConfig config_;
  Adafruit_ST7789 *st7789_ = nullptr;
#if FLOW32_ENABLE_ST7735
  Adafruit_ST7735 *st7735_ = nullptr;
#endif
  Adafruit_ST77xx *driver_ = nullptr;
  int16_t panelWidth_ = 0;
  int16_t panelHeight_ = 0;
};
