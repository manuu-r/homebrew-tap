#include "ui.h"

#include <Fonts/FreeSans9pt7b.h>
#include <Fonts/FreeSansBold12pt7b.h>
#include <Fonts/FreeSansBold18pt7b.h>

#include <string.h>

#include "config.h"
#include "emoji.h"
#include "theme.h"

// ---------------------------------------------------------------------------
// Layout, 240 x 320 portrait.
//
//   0  .. 34   header     provider + window
//   38 .. 178  icon       the drawn "emoji", biggest element on screen
//   178.. 218  word       mood word, auto-shrinks to fit 240 px
//   220.. 250  ticker     stock-style delta since the last change
//   250.. 310  gauge      AVAILABLE label + progress bar
// ---------------------------------------------------------------------------

static const int ZH_Y = 0,   ZH_H = 34;
static const int ZI_Y = 38,  ZI_H = 140;
static const int ZW_Y = 178, ZW_H = 40;
static const int ZD_Y = 220, ZD_H = 30;
static const int ZB_Y = 250, ZB_H = 60;

static const int ICON_CY = ZI_Y + ZI_H / 2;
static const int ICON_S  = 112;

static const int BAR_X = 18, BAR_W = 204, BAR_Y = 276, BAR_H = 24, BAR_R = 8;

// ---------------------------------------------------------------------------

static void textBounds(Adafruit_GFX &g, const GFXfont *f, const char *s, int16_t &x1, int16_t &y1,
                       uint16_t &w, uint16_t &h) {
  g.setFont(f);
  g.getTextBounds(s, 0, 0, &x1, &y1, &w, &h);
}

static void drawCentered(Adafruit_GFX &g, const GFXfont *f, const char *s, int topY,
                         uint16_t color) {
  int16_t x1, y1;
  uint16_t w, h;
  textBounds(g, f, s, x1, y1, w, h);
  g.setTextColor(color);
  g.setCursor((TFT_W - (int)w) / 2 - x1, topY - y1);
  g.print(s);
}

static void drawLeft(Adafruit_GFX &g, const GFXfont *f, const char *s, int x, int topY,
                     uint16_t color) {
  int16_t x1, y1;
  uint16_t w, h;
  textBounds(g, f, s, x1, y1, w, h);
  g.setTextColor(color);
  g.setCursor(x - x1, topY - y1);
  g.print(s);
}

static void drawRight(Adafruit_GFX &g, const GFXfont *f, const char *s, int rightX, int topY,
                      uint16_t color) {
  int16_t x1, y1;
  uint16_t w, h;
  textBounds(g, f, s, x1, y1, w, h);
  g.setTextColor(color);
  g.setCursor(rightX - (int)w - x1, topY - y1);
  g.print(s);
}

static void drawEllipsized(Adafruit_GFX &g, const GFXfont *f, const char *s, int x, int topY,
                           int maxW, uint16_t color) {
  char buf[96];
  strncpy(buf, s ? s : "", sizeof(buf) - 1);
  buf[sizeof(buf) - 1] = '\0';
  size_t len = strlen(buf);
  int16_t x1, y1;
  uint16_t w, h;
  textBounds(g, f, buf, x1, y1, w, h);
  while ((int)w > maxW && len > 4) {
    len--;
    buf[len - 3] = '.';
    buf[len - 2] = '.';
    buf[len - 1] = '.';
    buf[len] = '\0';
    textBounds(g, f, buf, x1, y1, w, h);
  }
  drawLeft(g, f, buf, x, topY, color);
}

static void drawTwoLines(Adafruit_GFX &g, const char *text, int x, int topY, int maxW,
                         uint16_t color) {
  char first[48] = {};
  char second[72] = {};
  const char *src = text ? text : "";
  const size_t length = strlen(src);
  size_t split = length;
  if (split > 27) {
    split = 27;
    while (split > 12 && src[split] != ' ') split--;
    if (split <= 12) split = 27;
  }
  const size_t firstLen = split < sizeof(first) - 1 ? split : sizeof(first) - 1;
  memcpy(first, src, firstLen);
  first[firstLen] = '\0';
  const char *rest = src + split;
  while (*rest == ' ') rest++;
  strncpy(second, rest, sizeof(second) - 1);
  drawEllipsized(g, &FreeSans9pt7b, first, x, topY, maxW, color);
  if (second[0]) drawEllipsized(g, &FreeSans9pt7b, second, x, topY + 22, maxW, color);
}

static void drawPager(Adafruit_GFX &g, uint8_t active) {
  const int count = 5;
  const int gap = 14;
  const int start = (TFT_W - (count - 1) * gap) / 2;
  for (int i = 0; i < count; i++) {
    const uint16_t color = (i == active) ? C_TEXT : C_BORDER;
    g.fillCircle(start + i * gap, 311, i == active ? 3 : 2, color);
  }
}

static void drawPageChrome(Adafruit_GFX &g, const char *title, const char *subtitle,
                           uint16_t accent, uint8_t page) {
  g.fillScreen(C_BG);
  drawLeft(g, &FreeSansBold12pt7b, title, 14, 8, C_TEXT);
  drawRight(g, &FreeSans9pt7b, subtitle, TFT_W - 14, 11, C_MUTED);
  g.fillRoundRect(14, 34, TFT_W - 28, 3, 1, accent);
  drawPager(g, page);
}

// "Getting-Peckish" does not fit at 18 pt, so step down until it does.
static const GFXfont *fitFont(Adafruit_GFX &g, const char *s, int maxW) {
  static const GFXfont *ladder[] = {&FreeSansBold18pt7b, &FreeSansBold12pt7b, &FreeSans9pt7b};
  for (uint8_t i = 0; i < 3; i++) {
    int16_t x1, y1;
    uint16_t w, h;
    textBounds(g, ladder[i], s, x1, y1, w, h);
    if ((int)w <= maxW) return ladder[i];
  }
  return ladder[2];
}

// ---------------------------------------------------------------------------

struct RenderState {
  bool         init = false;
  ProviderKind provider = PROV_CLAUDE;
  Mood         mood = MOOD_DEAD;
  bool         havePct = false;
  int          pctI = -1;      // rounded percent
  int          deltaI = 0;     // signed tenths of a point
  bool         haveDelta = false;
  bool         stale = false;
};

static RenderState s_last;

void uiInvalidate() { s_last.init = false; }

// ---------------------------------------------------------------------------

static void drawHeader(Adafruit_GFX &g, const UiModel &m, uint16_t accent) {
  g.fillRect(0, ZH_Y, TFT_W, ZH_H, C_BG);
  drawLeft(g, &FreeSansBold12pt7b, providerName(m.provider), 14, ZH_Y + 6, C_TEXT);
  drawRight(g, &FreeSans9pt7b, providerWindow(m.provider), TFT_W - 14, ZH_Y + 10, C_MUTED);
  g.fillRect(14, ZH_Y + ZH_H - 4, TFT_W - 28, 3, accent);

  if (m.stale) g.fillCircle(TFT_W - 7, ZH_Y + 7, 3, C_DOWN);
  drawPager(g, m.provider == PROV_CODEX ? 0 : 1);
}

static void drawIcon(Adafruit_GFX &g, const UiModel &m, Mood mood) {
  g.fillRect(0, ZI_Y, TFT_W, ZI_H, C_BG);
  g.fillRoundRect(10, ZI_Y + 2, TFT_W - 20, ZI_H - 6, 12, C_CARD);
  g.drawRoundRect(10, ZI_Y + 2, TFT_W - 20, ZI_H - 6, 12, C_BORDER);
  if (!m.havePct) {
    drawCentered(g, &FreeSansBold18pt7b, "?", ICON_CY - 20, C_MUTED);
    return;
  }
  drawMoodIcon(g, mood, TFT_W / 2, ICON_CY, ICON_S);
}

static void drawWord(Adafruit_GFX &g, const UiModel &m, Mood mood, uint16_t accent) {
  g.fillRect(0, ZW_Y, TFT_W, ZW_H, C_BG);
  const char *word = m.havePct ? moodWord(mood) : "No-Data";
  const GFXfont *f = fitFont(g, word, TFT_W - 16);

  int16_t x1, y1;
  uint16_t w, h;
  textBounds(g, f, word, x1, y1, w, h);
  drawCentered(g, f, word, ZW_Y + (ZW_H - (int)h) / 2, m.havePct ? accent : C_MUTED);
}

static void drawTicker(Adafruit_GFX &g, const UiModel &m, Mood mood) {
  g.fillRect(0, ZD_Y, TFT_W, ZD_H, C_BG);

  // At zero the screen is meant to sit still on DEAD for the full page, so
  // the ticker is suppressed rather than showing the drop that got us here.
  if (!m.havePct || mood == MOOD_DEAD) return;

  char buf[16];
  uint16_t col;
  if (!m.haveDelta || fabsf(m.delta) < 0.05f) {
    snprintf(buf, sizeof(buf), "0.0%%");
    col = C_FLAT;
  } else {
    snprintf(buf, sizeof(buf), "%.1f%%", fabsf(m.delta));
    col = (m.delta > 0) ? C_UP : C_DOWN;
  }

  int16_t x1, y1;
  uint16_t w, h;
  textBounds(g, &FreeSansBold12pt7b, buf, x1, y1, w, h);

  const int arrowW = 14;
  const int total = arrowW + 8 + (int)w;
  const int startX = (TFT_W - total) / 2;
  const int midY = ZD_Y + ZD_H / 2;

  if (!m.haveDelta || fabsf(m.delta) < 0.05f) {
    g.fillRect(startX, midY - 2, arrowW, 4, col);
  } else if (m.delta > 0) {
    g.fillTriangle(startX + arrowW / 2, midY - 8, startX, midY + 6, startX + arrowW, midY + 6, col);
  } else {
    g.fillTriangle(startX + arrowW / 2, midY + 8, startX, midY - 6, startX + arrowW, midY - 6, col);
  }

  g.setFont(&FreeSansBold12pt7b);
  g.setTextColor(col);
  g.setCursor(startX + arrowW + 8 - x1, midY - (int)h / 2 - y1);
  g.print(buf);
}

static void drawGauge(Adafruit_GFX &g, const UiModel &m, uint16_t accent) {
  g.fillRect(0, ZB_Y, TFT_W, ZB_H, C_BG);
  g.fillRoundRect(10, ZB_Y, TFT_W - 20, ZB_H - 4, 10, C_CARD);
  g.drawRoundRect(10, ZB_Y, TFT_W - 20, ZB_H - 4, 10, C_BORDER);

  char pctBuf[8];
  if (m.havePct) snprintf(pctBuf, sizeof(pctBuf), "%d%%", (int)lroundf(m.pct));
  else snprintf(pctBuf, sizeof(pctBuf), "--");

  drawLeft(g, &FreeSans9pt7b, "AVAILABLE", BAR_X, ZB_Y + 6, C_MUTED);
  drawRight(g, &FreeSansBold12pt7b, pctBuf, BAR_X + BAR_W, ZB_Y + 2,
            m.havePct ? accent : C_MUTED);

  g.fillRoundRect(BAR_X, BAR_Y, BAR_W, BAR_H, BAR_R, C_TRACK);

  if (m.havePct && m.pct > 0.0f) {
    int fw = (int)lroundf(BAR_W * (m.pct / 100.0f));
    if (fw < 4) fw = 4;
    if (fw > BAR_W) fw = BAR_W;
    // A rounded rect narrower than its own diameter renders as a blob.
    if (fw >= 2 * BAR_R) g.fillRoundRect(BAR_X, BAR_Y, fw, BAR_H, BAR_R, accent);
    else g.fillRect(BAR_X + 2, BAR_Y + 2, fw, BAR_H - 4, accent);
  }
  g.drawRoundRect(BAR_X, BAR_Y, BAR_W, BAR_H, BAR_R, C_TRACK);
}

// ---------------------------------------------------------------------------

static const int ST_TITLE_Y = 120, ST_DETAIL_Y = 156, ST_BAR_Y = 200, ST_BAR_H = 12;

static void drawStatusBar(Adafruit_GFX &g, int pct) {
  if (pct < 0) return;
  if (pct > 100) pct = 100;
  g.fillRoundRect(BAR_X, ST_BAR_Y, BAR_W, ST_BAR_H, 6, C_TRACK);
  const int fw = BAR_W * pct / 100;
  if (fw >= ST_BAR_H) g.fillRoundRect(BAR_X, ST_BAR_Y, fw, ST_BAR_H, 6, C_WELLFED);
  else if (fw > 0) g.fillRect(BAR_X + 1, ST_BAR_Y + 2, fw, ST_BAR_H - 4, C_WELLFED);
}

void uiStatus(Adafruit_GFX &g, const char *title, const char *detail, int pct) {
  g.fillScreen(C_BG);
  uiInvalidate();

  drawCentered(g, &FreeSansBold12pt7b, title, ST_TITLE_Y, C_TEXT);
  if (detail && *detail) drawCentered(g, &FreeSans9pt7b, detail, ST_DETAIL_Y, C_MUTED);
  drawStatusBar(g, pct);
}

void uiStatusProgress(Adafruit_GFX &g, const char *detail, int pct) {
  g.fillRect(0, ST_DETAIL_Y - 2, TFT_W, 26, C_BG);
  if (detail && *detail) drawCentered(g, &FreeSans9pt7b, detail, ST_DETAIL_Y, C_MUTED);
  drawStatusBar(g, pct);
}

void uiRender(Adafruit_GFX &g, const UiModel &m) {
  const Mood mood = m.havePct ? moodFromPct(m.pct) : MOOD_DEAD;
  const uint16_t accent = m.havePct ? moodColor(mood) : C_MUTED;

  const int pctI = m.havePct ? (int)lroundf(m.pct) : -1;
  const int deltaI = (int)lroundf(m.delta * 10.0f);

  const bool first = !s_last.init;
  if (first) g.fillScreen(C_BG);

  const bool provChanged = first || m.provider != s_last.provider;
  const bool moodChanged = first || mood != s_last.mood || m.havePct != s_last.havePct;

  if (provChanged || m.stale != s_last.stale || moodChanged) drawHeader(g, m, accent);
  if (provChanged || moodChanged) drawIcon(g, m, mood);
  if (provChanged || moodChanged) drawWord(g, m, mood, accent);
  if (provChanged || moodChanged || deltaI != s_last.deltaI || m.haveDelta != s_last.haveDelta)
    drawTicker(g, m, mood);
  if (provChanged || moodChanged || pctI != s_last.pctI) drawGauge(g, m, accent);

  s_last.init = true;
  s_last.provider = m.provider;
  s_last.mood = mood;
  s_last.havePct = m.havePct;
  s_last.pctI = pctI;
  s_last.deltaI = deltaI;
  s_last.haveDelta = m.haveDelta;
  s_last.stale = m.stale;
}

void uiRenderCalendar(Adafruit_GFX &g, const DashboardData &data) {
  uiInvalidate();
  drawPageChrome(g, "CALENDAR", "NEXT", C_BLUE, 2);

  if (!data.calendarEnabled) {
    drawCentered(g, &FreeSansBold12pt7b, "Calendar off", 142, C_MUTED);
    return;
  }
  if (data.eventCount == 0) {
    const char *message = data.calendarError[0] ? data.calendarError : "No upcoming events";
    drawTwoLines(g, message, 20, 136, TFT_W - 40, C_MUTED);
    return;
  }

  for (uint8_t i = 0; i < data.eventCount; i++) {
    const int y = 48 + i * 82;
    g.fillRoundRect(10, y, TFT_W - 20, 70, 10, C_CARD);
    g.drawRoundRect(10, y, TFT_W - 20, 70, 10, C_BORDER);
    drawLeft(g, &FreeSans9pt7b, data.events[i].allDay ? "ALL DAY" : "UPCOMING", 20, y + 8,
             C_BLUE);
    drawTwoLines(g, data.events[i].title, 20, y + 29, TFT_W - 40, C_TEXT);
  }
}

void uiRenderTickers(Adafruit_GFX &g, const DashboardData &data) {
  uiInvalidate();
  drawPageChrome(g, "MARKETS", "LIVE", C_VIOLET, 3);

  if (!data.tickersEnabled) {
    drawCentered(g, &FreeSansBold12pt7b, "Tickers off", 142, C_MUTED);
    return;
  }
  if (data.tickerCount == 0) {
    const char *message = data.tickerError[0] ? data.tickerError : "Quotes unavailable";
    drawTwoLines(g, message, 20, 136, TFT_W - 40, C_MUTED);
    return;
  }

  for (uint8_t i = 0; i < data.tickerCount; i++) {
    const int y = 48 + i * 60;
    const TickerEntry &ticker = data.tickers[i];
    g.fillRoundRect(10, y, TFT_W - 20, 50, 9, C_CARD);
    g.drawRoundRect(10, y, TFT_W - 20, 50, 9, C_BORDER);
    drawEllipsized(g, &FreeSans9pt7b, ticker.label, 20, y + 8, 100, C_TEXT);

    char price[20];
    snprintf(price, sizeof(price), "%.2f", (double)ticker.price);
    drawLeft(g, &FreeSansBold12pt7b, price, 20, y + 27, C_TEXT);

    char change[16];
    if (ticker.haveChange) snprintf(change, sizeof(change), "%+.2f%%", (double)ticker.changePct);
    else snprintf(change, sizeof(change), "--");
    const uint16_t color = !ticker.haveChange ? C_MUTED
                            : ticker.changePct > 0 ? C_UP
                            : ticker.changePct < 0 ? C_DOWN
                                                   : C_FLAT;
    drawRight(g, &FreeSansBold12pt7b, change, TFT_W - 20, y + 27, color);
  }
}

void uiRenderTodos(Adafruit_GFX &g, const DashboardData &data) {
  uiInvalidate();
  drawPageChrome(g, "TODAY", "TO-DO", C_WELLFED, 4);

  if (data.todoCount == 0) {
    drawCentered(g, &FreeSansBold12pt7b, "All clear", 142, C_MUTED);
    return;
  }

  for (uint8_t i = 0; i < data.todoCount; i++) {
    const int y = 48 + i * 49;
    const TodoEntry &todo = data.todos[i];
    g.fillRoundRect(10, y, TFT_W - 20, 40, 8, C_CARD);
    g.drawRoundRect(10, y, TFT_W - 20, 40, 8, C_BORDER);
    g.drawRoundRect(20, y + 12, 14, 14, 3, todo.completed ? C_WELLFED : C_MUTED);
    if (todo.completed) {
      g.drawLine(23, y + 19, 26, y + 23, C_WELLFED);
      g.drawLine(26, y + 23, 32, y + 15, C_WELLFED);
    }
    drawEllipsized(g, &FreeSans9pt7b, todo.title, 44, y + 11, TFT_W - 60,
                   todo.completed ? C_MUTED : C_TEXT);
  }

  if (data.todoTotal > data.todoCount) {
    char more[20];
    snprintf(more, sizeof(more), "+%u more", (unsigned)(data.todoTotal - data.todoCount));
    drawRight(g, &FreeSans9pt7b, more, TFT_W - 14, 288, C_MUTED);
  }
}
