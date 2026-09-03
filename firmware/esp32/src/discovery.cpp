#include "discovery.h"

#include <WiFi.h>
#include <esp_task_wdt.h>

#include "config.h"

// Reads the HTTP status line and returns the code, or -1 on timeout.
static int readStatusCode(WiFiClient &c) {
  const uint32_t deadline = millis() + HTTP_READ_TIMEOUT_MS;
  char line[64];
  size_t n = 0;

  while (millis() < deadline) {
    while (c.available()) {
      const int ch = c.read();
      if (ch < 0) break;
      if (ch == '\n') {
        line[n] = '\0';
        // "HTTP/1.1 200 OK"
        const char *sp = strchr(line, ' ');
        return sp ? atoi(sp + 1) : -1;
      }
      if (ch != '\r' && n < sizeof(line) - 1) line[n++] = (char)ch;
    }
    if (!c.connected() && !c.available()) break;
    delay(1);
  }
  return -1;
}

bool probeHealth(const IPAddress &ip) {
  WiFiClient client;
  if (!client.connect(ip, GAUGE_PORT, (int32_t)TCP_CONNECT_TIMEOUT_MS)) {
    client.stop();
    return false;
  }

  client.printf("GET %s HTTP/1.1\r\nHost: %s:%u\r\n"
                "User-Agent: gauge-display/1.0\r\nConnection: close\r\n\r\n",
                HEALTH_PATH, ip.toString().c_str(), (unsigned)GAUGE_PORT);

  const int code = readStatusCode(client);
  client.stop();

  // 2xx only. 401/403 mean the service is there but gated behind auth, which
  // the spec explicitly excludes.
  return code >= 200 && code < 300;
}

bool discoverHealthHost(IPAddress &found, SweepProgressFn onProgress) {
  const IPAddress self = WiFi.localIP();
  const IPAddress mask = WiFi.subnetMask();
  if (self[0] == 0) return false;

  // Scan the local /24 regardless of how wide the real mask is; sweeping a /16
  // would take hours and the service is realistically on our own segment.
  if (mask[0] != 255 || mask[1] != 255 || mask[2] != 255) {
    Serial.printf("[scan] mask %s is wider than /24, scanning local /24 only\n",
                  mask.toString().c_str());
  }

  Serial.printf("[scan] sweeping %u.%u.%u.1-254 port %u for %s\n", self[0], self[1], self[2],
                (unsigned)GAUGE_PORT, HEALTH_PATH);

  for (int host = 1; host <= 254; host++) {
    esp_task_wdt_reset();

    if (host == self[3]) continue;  // that's us
    const IPAddress ip(self[0], self[1], self[2], (uint8_t)host);

    if (onProgress && (host % 8 == 0 || host == 1)) onProgress(ip, host * 100 / 254);

    if (probeHealth(ip)) {
      Serial.printf("[scan] hit: %s\n", ip.toString().c_str());
      found = ip;
      if (onProgress) onProgress(ip, 100);
      return true;
    }
  }

  Serial.println("[scan] no host answered /health");
  return false;
}
