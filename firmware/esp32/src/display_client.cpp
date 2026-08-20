// ---------------------------------------------------------------------------
// Gauge display client - ESP32 + ST7789 240x320.
//
//   boot -> Wi-Fi -> find the gauge server (unauthenticated GET /health on the
//   local /24) -> GET /v1/quota once a minute with the API token -> alternate
//   the screen: Claude (hourly window), Codex (weekly window), repeat.
//
// The remaining percentage picks a mood, drawn as a big icon over one word,
// with a stock-ticker delta and an availability bar underneath.
//
// The UDP ("BLE-style") transport served by `gauge --ble` is not implemented
// here; see README.md for the protocol if you want to add it.
//
// Servo pins are reserved in config.h but deliberately never driven.
// ---------------------------------------------------------------------------

#include <Adafruit_GFX.h>
#include <Adafruit_ST7789.h>
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
static IPAddress g_host;
static bool      g_haveHost = false;
static uint32_t  g_lastPoll = 0;
static uint32_t  g_tick = 0;
static uint8_t   g_failures = 0;
static bool      g_stale = false;
static String    g_lastErr;

static bool haveAnyData() {
  for (uint8_t i = 0; i < PROV_COUNT; i++)
    if (g_track[i].have) return true;
  return false;
}

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
  tft.fillScreen(C_BG);
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
  quotaResetPathCache();
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

static void doPoll() {
  QuotaReading r;
  String err;

  if (!quotaFetch(g_host, GAUGE_PORT, r, err)) {
    g_failures++;
    g_stale = true;
    g_lastErr = err;
    Serial.printf("[poll] failed (%u/%u): %s\n", g_failures, FAILURES_BEFORE_REDISCOVER,
                  err.c_str());

    // A 401 means we found the right box and the token is wrong, so re-scanning
    // would only find the same box again.
    if (err.startsWith("401")) return;

    if (g_failures >= FAILURES_BEFORE_REDISCOVER) {
      Serial.println("[poll] dropping host, will re-scan");
      g_haveHost = false;
      g_failures = 0;
      quotaResetPathCache();
      prefs.remove("host");
    }
    return;
  }

  g_failures = 0;
  g_stale = false;
  g_lastErr = "";

  for (uint8_t i = 0; i < PROV_COUNT; i++) {
    if (!r.valid[i]) continue;
    Tracked &t = g_track[i];
    if (!t.have) {
      t.pct = r.pct[i];
      t.have = true;
    } else if (fabsf(r.pct[i] - t.pct) > 0.05f) {
      t.prev = t.pct;
      t.pct = r.pct[i];
      t.haveDelta = true;
    }
    Serial.printf("[poll] %s %s %.1f%%\n", providerName((ProviderKind)i),
                  providerWindow((ProviderKind)i), t.pct);
  }
}

static void render() {
  // Nothing has ever parsed: show the reason instead of an empty dashboard.
  if (!haveAnyData() && !g_lastErr.isEmpty()) {
    uiStatus(tft, g_lastErr.startsWith("401") ? "TOKEN?" : "GAUGE ERROR", g_lastErr.c_str(), -1);
    return;
  }

  UiModel m;
  m.provider = (g_tick % 2 == 0) ? PROV_CLAUDE : PROV_CODEX;

  const Tracked &t = g_track[m.provider];
  m.havePct = t.have;
  m.pct = t.pct;
  m.haveDelta = t.haveDelta;
  m.delta = t.haveDelta ? (t.pct - t.prev) : 0.0f;
  m.stale = g_stale;

  uiRender(tft, m);
}

// ---------------------------------------------------------------------------

void setup() {
  Serial.begin(115200);
  delay(100);
  Serial.println("\n[boot] gauge display client");

  // Servos are intentionally left alone. GPIO12 in particular is a strapping
  // pin; driving or pulling it at boot can change the flash voltage.

  wdtBegin();
  displayBegin();
  uiStatus(tft, "GAUGE", "booting", -1);

  prefs.begin("gauge", false);

  while (!wifiConnect()) {
    esp_task_wdt_reset();
    delay(2000);
  }

  // Poll immediately rather than waiting out the first minute.
  g_lastPoll = millis() - POLL_INTERVAL_MS;
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

  if (millis() - g_lastPoll >= POLL_INTERVAL_MS) {
    g_lastPoll = millis();

    if (ensureHost()) {
      doPoll();
      render();
      g_tick++;
    }
  }

  delay(50);
}
