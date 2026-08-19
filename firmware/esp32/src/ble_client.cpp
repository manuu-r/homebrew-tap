#include <Arduino.h>
#include <ArduinoJson.h>
#include <WiFi.h>
#include <WiFiUdp.h>

#if __has_include("gauge_config.h")
#include "gauge_config.h"
#else
#include "gauge_config.example.h"
#endif

namespace {

// Gauge's --ble mode is a UDP protocol over Wi-Fi, not Bluetooth LE GATT.
constexpr unsigned long kConnectTimeoutMs = 15000;
constexpr unsigned long kRequestTimeoutMs = 2000;
constexpr unsigned long kRetryDelayMs = 5000;
constexpr unsigned long kPollIntervalMs = 30000;
constexpr std::uint16_t kLocalPort = 50000;
constexpr int kMaxResponseSize = 8 * 1024;

WiFiUDP udp;
bool udp_started = false;
unsigned long next_poll_ms = 0;

void discardPacket() {
    while (udp.available() > 0) {
        udp.read();
    }
}

bool connectWifi() {
    if (WiFi.status() == WL_CONNECTED) {
        return true;
    }

    if (udp_started) {
        udp.stop();
        udp_started = false;
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

bool startUdp() {
    if (!udp_started) {
        udp_started = udp.begin(kLocalPort) == 1;
        if (!udp_started) {
            Serial.println("Could not open the UDP socket");
        }
    }
    return udp_started;
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
    if (!startUdp()) {
        return false;
    }

    IPAddress server_ip;
    if (WiFi.hostByName(gauge_config::kGaugeHost, server_ip) != 1) {
        Serial.println("Could not resolve the Gauge host");
        return false;
    }

    while (udp.parsePacket() > 0) {
        discardPacket();
    }

    const String request = String("GET /v1/quota\nAuthorization: Bearer ") +
                           gauge_config::kGaugeToken + "\n";
    if (udp.beginPacket(server_ip, gauge_config::kUdpPort) != 1) {
        Serial.println("Could not start the UDP request");
        return false;
    }
    udp.print(request);
    if (udp.endPacket() != 1) {
        Serial.println("Could not send the UDP request");
        return false;
    }

    const unsigned long started = millis();
    while (millis() - started < kRequestTimeoutMs) {
        const int packet_size = udp.parsePacket();
        if (packet_size <= 0) {
            delay(10);
            continue;
        }
        if (udp.remoteIP() != server_ip || udp.remotePort() != gauge_config::kUdpPort) {
            discardPacket();
            continue;
        }
        if (packet_size > kMaxResponseSize) {
            discardPacket();
            Serial.println("UDP response is too large");
            return false;
        }

        String response;
        if (!response.reserve(packet_size)) {
            discardPacket();
            Serial.println("Not enough memory for the UDP response");
            return false;
        }
        while (udp.available() > 0) {
            response += static_cast<char>(udp.read());
        }
        return printQuota(response);
    }

    Serial.println("UDP request timed out");
    return false;
}

bool pollDue(unsigned long now) {
    return static_cast<long>(now - next_poll_ms) >= 0;
}

}  // namespace

void setup() {
    Serial.begin(115200);
    delay(100);
    Serial.println("Gauge BLE-style UDP client starting");
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
