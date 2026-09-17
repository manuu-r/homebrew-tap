// ---------------------------------------------------------------------------
// Gauge accessory - ESP32 + ST7789 240x320 + two-servo head.
//
//   unpaired: advertise the Gauge pairing service, show the BLE number, accept
//             it with the BOOT button, receive Wi-Fi + credentials, reboot.
//   paired:   join Wi-Fi, find Gauge over Bonjour, pull /v1/dashboard with the
//             bearer token, cache it in NVS, and rotate one page per quota
//             group, then Calendar and To-do. Hold BOOT for 5 s to unpair.
// ---------------------------------------------------------------------------

#include <Adafruit_ST7789.h>
#include <ArduinoJson.h>
#include <ESP32Servo.h>
#include <Preferences.h>
#include <WiFi.h>

#include "config.h"
#include "dashboard_parse.h"
#include "gauge_client.h"
#include "pairing.h"
#include "ui.h"

// Bit-banged SPI; see PIN_TFT_SCLK in config.h.
static Adafruit_ST7789 tft(PIN_TFT_CS, PIN_TFT_DC, PIN_TFT_MOSI, PIN_TFT_SCLK, PIN_TFT_RST);
static Preferences prefs;
static Servo middle, bottom;

static bool paired = false;
static String deviceName;
static DashboardData dash;
static bool haveDash = false;
static bool stale = true;
static String cachedBody;
static uint32_t fetchedAtMs = 0;  // when dash.generatedAt was current
static uint32_t nextFetchMs = 0;
static uint32_t pageShownMs = 0;
static uint8_t page = 0;

// -------------------------------------------------------------- identity ---

static String macSuffix(int bytes) {
  const uint64_t mac = ESP.getEfuseMac();
  String out;
  for (int i = 6 - bytes; i < 6; i++) {
    char hex[3];
    snprintf(hex, sizeof(hex), "%02x", (uint8_t)(mac >> (8 * i)));
    out += hex;
  }
  return out;
}

// ------------------------------------------------------------------ head ---

static void headDownAndShake() {
  middle.write(SERVO_MIDDLE_DOWN);
  delay(SERVO_HEAD_DOWN_HOLD_MS);
  for (int i = 0; i < 4; i++) {
    bottom.write(SERVO_BOTTOM_CENTER + (i % 2 ? SERVO_BOTTOM_SWING : -SERVO_BOTTOM_SWING));
    delay(SERVO_SHAKE_HOLD_MS);
  }
  bottom.write(SERVO_BOTTOM_CENTER);
}

static void slowHeadUp() {
  const int step = SERVO_MIDDLE_UP < SERVO_MIDDLE_DOWN ? -1 : 1;
  for (int angle = SERVO_MIDDLE_DOWN; angle != SERVO_MIDDLE_UP; angle += step) {
    middle.write(angle);
    delay(SERVO_RISE_STEP_MS);
  }
}

// --------------------------------------------------------------- pairing ---

// Runs on the Bluetooth task while macOS shows the same number.
static bool confirmNumber(uint32_t number) {
  uiCompare(tft, number);
  const uint32_t start = millis();
  while (millis() - start < PAIR_CONFIRM_TIMEOUT_MS) {
    if (digitalRead(PIN_BUTTON) == LOW) {
      uiStatus(tft, "PAIRING", "Waiting for Gauge", -1);
      return true;
    }
    delay(20);
  }
  uiPairing(tft, deviceName.c_str());
  return false;
}

static void forgetAndRestart(const char *reason) {
  uiStatus(tft, "UNPAIRED", reason, -1);
  prefs.clear();  // Wi-Fi, server ID, token, and cached dashboard
  delay(2000);
  ESP.restart();
}

// ------------------------------------------------------------- dashboard ---

// Replaces the shown snapshot only with a complete, valid schema 1 document.
static bool adopt(const String &body) {
  JsonDocument doc;
  DashboardData next;
  if (deserializeJson(doc, body) || !dashboardparse::parse(doc.as<JsonVariantConst>(), next)) {
    return false;
  }
  dash = next;
  haveDash = true;
  fetchedAtMs = millis();
  return true;
}

static void refresh() {
  String body;
  const gauge::Fetch result = gauge::dashboard(body);
  if (result == gauge::Fetch::Revoked) forgetAndRestart("Gauge forgot this device");

  uint32_t waitS = RETRY_MS / 1000;
  stale = !(result == gauge::Fetch::Ok && adopt(body));
  if (!stale) {
    if (body != cachedBody) prefs.putString("dash", body);  // dashboard.cache
    cachedBody = body;
    waitS = constrain(dash.refreshSeconds, MIN_REFRESH_S, MAX_REFRESH_S);
  }
  nextFetchMs = millis() + waitS * 1000;
}

static uint8_t pageCount() {
  return (dash.providerCount ? dash.providerCount : 1) + 2;  // quota, calendar, to-do
}

static void renderPage() {
  const int64_t now = dash.generatedAt + (millis() - fetchedAtMs) / 1000;
  const uint8_t pages = pageCount();
  const uint8_t quotaPages = pages - 2;
  if (page < quotaPages) {
    if (dash.providerCount) uiProvider(tft, dash.providers[page], now, stale, page, pages);
    else uiQuotaError(tft, dash, stale, page, pages);
  } else if (page == quotaPages) {
    uiCalendar(tft, dash, now, stale, page, pages);
  } else {
    uiTodos(tft, dash, stale, page, pages);
  }
}

// ------------------------------------------------------------------------

void setup() {
  Serial.begin(115200);
  pinMode(PIN_BUTTON, INPUT_PULLUP);
  if (PIN_TFT_BLK >= 0) {
    pinMode(PIN_TFT_BLK, OUTPUT);
    digitalWrite(PIN_TFT_BLK, HIGH);
  }
  tft.init(TFT_W, TFT_H, SPI_MODE0);
  tft.setRotation(TFT_ROTATION);
  middle.attach(PIN_SERVO_MIDDLE, SERVO_MIN_US, SERVO_MAX_US);
  bottom.attach(PIN_SERVO_BOTTOM, SERVO_MIN_US, SERVO_MAX_US);
  middle.write(SERVO_MIDDLE_UP);
  bottom.write(SERVO_BOTTOM_CENTER);

  prefs.begin("gauge", false);
  const String hostname = "gauge-display-" + macSuffix(3);
  WiFi.setHostname(hostname.c_str());
  paired = prefs.getBool("paired", false);

  if (!paired) {
    deviceName = "Gauge Display " + macSuffix(2);
    uiPairing(tft, deviceName.c_str());
    pairing::start(prefs, "gauge-esp32-" + macSuffix(6), deviceName, confirmNumber);
    return;
  }

  gauge::begin(prefs);
  cachedBody = prefs.getString("dash", "");
  if (cachedBody.length() && adopt(cachedBody)) renderPage();
  else uiStatus(tft, "CONNECTING", prefs.getString("ssid").c_str(), -1);
  WiFi.mode(WIFI_STA);
  WiFi.setAutoReconnect(true);
  WiFi.begin(prefs.getString("ssid").c_str(), prefs.getString("pass").c_str());
}

void loop() {
  if (!paired) {
    if (pairing::service()) {
      uiStatus(tft, "PAIRED", "Starting", -1);
      ESP.restart();  // releases Bluetooth before the Wi-Fi runtime starts
    }
    delay(20);
    return;
  }

  // Hold BOOT to unpair: tell Gauge first, then erase everything local.
  if (digitalRead(PIN_BUTTON) == LOW) {
    const uint32_t start = millis();
    while (digitalRead(PIN_BUTTON) == LOW && millis() - start < UNPAIR_HOLD_MS) delay(20);
    if (millis() - start >= UNPAIR_HOLD_MS) {
      uiStatus(tft, "UNPAIRING", "Telling Gauge", -1);
      gauge::revoke();
      forgetAndRestart("Ready to pair again");
    }
  }

  if (!haveDash) {
    if (WiFi.status() == WL_CONNECTED && millis() >= nextFetchMs) {
      uiStatus(tft, "FINDING GAUGE", "Open Gauge on your Mac", -1);
      refresh();
      if (haveDash) {
        page = 0;
        renderPage();
        pageShownMs = millis();
      } else {
        uiStatus(tft, "NO GAUGE", "Is Gauge open on this Wi-Fi?", -1);
      }
    }
    delay(50);
    return;
  }

  if (millis() - pageShownMs < PAGE_DURATION_MS) {
    delay(50);
    return;
  }
  headDownAndShake();
  page = (page + 1) % pageCount();
  if (page == 0 && millis() >= nextFetchMs) refresh();
  if (page >= pageCount()) page = 0;
  renderPage();
  slowHeadUp();
  pageShownMs = millis();
}
