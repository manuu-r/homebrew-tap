#include "ui.h"

#include <Fonts/FreeSans9pt7b.h>
#include <Fonts/FreeSansBold12pt7b.h>
#include <Fonts/FreeSansBold18pt7b.h>

#include <ctype.h>
#include <string.h>

#include "config.h"
#include "emoji.h"
#include "mood.h"
#include "theme.h"

// ---------------------------------------------------------------- text ---

static void bounds(Adafruit_GFX &g, const GFXfont *f, const char *s, int16_t &x1, int16_t &y1,
                   uint16_t &w, uint16_t &h) {
  g.setFont(f);
  g.getTextBounds(s, 0, 0, &x1, &y1, &w, &h);
}

// Draws `s` with its top edge at `topY`. `align`: -1 left of x, 0 centred on
// the screen, 1 right of x.
static void text(Adafruit_GFX &g, const GFXfont *f, const char *s, int x, int topY, uint16_t color,
                 int align = -1) {
  int16_t x1, y1;
  uint16_t w, h;
  bounds(g, f, s, x1, y1, w, h);
  const int left = align < 0 ? x : align > 0 ? x - (int)w : (TFT_W - (int)w) / 2;
  g.setTextColor(color);
  g.setCursor(left - x1, topY - y1);
  g.print(s);
}

static int width(Adafruit_GFX &g, const GFXfont *f, const char *s) {
  int16_t x1, y1;
  uint16_t w, h;
  bounds(g, f, s, x1, y1, w, h);
  return w;
}

static void ellipsized(Adafruit_GFX &g, const GFXfont *f, const char *s, int x, int topY,
                       int maxW, uint16_t color) {
  char buf[96];
  strncpy(buf, s ? s : "", sizeof(buf) - 1);
  buf[sizeof(buf) - 1] = '\0';
  size_t len = strlen(buf);
  int16_t x1, y1;
  uint16_t w, h;
  bounds(g, f, buf, x1, y1, w, h);
  while ((int)w > maxW && len > 4) {
    len--;
    strcpy(buf + len - 3, "...");
    bounds(g, f, buf, x1, y1, w, h);
  }
  text(g, f, buf, x, topY, color);
}

// Wraps at a space near 27 characters onto at most two lines.
static void twoLines(Adafruit_GFX &g, const char *s, int x, int topY, int maxW, uint16_t color) {
  char first[48] = {};
  size_t split = strlen(s);
  if (split > 27) {
    split = 27;
    while (split > 12 && s[split] != ' ') split--;
    if (split <= 12) split = 27;
  }
  memcpy(first, s, split < sizeof(first) ? split : sizeof(first) - 1);
  const char *rest = s + split;
  while (*rest == ' ') rest++;
  ellipsized(g, &FreeSans9pt7b, first, x, topY, maxW, color);
  if (*rest) ellipsized(g, &FreeSans9pt7b, rest, x, topY + 22, maxW, color);
}

// "now", "25m", "3h 5m", "2d 4h"
static void relative(char *out, size_t n, int64_t seconds) {
  if (seconds < 60) {
    snprintf(out, n, "now");
  } else if (seconds < 3600) {
    snprintf(out, n, "%dm", (int)(seconds / 60));
  } else if (seconds < 86400) {
    snprintf(out, n, "%dh %dm", (int)(seconds / 3600), (int)(seconds % 3600 / 60));
  } else {
    snprintf(out, n, "%dd %dh", (int)(seconds / 86400), (int)(seconds % 86400 / 3600));
  }
}

// ---------------------------------------------------------------- chrome ---

static void chrome(Adafruit_GFX &g, const char *title, const char *subtitle, uint16_t accent,
                   bool stale, uint8_t page, uint8_t pages) {
  g.fillScreen(C_BG);
  text(g, &FreeSans9pt7b, subtitle, TFT_W - 14, 11, C_MUTED, 1);
  ellipsized(g, &FreeSansBold12pt7b, title, 14, 8, TFT_W - 38 - width(g, &FreeSans9pt7b, subtitle),
             C_TEXT);
  g.fillRoundRect(14, 34, TFT_W - 28, 3, 1, accent);
  if (stale) g.fillCircle(TFT_W - 6, 6, 3, C_DOWN);

  const int gap = 12;
  const int start = (TFT_W - (pages - 1) * gap) / 2;
  for (uint8_t i = 0; i < pages; i++) {
    g.fillCircle(start + i * gap, 312, i == page ? 3 : 2, i == page ? C_TEXT : C_BORDER);
  }
}

static void card(Adafruit_GFX &g, int y, int h) {
  g.fillRoundRect(10, y, TFT_W - 20, h, 10, C_CARD);
  g.drawRoundRect(10, y, TFT_W - 20, h, 10, C_BORDER);
}

static void bar(Adafruit_GFX &g, int x, int y, int w, int h, float pct, uint16_t color) {
  g.fillRoundRect(x, y, w, h, h / 2, C_TRACK);
  const int fill = (int)(w * pct / 100.0f);
  if (fill >= h) g.fillRoundRect(x, y, fill, h, h / 2, color);
  else if (fill > 0) g.fillRect(x, y + 1, fill, h - 2, color);
}

// ---------------------------------------------------------------- screens ---

void uiStatus(Adafruit_GFX &g, const char *title, const char *detail, int pct) {
  g.fillScreen(C_BG);
  text(g, &FreeSansBold12pt7b, title, 0, 120, C_TEXT, 0);
  if (detail && *detail) text(g, &FreeSans9pt7b, detail, 0, 156, C_MUTED, 0);
  if (pct >= 0) bar(g, 18, 200, TFT_W - 36, 12, pct > 100 ? 100 : pct, C_WELLFED);
}

void uiPairing(Adafruit_GFX &g, const char *name) {
  g.fillScreen(C_BG);
  text(g, &FreeSansBold18pt7b, "PAIR", 0, 70, C_TEXT, 0);
  text(g, &FreeSans9pt7b, name, 0, 118, C_BLUE, 0);
  text(g, &FreeSans9pt7b, "In Gauge: Settings >", 0, 176, C_MUTED, 0);
  text(g, &FreeSans9pt7b, "Pair Accessory...", 0, 198, C_MUTED, 0);
}

void uiCompare(Adafruit_GFX &g, uint32_t number) {
  char digits[8];
  snprintf(digits, sizeof(digits), "%06lu", (unsigned long)(number % 1000000));
  g.fillScreen(C_BG);
  text(g, &FreeSans9pt7b, "Does the Mac show", 0, 70, C_MUTED, 0);
  text(g, &FreeSansBold18pt7b, digits, 0, 120, C_TEXT, 0);
  text(g, &FreeSans9pt7b, "Press BOOT to confirm", 0, 196, C_WELLFED, 0);
  text(g, &FreeSans9pt7b, "Wait to reject", 0, 220, C_MUTED, 0);
}

void uiProvider(Adafruit_GFX &g, const ProviderEntry &p, int64_t now, bool stale, uint8_t page,
                uint8_t pages) {
  const Mood mood = p.haveRemaining ? moodFromPct(p.remaining) : MOOD_DEAD;
  const uint16_t accent = p.haveRemaining ? moodColor(mood) : C_MUTED;
  char title[sizeof(p.name)];
  for (size_t i = 0; i < sizeof(title); i++) title[i] = (char)toupper((unsigned char)p.name[i]);

  char headline[8] = "--";
  if (p.haveRemaining) snprintf(headline, sizeof(headline), "%d%%", (int)lroundf(p.remaining));
  chrome(g, title, headline, accent, stale, page, pages);

  card(g, 44, 110);
  if (p.haveRemaining) drawMoodIcon(g, mood, TFT_W / 2, 99, 88);
  else text(g, &FreeSansBold18pt7b, "?", 0, 84, C_MUTED, 0);
  text(g, &FreeSansBold12pt7b, p.haveRemaining ? moodWord(mood) : "No data", 0, 162, accent, 0);

  for (uint8_t i = 0; i < p.limitCount; i++) {
    const LimitEntry &limit = p.limits[i];
    const int y = 198 + i * 36;
    char right[24];
    int length = snprintf(right, sizeof(right), "%d%%", (int)lroundf(limit.remaining));
    if (limit.resetsAt > now) {
      char in[12];
      relative(in, sizeof(in), limit.resetsAt - now);
      snprintf(right + length, sizeof(right) - length, "  %s", in);
    }
    text(g, &FreeSans9pt7b, right, TFT_W - 18, y, C_MUTED, 1);
    ellipsized(g, &FreeSans9pt7b, limit.label, 18, y, TFT_W - 44 - width(g, &FreeSans9pt7b, right),
               C_TEXT);
    bar(g, 18, y + 18, TFT_W - 36, 7, limit.remaining, moodColor(moodFromPct(limit.remaining)));
  }
}

void uiQuotaError(Adafruit_GFX &g, const DashboardData &d, bool stale, uint8_t page,
                  uint8_t pages) {
  chrome(g, "QUOTA", "", C_MUTED, stale, page, pages);
  const char *message = d.settingsError[0] ? d.settingsError
                        : d.quotaError[0]  ? d.quotaError
                                           : "Every provider is off";
  twoLines(g, message, 20, 136, TFT_W - 40, C_MUTED);
}

void uiCalendar(Adafruit_GFX &g, const DashboardData &d, int64_t now, bool stale, uint8_t page,
                uint8_t pages) {
  chrome(g, "CALENDAR", "NEXT", C_BLUE, stale, page, pages);
  if (!d.calendarEnabled || d.eventCount == 0) {
    const char *message = !d.calendarEnabled   ? "Calendar off"
                          : d.calendarError[0] ? d.calendarError
                                               : "No upcoming events";
    twoLines(g, message, 20, 136, TFT_W - 40, C_MUTED);
    return;
  }
  for (uint8_t i = 0; i < d.eventCount; i++) {
    const CalendarEntry &event = d.events[i];
    const int y = 48 + i * 86;
    char when[16] = "ALL DAY";
    if (!event.allDay) {
      if (event.startsAt <= now) snprintf(when, sizeof(when), "NOW");
      else {
        char in[12];
        relative(in, sizeof(in), event.startsAt - now);
        snprintf(when, sizeof(when), "IN %s", in);
      }
    }
    card(g, y, 76);
    text(g, &FreeSans9pt7b, when, 20, y + 8, C_BLUE);
    twoLines(g, event.title, 20, y + 30, TFT_W - 40, C_TEXT);
  }
}

void uiTodos(Adafruit_GFX &g, const DashboardData &d, bool stale, uint8_t page, uint8_t pages) {
  chrome(g, "TO-DO", "TODAY", C_WELLFED, stale, page, pages);
  if (d.todoCount == 0) {
    text(g, &FreeSansBold12pt7b, "All clear", 0, 142, C_MUTED, 0);
    return;
  }
  for (uint8_t i = 0; i < d.todoCount; i++) {
    const TodoEntry &todo = d.todos[i];
    const int y = 48 + i * 49;
    const uint16_t color = todo.completed ? C_MUTED : C_TEXT;
    card(g, y, 40);
    g.drawRoundRect(20, y + 12, 14, 14, 3, todo.completed ? C_WELLFED : C_MUTED);
    if (todo.completed) {
      g.drawLine(23, y + 19, 26, y + 23, C_WELLFED);
      g.drawLine(26, y + 23, 32, y + 15, C_WELLFED);
      g.drawFastHLine(44, y + 20, TFT_W - 64, C_MUTED);  // strikethrough, as in Gauge
    }
    ellipsized(g, &FreeSans9pt7b, todo.title, 44, y + 11, TFT_W - 64, color);
  }
  if (d.todoTotal > d.todoCount) {
    char more[20];
    snprintf(more, sizeof(more), "+%u more", (unsigned)(d.todoTotal - d.todoCount));
    text(g, &FreeSans9pt7b, more, TFT_W - 14, 290, C_MUTED, 1);
  }
}
