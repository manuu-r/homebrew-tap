#include "flow32/graphics/Display.h"
#include "flow32/graphics/AAFont.h"

#include <esp_heap_caps.h>
#include <math.h>
#include <string.h>

Display::Display(const DisplayPanel &panel, DisplayTransport &transport)
    : Adafruit_GFX(panel.width, panel.height), panel_(panel),
      transport_(transport), targetW_(panel.width), targetH_(panel.height) {}

Display::~Display() { end(); }

bool Display::begin() {
  if (ready()) return true;
  if (panel_.width <= 0 || panel_.height <= 0) {
    return false;
  }
  if (!transport_.begin(panel_)) return false;
  transportBegun_ = true;

  setBacklight(true);

  const size_t bytes = bufferBytes();
  if (bytes == 0) {
    end();
    return false;
  }
  if (panel_.preferPsram) {
    fbPanel_ = (uint16_t *)heap_caps_malloc(
        bytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
  }
  if (!fbPanel_) {
    fbPanel_ = (uint16_t *)heap_caps_malloc(bytes,
                                            MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
  }
  if (!fbPanel_) {
    end();
    return false;
  }
  memset(fbPanel_, 0, bytes);
  fb_ = fbPanel_;
  targetW_ = panel_.width;
  targetH_ = panel_.height;

  useFontRole(FontRole::Body);
  setTextColor(0xFFFF);
  setTextWrap(false);
  return true;
}

void Display::end() {
  if (fbPanel_ || transportBegun_) setBacklight(false);
  fb_ = nullptr;
  if (fbPanel_) {
    heap_caps_free(fbPanel_);
    fbPanel_ = nullptr;
  }
  if (transportBegun_) {
    transport_.end();
    transportBegun_ = false;
  }
  targetPushed_ = false;
}

void Display::setBacklight(bool on) { transport_.setBacklight(on); }

size_t Display::bufferBytes() const {
  if (panel_.width <= 0 || panel_.height <= 0) return 0;
  const size_t width = static_cast<size_t>(panel_.width);
  const size_t height = static_cast<size_t>(panel_.height);
  if (width > static_cast<size_t>(-1) / height) return 0;
  const size_t pixels = width * height;
  if (pixels > static_cast<size_t>(-1) / sizeof(uint16_t)) return 0;
  return pixels * sizeof(uint16_t);
}

void Display::pushDrawTarget(uint16_t *fb, int16_t w, int16_t h) {
  if (targetPushed_ || !fb || w <= 0 || h <= 0) return;
  savedFb_ = fb_;
  savedW_ = targetW_;
  savedH_ = targetH_;
  fb_ = fb;
  targetW_ = w;
  targetH_ = h;
  targetPushed_ = true;
  clearClip();
}

void Display::popDrawTarget() {
  if (!targetPushed_) return;
  fb_ = savedFb_;
  targetW_ = savedW_;
  targetH_ = savedH_;
  targetPushed_ = false;
  clearClip();
}

const FontFace &Display::fontFace(FontRole role) const {
  static const FontFace kDefault{};
  return fonts_ ? fonts_->face(role) : kDefault;
}

void Display::useFontRole(FontRole role) { setFont(fontFace(role).gfx); }

void Display::clear(uint16_t color) {
  if (!fb_) return;
  const size_t n = (size_t)targetW_ * (size_t)targetH_;
  if (color == 0) {
    memset(fb_, 0, n * sizeof(uint16_t));
    return;
  }
  for (size_t i = 0; i < n; i++) fb_[i] = color;
}

void Display::setClip(int16_t x, int16_t y, int16_t w, int16_t h) {
  clipToTarget(x, y, w, h);
  clipX_ = x;
  clipY_ = y;
  clipW_ = w;
  clipH_ = h;
  clipEnabled_ = !((w <= 0) || (h <= 0));
}

void Display::clearClip() {
  clipEnabled_ = false;
  clipX_ = 0;
  clipY_ = 0;
  clipW_ = targetW_;
  clipH_ = targetH_;
}

bool Display::inClip(int16_t x, int16_t y) const {
  if (!clipEnabled_) return true;
  return x >= clipX_ && y >= clipY_ && x < clipX_ + clipW_ && y < clipY_ + clipH_;
}

void Display::clipToTarget(int16_t &x, int16_t &y, int16_t &w, int16_t &h) const {
  if (x < 0) {
    w += x;
    x = 0;
  }
  if (y < 0) {
    h += y;
    y = 0;
  }
  if (x + w > targetW_) w = targetW_ - x;
  if (y + h > targetH_) h = targetH_ - y;
  if (w < 0) w = 0;
  if (h < 0) h = 0;
}

void Display::clipToDraw(int16_t &x, int16_t &y, int16_t &w, int16_t &h) const {
  clipToTarget(x, y, w, h);
  if (!clipEnabled_) return;
  if (x < clipX_) {
    w -= (clipX_ - x);
    x = clipX_;
  }
  if (y < clipY_) {
    h -= (clipY_ - y);
    y = clipY_;
  }
  if (x + w > clipX_ + clipW_) w = clipX_ + clipW_ - x;
  if (y + h > clipY_ + clipH_) h = clipY_ + clipH_ - y;
  if (w < 0) w = 0;
  if (h < 0) h = 0;
}

void Display::present() { present(0, 0, panel_.width, panel_.height); }

void Display::present(int16_t x, int16_t y, int16_t w, int16_t h) {
  if (!fbPanel_ || !transportBegun_ || w <= 0 || h <= 0) return;

  if (x < 0) {
    w += x;
    x = 0;
  }
  if (y < 0) {
    h += y;
    y = 0;
  }
  if (x + w > panel_.width) w = panel_.width - x;
  if (y + h > panel_.height) h = panel_.height - y;
  if (w <= 0 || h <= 0) return;

  transport_.present(fbPanel_ + static_cast<int32_t>(y) * panel_.width + x, x,
                     y, w, h, panel_.width);
}

void Display::presentBuffer(const uint16_t *src, int16_t x, int16_t y, int16_t w,
                            int16_t h) {
  // src is tightly packed (stride == w).
  if (!src || !transportBegun_ || w <= 0 || h <= 0) return;
  const int16_t srcStride = w;

  if (x < 0) {
    src -= x;
    w += x;
    x = 0;
  }
  if (y < 0) {
    src += static_cast<int32_t>(-y) * srcStride;
    h += y;
    y = 0;
  }
  if (x + w > panel_.width) w = panel_.width - x;
  if (y + h > panel_.height) h = panel_.height - y;
  if (w <= 0 || h <= 0) return;

  transport_.present(src, x, y, w, h, srcStride);
}

void Display::drawPixel(int16_t x, int16_t y, uint16_t color) {
  if (!fb_ || x < 0 || y < 0 || x >= targetW_ || y >= targetH_) return;
  if (!inClip(x, y)) return;
  fb_[(int32_t)y * targetW_ + x] = color;
}

uint16_t Display::getPixel(int16_t x, int16_t y) const {
  if (!fb_ || x < 0 || y < 0 || x >= targetW_ || y >= targetH_) return 0;
  return fb_[(int32_t)y * targetW_ + x];
}

void Display::blendPixel(int16_t x, int16_t y, uint16_t fg, uint8_t cover4) {
  if (!fb_ || cover4 == 0) return;
  if (x < 0 || y < 0 || x >= targetW_ || y >= targetH_) return;
  if (!inClip(x, y)) return;
  if (cover4 >= 15) {
    fb_[(int32_t)y * targetW_ + x] = fg;
    return;
  }
  uint16_t *p = &fb_[(int32_t)y * targetW_ + x];
  *p = AAFontDraw::blend565(fg, *p, cover4);
}

void Display::drawFastHLine(int16_t x, int16_t y, int16_t w, uint16_t color) {
  if (!fb_ || y < 0 || y >= targetH_) return;
  if (clipEnabled_ && (y < clipY_ || y >= clipY_ + clipH_)) return;
  if (x < 0) {
    w += x;
    x = 0;
  }
  if (x + w > targetW_) w = targetW_ - x;
  if (clipEnabled_) {
    if (x < clipX_) {
      w -= (clipX_ - x);
      x = clipX_;
    }
    if (x + w > clipX_ + clipW_) w = clipX_ + clipW_ - x;
  }
  if (w <= 0) return;
  uint16_t *row = fb_ + (int32_t)y * targetW_ + x;
  for (int16_t i = 0; i < w; i++) row[i] = color;
}

void Display::drawFastVLine(int16_t x, int16_t y, int16_t h, uint16_t color) {
  if (!fb_ || x < 0 || x >= targetW_) return;
  if (clipEnabled_ && (x < clipX_ || x >= clipX_ + clipW_)) return;
  if (y < 0) {
    h += y;
    y = 0;
  }
  if (y + h > targetH_) h = targetH_ - y;
  if (clipEnabled_) {
    if (y < clipY_) {
      h -= (clipY_ - y);
      y = clipY_;
    }
    if (y + h > clipY_ + clipH_) h = clipY_ + clipH_ - y;
  }
  if (h <= 0) return;
  for (int16_t i = 0; i < h; i++) {
    fb_[(int32_t)(y + i) * targetW_ + x] = color;
  }
}

void Display::fillRect(int16_t x, int16_t y, int16_t w, int16_t h,
                       uint16_t color) {
  if (!fb_) return;
  clipToDraw(x, y, w, h);
  for (int16_t row = 0; row < h; row++) {
    drawFastHLine(x, y + row, w, color);
  }
}

namespace {

inline uint8_t coverFromSd(float sd) {
  // sd <= -0.5 → opaque, sd >= +0.5 → transparent
  const float c = 0.5f - sd;
  if (c <= 0.f) return 0;
  if (c >= 1.f) return 15;
  return static_cast<uint8_t>(c * 15.f + 0.5f);
}

inline float clampf01(float v) {
  if (v <= 0.f) return 0.f;
  if (v >= 1.f) return 1.f;
  return v;
}

inline int16_t clampRadius(int16_t w, int16_t h, int16_t r) {
  if (r < 0) r = 0;
  if (r * 2 > w) r = w / 2;
  if (r * 2 > h) r = h / 2;
  return r;
}

} // namespace

void Display::fillRoundRect(int16_t x, int16_t y, int16_t w, int16_t h,
                            int16_t r, uint16_t color) {
  if (!fb_ || w <= 0 || h <= 0) return;
  r = clampRadius(w, h, r);
  if (r <= 0) {
    fillRect(x, y, w, h, color);
    return;
  }

  // Solid body — AA only needed in the four corner quarters.
  if (h > 2 * r) {
    fillRect(x, static_cast<int16_t>(y + r), w, static_cast<int16_t>(h - 2 * r),
             color);
  }
  if (w > 2 * r) {
    fillRect(static_cast<int16_t>(x + r), y, static_cast<int16_t>(w - 2 * r), r,
             color);
    fillRect(static_cast<int16_t>(x + r), static_cast<int16_t>(y + h - r),
             static_cast<int16_t>(w - 2 * r), r, color);
  }

  const float rf = static_cast<float>(r);
  auto paintCorner = [&](int16_t x0, int16_t y0, float cx, float cy) {
    for (int16_t j = 0; j < r; j++) {
      for (int16_t i = 0; i < r; i++) {
        const float dx = (x0 + i + 0.5f) - cx;
        const float dy = (y0 + j + 0.5f) - cy;
        const float sd = sqrtf(dx * dx + dy * dy) - rf;
        const uint8_t cover = coverFromSd(sd);
        if (cover == 0) continue;
        blendPixel(static_cast<int16_t>(x0 + i), static_cast<int16_t>(y0 + j),
                   color, cover);
      }
    }
  };

  paintCorner(x, y, x + rf, y + rf);                                         // TL
  paintCorner(static_cast<int16_t>(x + w - r), y, x + w - rf, y + rf);       // TR
  paintCorner(x, static_cast<int16_t>(y + h - r), x + rf, y + h - rf);       // BL
  paintCorner(static_cast<int16_t>(x + w - r), static_cast<int16_t>(y + h - r),
              x + w - rf, y + h - rf);                                       // BR
}

void Display::strokeRoundRect(int16_t x, int16_t y, int16_t w, int16_t h,
                              int16_t r, uint8_t strokeW, uint16_t color,
                              bool outside) {
  if (!fb_ || w <= 0 || h <= 0 || strokeW == 0) return;
  r = clampRadius(w, h, r);
  const int16_t sw = static_cast<int16_t>(strokeW);

  if (r <= 0) {
    // Sharp rect — solid stroke rings (edges are axis-aligned).
    for (int16_t i = 0; i < sw; i++) {
      int16_t rx, ry, rw, rh;
      if (outside) {
        rx = static_cast<int16_t>(x - (i + 1));
        ry = static_cast<int16_t>(y - (i + 1));
        rw = static_cast<int16_t>(w + 2 * (i + 1));
        rh = static_cast<int16_t>(h + 2 * (i + 1));
      } else {
        rx = static_cast<int16_t>(x + i);
        ry = static_cast<int16_t>(y + i);
        rw = static_cast<int16_t>(w - 2 * i);
        rh = static_cast<int16_t>(h - 2 * i);
        if (rw <= 0 || rh <= 0) break;
      }
      drawFastHLine(rx, ry, rw, color);
      drawFastHLine(rx, static_cast<int16_t>(ry + rh - 1), rw, color);
      if (rh > 2) {
        drawFastVLine(rx, static_cast<int16_t>(ry + 1),
                      static_cast<int16_t>(rh - 2), color);
        drawFastVLine(static_cast<int16_t>(rx + rw - 1),
                      static_cast<int16_t>(ry + 1),
                      static_cast<int16_t>(rh - 2), color);
      }
    }
    return;
  }

  const float rf = static_cast<float>(r);
  const float swf = static_cast<float>(sw);
  const float outerR = outside ? (rf + swf) : rf;
  const float innerR = outside ? rf : (rf - swf);
  const int16_t scan = outside ? static_cast<int16_t>(r + sw) : r;

  // Straight edge bands (integer-aligned — no AA needed).
  const int16_t midW = static_cast<int16_t>(w - 2 * r);
  const int16_t midH = static_cast<int16_t>(h - 2 * r);
  if (midW > 0) {
    if (outside) {
      fillRect(static_cast<int16_t>(x + r), static_cast<int16_t>(y - sw), midW,
               sw, color);
      fillRect(static_cast<int16_t>(x + r), static_cast<int16_t>(y + h), midW, sw,
               color);
    } else {
      fillRect(static_cast<int16_t>(x + r), y, midW, sw, color);
      fillRect(static_cast<int16_t>(x + r), static_cast<int16_t>(y + h - sw),
               midW, sw, color);
    }
  }
  if (midH > 0) {
    if (outside) {
      fillRect(static_cast<int16_t>(x - sw), static_cast<int16_t>(y + r), sw,
               midH, color);
      fillRect(static_cast<int16_t>(x + w), static_cast<int16_t>(y + r), sw,
               midH, color);
    } else {
      fillRect(x, static_cast<int16_t>(y + r), sw, midH, color);
      fillRect(static_cast<int16_t>(x + w - sw), static_cast<int16_t>(y + r), sw,
               midH, color);
    }
  }

  auto paintCornerStroke = [&](int16_t x0, int16_t y0, float cx, float cy) {
    for (int16_t j = 0; j < scan; j++) {
      for (int16_t i = 0; i < scan; i++) {
        const float dx = (x0 + i + 0.5f) - cx;
        const float dy = (y0 + j + 0.5f) - cy;
        const float dist = sqrtf(dx * dx + dy * dy);
        float a;
        if (outside) {
          a = clampf01(0.5f - (dist - outerR)) -
              clampf01(0.5f - (dist - innerR));
        } else if (innerR > 0.f) {
          a = clampf01(0.5f - (dist - outerR)) -
              clampf01(0.5f - (dist - innerR));
        } else {
          // Stroke eats the whole corner disc.
          a = clampf01(0.5f - (dist - outerR));
        }
        if (a <= 0.f) continue;
        const uint8_t cover =
            a >= 1.f ? 15 : static_cast<uint8_t>(a * 15.f + 0.5f);
        blendPixel(static_cast<int16_t>(x0 + i), static_cast<int16_t>(y0 + j),
                   color, cover);
      }
    }
  };

  if (outside) {
    paintCornerStroke(static_cast<int16_t>(x - sw), static_cast<int16_t>(y - sw),
                      x + rf, y + rf);
    paintCornerStroke(static_cast<int16_t>(x + w - r),
                      static_cast<int16_t>(y - sw), x + w - rf, y + rf);
    paintCornerStroke(static_cast<int16_t>(x - sw),
                      static_cast<int16_t>(y + h - r), x + rf, y + h - rf);
    paintCornerStroke(static_cast<int16_t>(x + w - r),
                      static_cast<int16_t>(y + h - r), x + w - rf, y + h - rf);
  } else {
    paintCornerStroke(x, y, x + rf, y + rf);
    paintCornerStroke(static_cast<int16_t>(x + w - r), y, x + w - rf, y + rf);
    paintCornerStroke(x, static_cast<int16_t>(y + h - r), x + rf, y + h - rf);
    paintCornerStroke(static_cast<int16_t>(x + w - r),
                      static_cast<int16_t>(y + h - r), x + w - rf, y + h - rf);
  }
}

void Display::drawRoundRect(int16_t x, int16_t y, int16_t w, int16_t h,
                            int16_t r, uint16_t color) {
  strokeRoundRect(x, y, w, h, r, 1, color, /*outside=*/false);
}

void Display::drawRGBBitmap(int16_t x, int16_t y, const uint16_t *bitmap,
                            int16_t w, int16_t h) {
  if (!fb_ || !bitmap) return;
  for (int16_t j = 0; j < h; j++) {
    int16_t dy = y + j;
    if (dy < 0 || dy >= targetH_) continue;
    if (clipEnabled_ && (dy < clipY_ || dy >= clipY_ + clipH_)) continue;
    for (int16_t i = 0; i < w; i++) {
      int16_t dx = x + i;
      if (dx < 0 || dx >= targetW_) continue;
      if (!inClip(dx, dy)) continue;
      fb_[(int32_t)dy * targetW_ + dx] = bitmap[(int32_t)j * w + i];
    }
  }
}

void Display::blitHLine(int16_t x, int16_t y, const uint16_t *src, int16_t w) {
  if (!fb_ || !src || y < 0 || y >= targetH_) return;
  if (clipEnabled_ && (y < clipY_ || y >= clipY_ + clipH_)) return;
  if (x < 0) {
    w += x;
    src -= x;
    x = 0;
  }
  if (x + w > targetW_) w = targetW_ - x;
  if (clipEnabled_) {
    if (x < clipX_) {
      const int16_t skip = clipX_ - x;
      w -= skip;
      src += skip;
      x = clipX_;
    }
    if (x + w > clipX_ + clipW_) w = clipX_ + clipW_ - x;
  }
  if (w <= 0) return;
  memcpy(fb_ + (int32_t)y * targetW_ + x, src, (size_t)w * sizeof(uint16_t));
}

bool Display::insideRound(int16_t x, int16_t y) const {
  return insideRound(x, y, panel_.cornerRadius);
}

bool Display::insideRound(int16_t x, int16_t y, int16_t r) const {
  if (x < 0 || y < 0 || x >= panel_.width || y >= panel_.height) return false;
  if (r <= 0) return true;

  if (x < r && y < r) {
    const int32_t dx = r - 1 - x;
    const int32_t dy = r - 1 - y;
    return dx * dx + dy * dy <= (int32_t)(r - 1) * (r - 1);
  }
  if (x >= panel_.width - r && y < r) {
    const int32_t dx = x - (panel_.width - r);
    const int32_t dy = r - 1 - y;
    return dx * dx + dy * dy <= (int32_t)(r - 1) * (r - 1);
  }
  if (x < r && y >= panel_.height - r) {
    const int32_t dx = r - 1 - x;
    const int32_t dy = y - (panel_.height - r);
    return dx * dx + dy * dy <= (int32_t)(r - 1) * (r - 1);
  }
  if (x >= panel_.width - r && y >= panel_.height - r) {
    const int32_t dx = x - (panel_.width - r);
    const int32_t dy = y - (panel_.height - r);
    return dx * dx + dy * dy <= (int32_t)(r - 1) * (r - 1);
  }
  return true;
}

void Display::drawPanelRoundRect(uint16_t color) {
  const int16_t r = panel_.cornerRadius;
  if (r <= 0) {
    strokeRoundRect(0, 0, panel_.width, panel_.height, 0, 2, color,
                    /*outside=*/false);
    return;
  }
  strokeRoundRect(0, 0, panel_.width, panel_.height, r, 2, color,
                  /*outside=*/false);
}

void Display::fillPanelRoundRect(uint16_t color) {
  if (panel_.cornerRadius <= 0) {
    fillRect(0, 0, panel_.width, panel_.height, color);
    return;
  }
  fillRoundRect(0, 0, panel_.width, panel_.height, panel_.cornerRadius, color);
}
