#include "emoji.h"
#include "theme.h"

// ---------------------------------------------------------------------------
// Small primitive helpers
// ---------------------------------------------------------------------------

// Scale a design coordinate authored against a nominal 100 px box.
static inline int sc(int s, float v) { return (int)lroundf(s * v * 0.01f); }

static void thickLine(Adafruit_GFX &g, int x0, int y0, int x1, int y1, int th, uint16_t c) {
  if (th < 1) th = 1;
  const float dx = (float)(x1 - x0), dy = (float)(y1 - y0);
  const float len = sqrtf(dx * dx + dy * dy);
  const float nx = (len > 0.001f) ? (-dy / len) : 0.0f;
  const float ny = (len > 0.001f) ? (dx / len) : 0.0f;
  for (int o = -th / 2; o <= th / 2; o++) {
    const int ox = (int)lroundf(nx * o), oy = (int)lroundf(ny * o);
    g.drawLine(x0 + ox, y0 + oy, x1 + ox, y1 + oy, c);
  }
}

static void thickCurve(Adafruit_GFX &g, int x0, int y0, int cxp, int cyp, int x1, int y1,
                       int th, uint16_t c) {
  int px = x0, py = y0;
  for (int i = 1; i <= 20; i++) {
    const float t = i / 20.0f, mt = 1.0f - t;
    const int x = (int)lroundf(mt * mt * x0 + 2 * mt * t * cxp + t * t * x1);
    const int y = (int)lroundf(mt * mt * y0 + 2 * mt * t * cyp + t * t * y1);
    thickLine(g, px, py, x, y, th, c);
    px = x;
    py = y;
  }
}

// A bone laid along `angleDeg`, with the two lobes at each end.
static void boneRot(Adafruit_GFX &g, int cx, int cy, int len, float angleDeg, int th, uint16_t c) {
  const float a = angleDeg * 0.01745329f;
  const float dx = cosf(a), dy = sinf(a);
  const int hx = (int)lroundf(dx * len * 0.5f), hy = (int)lroundf(dy * len * 0.5f);
  thickLine(g, cx - hx, cy - hy, cx + hx, cy + hy, th, c);

  const int lr = (th * 3) / 4 > 2 ? (th * 3) / 4 : 2;
  const int ox = (int)lroundf(-dy * th * 0.60f), oy = (int)lroundf(dx * th * 0.60f);
  g.fillCircle(cx - hx + ox, cy - hy + oy, lr, c);
  g.fillCircle(cx - hx - ox, cy - hy - oy, lr, c);
  g.fillCircle(cx + hx + ox, cy + hy + oy, lr, c);
  g.fillCircle(cx + hx - ox, cy + hy - oy, lr, c);
}

static void skullFace(Adafruit_GFX &g, int cx, int cy, int s, uint16_t bone, uint16_t hollow) {
  // cranium
  g.fillRoundRect(cx - sc(s, 38), cy - sc(s, 40), sc(s, 76), sc(s, 66), sc(s, 30), bone);
  // jaw
  g.fillRoundRect(cx - sc(s, 23), cy + sc(s, 12), sc(s, 46), sc(s, 25), sc(s, 8), bone);
  // eye sockets
  const int er = sc(s, 12);
  g.fillCircle(cx - sc(s, 17), cy - sc(s, 10), er, hollow);
  g.fillCircle(cx + sc(s, 17), cy - sc(s, 10), er, hollow);
  // nose
  g.fillTriangle(cx, cy + sc(s, 1), cx - sc(s, 6), cy + sc(s, 13), cx + sc(s, 6), cy + sc(s, 13), hollow);
  // teeth
  const int ty = cy + sc(s, 17);
  for (int i = -1; i <= 1; i++) g.drawFastVLine(cx + i * sc(s, 9), ty, sc(s, 17), hollow);
}

// ---------------------------------------------------------------------------
// 100-80  Well-Fed  - steak
// ---------------------------------------------------------------------------
static void drawSteak(Adafruit_GFX &g, int cx, int cy, int s) {
  const uint16_t FAT  = rgb565(243, 231, 212);
  const uint16_t MEAT = rgb565(186, 44, 50);
  const uint16_t SEAR = rgb565(132, 28, 36);
  const uint16_t MARB = rgb565(232, 176, 170);

  const int w = sc(s, 96), h = sc(s, 70);
  const int x = cx - w / 2, y = cy - h / 2;
  const int rim = sc(s, 7) > 2 ? sc(s, 7) : 2;

  // fat rim, with an extra lobe so the silhouette isn't a plain pill
  g.fillRoundRect(x, y, w, h, h / 3, FAT);
  g.fillCircle(x + sc(s, 30), y + sc(s, 18), sc(s, 23), FAT);

  g.fillRoundRect(x + rim, y + rim, w - 2 * rim, h - 2 * rim, (h - 2 * rim) / 3, MEAT);
  g.fillCircle(x + sc(s, 30), y + sc(s, 18), sc(s, 23) - rim, MEAT);

  // seared underside
  g.fillRoundRect(x + rim, y + h - rim - sc(s, 16), w - 2 * rim, sc(s, 16), sc(s, 8), SEAR);

  // marbling
  thickLine(g, cx - sc(s, 22), cy - sc(s, 6), cx - sc(s, 4), cy - sc(s, 13), sc(s, 4), MARB);
  thickLine(g, cx + sc(s, 2), cy + sc(s, 3), cx + sc(s, 22), cy - sc(s, 3), sc(s, 4), MARB);
  thickLine(g, cx - sc(s, 14), cy + sc(s, 10), cx + sc(s, 2), cy + sc(s, 12), sc(s, 3), MARB);

  // little rib bone poking out on the left
  boneRot(g, x - sc(s, 2), cy + sc(s, 14), sc(s, 16), 20.0f, sc(s, 6), rgb565(250, 247, 240));
}

// ---------------------------------------------------------------------------
// 79-60  Getting-Peckish  - cookie
// ---------------------------------------------------------------------------
static void drawCookie(Adafruit_GFX &g, int cx, int cy, int s) {
  const uint16_t EDGE  = rgb565(180, 130, 68);
  const uint16_t DOUGH = rgb565(216, 168, 100);
  const uint16_t CHIP  = rgb565(72, 42, 26);

  const int r = sc(s, 47);
  g.fillCircle(cx, cy, r, EDGE);
  g.fillCircle(cx, cy, r - (sc(s, 4) > 1 ? sc(s, 4) : 1), DOUGH);

  static const int8_t px[] = {-26,   4,  24, -10,  16, -22,   1};
  static const int8_t py[] = {-18, -27,   3,  13,  25,  20,  -3};
  const int cr = sc(s, 7) > 2 ? sc(s, 7) : 2;
  for (uint8_t i = 0; i < sizeof(px); i++) {
    g.fillCircle(cx + sc(s, px[i]), cy + sc(s, py[i]), cr, CHIP);
  }
  // crumbs
  g.fillCircle(cx - sc(s, 34), cy + sc(s, 8), sc(s, 3), CHIP);
  g.fillCircle(cx + sc(s, 31), cy - sc(s, 16), sc(s, 3), CHIP);
}

// ---------------------------------------------------------------------------
// 59-40  Hungry  - burger
// ---------------------------------------------------------------------------
static void drawBurger(Adafruit_GFX &g, int cx, int cy, int s) {
  const uint16_t BUN     = rgb565(226, 164, 84);
  const uint16_t SESAME  = rgb565(248, 232, 200);
  const uint16_t LETTUCE = rgb565(96, 176, 76);
  const uint16_t PATTY   = rgb565(112, 64, 38);
  const uint16_t CHEESE  = rgb565(244, 186, 62);

  const int w = sc(s, 92);
  const int x = cx - w / 2;
  const int top = cy - sc(s, 42);

  // domed top bun: draw a tall pill, then square off its lower half
  const int domeH = sc(s, 34);
  g.fillRoundRect(x, top, w, domeH * 2, domeH, BUN);
  g.fillRect(x, top + domeH, w, domeH, C_BG);

  g.fillCircle(cx - sc(s, 18), top + sc(s, 13), sc(s, 3), SESAME);
  g.fillCircle(cx + sc(s, 6),  top + sc(s, 9),  sc(s, 3), SESAME);
  g.fillCircle(cx + sc(s, 24), top + sc(s, 18), sc(s, 3), SESAME);

  // cheese slice with drips
  const int cy0 = top + domeH;
  g.fillRect(x + sc(s, 2), cy0, w - sc(s, 4), sc(s, 7), CHEESE);
  g.fillTriangle(x + sc(s, 12), cy0 + sc(s, 7), x + sc(s, 22), cy0 + sc(s, 7),
                 x + sc(s, 17), cy0 + sc(s, 15), CHEESE);
  g.fillTriangle(x + sc(s, 58), cy0 + sc(s, 7), x + sc(s, 68), cy0 + sc(s, 7),
                 x + sc(s, 63), cy0 + sc(s, 14), CHEESE);

  // lettuce frill
  const int ly = cy0 + sc(s, 7);
  g.fillRect(x - sc(s, 3), ly, w + sc(s, 6), sc(s, 7), LETTUCE);
  for (int i = 0; i < 5; i++) {
    const int lx = x - sc(s, 3) + i * (w + sc(s, 6)) / 5;
    g.fillTriangle(lx, ly + sc(s, 7), lx + (w + sc(s, 6)) / 5, ly + sc(s, 7),
                   lx + (w + sc(s, 6)) / 10, ly + sc(s, 14), LETTUCE);
  }

  // patty
  g.fillRoundRect(x + sc(s, 1), ly + sc(s, 12), w - sc(s, 2), sc(s, 16), sc(s, 5), PATTY);

  // bottom bun
  g.fillRoundRect(x + sc(s, 3), ly + sc(s, 27), w - sc(s, 6), sc(s, 19), sc(s, 8), BUN);
}

// ---------------------------------------------------------------------------
// 39-20  Starving  - wilted flower
// ---------------------------------------------------------------------------
static void drawWiltedFlower(Adafruit_GFX &g, int cx, int cy, int s) {
  const uint16_t STEM  = rgb565(86, 124, 64);
  const uint16_t LEAF  = rgb565(104, 142, 74);
  const uint16_t PETAL = rgb565(168, 54, 74);
  const uint16_t PDARK = rgb565(118, 36, 52);
  const uint16_t CORE  = rgb565(86, 62, 30);

  // stem: up from the base, then arcing over to the right and drooping
  const int baseX = cx - sc(s, 14), baseY = cy + sc(s, 44);
  thickCurve(g, baseX, baseY, cx - sc(s, 22), cy - sc(s, 20), cx + sc(s, 14), cy - sc(s, 26),
             sc(s, 5), STEM);

  // drooping leaf
  g.fillTriangle(baseX + sc(s, 2), cy + sc(s, 16), baseX - sc(s, 24), cy + sc(s, 30),
                 baseX + sc(s, 4), cy + sc(s, 34), LEAF);

  // head hanging off the end of the stem
  const int hx = cx + sc(s, 18), hy = cy - sc(s, 12);
  const int pr = sc(s, 12) > 3 ? sc(s, 12) : 3;
  g.fillCircle(hx - sc(s, 10), hy + sc(s, 2),  pr, PDARK);
  g.fillCircle(hx + sc(s, 8),  hy + sc(s, 4),  pr, PDARK);
  g.fillCircle(hx - sc(s, 2),  hy + sc(s, 14), pr, PETAL);
  g.fillCircle(hx + sc(s, 2),  hy - sc(s, 6),  pr, PETAL);
  g.fillCircle(hx, hy + sc(s, 4), sc(s, 8) > 2 ? sc(s, 8) : 2, CORE);

  // shed petals on the ground
  g.fillCircle(cx - sc(s, 30), cy + sc(s, 40), sc(s, 6), PDARK);
  g.fillCircle(cx + sc(s, 22), cy + sc(s, 44), sc(s, 5), PETAL);
}

// ---------------------------------------------------------------------------
// 19-5  Feral  - bone
// ---------------------------------------------------------------------------
static void drawBone(Adafruit_GFX &g, int cx, int cy, int s) {
  const uint16_t BONE  = rgb565(246, 243, 232);
  const uint16_t SHADE = rgb565(198, 192, 176);
  boneRot(g, cx, cy + sc(s, 3), sc(s, 62), -18.0f, sc(s, 15), SHADE);
  boneRot(g, cx, cy, sc(s, 62), -18.0f, sc(s, 14), BONE);
}

// ---------------------------------------------------------------------------
// 4-1  Near-Death  - skull and crossbones
// ---------------------------------------------------------------------------
static void drawSkullCrossbones(Adafruit_GFX &g, int cx, int cy, int s) {
  const uint16_t BONE = rgb565(238, 235, 226);
  boneRot(g, cx, cy + sc(s, 24), sc(s, 84),  32.0f, sc(s, 9), BONE);
  boneRot(g, cx, cy + sc(s, 24), sc(s, 84), -32.0f, sc(s, 9), BONE);
  skullFace(g, cx, cy - sc(s, 12), sc(s, 82), BONE, C_BG);
}

// ---------------------------------------------------------------------------
// 0  DEAD  - skull
// ---------------------------------------------------------------------------
static void drawSkull(Adafruit_GFX &g, int cx, int cy, int s) {
  skullFace(g, cx, cy, s, rgb565(232, 229, 220), C_BG);
}

// ---------------------------------------------------------------------------

void drawMoodIcon(Adafruit_GFX &g, Mood m, int cx, int cy, int s) {
  switch (m) {
    case MOOD_WELLFED:   drawSteak(g, cx, cy, s);           break;
    case MOOD_PECKISH:   drawCookie(g, cx, cy, s);          break;
    case MOOD_HUNGRY:    drawBurger(g, cx, cy, s);          break;
    case MOOD_STARVING:  drawWiltedFlower(g, cx, cy, s);    break;
    case MOOD_FERAL:     drawBone(g, cx, cy, s);            break;
    case MOOD_NEARDEATH: drawSkullCrossbones(g, cx, cy, s); break;
    case MOOD_DEAD:      drawSkull(g, cx, cy, s);           break;
  }
}
