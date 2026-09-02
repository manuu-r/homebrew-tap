#include "flow32/graphics/AAFont.h"
#include "flow32/graphics/Display.h"

namespace AAFontDraw {

uint16_t blend565(uint16_t fg, uint16_t bg, uint8_t cover4) {
  if (cover4 == 0) return bg;
  if (cover4 >= 15) return fg;

  const uint8_t a = cover4; // 0..15
  const uint8_t ia = 15 - a;

  const uint8_t fr = (fg >> 11) & 0x1F;
  const uint8_t fg_ = (fg >> 5) & 0x3F;
  const uint8_t fb = fg & 0x1F;

  const uint8_t br = (bg >> 11) & 0x1F;
  const uint8_t bg_ = (bg >> 5) & 0x3F;
  const uint8_t bb = bg & 0x1F;

  const uint8_t r = (fr * a + br * ia) / 15;
  const uint8_t g = (fg_ * a + bg_ * ia) / 15;
  const uint8_t b = (fb * a + bb * ia) / 15;
  return static_cast<uint16_t>((r << 11) | (g << 5) | b);
}

uint32_t foldCodepoint(uint32_t cp) {
  switch (cp) {
  case 0x2018: // ‘
  case 0x2019: // ’
  case 0x02BC:
    return '\'';
  case 0x201C: // “
  case 0x201D: // ”
    return '"';
  case 0x2013: // –
  case 0x2014: // —
  case 0x2212: // −
    return '-';
  case 0x2022: // •
  case 0x2219: // ∙
  case 0x00B7: // · (already Latin-1; listed for clarity)
    return 0x00B7;
  case 0x2026: // …
    return 0x00B7; // closest single-glyph stand-in in Latin-1
  default:
    return cp;
  }
}

const AAGlyph *glyph(const AAFont &font, uint32_t cp) {
  cp = foldCodepoint(cp);
  if (cp > 0xFF) {
    cp = static_cast<uint32_t>('?');
  }
  uint8_t uc = static_cast<uint8_t>(cp);
  if (uc < font.first || uc > font.last) {
    uc = static_cast<uint8_t>('?');
    if (uc < font.first || uc > font.last) return nullptr;
  }
  return &font.glyph[uc - font.first];
}

int16_t charWidth(const AAFont &font, uint32_t cp) {
  const AAGlyph *g = glyph(font, cp);
  if (!g) return 0;
  return pgm_read_byte(&g->xAdvance);
}

int16_t textWidth(const AAFont &font, const char *text, size_t len) {
  int16_t w = 0;
  for (size_t i = 0; i < len; i++) {
    w = static_cast<int16_t>(w + charWidth(font, static_cast<uint8_t>(text[i])));
  }
  return w;
}

void drawChar(Display &display, const AAFont &font, int16_t baselineX,
              int16_t baselineY, uint32_t cp, uint16_t color) {
  const AAGlyph *g = glyph(font, cp);
  if (!g) return;
  const uint8_t gw = pgm_read_byte(&g->width);
  const uint8_t gh = pgm_read_byte(&g->height);
  if (gw == 0 || gh == 0) return;

  const uint16_t bitmapOffset = pgm_read_word(&g->bitmapOffset);
  const int8_t xOff = static_cast<int8_t>(pgm_read_byte(&g->xOffset));
  const int8_t yOff = static_cast<int8_t>(pgm_read_byte(&g->yOffset));

  const int16_t x0 = static_cast<int16_t>(baselineX + xOff);
  const int16_t y0 = static_cast<int16_t>(baselineY + yOff);
  const uint8_t *bits = font.bitmap + bitmapOffset;
  const uint16_t total = static_cast<uint16_t>(gw) * gh;

  for (uint16_t i = 0; i < total; i++) {
    const uint8_t byte = pgm_read_byte(&bits[i >> 1]);
    const uint8_t cover = (i & 1) ? (byte & 0x0F) : (byte >> 4);
    if (cover == 0) continue;

    const int16_t x = static_cast<int16_t>(x0 + (i % gw));
    const int16_t y = static_cast<int16_t>(y0 + (i / gw));
    display.blendPixel(x, y, color, cover);
  }
}

} // namespace AAFontDraw
