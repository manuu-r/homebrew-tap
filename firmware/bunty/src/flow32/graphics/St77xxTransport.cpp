#include "flow32/graphics/St77xxTransport.h"

#include "flow32/graphics/DisplayPanel.h"

#include <Arduino.h>

bool St77xxTransport::begin(const DisplayPanel &panel) {
  if (driver_) return true;
  if (panel.width <= 0 || panel.height <= 0 || !config_.spi ||
      config_.pinCs < 0 || config_.pinDc < 0 || config_.pinMosi < 0 ||
      config_.pinSclk < 0 || config_.gramWidth <= 0 ||
      config_.gramHeight <= 0) {
    return false;
  }

  config_.spi->begin(config_.pinSclk, config_.pinMiso, config_.pinMosi, -1);
  if (config_.chip == PanelChip::ST7735) {
    st7735_ = new Adafruit_ST7735(config_.spi, config_.pinCs, config_.pinDc,
                                 config_.pinRst);
    if (!st7735_) return false;
    st7735_->initR(config_.st7735Init);
    driver_ = st7735_;
  } else {
    st7789_ = new Adafruit_ST7789(config_.spi, config_.pinCs, config_.pinDc,
                                 config_.pinRst);
    if (!st7789_) return false;
    st7789_->init(config_.gramWidth, config_.gramHeight);
    driver_ = st7789_;
  }
  driver_->setSPISpeed(config_.spiHz);
  driver_->setRotation(config_.rotation);
  panelWidth_ = panel.width;
  panelHeight_ = panel.height;
  if (config_.pinBacklight >= 0) {
    pinMode(config_.pinBacklight, OUTPUT);
    setBacklight(false);
  }
  return true;
}

void St77xxTransport::end() {
  if (driver_) setBacklight(false);
  delete st7735_;
  delete st7789_;
  st7735_ = nullptr;
  st7789_ = nullptr;
  driver_ = nullptr;
  panelWidth_ = 0;
  panelHeight_ = 0;
}

void St77xxTransport::setBacklight(bool enabled) {
  if (config_.pinBacklight < 0) return;
  const bool high = config_.backlightActiveHigh ? enabled : !enabled;
  digitalWrite(config_.pinBacklight, high ? HIGH : LOW);
}

void St77xxTransport::present(const uint16_t *pixels, int16_t x, int16_t y,
                              int16_t width, int16_t height, int16_t stride) {
  if (!driver_ || !pixels || width <= 0 || height <= 0 || stride < width ||
      x < 0 || y < 0 || x + width > panelWidth_ ||
      y + height > panelHeight_) {
    return;
  }
  driver_->startWrite();
  driver_->setAddrWindow(x, config_.panelYOffset + y,
                         static_cast<uint16_t>(width),
                         static_cast<uint16_t>(height));
  if (stride == width) {
    driver_->writePixels(const_cast<uint16_t *>(pixels),
                         static_cast<uint32_t>(width) * height);
  } else {
    for (int16_t row = 0; row < height; ++row) {
      driver_->writePixels(
          const_cast<uint16_t *>(pixels + static_cast<int32_t>(row) * stride),
          static_cast<uint32_t>(width));
    }
  }
  driver_->endWrite();
}
