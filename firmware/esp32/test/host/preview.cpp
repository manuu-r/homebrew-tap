// Renders the real ui.cpp / emoji.cpp drawing code into a PNG-able image so the
// layout can be checked without flashing hardware.
//
//   cd test/host && ./preview.sh

#include <Adafruit_GFX.h>

#include <cstdio>
#include <vector>

#include "../../src/config.h"
#include "../../src/theme.h"
#include "../../src/ui.h"

class HostCanvas : public Adafruit_GFX {
 public:
  HostCanvas(int w, int h) : Adafruit_GFX(w, h), buf(w * h, 0) {}
  void drawPixel(int16_t x, int16_t y, uint16_t c) override {
    if (x < 0 || y < 0 || x >= width() || y >= height()) return;
    buf[(size_t)y * width() + x] = c;
  }
  size_t write(uint8_t c) override { return Adafruit_GFX::write(c); }
  std::vector<uint16_t> buf;
};

struct Shot {
  const char  *label;
  ProviderKind provider;
  bool         havePct;
  float        pct;
  bool         haveDelta;
  float        delta;
  bool         stale;
};

static const Shot kShots[] = {
    {"well-fed",   PROV_CLAUDE, true,  92.0f, true,   3.5f, false},
    {"peckish",    PROV_CODEX,  true,  71.0f, true,  -6.0f, false},
    {"hungry",     PROV_CLAUDE, true,  48.0f, true, -11.2f, false},
    {"starving",   PROV_CODEX,  true,  27.0f, true,  -4.0f, false},
    {"feral",      PROV_CLAUDE, true,  12.0f, true,  -8.5f, false},
    {"near-death", PROV_CODEX,  true,   3.0f, true,  -1.5f, false},
    {"dead",       PROV_CLAUDE, true,   0.0f, true,  -3.0f, false},
    {"no-data",    PROV_CODEX,  false,  0.0f, false,  0.0f, true},
};
static const int NSHOT = sizeof(kShots) / sizeof(kShots[0]);

static const int COLS = 4, GAP = 8;

int main() {
  const int rows = (NSHOT + COLS - 1) / COLS;
  const int W = COLS * TFT_W + (COLS + 1) * GAP;
  const int H = rows * TFT_H + (rows + 1) * GAP;
  std::vector<uint8_t> out((size_t)W * H * 3, 24);

  for (int i = 0; i < NSHOT; i++) {
    HostCanvas c(TFT_W, TFT_H);
    uiInvalidate();

    UiModel m;
    m.provider = kShots[i].provider;
    m.havePct = kShots[i].havePct;
    m.pct = kShots[i].pct;
    m.haveDelta = kShots[i].haveDelta;
    m.delta = kShots[i].delta;
    m.stale = kShots[i].stale;
    uiRender(c, m);

    const int ox = GAP + (i % COLS) * (TFT_W + GAP);
    const int oy = GAP + (i / COLS) * (TFT_H + GAP);
    for (int y = 0; y < TFT_H; y++) {
      for (int x = 0; x < TFT_W; x++) {
        const uint16_t p = c.buf[(size_t)y * TFT_W + x];
        const size_t o = ((size_t)(oy + y) * W + (ox + x)) * 3;
        out[o + 0] = (uint8_t)(((p >> 11) & 0x1F) * 255 / 31);
        out[o + 1] = (uint8_t)(((p >> 5) & 0x3F) * 255 / 63);
        out[o + 2] = (uint8_t)((p & 0x1F) * 255 / 31);
      }
    }
    printf("  rendered %s\n", kShots[i].label);
  }

  FILE *f = fopen("preview.ppm", "wb");
  if (!f) return 1;
  fprintf(f, "P6\n%d %d\n255\n", W, H);
  fwrite(out.data(), 1, out.size(), f);
  fclose(f);
  printf("wrote preview.ppm (%dx%d)\n", W, H);
  return 0;
}
