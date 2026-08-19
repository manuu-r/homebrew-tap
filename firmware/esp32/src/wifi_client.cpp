#include <Arduino.h>
#include <ArduinoJson.h>
#include <HTTPClient.h>
#include <WiFi.h>

#if __has_include("gauge_config.h")
#include "gauge_config.h"
#else
#include "gauge_config.example.h"
#endif

namespace {

constexpr unsigned long kConnectTimeoutMs = 15000;
constexpr unsigned long kRequestTimeoutMs = 5000;
constexpr unsigned long kRetryDelayMs = 5000;
constexpr unsigned long kPollIntervalMs = 30000;

unsigned long next_poll_ms = 0;

bool connectWifi() {
    if (WiFi.status() == WL_CONNECTED) {
        return true;
    }

    WiFi.mode(WIFI_STA);
    WiFi.begin(gauge_config::kWifiSsid, gauge_config::kWifiPassword);

    Serial.print("Wi-Fi connecting");
    const unsigned long started = millis();
    while (WiFi.status() != WL_CONNECTED && millis() - started < kConnectTimeoutMs) {
        delay(250);
        Serial.print('.');
    }
    Serial.println();

    if (WiFi.status() != WL_CONNECTED) {
        Serial.println("Wi-Fi connection timed out");
        WiFi.disconnect();
        return false;
    }

    Serial.print("Wi-Fi connected: ");
    Serial.println(WiFi.localIP());
    return true;
}

bool printQuota(const String& body) {
    JsonDocument document;
    const DeserializationError error = deserializeJson(document, body);
    if (error) {
        Serial.printf("JSON parse failed: %s\n", error.c_str());
        return false;
    }

    const JsonArrayConst providers = document["providers"].as<JsonArrayConst>();
    if (providers.isNull()) {
        Serial.println("Response does not contain a providers array");
        return false;
    }

    Serial.printf(
        "generated_at: %llu\n",
        document["generated_at"].as<unsigned long long>()
    );
    for (const JsonObjectConst provider : providers) {
        const char* name = provider["name"] | "(unknown)";
        const int remaining = provider["remaining_percent"] | -1;
        Serial.printf("%s: %d%%\n", name, remaining);
    }

    for (const JsonVariantConst error_message :
         document["errors"].as<JsonArrayConst>()) {
        Serial.print("warning: ");
        Serial.println(error_message.as<const char*>());
    }
    return true;
}

bool fetchQuota() {
    WiFiClient client;
    HTTPClient http;
    const String url = String("http://") + gauge_config::kGaugeHost + ':' +
                       gauge_config::kHttpPort + "/v1/quota";

    http.setConnectTimeout(kRequestTimeoutMs);
    http.setTimeout(kRequestTimeoutMs);
    if (!http.begin(client, url)) {
        Serial.println("Could not start the HTTP request");
        return false;
    }

    http.addHeader(
        "Authorization",
        String("Bearer ") + gauge_config::kGaugeToken
    );
    const int status = http.GET();
    bool success = false;

    if (status == HTTP_CODE_OK) {
        success = printQuota(http.getString());
    } else if (status > 0) {
        Serial.printf("HTTP request failed with status %d\n", status);
        Serial.println(http.getString());
    } else {
        Serial.printf("HTTP request failed: %s\n", HTTPClient::errorToString(status).c_str());
    }

    http.end();
    return success;
}

bool pollDue(unsigned long now) {
    return static_cast<long>(now - next_poll_ms) >= 0;
}

}  // namespace

void setup() {
    Serial.begin(115200);
    delay(100);
    Serial.println("Gauge Wi-Fi HTTP client starting");
}

void loop() {
    if (!connectWifi()) {
        delay(kRetryDelayMs);
        return;
    }

    if (pollDue(millis())) {
        fetchQuota();
        next_poll_ms = millis() + kPollIntervalMs;
    }
    delay(25);
}
