#include "gauge_client.h"

#include <ESPmDNS.h>
#include <HTTPClient.h>
#include <Preferences.h>
#include <WiFi.h>

#include "config.h"

namespace gauge {
namespace {

String serverId_;
String authorization_;
uint16_t port_ = 0;
bool mdnsStarted_ = false;

// Picks the `_gauge._tcp` service whose TXT `id` is the commissioned server ID.
// Multicast browsing is unreliable on some networks, so fall back to the Mac's
// Bonjour hostname, which Gauge derives as gauge-<first 12 of server_id>.local.
bool locate(IPAddress &address, uint16_t &port) {
  const int count = MDNS.queryService("gauge", "tcp");
  for (int i = 0; i < count; i++) {
    if (MDNS.txt(i, "id") == serverId_ && MDNS.IP(i) != IPAddress() && MDNS.port(i) != 0) {
      address = MDNS.IP(i);
      port = MDNS.port(i);
      return true;
    }
  }
  address = MDNS.queryHost("gauge-" + serverId_.substring(0, 12), MDNS_QUERY_MS);
  port = port_;
  return address != IPAddress();
}

int request(const char *method, const char *path, String *body) {
  if (WiFi.status() != WL_CONNECTED) return -1;
  if (!mdnsStarted_) mdnsStarted_ = MDNS.begin(WiFi.getHostname());
  IPAddress address;
  uint16_t port = 0;
  if (!mdnsStarted_ || !locate(address, port)) {
    Serial.println("[gauge] not found on this network");
    return -1;
  }

  WiFiClient client;
  HTTPClient http;
  http.setConnectTimeout(HTTP_TIMEOUT_MS);
  http.setTimeout(HTTP_TIMEOUT_MS);
  if (!http.begin(client, address.toString(), port, path)) return -1;
  http.addHeader("Accept", "application/json");
  http.addHeader("Authorization", authorization_);  // never in the URL
  const int status = http.sendRequest(method);
  if (body && status == HTTP_CODE_OK && http.getSize() > 0 &&
      http.getSize() <= (int)MAX_DASHBOARD_BYTES) {
    *body = http.getString();
  }
  http.end();
  Serial.printf("[gauge] %s %s:%u%s -> %d\n", method, address.toString().c_str(), port, path,
                status);
  return status;
}

}  // namespace

void begin(Preferences &prefs) {
  serverId_ = prefs.getString("server");
  authorization_ = "Bearer " + prefs.getString("token");
  port_ = prefs.getUShort("port");
}

Fetch dashboard(String &body) {
  body = "";
  const int status = request("GET", DASHBOARD_PATH, &body);
  if (status == HTTP_CODE_UNAUTHORIZED) return Fetch::Revoked;
  return status == HTTP_CODE_OK && body.length() > 0 ? Fetch::Ok : Fetch::Failed;
}

void revoke() { request("DELETE", ACCESSORY_PATH, nullptr); }

}  // namespace gauge
