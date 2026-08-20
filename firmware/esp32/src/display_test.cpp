// ---------------------------------------------------------------------------
// Display bring-up diagnostic.  pio run -e displaytest -t upload
//
// Walks every plausible way of talking to the panel and narrates each step on
// serial, so you can match what the screen does against what the board tried:
//
//   1. Software SPI (bit-banged on the same pins). Bypasses HSPI entirely, so
//      if this works the wiring is good and the hardware-SPI setup is at fault.
//   2. Hardware SPI, SPI_MODE0, slow then fast.
//   3. Hardware SPI, SPI_MODE3 (what many non-Adafruit ST7789 boards want).
//
// Each stage fills RED -> GREEN -> BLUE -> a quadrant/text pattern. Any stage
// that changes the screen is a working configuration.
// ---------------------------------------------------------------------------

#include <Adafruit_GFX.h>
#include <Adafruit_ST7789.h>
#include <SPI.h>

#include "config.h"
#include "mood.h"
#include "ui.h"

static SPIClass tftSPI(HSPI);

static Adafruit_ST7789 tftSw(PIN_TFT_CS, PIN_TFT_DC, PIN_TFT_MOSI, PIN_TFT_SCLK, PIN_TFT_RST);
static Adafruit_ST7789 tftHw(&tftSPI, PIN_TFT_CS, PIN_TFT_DC, PIN_TFT_RST);

static void pulseReset() {
  if (PIN_TFT_RST < 0) return;
  pinMode(PIN_TFT_RST, OUTPUT);
  digitalWrite(PIN_TFT_RST, HIGH);
  delay(50);
  digitalWrite(PIN_TFT_RST, LOW);
  delay(50);
  digitalWrite(PIN_TFT_RST, HIGH);
  delay(150);
}

static void cycle(Adafruit_ST7789 &t, const char *label) {
  Serial.printf("  [%s] RED\n", label);
  t.fillScreen(ST77XX_RED);
  delay(2000);
  Serial.printf("  [%s] GREEN\n", label);
  t.fillScreen(ST77XX_GREEN);
  delay(2000);
  Serial.printf("  [%s] BLUE\n", label);
  t.fillScreen(ST77XX_BLUE);
  delay(2000);

  Serial.printf("  [%s] quadrants + text\n", label);
  t.fillScreen(ST77XX_BLACK);
  t.fillRect(0, 0, TFT_W / 2, TFT_H / 2, ST77XX_RED);
  t.fillRect(TFT_W / 2, TFT_H / 2, TFT_W / 2, TFT_H / 2, ST77XX_CYAN);
  t.setFont();  // built-in font, simplest possible text path
  t.setTextColor(ST77XX_WHITE);
  t.setTextSize(3);
  t.setCursor(8, TFT_H / 2 - 12);
  t.print(label);
  delay(3000);
}

void setup() {
  Serial.begin(115200);
  delay(300);
  Serial.println("\n\n########## display bring-up test ##########");
  Serial.printf("pins: CS=%d DC=%d RST=%d MOSI=%d SCLK=%d BLK=%d\n", PIN_TFT_CS, PIN_TFT_DC,
                PIN_TFT_RST, PIN_TFT_MOSI, PIN_TFT_SCLK, PIN_TFT_BLK);
  Serial.printf("panel: %ux%u rotation %u\n", TFT_W, TFT_H, TFT_ROTATION);

  if (PIN_TFT_BLK >= 0) {
    pinMode(PIN_TFT_BLK, OUTPUT);
    digitalWrite(PIN_TFT_BLK, HIGH);
    Serial.println("backlight pin driven HIGH");
  }

  // ---- 1. software SPI ----------------------------------------------------
  Serial.println("\n===== STAGE 1: SOFTWARE SPI (bit-banged) =====");
  pulseReset();
  tftSw.init(TFT_W, TFT_H, SPI_MODE0);
  tftSw.setRotation(TFT_ROTATION);
  cycle(tftSw, "SW SPI");

  // ---- 2. hardware SPI, mode 0 -------------------------------------------
  Serial.println("\n===== STAGE 2: HW SPI (HSPI remap) MODE0 @ 10 MHz =====");
  tftSPI.begin(PIN_TFT_SCLK, PIN_TFT_MISO, PIN_TFT_MOSI, PIN_TFT_CS);
  pulseReset();
  tftHw.init(TFT_W, TFT_H, SPI_MODE0);
  tftHw.setSPISpeed(10000000UL);
  tftHw.setRotation(TFT_ROTATION);
  cycle(tftHw, "HW M0 10M");

  Serial.println("\n===== STAGE 3: HW SPI MODE0 @ 40 MHz =====");
  tftHw.setSPISpeed(40000000UL);
  cycle(tftHw, "HW M0 40M");

  // ---- 3. hardware SPI, mode 3 -------------------------------------------
  Serial.println("\n===== STAGE 4: HW SPI MODE3 @ 10 MHz =====");
  pulseReset();
  tftHw.init(TFT_W, TFT_H, SPI_MODE3);
  tftHw.setSPISpeed(10000000UL);
  tftHw.setRotation(TFT_ROTATION);
  cycle(tftHw, "HW M3 10M");

  // ---- 4. the real UI, on whichever config was left active ----------------
  Serial.println("\n===== STAGE 5: real dashboard frame =====");
  UiModel m;
  m.provider = PROV_CLAUDE;
  m.havePct = true;
  m.pct = 40.0f;
  m.haveDelta = true;
  m.delta = -3.0f;
  uiInvalidate();
  uiRender(tftHw, m);

  Serial.println("\n########## test complete, looping stage 1 ##########");
}

void loop() {
  // Re-run software SPI forever: it is the configuration most likely to work,
  // so a screen that only comes alive here tells you it is a hardware-SPI issue.
  pulseReset();
  tftSw.init(TFT_W, TFT_H, SPI_MODE0);
  tftSw.setRotation(TFT_ROTATION);
  cycle(tftSw, "SW SPI");
}
