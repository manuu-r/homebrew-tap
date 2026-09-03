#include <Adafruit_GFX.h>
#include <ArduinoJson.h>
#include <BLEAdvertising.h>
#include <BLECharacteristic.h>
#include <BLEDevice.h>
#include <BLEServer.h>
#include <BLESecurity.h>
#include <ESPmDNS.h>
#include <HTTPClient.h>
#include <Preferences.h>
#include <WiFi.h>
#include <cctype>
#include <cstring>
#include <driver/i2s.h>
#include <esp_err.h>
#include <esp_gap_ble_api.h>
#include <esp_system.h>
#include "flow32/graphics/Display.h"
#include "flow32/graphics/St77xxTransport.h"
#include <freertos/semphr.h>

#include "BuntyAnimations.h"
#include "BuntyFonts.h"
#include "BuntyVoice.h"
#include "IotGatewayCredentials.h"
#include "ServoMotion.h"
#include "TapInput.h"

// Pre-decoded greeting supplied by audio/bunty_audio.S.
extern "C" const int16_t buntyAudioPcm[];
extern "C" const int16_t buntyAudioPcmEnd[];

namespace {

#define TFT_CS 10
#define TFT_DC 9
#define TFT_RST 14
#define TFT_BL 21
#define TFT_MOSI 11
#define TFT_SCLK 12

#define TURN_SERVO_PIN 7

#define AMP_SD_PIN 13
#define AMP_BCLK 4
#define AMP_LRC 5
#define AMP_DIN 6

#define MIC_BCLK 15
#define MIC_WS 16
#define MIC_DATA 17

constexpr uint16_t kScreenWidth = 240;
constexpr uint16_t kScreenHeight = 320;
constexpr uint32_t kTftFrequency = 40000000;
constexpr uint8_t kBacklightChannel = 0;
// Keep servo support available for later calibration without driving either
// motor in the current firmware.
constexpr bool kServoMotorsEnabled = false;
constexpr uint8_t kTurnServoChannel = 2;
constexpr uint8_t kTurnServoHomeAngle = 100;

constexpr uint16_t rgb565(uint8_t red, uint8_t green, uint8_t blue) {
  return static_cast<uint16_t>(((red & 0xF8) << 8) |
                               ((green & 0xFC) << 3) | (blue >> 3));
}

constexpr uint16_t kBackground = rgb565(6, 8, 15);
constexpr uint16_t kSurface = rgb565(17, 21, 33);
constexpr uint16_t kBorder = rgb565(38, 44, 62);
constexpr uint16_t kText = rgb565(246, 248, 252);
constexpr uint16_t kMuted = rgb565(142, 151, 172);
constexpr uint16_t kAccent = rgb565(111, 97, 255);
constexpr uint16_t kSuccess = rgb565(88, 226, 173);
constexpr uint16_t kDanger = rgb565(255, 102, 118);

constexpr char kProtocol[] = "dev.gauge.accessory";
constexpr char kPairingProtocol[] = "dev.gauge.pairing";
constexpr char kCommissionProtocol[] = "dev.gauge.commission";
constexpr char kDashboardProtocol[] = "dev.gauge.dashboard";
constexpr uint16_t kPairingProtocolVersion = 1;
constexpr char kFirmwareVersion[] = "0.1.0";
constexpr char kPairingServiceUuid[] =
    "c9cce6f3-bf10-4e6d-b719-f32911bbba89";
constexpr char kIdentityCharacteristicUuid[] =
    "1db9634c-a20f-43c6-8ed9-69ceda338178";
constexpr char kConfigCharacteristicUuid[] =
    "289be295-d110-411b-888c-c80a601177fa";
constexpr char kStatusCharacteristicUuid[] =
    "e763eccb-fa4c-4e3a-9211-850513371105";
constexpr uint8_t kFrameMagic = 0x47;
constexpr size_t kFrameHeaderSize = 3;
constexpr size_t kMaximumConfigBytes = 4335;

constexpr i2s_port_t kAudioPort = I2S_NUM_0;
constexpr i2s_port_t kMicrophonePort = I2S_NUM_1;
constexpr uint32_t kAudioSampleRate = 22050;
constexpr int32_t kGreetingVolumePercent = 30;
constexpr uint32_t kWifiJoinTimeoutMs = 30000;
constexpr uint32_t kPairingShutdownDelayMs = 2500;
constexpr uint32_t kDashboardPageTimeMs = 11000;
constexpr uint32_t kDefaultDashboardRefreshMs = 120000;
constexpr uint8_t kDashboardReconnectAfterFailures = 3;
constexpr uint32_t kDashboardIdleSleepMs = 120000;
constexpr uint32_t kVoiceStartRetryMs = 5000;
// A faint ~3% PWM glow: enough to see the sleeping eyes and know Bunty is on
// without lighting the room or desk.
constexpr uint8_t kSleepBacklightDuty = 8;
constexpr uint32_t kWakeTapMinimumLevel = 60000;
constexpr uint32_t kWakeTapNoiseMultiplier = 10;
constexpr uint32_t kUnpairTapMinimumLevel = 180000;
constexpr uint32_t kUnpairTapNoiseMultiplier = 16;
constexpr uint32_t kUnpairMinimumGapMs = 240;
constexpr uint32_t kUnpairMaximumGapMs = 700;
constexpr uint32_t kUnpairRhythmToleranceMs = 220;
constexpr uint32_t kUnpairConfirmationGuardMs = 1000;
constexpr uint32_t kUnpairConfirmationTimeoutMs = 6000;
constexpr size_t kMaximumDashboardBytes = 12288;

DisplayPanel makePanel() {
  DisplayPanel panel;
  panel.id = "bunty-st7789";
  panel.width = kScreenWidth;
  panel.height = kScreenHeight;
  return panel;
}

St77xxConfig makeDisplayTransport() {
  St77xxConfig config;
  config.chip = PanelChip::ST7789;
  config.spi = &SPI;
  config.pinCs = TFT_CS;
  config.pinDc = TFT_DC;
  config.pinRst = TFT_RST;
  config.pinMosi = TFT_MOSI;
  config.pinSclk = TFT_SCLK;
  config.gramWidth = kScreenWidth;
  config.gramHeight = kScreenHeight;
  config.rotation = 0;
  config.spiHz = kTftFrequency;
  // Backlight remains application-owned because Bunty fades it with LEDC.
  config.pinBacklight = -1;
  return config;
}

const DisplayPanel kPanel = makePanel();
St77xxTransport displayTransport(makeDisplayTransport());

// Flow32's Display is an Adafruit_GFX that paints into an off-screen
// framebuffer, so a screen arrives in one present() instead of tearing in
// shape by shape.
Display display(kPanel, displayTransport);
BuntyAnimations buntyAnimations(display);
SemaphoreHandle_t displayMutex = nullptr;
Preferences preferences;
TapInput tapInput;
BuntyVoice buntyVoice;
ServoMotion turnServoMotion;
String deviceName;
String hardwareId;
BLEServer *pairingServer = nullptr;
BLECharacteristic *pairingStatus = nullptr;
volatile bool configurationReady = false;
volatile bool pairingAuthenticated = false;
// A BLE bond can finish before the application-level commission document is
// accepted. If that connection then disappears, keeping the bond would make
// macOS reconnect silently on the next attempt and no comparison code would
// be shown. The loop handles the recovery outside Bluedroid's callback task.
volatile bool freshPairingRestartRequested = false;
// -1 while the controller is applying the static random address, 0 on
// failure, and 1 when it is safe to start advertising with that address.
volatile int8_t pairingAddressState = -1;
bool pairingStarted = false;
bool pairingScreenVisible = false;
bool wifiJoinStarted = false;
uint32_t wifiJoinStartedAt = 0;
uint32_t pairingCompletedAt = 0;
String pairingError;
String configBuffer;
uint8_t expectedFrames = 0;
uint8_t nextFrame = 0;
uint32_t lastActivityUpdate = 0;
uint8_t activityFrame = 0;
bool showingGreetingEyes = false;
volatile bool greetingPlaying = false;

bool runtimeStarted = false;
bool voiceStarted = false;
bool voiceFaceActive = false;
uint32_t lastMouthFrameAt = 0;
uint32_t lastVoiceHeartbeatAt = 0;
uint32_t nextVoiceStartAt = 0;
bool mdnsStarted = false;
bool dashboardAvailable = false;
bool dashboardOnline = false;
wl_status_t lastWifiStatus = WL_NO_SHIELD;
String dashboardJson;
uint32_t dashboardRefreshMs = kDefaultDashboardRefreshMs;
uint32_t nextDashboardFetchAt = 0;
uint32_t nextDashboardPageAt = 0;
uint8_t dashboardPage = 0;
uint16_t dashboardRound = 0;
uint8_t dashboardConnectFailures = 0;
uint8_t backlightDuty = 0;

enum class RuntimeDisplayState {
  Dashboard,
  Sleeping,
  Waking,
};

RuntimeDisplayState runtimeDisplayState = RuntimeDisplayState::Dashboard;
uint32_t lastRuntimeInteractionAt = 0;

enum class ResetGestureState {
  Idle,
  AwaitingConfirmation,
};

ResetGestureState resetGestureState = ResetGestureState::Idle;
uint8_t resetTapCount = 0;
uint32_t resetLastTapAt = 0;
uint32_t resetFirstGapMs = 0;
uint32_t resetPromptedAt = 0;

// BLE security callbacks run on a Bluedroid task while dashboard and animation
// rendering runs on Arduino's loop task. Flow32's framebuffer and SPI transport
// are deliberately serialized across those tasks. A recursive mutex lets a
// complete screen call helpers that also present a small protected region.
class DisplayGuard {
 public:
  DisplayGuard() {
    if (displayMutex) {
      locked_ = xSemaphoreTakeRecursive(displayMutex, portMAX_DELAY) == pdTRUE;
    }
  }

  ~DisplayGuard() {
    if (locked_) xSemaphoreGiveRecursive(displayMutex);
  }

 private:
  bool locked_ = false;
};

void drawRuntimeTopBar(bool present);

uint32_t lastStatusUpdate = 0;

// Mirrors device_identifier() in src/provisioning.rs. Gauge reads this stable
// hardware-derived value only after BLE Secure Connections authentication and
// sends it back inside the protected commissioning document.
bool isIdentifier(const char *value) {
  const size_t length = strlen(value);
  if (length == 0 || length > 64) return false;
  for (size_t i = 0; i < length; ++i) {
    const char ch = value[i];
    if (!std::isalnum(static_cast<unsigned char>(ch)) && ch != '-' &&
        ch != '_' && ch != '.') {
      return false;
    }
  }
  return true;
}

bool isHexToken(const char *token) {
  if (strlen(token) != 64) return false;
  for (size_t i = 0; i < 64; ++i) {
    if (!std::isxdigit(static_cast<unsigned char>(token[i]))) return false;
  }
  return true;
}

bool isIotGatewayToken(const char *token) {
  if (strlen(token) != 47 || strncmp(token, "iot_", 4) != 0) return false;
  for (size_t i = 4; i < 47; ++i) {
    const char ch = token[i];
    if (!std::isalnum(static_cast<unsigned char>(ch)) && ch != '-' &&
        ch != '_') {
      return false;
    }
  }
  return true;
}

void syncIotGatewayCredential() {
  constexpr char compiledToken[] = IOT_GATEWAY_TOKEN;
  constexpr char compiledDevice[] = IOT_GATEWAY_DEVICE_ID;
  if (!isIotGatewayToken(compiledToken) ||
      strcmp(compiledDevice, "bunty") != 0) {
    Serial.println("[iot-gateway] no valid credential embedded in this build");
    return;
  }

  if (preferences.getString("iot_token", "") == compiledToken &&
      preferences.getString("iot_device", "") == compiledDevice) {
    Serial.println("[iot-gateway] credential already stored");
    return;
  }

  const bool saved = preferences.putString("iot_token", compiledToken) > 0 &&
                     preferences.putString("iot_device", compiledDevice) > 0;
  Serial.println(saved ? "[iot-gateway] stored flashed credential"
                       : "[iot-gateway] could not store flashed credential");
}

void loadIdentity() {
  const uint64_t chipId = ESP.getEfuseMac();
  char name[24];
  snprintf(name, sizeof(name), "Bunty-%06llX",
           static_cast<unsigned long long>(chipId & 0xFFFFFFULL));
  deviceName = name;
  char identifier[24];
  snprintf(identifier, sizeof(identifier), "bunty-%012llx",
           static_cast<unsigned long long>(chipId & 0xFFFFFFFFFFFFULL));
  hardwareId = identifier;
}

void drawCentered(const GFXfont *font, const char *text, int16_t top,
                  uint16_t color) {
  display.setFont(font);
  display.setTextSize(1);
  display.setTextColor(color);
  display.setTextWrap(false);

  int16_t x1 = 0;
  int16_t y1 = 0;
  uint16_t width = 0;
  uint16_t height = 0;
  display.getTextBounds(text, 0, 0, &x1, &y1, &width, &height);
  display.setCursor((kScreenWidth - static_cast<int16_t>(width)) / 2 - x1,
                    top - y1);
  display.print(text);
}

void drawPairingActivity() {
  DisplayGuard guard;
  display.fillRect(94, 254, 52, 20, kBackground);
  for (uint8_t i = 0; i < 3; ++i) {
    const uint16_t color = i == activityFrame ? kAccent : kBorder;
    display.fillCircle(106 + i * 14, 264, i == activityFrame ? 4 : 3, color);
  }
  display.present(94, 254, 52, 20);
}

void showPairingScreen() {
  DisplayGuard guard;
  display.fillScreen(kBackground);
  // Keep every line inside a 216 px safe area. The previous 18 pt
  // "PAIR IN GAUGE" heading was 268 px wide on this 240 px panel.
  drawCentered(BuntyFonts::kHeading, "PAIR BUNTY", 72, kText);
  drawCentered(BuntyFonts::kBody, "Open Gauge on your Mac", 133, kMuted);
  drawCentered(BuntyFonts::kBody, "Choose Pair Accessory", 165, kMuted);
  drawCentered(BuntyFonts::kBody, "Then compare the code", 197, kMuted);
  drawPairingActivity();
  display.present();
  pairingScreenVisible = true;
}

void showNumericComparison(uint32_t pin) {
  pairingScreenVisible = false;
  DisplayGuard guard;
  char digits[7];
  snprintf(digits, sizeof(digits), "%06lu", static_cast<unsigned long>(pin));
  display.fillScreen(kBackground);
  drawCentered(BuntyFonts::kBody, "DOES THE MAC SHOW", 66, kMuted);
  drawCentered(BuntyFonts::kComparison, digits, 112, kText);
  drawCentered(BuntyFonts::kBody, "1 TAP  YES", 197, kSuccess);
  drawCentered(BuntyFonts::kBody, "2 TAPS  NO", 229, kDanger);
  display.present();
}

void showResetConfirmation() {
  DisplayGuard guard;
  display.fillScreen(kBackground);
  drawRuntimeTopBar(false);
  drawCentered(BuntyFonts::kHeading, "UNPAIR?", 76, kText);
  drawCentered(BuntyFonts::kBody, "Bunty will forget this Mac", 139, kMuted);
  drawCentered(BuntyFonts::kBody, "Pause 1 sec, then tap", 171, kSuccess);
  drawCentered(BuntyFonts::kBody, "Wait 6 seconds to cancel", 203, kMuted);
  display.present();
}

// Bluetooth's rune: a spine, a flag at each end, and the two strokes that
// cross it.
void drawBluetoothIcon(int16_t cx, int16_t cy, uint16_t color) {
  constexpr int16_t w = 3;
  constexpr int16_t h = 5;
  display.drawLine(cx, cy - h, cx, cy + h, color);
  display.drawLine(cx, cy - h, cx + w, cy - h / 2, color);
  display.drawLine(cx + w, cy - h / 2, cx - w, cy + h / 2, color);
  display.drawLine(cx, cy + h, cx + w, cy + h / 2, color);
  display.drawLine(cx + w, cy + h / 2, cx - w, cy - h / 2, color);
}

// Three arcs over a dot. Adafruit's quarter-circle helper draws the top half
// with the top-left and top-right corner bits set.
void drawWifiIcon(int16_t cx, int16_t baseY, uint16_t color) {
  for (int16_t r = 2; r <= 6; r += 2) {
    display.drawCircleHelper(cx, baseY, r, 0x1 | 0x2, color);
  }
  display.fillCircle(cx, baseY, 1, color);
}

void showSpeakingEyes() {
  DisplayGuard guard;
  display.fillScreen(kBackground);
  drawRuntimeTopBar(false);
  buntyAnimations.beginSpeaking(millis(), false);
  display.present();
  lastStatusUpdate = millis();
  showingGreetingEyes = true;
  Serial.println("[animation] speaking eyes");
}

// Streams the embedded PCM straight at the MAX98357. The amp is left shut down
// outside this call so it does not hiss while idle.
void playGreeting() {
  const size_t samples = static_cast<size_t>(buntyAudioPcmEnd - buntyAudioPcm);
  if (samples == 0) return;

  i2s_config_t config = {};
  config.mode = static_cast<i2s_mode_t>(I2S_MODE_MASTER | I2S_MODE_TX);
  config.sample_rate = kAudioSampleRate;
  config.bits_per_sample = I2S_BITS_PER_SAMPLE_16BIT;
  config.channel_format = I2S_CHANNEL_FMT_RIGHT_LEFT;
  config.communication_format = I2S_COMM_FORMAT_STAND_I2S;
  config.intr_alloc_flags = ESP_INTR_FLAG_LEVEL1;
  // Four short buffers are ample at 22.05 kHz and leave substantially more
  // internal DMA-capable heap available than the previous 8 x 256 layout.
  config.dma_buf_count = 4;
  config.dma_buf_len = 128;
  config.tx_desc_auto_clear = true;
  if (i2s_driver_install(kAudioPort, &config, 0, nullptr) != ESP_OK) {
    Serial.println("[audio] could not install the I2S driver");
    return;
  }

  i2s_pin_config_t pins = {};
  pins.mck_io_num = I2S_PIN_NO_CHANGE;
  pins.bck_io_num = AMP_BCLK;
  pins.ws_io_num = AMP_LRC;
  pins.data_out_num = AMP_DIN;
  pins.data_in_num = I2S_PIN_NO_CHANGE;
  if (i2s_set_pin(kAudioPort, &pins) != ESP_OK) {
    Serial.println("[audio] could not route I2S to the amplifier");
    i2s_driver_uninstall(kAudioPort);
    return;
  }

  digitalWrite(AMP_SD_PIN, HIGH);
  delay(10);

  // SD tied high makes the MAX98357 average both slots, so each mono sample is
  // written to left and right to keep full output level.
  int16_t frames[256];
  const size_t frameCapacity = (sizeof(frames) / sizeof(frames[0])) / 2;
  size_t played = 0;
  while (played < samples) {
    size_t chunk = frameCapacity;
    if (chunk > samples - played) chunk = samples - played;
    for (size_t i = 0; i < chunk; ++i) {
      const int16_t sample = static_cast<int16_t>(
          static_cast<int32_t>(buntyAudioPcm[played + i]) *
          kGreetingVolumePercent / 100);
      frames[i * 2] = sample;
      frames[i * 2 + 1] = sample;
    }
    size_t written = 0;
    if (i2s_write(kAudioPort, frames, chunk * 2 * sizeof(int16_t), &written,
                  portMAX_DELAY) != ESP_OK) {
      break;
    }
    played += chunk;
  }

  i2s_zero_dma_buffer(kAudioPort);
  digitalWrite(AMP_SD_PIN, LOW);
  i2s_driver_uninstall(kAudioPort);
  Serial.printf("[audio] played %u samples\n", static_cast<unsigned>(played));
}

void greetingTask(void *parameters) {
  (void)parameters;
  playGreeting();
  greetingPlaying = false;
  tapInput.reset();
  tapInput.suppressFor(1000);
  vTaskDelete(nullptr);
}

// Speaking is pushed onto its own task so loop() stays free to animate.
void startGreeting() {
  greetingPlaying = true;
  tapInput.reset();
  tapInput.suppressFor(1000);
  xTaskCreatePinnedToCore(greetingTask, "greeting", 4096, nullptr, 1, nullptr,
                          0);
}

void showError(const char *message) {
  DisplayGuard guard;
  display.fillScreen(kBackground);
  drawCentered(BuntyFonts::kHeading, "Try again", 132, kDanger);
  drawCentered(BuntyFonts::kBody, message, 176, kMuted);
  display.present();
  Serial.printf("[pairing] %s\n", message);
}

void fadeBacklightTo(uint8_t targetDuty, uint8_t steps = 28) {
  const int16_t startDuty = backlightDuty;
  if (startDuty == targetDuty) return;
  for (uint8_t step = 1; step <= steps; ++step) {
    const int16_t duty = startDuty +
                         (static_cast<int16_t>(targetDuty) - startDuty) * step /
                             steps;
    ledcWrite(kBacklightChannel, static_cast<uint8_t>(duty));
    delay(targetDuty > startDuty ? 8 : 5);
  }
  backlightDuty = targetDuty;
}

void fadeBacklight(bool on) {
  fadeBacklightTo(on ? 255 : 0);
}

void setPairingStatus(const char *status) {
  if (pairingStatus) pairingStatus->setValue(status);
  Serial.printf("[pairing] %s\n", status);
}

bool validWifiPassword(const char *password) {
  const size_t length = strlen(password);
  if (length < 64) return true;
  if (length != 64) return false;
  for (size_t i = 0; i < length; ++i) {
    if (!std::isxdigit(static_cast<unsigned char>(password[i]))) return false;
  }
  return true;
}

bool saveCommission(JsonDocument &document) {
  const char *protocol = document["protocol"] | "";
  const uint16_t commissionVersion = document["version"] | 0;
  JsonObjectConst wifi = document["wifi"].as<JsonObjectConst>();
  JsonObjectConst accessory = document["accessory"].as<JsonObjectConst>();
  const char *ssid = wifi["ssid"] | "";
  const char *password = wifi["password"] | "";
  const uint16_t version = accessory["version"] | 0;
  const char *configuredDevice = accessory["device_id"] | "";
  const char *serverId = accessory["server_id"] | "";
  const char *token = accessory["bearer_token"] | "";
  const char *service = accessory["service_type"] | "";
  const char *path = accessory["dashboard_path"] | "";
  const char *accessoryProtocol = accessory["protocol"] | "";
  const uint16_t port = accessory["server_port"] | 0;
  const size_t passwordLength = strlen(password);

  if (strcmp(protocol, kCommissionProtocol) != 0 || commissionVersion != 1 ||
      strlen(ssid) == 0 || strlen(ssid) > 32 ||
      !validWifiPassword(password) ||
      strcmp(accessoryProtocol, kProtocol) != 0 || version != 1 ||
      hardwareId != configuredDevice ||
      !isIdentifier(configuredDevice) || strlen(serverId) == 0 ||
      strlen(serverId) > 64 || !isHexToken(token) ||
      strcmp(service, "_gauge._tcp.local.") != 0 ||
      strcmp(path, "/v1/dashboard") != 0 || port == 0) {
    return false;
  }

  return preferences.putUShort("api", version) == sizeof(uint16_t) &&
         preferences.putString("device_id", configuredDevice) > 0 &&
         preferences.putString("server_id", serverId) > 0 &&
         preferences.putString("token", token) > 0 &&
         preferences.putString("service", service) > 0 &&
         preferences.putString("path", path) > 0 &&
         preferences.putUShort("port", port) == sizeof(uint16_t) &&
         preferences.putString("wifi_ssid", ssid) > 0 &&
         preferences.putString("wifi_pass", password) == passwordLength;
}

class ConfigCallbacks final : public BLECharacteristicCallbacks {
 public:
  void onWrite(BLECharacteristic *characteristic) override {
    if (!pairingAuthenticated) {
      reject("error:not authenticated");
      return;
    }
    const std::string frame = characteristic->getValue();
    if (frame.size() < kFrameHeaderSize ||
        static_cast<uint8_t>(frame[0]) != kFrameMagic) {
      reject("error:bad setup frame");
      return;
    }
    const uint8_t sequence = static_cast<uint8_t>(frame[1]);
    const uint8_t total = static_cast<uint8_t>(frame[2]);
    if (total == 0) {
      reject("error:bad frame count");
      return;
    }
    if (sequence == 0) {
      configBuffer = "";
      configBuffer.reserve(static_cast<size_t>(total) * 17);
      expectedFrames = total;
      nextFrame = 0;
      configurationReady = false;
      setPairingStatus("receiving");
    }
    if (expectedFrames != total || sequence != nextFrame ||
        configBuffer.length() + frame.size() - kFrameHeaderSize >
            kMaximumConfigBytes) {
      reject("error:frame order");
      return;
    }
    configBuffer.concat(frame.data() + kFrameHeaderSize,
                        frame.size() - kFrameHeaderSize);
    ++nextFrame;
    if (nextFrame != expectedFrames) return;

    JsonDocument document;
    if (deserializeJson(document, configBuffer) || !saveCommission(document)) {
      reject("error:invalid setup");
      return;
    }
    // Commissioning is complete; release the frame buffer before Wi-Fi, BLE,
    // the dashboard document, and the audio task compete for heap.
    configBuffer = static_cast<const char *>(nullptr);
    expectedFrames = 0;
    nextFrame = 0;
    setPairingStatus("joining");
    configurationReady = true;
  }

 private:
  void reject(const char *status) {
    configBuffer = "";
    expectedFrames = 0;
    nextFrame = 0;
    configurationReady = false;
    setPairingStatus(status);
  }
};

class PairingSecurityCallbacks final : public BLESecurityCallbacks {
 public:
  uint32_t onPassKeyRequest() override { return 0; }

  void onPassKeyNotify(uint32_t passKey) override {
    Serial.printf("[pairing] passkey %06lu\n",
                  static_cast<unsigned long>(passKey));
  }

  bool onSecurityRequest() override { return true; }

  void onAuthenticationComplete(esp_ble_auth_cmpl_t result) override {
    pairingAuthenticated = result.success;
    if (!result.success) {
      Serial.printf("[pairing] authentication failed: 0x%02x\n",
                    result.fail_reason);
      showPairingScreen();
    }
  }

  bool onConfirmPIN(uint32_t pin) override {
    showNumericComparison(pin);
    const TapDecision decision = tapInput.waitForPairingDecision();
    const bool accepted = decision == TapDecision::Confirm;
    if (accepted) {
      DisplayGuard guard;
      display.fillScreen(kBackground);
      drawCentered(BuntyFonts::kHeading, "CONFIRMED", 122, kSuccess);
      drawCentered(BuntyFonts::kBody, "Finishing secure setup", 174, kMuted);
      display.present();
    } else {
      showPairingScreen();
    }
    return accepted;
  }
};

class PairingServerCallbacks final : public BLEServerCallbacks {
 public:
  void onDisconnect(BLEServer *) override {
    const bool authenticatedButUncommissioned =
        pairingAuthenticated && pairingCompletedAt == 0;
    pairingAuthenticated = false;
    if (authenticatedButUncommissioned) {
      freshPairingRestartRequested = true;
      return;
    }
    if (pairingStarted && pairingCompletedAt == 0) {
      BLEDevice::startAdvertising();
      showPairingScreen();
    }
  }
};

ConfigCallbacks configCallbacks;
PairingSecurityCallbacks securityCallbacks;
PairingServerCallbacks serverCallbacks;
BLESecurity pairingSecurity;

void clearBondedPeersIfRequested() {
  if (!preferences.getBool("clear_bonds", false)) return;
  const int count = esp_ble_get_bond_device_num();
  if (count > 0) {
    auto *peers = static_cast<esp_ble_bond_dev_t *>(
        malloc(sizeof(esp_ble_bond_dev_t) * count));
    int found = count;
    if (peers && esp_ble_get_bond_device_list(&found, peers) == ESP_OK) {
      for (int i = 0; i < found; ++i) {
        esp_ble_remove_bond_device(peers[i].bd_addr);
      }
    }
    free(peers);
  }
  preferences.remove("clear_bonds");
}

void loadPairingAddress(esp_bd_addr_t address) {
  const bool stored =
      preferences.getBytesLength("ble_addr") == sizeof(esp_bd_addr_t);
  if (stored) {
    preferences.getBytes("ble_addr", address, sizeof(esp_bd_addr_t));
  } else {
    esp_fill_random(address, sizeof(esp_bd_addr_t));
  }
  // esp_bd_addr_t stores the most-significant octet first. Static random
  // addresses require address bits 47:46 to be 0b11; setting address[5]
  // instead made Bluedroid reject the value and silently prevented adverts.
  address[0] = static_cast<uint8_t>((address[0] & 0x3f) | 0xc0);
  preferences.putBytes("ble_addr", address, sizeof(esp_bd_addr_t));
}

void pairingGapEvent(esp_gap_ble_cb_event_t event,
                     esp_ble_gap_cb_param_t *parameter) {
  if (event != ESP_GAP_BLE_SET_STATIC_RAND_ADDR_EVT) return;
  pairingAddressState =
      parameter->set_rand_addr_cmpl.status == ESP_BT_STATUS_SUCCESS ? 1 : 0;
}

bool startPairing() {
  BLEDevice::init(deviceName.c_str());
  BLEDevice::setMTU(185);
  BLEDevice::setEncryptionLevel(ESP_BLE_SEC_ENCRYPT_MITM);
  BLEDevice::setSecurityCallbacks(&securityCallbacks);
  BLEDevice::setCustomGapHandler(pairingGapEvent);
  pairingSecurity.setAuthenticationMode(ESP_LE_AUTH_REQ_SC_MITM_BOND);
  pairingSecurity.setCapability(ESP_IO_CAP_IO);
  pairingSecurity.setKeySize(16);
  pairingSecurity.setInitEncryptionKey(ESP_BLE_ENC_KEY_MASK |
                                       ESP_BLE_ID_KEY_MASK);
  pairingSecurity.setRespEncryptionKey(ESP_BLE_ENC_KEY_MASK |
                                       ESP_BLE_ID_KEY_MASK);
  clearBondedPeersIfRequested();

  pairingServer = BLEDevice::createServer();
  if (!pairingServer) {
    pairingError = "BLE server";
    return false;
  }
  pairingServer->setCallbacks(&serverCallbacks);
  BLEService *service = pairingServer->createService(kPairingServiceUuid);
  BLECharacteristic *identity = service->createCharacteristic(
      kIdentityCharacteristicUuid, BLECharacteristic::PROPERTY_READ);
  identity->setAccessPermissions(ESP_GATT_PERM_READ_ENC_MITM);
  JsonDocument identityDocument;
  identityDocument["protocol"] = kPairingProtocol;
  identityDocument["version"] = kPairingProtocolVersion;
  identityDocument["device_id"] = hardwareId;
  identityDocument["name"] = deviceName;
  identityDocument["kind"] = "display";
  identityDocument["firmware_version"] = kFirmwareVersion;
  identityDocument["capabilities"][0] = "dashboard.pull";
  identityDocument["capabilities"][1] = "dashboard.cache";
  identityDocument["capabilities"][2] = "display";
  identityDocument["capabilities"][3] = "audio.output";
  String identityJson;
  serializeJson(identityDocument, identityJson);
  identity->setValue(identityJson.c_str());

  BLECharacteristic *configuration = service->createCharacteristic(
      kConfigCharacteristicUuid, BLECharacteristic::PROPERTY_WRITE);
  configuration->setAccessPermissions(ESP_GATT_PERM_WRITE_ENC_MITM);
  configuration->setCallbacks(&configCallbacks);

  pairingStatus = service->createCharacteristic(
      kStatusCharacteristicUuid, BLECharacteristic::PROPERTY_READ);
  pairingStatus->setAccessPermissions(ESP_GATT_PERM_READ_ENC_MITM);
  pairingStatus->setValue("ready");
  service->start();

  BLEAdvertising *advertising = BLEDevice::getAdvertising();
  esp_bd_addr_t address;
  loadPairingAddress(address);
  pairingAddressState = -1;
  advertising->setDeviceAddress(address, BLE_ADDR_TYPE_RANDOM);
  const uint32_t addressDeadline = millis() + 1500;
  while (pairingAddressState < 0 &&
         static_cast<int32_t>(millis() - addressDeadline) < 0) {
    delay(5);
  }
  if (pairingAddressState != 1) {
    pairingError = "BLE address";
    Serial.println("[pairing] static random address was rejected");
    return false;
  }
  advertising->addServiceUUID(kPairingServiceUuid);
  advertising->setScanResponse(true);
  advertising->setMinPreferred(0x06);
  advertising->setMaxPreferred(0x12);
  BLEDevice::startAdvertising();

  pairingStarted = true;
  pairingScreenVisible = true;
  Serial.printf("[pairing] advertising as %s\n", deviceName.c_str());
  return true;
}

void stopPairing() {
  if (!pairingStarted) return;
  BLEDevice::stopAdvertising();
  BLEDevice::deinit(true);
  pairingServer = nullptr;
  pairingStatus = nullptr;
  pairingStarted = false;
  Serial.println("[pairing] Bluetooth stopped");
}

void drawText(const GFXfont *font, const String &text, int16_t x, int16_t y,
              uint16_t color) {
  display.setFont(font);
  display.setTextSize(1);
  display.setTextColor(color);
  display.setTextWrap(false);
  display.setCursor(x, y);
  display.print(text);
}

uint16_t textWidth(const GFXfont *font, const String &text) {
  display.setFont(font);
  display.setTextSize(1);
  display.setTextWrap(false);
  int16_t x = 0;
  int16_t y = 0;
  uint16_t width = 0;
  uint16_t height = 0;
  display.getTextBounds(text.c_str(), 0, 0, &x, &y, &width, &height);
  return width;
}

String fitText(const GFXfont *font, const String &value,
               uint16_t maximumWidth) {
  if (textWidth(font, value) <= maximumWidth) return value;

  String shortened = value;
  const String ellipsis = "...";
  while (!shortened.isEmpty()) {
    size_t start = shortened.length() - 1;
    while (start > 0 &&
           (static_cast<uint8_t>(shortened[start]) & 0xc0) == 0x80) {
      --start;
    }
    shortened.remove(start);
    const String candidate = shortened + ellipsis;
    if (textWidth(font, candidate) <= maximumWidth) return candidate;
  }
  return ellipsis;
}

void drawTextRight(const GFXfont *font, const String &text, int16_t right,
                   int16_t y, uint16_t color) {
  display.setFont(font);
  display.setTextSize(1);
  display.setTextWrap(false);
  int16_t xOffset = 0;
  int16_t yOffset = 0;
  uint16_t width = 0;
  uint16_t height = 0;
  display.getTextBounds(text.c_str(), 0, 0, &xOffset, &yOffset, &width,
                        &height);
  drawText(font, text, right - static_cast<int16_t>(width) - xOffset, y,
           color);
}

void drawRuntimeTopBar(bool present) {
  const bool online = WiFi.status() == WL_CONNECTED;
  display.fillRect(0, 0, kScreenWidth, 32, kSurface);
  drawBluetoothIcon(14, 16, kSuccess);
  drawText(BuntyFonts::kBody, "PAIRED", 25, 21, kSuccess);
  drawWifiIcon(149, 20, online ? kSuccess : kBorder);
  drawText(BuntyFonts::kBody, online ? "ONLINE" : "OFFLINE", 162, 21,
           online ? kSuccess : kMuted);
  display.drawFastHLine(0, 31, kScreenWidth, kBorder);
  if (present) display.present(0, 0, kScreenWidth, 32);
}

void drawDashboardHeader(const String &title) {
  display.fillScreen(kBackground);
  drawRuntimeTopBar(false);
  drawText(BuntyFonts::kHeading,
           fitText(BuntyFonts::kHeading, title, 216), 12, 74, kText);
  display.drawFastHLine(12, 89, 216, kBorder);
}

void drawDashboardFooter() {
  display.drawFastHLine(12, 292, 216, kBorder);
  drawText(BuntyFonts::kBody, "3 firm taps: unpair", 12, 313, kMuted);
  drawTextRight(BuntyFonts::kBody, dashboardOnline ? "LIVE" : "CACHED", 228,
                313, dashboardOnline ? kSuccess : kMuted);
}

bool validDashboard(const String &body, JsonDocument &document) {
  if (body.isEmpty() || body.length() > kMaximumDashboardBytes ||
      deserializeJson(document, body)) {
    return false;
  }
  const char *protocol = document["protocol"] | "";
  return strcmp(protocol, kDashboardProtocol) == 0 &&
         static_cast<uint16_t>(document["schema_version"] | 0) == 1;
}

uint8_t dashboardPageCount(const JsonDocument &document) {
  const size_t providers =
      document["quota"]["providers"].as<JsonArrayConst>().size();
  return static_cast<uint8_t>(3 + providers);
}

size_t pageOffset(size_t itemCount, size_t itemsPerPage) {
  if (itemCount <= itemsPerPage) return 0;
  return (static_cast<size_t>(dashboardRound) * itemsPerPage) % itemCount;
}

void renderDashboard() {
  if (runtimeDisplayState != RuntimeDisplayState::Dashboard) return;
  DisplayGuard guard;
  JsonDocument document;
  if (!dashboardAvailable || !validDashboard(dashboardJson, document)) {
    display.fillScreen(kBackground);
    drawRuntimeTopBar(false);
    drawCentered(BuntyFonts::kHeading, "SYNCING", 91, kText);
    drawCentered(BuntyFonts::kBody, "Fetching Gauge stats", 158, kMuted);
    drawCentered(BuntyFonts::kBody, "Will retry automatically", 194, kMuted);
    display.present();
    return;
  }

  JsonArrayConst providers =
      document["quota"]["providers"].as<JsonArrayConst>();
  const uint8_t pageCount = dashboardPageCount(document);
  if (dashboardPage >= pageCount) dashboardPage = 0;
  int16_t y = 119;
  if (dashboardPage == 0) {
    drawDashboardHeader("AI USAGE");
    const size_t offset = pageOffset(providers.size(), 4);
    for (size_t i = offset; i < providers.size() && i < offset + 4; ++i) {
      JsonObjectConst provider = providers[i].as<JsonObjectConst>();
      const String name =
          String(static_cast<const char *>(provider["name"] | "Agent"));
      const int remaining = provider["remaining_percent"] | -1;
      drawText(BuntyFonts::kBody, fitText(BuntyFonts::kBody, name, 120), 14, y,
               kText);
      drawTextRight(BuntyFonts::kHeading,
                    remaining >= 0 ? String(remaining) + "%" : "--", 228,
                    y + 3, remaining >= 20 ? kSuccess : kDanger);
      y += 42;
    }
  } else if (dashboardPage <= providers.size()) {
    JsonObjectConst provider =
        providers[dashboardPage - 1].as<JsonObjectConst>();
    const String name =
        String(static_cast<const char *>(provider["name"] | "AI LIMITS"));
    drawDashboardHeader(name);
    JsonArrayConst limits = provider["limits"].as<JsonArrayConst>();
    const size_t offset = pageOffset(limits.size(), 6);
    y = 116;
    for (size_t i = offset; i < limits.size() && i < offset + 6; ++i) {
      JsonObjectConst limit = limits[i].as<JsonObjectConst>();
      const String label =
          String(static_cast<const char *>(limit["label"] | "Limit"));
      const int remaining = limit["remaining_percent"] | -1;
      drawText(BuntyFonts::kBody, fitText(BuntyFonts::kBody, label, 130), 14, y,
               kText);
      drawTextRight(BuntyFonts::kBody,
                    remaining >= 0 ? String(remaining) + "% left" : "--",
                    228, y, kMuted);
      y += 29;
    }
  } else if (dashboardPage == providers.size() + 1) {
    drawDashboardHeader("CALENDAR");
    JsonArrayConst events =
        document["calendar"]["events"].as<JsonArrayConst>();
    y = 116;
    if (events.size() == 0) {
      drawText(BuntyFonts::kBody, "No upcoming events", 14, y, kMuted);
    }
    const size_t offset = pageOffset(events.size(), 5);
    for (size_t i = offset; i < events.size() && i < offset + 5; ++i) {
      JsonObjectConst event = events[i].as<JsonObjectConst>();
      const String title =
          String(static_cast<const char *>(event["title"] | "Untitled"));
      drawText(BuntyFonts::kBody, fitText(BuntyFonts::kBody, title, 212), 14, y,
               kText);
      y += 34;
    }
  } else {
    drawDashboardHeader("TO-DO");
    JsonArrayConst todos = document["todos"].as<JsonArrayConst>();
    y = 116;
    if (todos.size() == 0) {
      drawText(BuntyFonts::kBody, "Nothing left to do", 14, y, kMuted);
    }
    const size_t offset = pageOffset(todos.size(), 6);
    for (size_t i = offset; i < todos.size() && i < offset + 6; ++i) {
      JsonObjectConst todo = todos[i].as<JsonObjectConst>();
      const bool complete = todo["completed"] | false;
      const String title =
          String(static_cast<const char *>(todo["title"] | "Untitled"));
      const String line = String(complete ? "[x] " : "[ ] ") + title;
      drawText(BuntyFonts::kBody, fitText(BuntyFonts::kBody, line, 212), 14, y,
               complete ? kMuted : kText);
      y += 29;
    }
  }
  drawDashboardFooter();
  display.present();
  nextDashboardPageAt = millis() + kDashboardPageTimeMs;
}

bool tapMeetsStrength(const TapEvent &event, uint32_t minimumLevel,
                      uint32_t noiseMultiplier) {
  const uint32_t floor = event.noiseFloor == 0 ? 1 : event.noiseFloor;
  return event.level >= minimumLevel &&
         static_cast<uint64_t>(event.level) >=
             static_cast<uint64_t>(floor) * noiseMultiplier;
}

void clearResetTapSequence() {
  resetTapCount = 0;
  resetLastTapAt = 0;
  resetFirstGapMs = 0;
}

void enterDisplaySleep() {
  runtimeDisplayState = RuntimeDisplayState::Sleeping;
  clearResetTapSequence();
  tapInput.reset();
  tapInput.suppressFor(500);
  {
    DisplayGuard guard;
    buntyAnimations.beginSleeping(millis());
  }
  fadeBacklightTo(kSleepBacklightDuty);
  Serial.println("[display] sleeping; background sync remains active");
}

void wakeDisplay() {
  runtimeDisplayState = RuntimeDisplayState::Waking;
  clearResetTapSequence();
  const uint32_t now = millis();
  // The wake knock and its enclosure echoes must never seed an unpair
  // gesture when the dashboard returns.
  tapInput.reset();
  tapInput.suppressFor(BuntyAnimations::wakeDurationMs() + 400);
  {
    DisplayGuard guard;
    buntyAnimations.beginWaking(now);
  }
  fadeBacklightTo(255);
  Serial.println("[display] wake tap accepted");
}

// Returns true while the short wake animation intentionally owns the screen.
// Sleep animation does not pause network work, so stats continue refreshing
// and the newest cached page is ready as soon as Bunty wakes.
bool serviceEmotionState() {
  const uint32_t now = millis();
  if (runtimeDisplayState == RuntimeDisplayState::Sleeping) {
    TapEvent event;
    if (tapInput.poll(&event) &&
        tapMeetsStrength(event, kWakeTapMinimumLevel,
                         kWakeTapNoiseMultiplier)) {
      wakeDisplay();
      return true;
    }
    {
      DisplayGuard guard;
      buntyAnimations.serviceSleeping(now);
    }
    return false;
  }

  if (runtimeDisplayState != RuntimeDisplayState::Waking) return false;

  // Normally suppression prevents events here; draining is an extra boundary
  // between the wake gesture and the destructive runtime gesture recognizer.
  while (tapInput.poll()) {
  }
  bool stillWaking = false;
  {
    DisplayGuard guard;
    stillWaking = buntyAnimations.serviceWaking(now);
  }
  if (!stillWaking) {
    runtimeDisplayState = RuntimeDisplayState::Dashboard;
    lastRuntimeInteractionAt = now;
    nextDashboardPageAt = now + kDashboardPageTimeMs;
    tapInput.reset();
    tapInput.suppressFor(500);
    renderDashboard();
    return false;
  }
  return true;
}

void loadDashboard() {
  if (!preferences.isKey("dashboard")) {
    dashboardJson = "";
    dashboardAvailable = false;
    return;
  }
  const size_t length = preferences.getBytesLength("dashboard");
  if (length == 0 || length > kMaximumDashboardBytes) {
    dashboardJson = "";
    dashboardAvailable = false;
    return;
  }
  auto *body = static_cast<char *>(malloc(length + 1));
  if (!body || preferences.getBytes("dashboard", body, length) != length) {
    free(body);
    dashboardJson = "";
    dashboardAvailable = false;
    return;
  }
  body[length] = '\0';
  dashboardJson = String(body, length);
  free(body);
  JsonDocument document;
  dashboardAvailable = validDashboard(dashboardJson, document);
}

bool startMdns() {
  if (mdnsStarted) return true;
  mdnsStarted = MDNS.begin(hardwareId.c_str());
  if (!mdnsStarted) Serial.println("[dashboard] mDNS start failed");
  return mdnsStarted;
}

bool sameSubnet(const IPAddress &left, const IPAddress &right,
                const IPAddress &mask) {
  for (uint8_t index = 0; index < 4; ++index) {
    if ((left[index] & mask[index]) != (right[index] & mask[index])) {
      return false;
    }
  }
  return true;
}

bool locateGauge(IPAddress &address, uint16_t &port, String &path) {
  if (!startMdns()) return false;
  const String expectedServer = preferences.getString("server_id", "");
  const int services = MDNS.queryService("gauge", "tcp");
  IPAddress fallback;
  uint16_t fallbackPort = 0;
  for (int i = 0; i < services; ++i) {
    if (MDNS.txt(i, "id") == expectedServer) {
      const IPAddress candidate = MDNS.IP(i);
      const uint16_t candidatePort = MDNS.port(i);
      if (candidate == IPAddress() || candidatePort == 0 ||
          candidate[0] == 127) {
        continue;
      }
      if (sameSubnet(candidate, WiFi.localIP(), WiFi.subnetMask())) {
        address = candidate;
        port = candidatePort;
        path = preferences.getString("path", "/v1/dashboard");
        Serial.printf("[dashboard] discovered Gauge at %s:%u\n",
                      address.toString().c_str(), port);
        return true;
      }
      if (fallback == IPAddress()) {
        fallback = candidate;
        fallbackPort = candidatePort;
      }
    }
  }
  if (fallback == IPAddress()) return false;
  address = fallback;
  port = fallbackPort;
  path = preferences.getString("path", "/v1/dashboard");
  Serial.printf("[dashboard] discovered off-subnet Gauge at %s:%u\n",
                address.toString().c_str(), port);
  return true;
}

void resetMdns() {
  if (!mdnsStarted) return;
  MDNS.end();
  mdnsStarted = false;
}

void noteDashboardConnectFailure(const IPAddress &address, uint16_t port) {
  if (dashboardConnectFailures < UINT8_MAX) ++dashboardConnectFailures;
  Serial.printf(
      "[dashboard] TCP connect failed to %s:%u (%u/%u, RSSI %d dBm)\n",
      address.toString().c_str(), port, dashboardConnectFailures,
      kDashboardReconnectAfterFailures, WiFi.RSSI());

  // Bonjour can retain a stale answer across a Mac sleep/wake or DHCP lease
  // change. Drop it immediately so the next retry performs fresh discovery.
  resetMdns();
  if (dashboardConnectFailures < kDashboardReconnectAfterFailures) return;

  // ESP32 modem sleep can leave a long-lived LAN accessory with a stale ARP
  // or station route even though WiFi.status() still reports connected. Force
  // one clean station reconnect after repeated TCP failures; cached dashboard
  // data stays available while it heals.
  dashboardConnectFailures = 0;
  Serial.println("[dashboard] refreshing Wi-Fi connection");
  if (!WiFi.reconnect()) {
    WiFi.begin(preferences.getString("wifi_ssid", "").c_str(),
               preferences.getString("wifi_pass", "").c_str());
  }
}

bool fetchDashboard() {
  if (WiFi.status() != WL_CONNECTED) return false;
  IPAddress address;
  uint16_t port = 0;
  String path;
  if (!locateGauge(address, port, path)) return false;

  WiFiClient client;
  // Connect to the discovered numeric address ourselves. This avoids sending
  // the address back through hostByName() inside HTTPClient and gives recovery
  // logic an exact TCP failure boundary.
  if (!client.connect(address, port, 4000)) {
    noteDashboardConnectFailure(address, port);
    return false;
  }
  dashboardConnectFailures = 0;

  HTTPClient request;
  request.setConnectTimeout(4000);
  request.setTimeout(5000);
  if (!request.begin(client, address.toString(), port, path)) {
    noteDashboardConnectFailure(address, port);
    return false;
  }
  request.addHeader("Accept", "application/json");
  request.addHeader("Authorization",
                    "Bearer " + preferences.getString("token", ""));
  const int status = request.GET();
  if (status != HTTP_CODE_OK) {
    if (status < 0) {
      Serial.printf("[dashboard] HTTP %d (%s) via %s:%u\n", status,
                    HTTPClient::errorToString(status).c_str(),
                    address.toString().c_str(), port);
    } else {
      Serial.printf("[dashboard] HTTP %d via %s:%u\n", status,
                    address.toString().c_str(), port);
    }
    request.end();
    if (status < 0) noteDashboardConnectFailure(address, port);
    return false;
  }
  const String body = request.getString();
  request.end();
  JsonDocument document;
  if (!validDashboard(body, document)) {
    Serial.println("[dashboard] invalid snapshot");
    return false;
  }

  const uint32_t seconds = document["refresh_seconds"] | 120;
  dashboardRefreshMs = constrain(seconds, 30U, 900U) * 1000U;
  if (body != dashboardJson) {
    dashboardJson = body;
    if (preferences.putBytes("dashboard", dashboardJson.c_str(),
                             dashboardJson.length()) !=
        dashboardJson.length()) {
      Serial.println("[dashboard] could not persist snapshot");
    }
  }
  dashboardAvailable = true;
  Serial.printf("[dashboard] cached %u bytes\n",
                static_cast<unsigned>(dashboardJson.length()));
  return true;
}

void startRuntime(bool reconnectWifi) {
  runtimeStarted = true;
  runtimeDisplayState = RuntimeDisplayState::Dashboard;
  lastRuntimeInteractionAt = millis();
  clearResetTapSequence();
  loadDashboard();
  if (reconnectWifi) {
    WiFi.mode(WIFI_STA);
    WiFi.setAutoReconnect(true);
    WiFi.begin(preferences.getString("wifi_ssid", "").c_str(),
               preferences.getString("wifi_pass", "").c_str());
  }
  // startRuntime(false) follows a successful pairing connection, so apply the
  // reliability setting on both cold boot and newly paired paths.
  WiFi.setSleep(false);
  dashboardConnectFailures = 0;
  dashboardOnline = false;
  lastWifiStatus = WiFi.status();
  nextDashboardFetchAt = 0;
  dashboardPage = 0;
  dashboardRound = 0;
  nextVoiceStartAt = 0;
  if (!greetingPlaying) renderDashboard();
}

void notifyGaugeBeforeUnpairing() {
  if (WiFi.status() != WL_CONNECTED) return;
  IPAddress address;
  uint16_t port = 0;
  String ignoredPath;
  if (!locateGauge(address, port, ignoredPath)) return;

  WiFiClient client;
  HTTPClient request;
  request.setConnectTimeout(2000);
  request.setTimeout(2500);
  if (!request.begin(client, address.toString(), port, "/v1/accessory")) {
    return;
  }
  request.addHeader("Authorization",
                    "Bearer " + preferences.getString("token", ""));
  const int status = request.sendRequest("DELETE");
  request.end();
  Serial.printf("[unpair] Gauge returned HTTP %d\n", status);
}

void forgetPairing() {
  {
    DisplayGuard guard;
    display.fillScreen(kBackground);
    drawCentered(BuntyFonts::kHeading, "UNPAIRED", 121, kSuccess);
    drawCentered(BuntyFonts::kBody, "Restarting in pairing mode", 174, kMuted);
    display.present();
  }
  notifyGaugeBeforeUnpairing();
  WiFi.disconnect(true, true);
  preferences.clear();
  preferences.putBool("clear_bonds", true);
  delay(800);
  ESP.restart();
}

void serviceResetGesture() {
  const uint32_t loopNow = millis();
  if (greetingPlaying) return;

  if (resetGestureState == ResetGestureState::Idle && resetTapCount > 0 &&
      loopNow - resetLastTapAt > kUnpairMaximumGapMs) {
    clearResetTapSequence();
  }
  if (resetGestureState == ResetGestureState::AwaitingConfirmation &&
      loopNow - resetPromptedAt > kUnpairConfirmationTimeoutMs) {
    resetGestureState = ResetGestureState::Idle;
    clearResetTapSequence();
    lastRuntimeInteractionAt = loopNow;
    tapInput.reset();
    tapInput.suppressFor(500);
    renderDashboard();
    Serial.println("[unpair] confirmation timed out");
    return;
  }

  TapEvent event;
  if (!tapInput.poll(&event)) return;

  if (resetGestureState == ResetGestureState::AwaitingConfirmation) {
    if (static_cast<int32_t>(event.occurredAt - resetPromptedAt) < 0) return;
    const uint32_t sincePrompt = event.occurredAt - resetPromptedAt;
    if (sincePrompt < kUnpairConfirmationGuardMs ||
        sincePrompt > kUnpairConfirmationTimeoutMs ||
        !tapMeetsStrength(event, kUnpairTapMinimumLevel,
                          kUnpairTapNoiseMultiplier)) {
      return;
    }
    Serial.println("[unpair] guarded confirmation accepted");
    forgetPairing();
    return;
  }

  if (!tapMeetsStrength(event, kUnpairTapMinimumLevel,
                        kUnpairTapNoiseMultiplier)) {
    return;
  }
  lastRuntimeInteractionAt = loopNow;

  if (resetTapCount == 0) {
    resetTapCount = 1;
    resetLastTapAt = event.occurredAt;
  } else {
    const uint32_t gap = event.occurredAt - resetLastTapAt;
    if (gap < kUnpairMinimumGapMs) {
      // Ignore a quick enclosure echo without shifting the rhythm window.
      return;
    }
    if (gap > kUnpairMaximumGapMs) {
      resetTapCount = 1;
      resetFirstGapMs = 0;
      resetLastTapAt = event.occurredAt;
    } else if (resetTapCount == 1) {
      resetTapCount = 2;
      resetFirstGapMs = gap;
      resetLastTapAt = event.occurredAt;
    } else {
      const uint32_t rhythmDifference =
          gap > resetFirstGapMs ? gap - resetFirstGapMs
                                : resetFirstGapMs - gap;
      if (rhythmDifference <= kUnpairRhythmToleranceMs) {
        resetTapCount = 3;
      } else {
        // Keep the last two taps as the start of a fresh, regular rhythm.
        resetTapCount = 2;
        resetFirstGapMs = gap;
      }
      resetLastTapAt = event.occurredAt;
    }
  }

  Serial.printf("[unpair] deliberate tap %u/3 (level %u)\n",
                static_cast<unsigned>(resetTapCount),
                static_cast<unsigned>(event.level));
  if (resetTapCount == 3) {
    resetGestureState = ResetGestureState::AwaitingConfirmation;
    resetPromptedAt = millis();
    lastRuntimeInteractionAt = resetPromptedAt;
    clearResetTapSequence();
    // Discard any fourth tap or enclosure echo already queued with the
    // initiating rhythm. Confirmation only opens after a visible one-second
    // guard, and it must meet the same firm-tap threshold.
    tapInput.reset();
    tapInput.suppressFor(kUnpairConfirmationGuardMs);
    showResetConfirmation();
  }
}

void servicePairing() {
  if (freshPairingRestartRequested) {
    freshPairingRestartRequested = false;
    Serial.println(
        "[pairing] incomplete secure session; rotating BLE identity");
    // macOS may retain its half of the completed BLE bond. A new static random
    // address prevents that stale bond from suppressing numeric comparison on
    // the next attempt. startPairing() clears Bunty's link keys after reboot.
    preferences.remove("ble_addr");
    preferences.putBool("clear_bonds", true);
    delay(150);
    ESP.restart();
    return;
  }

  if (configurationReady && !wifiJoinStarted) {
    wifiJoinStarted = true;
    wifiJoinStartedAt = millis();
    WiFi.mode(WIFI_STA);
    WiFi.setAutoReconnect(true);
    // ESP32-S3 requires Wi-Fi modem sleep while the Bluedroid controller is
    // still active. Disabling it here aborts inside ESP-IDF's coexistence
    // power-management path. startRuntime() disables sleep after stopPairing()
    // has fully released Bluetooth.
    WiFi.setSleep(true);
    WiFi.begin(preferences.getString("wifi_ssid", "").c_str(),
               preferences.getString("wifi_pass", "").c_str());
  }

  if (wifiJoinStarted && pairingCompletedAt == 0 &&
      WiFi.status() == WL_CONNECTED) {
    preferences.putBool("paired", true);
    setPairingStatus("connected");
    pairingCompletedAt = millis();
  } else if (wifiJoinStarted && pairingCompletedAt == 0 &&
             millis() - wifiJoinStartedAt > kWifiJoinTimeoutMs) {
    setPairingStatus("error:wifi connection failed");
    configurationReady = false;
    wifiJoinStarted = false;
    WiFi.disconnect(false, false);
    showPairingScreen();
  }

  if (pairingCompletedAt != 0 &&
      millis() - pairingCompletedAt > kPairingShutdownDelayMs) {
    // Bluedroid owns a large portion of internal heap. Release it before the
    // I2S driver allocates DMA memory for the greeting.
    stopPairing();
    showSpeakingEyes();
    startGreeting();
    startRuntime(false);
    return;
  }
  if (pairingScreenVisible && millis() - lastActivityUpdate >= 420) {
    lastActivityUpdate = millis();
    activityFrame = (activityFrame + 1) % 3;
    drawPairingActivity();
  }
}

void serviceRuntime() {
  if (greetingPlaying) {
    tapInput.reset();
    return;
  }
  if (showingGreetingEyes) {
    showingGreetingEyes = false;
    runtimeDisplayState = RuntimeDisplayState::Dashboard;
    lastRuntimeInteractionAt = millis();
    renderDashboard();
  }

  if (runtimeDisplayState == RuntimeDisplayState::Dashboard) {
    serviceResetGesture();
    if (resetGestureState == ResetGestureState::AwaitingConfirmation) return;
  } else if (serviceEmotionState()) {
    return;
  }

  if (runtimeDisplayState == RuntimeDisplayState::Dashboard &&
      resetGestureState == ResetGestureState::Idle && resetTapCount == 0 &&
      millis() - lastRuntimeInteractionAt >= kDashboardIdleSleepMs) {
    enterDisplaySleep();
  }

  // Once local VAD has opened a turn, keep dashboard discovery, TCP timeouts,
  // and page rendering out of the latency-sensitive capture/playback path.
  if (buntyVoice.engaged()) return;

  const wl_status_t wifiStatus = WiFi.status();
  const bool wifiChanged = wifiStatus != lastWifiStatus;
  if (wifiChanged) {
    resetMdns();
    lastWifiStatus = wifiStatus;
    nextDashboardFetchAt = 0;
  }
  if (wifiStatus != WL_CONNECTED) {
    dashboardOnline = false;
  }
  if (wifiChanged) {
    if (runtimeDisplayState == RuntimeDisplayState::Dashboard) {
      renderDashboard();
    } else {
      DisplayGuard guard;
      drawRuntimeTopBar(true);
    }
  }
  if (static_cast<int32_t>(millis() - nextDashboardFetchAt) >= 0) {
    const bool wasOnline = dashboardOnline;
    const bool fetched = fetchDashboard();
    dashboardOnline = fetched;
    nextDashboardFetchAt =
        millis() + (fetched ? dashboardRefreshMs : 15000U);
    if ((fetched || wasOnline != dashboardOnline) &&
        runtimeDisplayState == RuntimeDisplayState::Dashboard) {
      renderDashboard();
    }
  }
  if (runtimeDisplayState == RuntimeDisplayState::Dashboard &&
      static_cast<int32_t>(millis() - nextDashboardPageAt) >= 0) {
    JsonDocument document;
    if (dashboardAvailable && validDashboard(dashboardJson, document)) {
      dashboardPage =
          static_cast<uint8_t>((dashboardPage + 1) % dashboardPageCount(document));
      if (dashboardPage == 0) ++dashboardRound;
      renderDashboard();
    }
  }
}

}  // namespace

void setup() {
  Serial.begin(115200);
  delay(100);

  displayMutex = xSemaphoreCreateRecursiveMutex();
  if (!displayMutex) {
    Serial.println("[display] could not create render mutex");
  }

  ledcSetup(kBacklightChannel, 12000, 8);
  ledcAttachPin(TFT_BL, kBacklightChannel);
  ledcWrite(kBacklightChannel, 0);
  // The explicit ST77xx transport initializes SPI; Display then claims ~150 KB
  // for its framebuffer before Wi-Fi and BLE take their share of the heap.
  if (!display.begin()) {
    Serial.println("[display] could not allocate the framebuffer");
    return;
  }
  Serial.printf("[display] free heap %u bytes, psram %u bytes\n",
                static_cast<unsigned>(ESP.getFreeHeap()),
                static_cast<unsigned>(ESP.getPsramSize()));
  display.clear(kBackground);

  // Retained behind a master switch so calibration can be restored without
  // reconstructing the servo setup. When disabled, GPIO 7 and GPIO 8 remain
  // at their reset defaults and no servo PWM is generated.
  if (kServoMotorsEnabled) {
    if (!turnServoMotion.begin(TURN_SERVO_PIN, kTurnServoChannel,
                               kTurnServoHomeAngle)) {
      Serial.println("[servo] could not initialize GPIO 7 turn PWM");
    } else {
      Serial.printf("[servo] GPIO 7 holding at %u degrees\n",
                    kTurnServoHomeAngle);
    }
  }

  // The amp idles shut down; playGreeting() wakes it for the clip only.
  pinMode(AMP_SD_PIN, OUTPUT);
  digitalWrite(AMP_SD_PIN, LOW);

  preferences.begin("bunty", false);
  syncIotGatewayCredential();
  loadIdentity();
  const bool tapsReady =
      tapInput.begin(kMicrophonePort, MIC_BCLK, MIC_WS, MIC_DATA);
  if (preferences.getBool("paired", false)) {
    startRuntime(true);
    fadeBacklight(true);
    return;
  }

  // Pairing mode means there is no valid application credential. Never reuse
  // a link-layer bond left behind by a failed or interrupted commission: a
  // fresh address makes macOS display numeric comparison every time Bunty
  // starts an uncommissioned pairing session.
  preferences.remove("ble_addr");
  preferences.putBool("clear_bonds", true);

  showPairingScreen();
  fadeBacklight(true);
  if (!tapsReady) {
    showError("Microphone not ready");
    return;
  }
  if (!startPairing()) showError(pairingError.c_str());
}

// Bunty's face while it answers: the existing speaking eyes plus a vector
// mouth whose opening follows the audio actually reaching the amplifier.
void serviceVoiceFace() {
  const uint32_t now = millis();
  const bool speaking = buntyVoice.speaking() &&
                        runtimeDisplayState == RuntimeDisplayState::Dashboard;

  if (speaking && !voiceFaceActive) {
    voiceFaceActive = true;
    DisplayGuard guard;
    display.clear(kBackground);
    buntyAnimations.beginSpeaking(now);
    drawRuntimeTopBar(true);
  } else if (!speaking && voiceFaceActive) {
    voiceFaceActive = false;
    // The dashboard owns the screen again as soon as Bunty stops talking.
    renderDashboard();
    return;
  }
  if (!voiceFaceActive) return;

  {
    DisplayGuard guard;
    buntyAnimations.serviceSpeaking(now);
  }
  // The eyes run at their own cadence; the mouth is throttled separately so a
  // 240 px band is not pushed over SPI on every 8 ms loop iteration.
  if (now - lastMouthFrameAt < 50) return;
  lastMouthFrameAt = now;
  DisplayGuard guard;
  buntyAnimations.drawSpeakingMouth(buntyVoice.speechLevel());
}

// Local VAD is always armed, but the cloud socket is turn-scoped: it opens only
// after speech and closes after the reply has physically finished playing.
const char *voiceStateName(BuntyVoice::State state) {
  switch (state) {
    case BuntyVoice::State::Disabled: return "disabled";
    case BuntyVoice::State::Offline: return "offline";
    case BuntyVoice::State::Connecting: return "connecting";
    case BuntyVoice::State::Listening: return "listening";
    case BuntyVoice::State::Uploading: return "uploading";
    case BuntyVoice::State::Thinking: return "thinking";
    case BuntyVoice::State::Speaking: return "speaking";
    case BuntyVoice::State::Draining: return "draining";
  }
  return "?";
}

void serviceVoice() {
  // A periodic line so voice status is visible without catching boot output,
  // which USB CDC drops whenever the monitor attaches late. The uptime makes a
  // reboot loop obvious: the counter restarts instead of climbing.
  const uint32_t heartbeatNow = millis();
  if (lastVoiceHeartbeatAt == 0 || heartbeatNow - lastVoiceHeartbeatAt >= 3000) {
    lastVoiceHeartbeatAt = heartbeatNow;
    Serial.printf(
        "[voice] hb up=%lus runtime=%d greeting=%d wifi=%d started=%d state=%s "
        "vad=%u/%u heap=%u psram=%u\n",
        static_cast<unsigned long>(heartbeatNow / 1000), runtimeStarted ? 1 : 0,
        greetingPlaying ? 1 : 0, WiFi.status() == WL_CONNECTED ? 1 : 0,
        voiceStarted ? 1 : 0, voiceStateName(buntyVoice.state()),
        static_cast<unsigned>(buntyVoice.inputLevel()),
        static_cast<unsigned>(buntyVoice.inputThreshold()),
        static_cast<unsigned>(ESP.getFreeHeap()),
        static_cast<unsigned>(ESP.getFreePsram()));
  }

  if (!runtimeStarted || greetingPlaying) return;

  if (!voiceStarted) {
    if (WiFi.status() != WL_CONNECTED) return;
    if (static_cast<int32_t>(heartbeatNow - nextVoiceStartAt) < 0) return;

    const String gatewayDevice = preferences.getString("iot_device", "");
    const String gatewayToken = preferences.getString("iot_token", "");
    Serial.printf("[voice] activating (credential=%s, microphone=%s)\n",
                  isIotGatewayToken(gatewayToken.c_str()) ? "ready" : "missing",
                  tapInput.available() ? "ready" : "missing");
    voiceStarted = buntyVoice.begin(
        &tapInput, kAudioPort, AMP_BCLK, AMP_LRC, AMP_DIN, AMP_SD_PIN,
        IOT_GATEWAY_HOST, gatewayDevice.c_str(), gatewayToken.c_str());
    if (!voiceStarted) {
      nextVoiceStartAt = millis() + kVoiceStartRetryMs;
      Serial.printf("[voice] activation failed; retrying in %lu ms\n",
                    static_cast<unsigned long>(kVoiceStartRetryMs));
    } else {
      Serial.println("[voice] activation complete");
    }
    return;
  }
  buntyVoice.service();

  // A conversation counts as interaction. Without this the dashboard's idle
  // timer would put Bunty to sleep mid-sentence, and only a physical tap could
  // wake it again.
  if (buntyVoice.engaged()) {
    lastRuntimeInteractionAt = millis();
    if (runtimeDisplayState == RuntimeDisplayState::Sleeping) wakeDisplay();
  }

  serviceVoiceFace();
}

void loop() {
  // Voice activation must not sit behind dashboard discovery/TCP. A sleeping
  // or unreachable Mac can block that local request for several seconds.
  serviceVoice();

  if (pairingStarted) {
    servicePairing();
  } else if (runtimeStarted) {
    serviceRuntime();
  }

  if (showingGreetingEyes && greetingPlaying) {
    const uint32_t now = millis();
    {
      DisplayGuard guard;
      buntyAnimations.serviceSpeaking(now);
    }
    if (now - lastStatusUpdate >= 1000) {
      lastStatusUpdate = now;
      DisplayGuard guard;
      drawRuntimeTopBar(true);
    }
  }
  delay(8);
}
