// Renders the real ui.cpp / emoji.cpp drawing code to preview.ppm so layouts
// can be checked without flashing hardware.
//
//   cd test/host && ./preview.sh

#include <Adafruit_GFX.h>

#include <cstdio>
#include <cstring>
#include <functional>
#include <vector>

#include "../../src/config.h"
#include "../../src/ui.h"

class HostCanvas : public Adafruit_GFX {
 public:
  HostCanvas() : Adafruit_GFX(TFT_W, TFT_H), buf(TFT_W * TFT_H, 0) {}
  void drawPixel(int16_t x, int16_t y, uint16_t c) override {
    if (x >= 0 && y >= 0 && x < width() && y < height()) buf[(size_t)y * width() + x] = c;
  }
  std::vector<uint16_t> buf;
};

static ProviderEntry provider(const char *name, float remaining, int limits) {
  static const char *labels[] = {"5-hour", "Weekly", "Weekly (Opus)"};
  ProviderEntry p;
  strcpy(p.name, name);
  p.haveRemaining = true;
  p.remaining = remaining;
  p.limitCount = limits;
  for (int i = 0; i < limits; i++) {
    strcpy(p.limits[i].label, labels[i]);
    p.limits[i].remaining = remaining + i * 20.0f;
    p.limits[i].resetsAt = 1000 + 7800 * (i + 1) * (i + 1);
  }
  return p;
}

int main() {
  const int64_t now = 1000;
  DashboardData d;
  d.providerCount = 2;
  d.providers[0] = provider("Claude", 43.0f, 3);
  d.providers[1] = provider("Codex Spark", 3.0f, 1);
  d.calendarEnabled = true;
  d.eventCount = 2;
  strcpy(d.events[0].title, "Design review with the hardware team");
  d.events[0].startsAt = now + 1500;
  strcpy(d.events[1].title, "Ship Gauge firmware");
  d.events[1].allDay = true;
  d.todoCount = 3;
  d.todoTotal = 7;
  strcpy(d.todos[0].title, "Review pull request");
  strcpy(d.todos[1].title, "Send invoice to client");
  d.todos[1].completed = true;
  strcpy(d.todos[2].title, "Test the two-axis head movement");

  const std::vector<std::function<void(HostCanvas &)>> shots = {
      [&](HostCanvas &c) { uiPairing(c, "Gauge Display a1b2"); },
      [&](HostCanvas &c) { uiCompare(c, 482913); },
      [&](HostCanvas &c) { uiProvider(c, d.providers[0], now, false, 0, 4); },
      [&](HostCanvas &c) { uiProvider(c, d.providers[1], now, true, 1, 4); },
      [&](HostCanvas &c) { uiCalendar(c, d, now, false, 2, 4); },
      [&](HostCanvas &c) { uiTodos(c, d, false, 3, 4); },
      [&](HostCanvas &c) { uiQuotaError(c, DashboardData(), false, 0, 3); },
      [&](HostCanvas &c) { uiStatus(c, "NO GAUGE", "Is Gauge open on this Wi-Fi?", -1); },
  };

  const int cols = 4, gap = 8, rows = ((int)shots.size() + cols - 1) / cols;
  const int W = cols * TFT_W + (cols + 1) * gap, H = rows * TFT_H + (rows + 1) * gap;
  std::vector<uint8_t> out((size_t)W * H * 3, 24);
  for (size_t i = 0; i < shots.size(); i++) {
    HostCanvas c;
    shots[i](c);
    const int ox = gap + (i % cols) * (TFT_W + gap), oy = gap + (i / cols) * (TFT_H + gap);
    for (int y = 0; y < TFT_H; y++) {
      for (int x = 0; x < TFT_W; x++) {
        const uint16_t p = c.buf[(size_t)y * TFT_W + x];
        const size_t o = ((size_t)(oy + y) * W + (ox + x)) * 3;
        out[o] = ((p >> 11) & 0x1F) * 255 / 31;
        out[o + 1] = ((p >> 5) & 0x3F) * 255 / 63;
        out[o + 2] = (p & 0x1F) * 255 / 31;
      }
    }
  }
  FILE *f = fopen("preview.ppm", "wb");
  if (!f) return 1;
  fprintf(f, "P6\n%d %d\n255\n", W, H);
  fwrite(out.data(), 1, out.size(), f);
  fclose(f);
  printf("wrote preview.ppm (%dx%d)\n", W, H);
  return 0;
}
