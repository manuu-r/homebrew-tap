// ---------------------------------------------------------------------------
// Gauge display client - ESP32 + ST7789 240x320.
//
//   boot -> Wi-Fi -> find the gauge server (unauthenticated GET /health on the
//   local /24) -> GET /v1/dashboard once -> show Codex, Claude, Calendar,
//   Markets, and To-do for 8 seconds each. Before each page the head looks
//   down, quickly shakes left/right, renders the next stat, and slowly rises
//   with it visible. A fresh snapshot is fetched when the cycle wraps.
//
// The remaining percentage picks a mood, drawn as a big icon over one word,
// with a stock-ticker delta and an availability bar underneath.
//
// The UDP ("BLE-style") transport served by `gauge --ble` is not implemented
// here; see README.md for the protocol if you want to add it.
//
// ---------------------------------------------------------------------------

#include <Adafruit_GFX.h>
#include <Adafruit_ST7789.h>
#include <ESP32Servo.h>
#include <Preferences.h>
#include <SPI.h>
#include <WiFi.h>
#include <esp_task_wdt.h>

#include "config.h"
#include "discovery.h"
#include "quota.h"
#include "theme.h"
#include "ui.h"

#if TFT_USE_HW_SPI
static SPIClass        tftSPI(HSPI);
static Adafruit_ST7789 tft(&tftSPI, PIN_TFT_CS, PIN_TFT_DC, PIN_TFT_RST);
#else
// Bit-banged on the same pins. See TFT_USE_HW_SPI in config.h for why.
static Adafruit_ST7789 tft(PIN_TFT_CS, PIN_TFT_DC, PIN_TFT_MOSI, PIN_TFT_SCLK, PIN_TFT_RST);
#endif

static Preferences prefs;
static Servo       middleServo;
static Servo       bottomServo;

// Per-provider history. `prev` is the value before the most recent *change*, so
// the ticker keeps showing the last real move instead of collapsing to 0.0% on
// every unchanged poll.
struct Tracked {
  bool  have = false;
  float pct = 0.0f;
  float prev = 0.0f;
  bool  haveDelta = false;
};

static Tracked   g_track[PROV_COUNT];
static DashboardData g_dashboard;
static IPAddress g_host;
static bool      g_haveHost = false;
static bool      g_haveDashboard = false;
static uint32_t  g_lastPageChange = 0;
static uint32_t  g_lastAttempt = 0;
static uint8_t   g_page = 0;
static uint8_t   g_failures = 0;
static bool      g_stale = false;
static String    g_lastErr;

enum DisplayPage : uint8_t {
  PAGE_CODEX = 0,
  PAGE_CLAUDE,
  PAGE_CALENDAR,
  PAGE_TICKERS,
  PAGE_TODOS,
  PAGE_COUNT,
};

// ---------------------------------------------------------------------------

static void wdtBegin() {
#if defined(ESP_ARDUINO_VERSION_MAJOR) && ESP_ARDUINO_VERSION_MAJOR >= 3
  esp_task_wdt_config_t cfg = {};
  cfg.timeout_ms = WDT_TIMEOUT_S * 1000;
  cfg.idle_core_mask = 0;
  cfg.trigger_panic = true;
  esp_task_wdt_reconfigure(&cfg);
#else
  esp_task_wdt_init(WDT_TIMEOUT_S, true);
#endif
  esp_task_wdt_add(NULL);
}

static void displayBegin() {
  if (PIN_TFT_BLK >= 0) {
    pinMode(PIN_TFT_BLK, OUTPUT);
    digitalWrite(PIN_TFT_BLK, HIGH);
  }
#if TFT_USE_HW_SPI
  tftSPI.begin(PIN_TFT_SCLK, PIN_TFT_MISO, PIN_TFT_MOSI, PIN_TFT_CS);
  tft.init(TFT_W, TFT_H, SPI_MODE0);
  tft.setSPISpeed(TFT_SPI_HZ);
#else
  tft.init(TFT_W, TFT_H, SPI_MODE0);
#endif
  tft.setRotation(TFT_ROTATION);
  tft.invertDisplay(false);
  tft.fillScreen(C_BG);
}

static void servosBegin() {
  middleServo.setPeriodHertz(50);
  bottomServo.setPeriodHertz(50);
  middleServo.attach(PIN_SERVO_MIDDLE, SERVO_MIN_US, SERVO_MAX_US);
  bottomServo.attach(PIN_SERVO_BOTTOM, SERVO_MIN_US, SERVO_MAX_US);
  middleServo.write(SERVO_MIDDLE_UP);
  bottomServo.write(SERVO_BOTTOM_CENTER);
}

static void holdHead(uint32_t durationMs) {
  const uint32_t start = millis();
  while (millis() - start < durationMs) {
    esp_task_wdt_reset();
    delay(20);
  }
}

// The middle servo pitches the head down. The bottom servo then performs a
// quick left/right shake; the ear servos on GPIO 22/23 are never attached.
static void headDownAndShake() {
  middleServo.write(SERVO_MIDDLE_DOWN);
  holdHead(SERVO_HEAD_DOWN_HOLD_MS);

  bottomServo.write(SERVO_BOTTOM_CENTER - SERVO_BOTTOM_SWING);
  holdHead(SERVO_SHAKE_HOLD_MS);
  bottomServo.write(SERVO_BOTTOM_CENTER + SERVO_BOTTOM_SWING);
  holdHead(SERVO_SHAKE_HOLD_MS);
  bottomServo.write(SERVO_BOTTOM_CENTER - SERVO_BOTTOM_SWING);
  holdHead(SERVO_SHAKE_HOLD_MS);
  bottomServo.write(SERVO_BOTTOM_CENTER + SERVO_BOTTOM_SWING);
  holdHead(SERVO_SHAKE_HOLD_MS);
  bottomServo.write(SERVO_BOTTOM_CENTER);
  holdHead(SERVO_SHAKE_HOLD_MS);
}

static void slowHeadUp() {
  const int step = SERVO_MIDDLE_UP < SERVO_MIDDLE_DOWN ? -1 : 1;
  for (int angle = SERVO_MIDDLE_DOWN; angle != SERVO_MIDDLE_UP; angle += step) {
    middleServo.write(angle);
    holdHead(SERVO_RISE_STEP_MS);
  }
  middleServo.write(SERVO_MIDDLE_UP);
  bottomServo.write(SERVO_BOTTOM_CENTER);
  holdHead(180);
}

static bool wifiConnect() {
  if (strlen(gauge_config::kWifiSsid) == 0) {
    uiStatus(tft, "NO SSID", "set kWifiSsid in gauge_config.h", -1);
    Serial.println("[wifi] kWifiSsid is empty - fill it in include/gauge_config.h");
    return false;
  }

  uiStatus(tft, "CONNECTING", gauge_config::kWifiSsid, 0);
  WiFi.mode(WIFI_STA);
  WiFi.setSleep(false);
  WiFi.begin(gauge_config::kWifiSsid, gauge_config::kWifiPassword);

  const uint32_t start = millis();
  while (WiFi.status() != WL_CONNECTED) {
    esp_task_wdt_reset();
    const uint32_t elapsed = millis() - start;
    if (elapsed > WIFI_CONNECT_TIMEOUT_MS) {
      uiStatus(tft, "WIFI FAILED", "retrying", -1);
      Serial.println("[wifi] connect timed out");
      return false;
    }
    uiStatusProgress(tft, gauge_config::kWifiSsid,
                     (int)(elapsed * 100 / WIFI_CONNECT_TIMEOUT_MS));
    delay(200);
  }

  Serial.printf("[wifi] %s, ip %s\n", gauge_config::kWifiSsid,
                WiFi.localIP().toString().c_str());
  return true;
}

static void onSweepProgress(const IPAddress &ip, int pct) {
  uiStatusProgress(tft, ip.toString().c_str(), pct);
}

static bool adoptHost(const IPAddress &ip, bool persist) {
  g_host = ip;
  g_haveHost = true;
  if (persist) prefs.putUInt("host", (uint32_t)ip);
  uiInvalidate();
  Serial.printf("[host] using %s\n", ip.toString().c_str());
  return true;
}

// Cached NVS host -> configured host -> full sweep.
static bool ensureHost() {
  if (g_haveHost) return true;

  if (!gauge_config::kAutoDiscover) {
    IPAddress pinned;
    if (!pinned.fromString(gauge_config::kGaugeHost)) {
      uiStatus(tft, "BAD HOST", gauge_config::kGaugeHost, -1);
      return false;
    }
    return adoptHost(pinned, false);
  }

  const uint32_t cached = prefs.getUInt("host", 0);
  if (cached != 0) {
    const IPAddress ip(cached);
    uiStatus(tft, "CHECKING HOST", ip.toString().c_str(), -1);
    if (probeHealth(ip)) return adoptHost(ip, false);
    Serial.println("[host] cached host is gone");
    prefs.remove("host");
  }

  IPAddress configured;
  if (configured.fromString(gauge_config::kGaugeHost)) {
    uiStatus(tft, "CHECKING HOST", gauge_config::kGaugeHost, -1);
    if (probeHealth(configured)) return adoptHost(configured, true);
  }

  uiStatus(tft, "SCANNING LAN", "", 0);
  IPAddress found;
  if (!discoverHealthHost(found, onSweepProgress)) {
    uiStatus(tft, "NO GAUGE", "nothing serving /health", -1);
    return false;
  }
  return adoptHost(found, true);
}

static bool doPoll() {
  DashboardData next;
  String err;

  if (!dashboardFetch(g_host, GAUGE_PORT, next, err)) {
    g_failures++;
    g_stale = true;
    g_lastErr = err;
    Serial.printf("[poll] failed (%u/%u): %s\n", g_failures, FAILURES_BEFORE_REDISCOVER,
                  err.c_str());

    // A 401 means we found the right box and the token is wrong, so re-scanning
    // would only find the same box again.
    if (err.startsWith("401")) return false;

    if (g_failures >= FAILURES_BEFORE_REDISCOVER) {
      Serial.println("[poll] dropping host, will re-scan");
      g_haveHost = false;
      g_failures = 0;
      prefs.remove("host");
    }
    return false;
  }

  g_failures = 0;
  g_stale = false;
  g_lastErr = "";
  g_dashboard = next;
  g_haveDashboard = true;

  for (uint8_t i = 0; i < PROV_COUNT; i++) {
    if (!next.quota.valid[i]) continue;
    Tracked &t = g_track[i];
    if (!t.have) {
      t.pct = next.quota.pct[i];
      t.have = true;
    } else if (fabsf(next.quota.pct[i] - t.pct) > 0.05f) {
      t.prev = t.pct;
      t.pct = next.quota.pct[i];
      t.haveDelta = true;
    }
    Serial.printf("[poll] %s %s %.1f%%\n", providerName((ProviderKind)i),
                  providerWindow((ProviderKind)i), t.pct);
  }
  Serial.printf("[poll] dashboard: %u events, %u tickers, %u todos\n", next.eventTotal,
                next.tickerTotal, next.todoTotal);
  return true;
}

static void renderProvider(ProviderKind provider) {
  UiModel m;
  m.provider = provider;

  const Tracked &t = g_track[provider];
  m.havePct = t.have;
  m.pct = t.pct;
  m.haveDelta = t.haveDelta;
  m.delta = t.haveDelta ? (t.pct - t.prev) : 0.0f;
  m.stale = g_stale;
  uiRender(tft, m);
}

static void renderPage() {
  if (!g_haveDashboard && !g_lastErr.isEmpty()) {
    uiStatus(tft, g_lastErr.startsWith("401") ? "TOKEN?" : "GAUGE ERROR", g_lastErr.c_str(), -1);
    return;
  }
  switch ((DisplayPage)g_page) {
    case PAGE_CODEX: renderProvider(PROV_CODEX); break;
    case PAGE_CLAUDE: renderProvider(PROV_CLAUDE); break;
    case PAGE_CALENDAR: uiRenderCalendar(tft, g_dashboard); break;
    case PAGE_TICKERS: uiRenderTickers(tft, g_dashboard); break;
    case PAGE_TODOS: uiRenderTodos(tft, g_dashboard); break;
    case PAGE_COUNT: break;
  }
}

// Keep the current stat visible until the physical gesture finishes. Only
// after the head is down and has shaken do we blank/refresh the screen. The
// next page is fully rendered while the head remains down, so it is already
// visible throughout the slow rise. On a cycle wrap, the whole dashboard is
// fetched before that render. Intermediate pages reuse the same snapshot.
static void revealPage(bool refreshDashboard) {
  headDownAndShake();
  tft.fillScreen(C_BG);
  uiInvalidate();
  if (refreshDashboard && ensureHost()) doPoll();
  uiInvalidate();
  renderPage();
  slowHeadUp();
  g_lastPageChange = millis();
}

// ---------------------------------------------------------------------------

void setup() {
  Serial.begin(115200);
  delay(100);
  Serial.println("\n[boot] gauge display client");

  wdtBegin();
  displayBegin();
  servosBegin();
  uiStatus(tft, "GAUGE", "booting", -1);

  prefs.begin("gauge", false);

  while (!wifiConnect()) {
    esp_task_wdt_reset();
    delay(2000);
  }

  // Fetch immediately, then keep every fetched snapshot on screen for one
  // complete five-page rotation.
  g_lastAttempt = millis() - 5000;
}

void loop() {
  esp_task_wdt_reset();

  if (WiFi.status() != WL_CONNECTED) {
    Serial.println("[wifi] link lost");
    g_stale = true;
    uiStatus(tft, "WIFI LOST", "reconnecting", -1);
    WiFi.disconnect();
    if (!wifiConnect()) {
      delay(2000);
      return;
    }
    uiInvalidate();
  }

  if (!g_haveDashboard) {
    if (millis() - g_lastAttempt >= 5000) {
      g_lastAttempt = millis();
      if (ensureHost() && doPoll()) {
        g_page = PAGE_CODEX;
        revealPage(false);
      } else {
        renderPage();
      }
    }
    delay(50);
    return;
  }

  if (millis() - g_lastPageChange >= PAGE_DURATION_MS) {
    if (g_page + 1 < PAGE_COUNT) {
      g_page++;
      revealPage(false);
    } else {
      g_page = PAGE_CODEX;
      revealPage(true);
    }
  }

  delay(50);
}
