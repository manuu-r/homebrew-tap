#include "pairing.h"

#include <ArduinoJson.h>
#include <BLEDevice.h>
#include <BLESecurity.h>
#include <BLEServer.h>
#include <Preferences.h>
#include <WiFi.h>
#include <atomic>
#include <cctype>
#include <esp_gap_ble_api.h>

#include "config.h"

namespace pairing {
namespace {

Preferences *prefs_ = nullptr;
String deviceId_;
ConfirmFn confirm_ = nullptr;
BLECharacteristic *status_ = nullptr;
BLESecurity security_;

// Written on the Bluetooth task, consumed in loop().
std::atomic<bool> authenticated_{false};
std::atomic<bool> received_{false};
char buffer_[MAX_COMMISSION_BYTES + 1];
size_t length_ = 0;
uint8_t expected_ = 0;
uint8_t next_ = 0;

void clearBonds();

uint32_t joinStartedAt_ = 0;
uint32_t connectedAt_ = 0;

void setStatus(const char *value) {
  status_->setValue(value);
  Serial.printf("[pairing] %s\n", value);
}

bool allChars(const char *s, size_t minLength, size_t maxLength, int (*ok)(int), const char *extra) {
  const size_t length = strlen(s);
  if (length < minLength || length > maxLength) return false;
  for (size_t i = 0; i < length; i++) {
    if (!ok((unsigned char)s[i]) && !strchr(extra, s[i])) return false;
  }
  return true;
}

// Validates every protocol field before anything is persisted.
bool save(JsonDocument &doc) {
  JsonObjectConst wifi = doc["wifi"];
  JsonObjectConst accessory = doc["accessory"];
  const char *ssid = wifi["ssid"] | "";
  const char *password = wifi["password"] | "";
  const char *serverId = accessory["server_id"] | "";
  const char *token = accessory["bearer_token"] | "";
  const uint16_t port = accessory["server_port"] | 0;
  const size_t passwordLength = strlen(password);

  if (strcmp(doc["protocol"] | "", COMMISSION_PROTOCOL) != 0 || (doc["version"] | 0) != 1 ||
      strcmp(accessory["protocol"] | "", ACCESSORY_PROTOCOL) != 0 ||
      (accessory["version"] | 0) != 1 || deviceId_ != (accessory["device_id"] | "") ||
      strcmp(accessory["service_type"] | "", SERVICE_TYPE) != 0 ||
      strcmp(accessory["dashboard_path"] | "", DASHBOARD_PATH) != 0 || port == 0 ||
      strlen(ssid) == 0 || strlen(ssid) > 32 ||
      (passwordLength >= 64 && !allChars(password, 64, 64, isxdigit, "")) ||
      !allChars(serverId, 1, 64, isalnum, "-_.") || !allChars(token, 64, 64, isxdigit, "")) {
    return false;
  }
  prefs_->putString("ssid", ssid);
  prefs_->putString("pass", password);
  prefs_->putString("server", serverId);
  prefs_->putString("token", token);
  prefs_->putUShort("port", port);
  return true;
}

String identity(const String &name) {
  JsonDocument doc;
  doc["protocol"] = PAIRING_PROTOCOL;
  doc["version"] = 1;
  doc["device_id"] = deviceId_;
  doc["name"] = name;
  doc["kind"] = "display";
  doc["firmware_version"] = FIRMWARE_VERSION;
  JsonArray capabilities = doc["capabilities"].to<JsonArray>();
  capabilities.add("dashboard.pull");
  capabilities.add("dashboard.cache");
  capabilities.add("display");
  String json;
  serializeJson(doc, json);
  return json;
}

// Reassembles the 20-byte frames: marker, sequence, total, then payload.
class ConfigWrites : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic *c) override {
    const uint8_t *frame = c->getData();
    const size_t size = c->getLength();
    if (received_) return;
    if (!authenticated_ || size < 3 || frame[0] != FRAME_MAGIC || frame[2] == 0) {
      return reject("error:bad setup frame");
    }
    if (frame[1] == 0) {
      length_ = 0;
      next_ = 0;
      expected_ = frame[2];
      setStatus("receiving");
    }
    if (frame[1] != next_ || frame[2] != expected_ || length_ + size - 3 > MAX_COMMISSION_BYTES) {
      return reject("error:frame order");
    }
    memcpy(buffer_ + length_, frame + 3, size - 3);
    length_ += size - 3;
    if (++next_ == expected_) received_ = true;
  }

  void reject(const char *reason) {
    length_ = expected_ = next_ = 0;
    setStatus(reason);
  }
};

class Security : public BLESecurityCallbacks {
  uint32_t onPassKeyRequest() override { return 0; }
  void onPassKeyNotify(uint32_t) override {}
  bool onSecurityRequest() override { return true; }
  bool onConfirmPIN(uint32_t number) override { return confirm_(number); }
  void onAuthenticationComplete(esp_ble_auth_cmpl_t result) override {
    authenticated_ = result.success;
    if (!result.success) Serial.printf("[pairing] authentication failed 0x%02x\n", result.fail_reason);
  }
};

class Connections : public BLEServerCallbacks {
  // A new connection starts from numeric comparison and an empty buffer.
  void onDisconnect(BLEServer *) override {
    authenticated_ = false;
    if (connectedAt_ == 0) {
      length_ = expected_ = next_ = 0;
      clearBonds();
      BLEDevice::startAdvertising();
    }
  }
};

// A link key from an earlier session would let macOS skip numeric comparison,
// or stall it if this device has already deleted its copy. Drop it so the Mac
// runs numeric comparison again.
void clearBonds() {
  int count = esp_ble_get_bond_device_num();
  if (count <= 0) return;
  esp_ble_bond_dev_t *peers = (esp_ble_bond_dev_t *)malloc(sizeof(esp_ble_bond_dev_t) * count);
  if (peers && esp_ble_get_bond_device_list(&count, peers) == ESP_OK) {
    for (int i = 0; i < count; i++) esp_ble_remove_bond_device(peers[i].bd_addr);
  }
  free(peers);
}

ConfigWrites configWrites;
Security securityCallbacks;
Connections connections;

}  // namespace

void start(Preferences &prefs, const String &deviceId, const String &name, ConfirmFn confirm) {
  prefs_ = &prefs;
  deviceId_ = deviceId;
  confirm_ = confirm;

  BLEDevice::init(name.c_str());
  BLEDevice::setEncryptionLevel(ESP_BLE_SEC_ENCRYPT_MITM);
  BLEDevice::setSecurityCallbacks(&securityCallbacks);
  // Secure Connections numeric comparison, with bonding. macOS will not
  // finish the encrypted read unless the accessory sets the Bonding flag.
  security_.setAuthenticationMode(ESP_LE_AUTH_REQ_SC_MITM_BOND);
  security_.setCapability(ESP_IO_CAP_IO);
  security_.setKeySize(16);
  security_.setInitEncryptionKey(ESP_BLE_ENC_KEY_MASK | ESP_BLE_ID_KEY_MASK);
  security_.setRespEncryptionKey(ESP_BLE_ENC_KEY_MASK | ESP_BLE_ID_KEY_MASK);
  clearBonds();

  BLEServer *server = BLEDevice::createServer();
  server->setCallbacks(&connections);
  BLEService *service = server->createService(PAIRING_SERVICE_UUID);

  BLECharacteristic *id = service->createCharacteristic(IDENTITY_UUID, BLECharacteristic::PROPERTY_READ);
  id->setAccessPermissions(ESP_GATT_PERM_READ_ENC_MITM);
  id->setValue(identity(name).c_str());

  BLECharacteristic *config = service->createCharacteristic(CONFIG_UUID, BLECharacteristic::PROPERTY_WRITE);
  config->setAccessPermissions(ESP_GATT_PERM_WRITE_ENC_MITM);
  config->setCallbacks(&configWrites);

  status_ = service->createCharacteristic(STATUS_UUID, BLECharacteristic::PROPERTY_READ);
  status_->setAccessPermissions(ESP_GATT_PERM_READ_ENC_MITM);
  status_->setValue("ready");
  service->start();

  BLEAdvertising *advertising = BLEDevice::getAdvertising();
  advertising->addServiceUUID(PAIRING_SERVICE_UUID);
  advertising->setScanResponse(true);
  BLEDevice::startAdvertising();
  Serial.printf("[pairing] advertising as %s (%s)\n", name.c_str(), deviceId.c_str());
}

bool service() {
  if (received_ && joinStartedAt_ == 0) {
    JsonDocument doc;
    const bool ok = !deserializeJson(doc, buffer_, length_) && save(doc);
    length_ = expected_ = next_ = 0;
    received_ = false;
    if (!ok) {
      setStatus("error:invalid setup");
      return false;
    }
    setStatus("joining");
    joinStartedAt_ = millis();
    WiFi.mode(WIFI_STA);
    WiFi.begin(prefs_->getString("ssid").c_str(), prefs_->getString("pass").c_str());
  }

  if (joinStartedAt_ != 0 && connectedAt_ == 0) {
    if (WiFi.status() == WL_CONNECTED) {
      prefs_->putBool("paired", true);
      setStatus("connected");
      connectedAt_ = millis();
    } else if (millis() - joinStartedAt_ > WIFI_JOIN_TIMEOUT_MS) {
      setStatus("error:wifi connection failed");
      WiFi.disconnect();
      joinStartedAt_ = 0;
    }
  }
  return connectedAt_ != 0 && millis() - connectedAt_ > PAIRED_RESTART_DELAY_MS;
}

}  // namespace pairing
