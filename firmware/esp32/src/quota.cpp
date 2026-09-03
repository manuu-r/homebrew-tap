#include "quota.h"

#include <ArduinoJson.h>
#include <HTTPClient.h>
#include <WiFiClient.h>
#include <esp_task_wdt.h>

#include "config.h"
#include "dashboard_parse.h"

// Hard ceiling on the response we will buffer. ArduinoJson v7 grows its pool on
// the heap, so without a cap a chatty endpoint could exhaust RAM.
static const size_t MAX_JSON_BYTES = 16384;

// Reads at most MAX_JSON_BYTES from the stream. Returns false if the body is
// larger than the cap; better to fail loudly than to half-parse.
static bool readBounded(WiFiClient *stream, int contentLength, String &body) {
  if (contentLength > (int)MAX_JSON_BYTES) return false;

  body = "";
  if (contentLength > 0) body.reserve((size_t)contentLength + 1);

  const uint32_t deadline = millis() + DASHBOARD_HTTP_TIMEOUT_MS;
  // One spare byte so each chunk can be NUL-terminated and appended with the
  // plain String::operator+=; the length-taking concat() overload is not
  // reliably public across cores. JSON never contains embedded NULs.
  char buf[257];
  while (millis() < deadline) {
    esp_task_wdt_reset();
    const size_t avail = stream->available();
    if (avail) {
      const size_t want = avail > (sizeof(buf) - 1) ? (sizeof(buf) - 1) : avail;
      const int n = stream->readBytes(buf, want);
      if (n <= 0) break;
      if (body.length() + (size_t)n > MAX_JSON_BYTES) return false;
      buf[n] = '\0';
      body += buf;
      if (contentLength > 0 && body.length() >= (size_t)contentLength) break;
    } else if (!stream->connected()) {
      break;
    } else {
      delay(2);
    }
  }
  return body.length() > 0;
}

bool dashboardFetch(const IPAddress &host, uint16_t port, DashboardData &out, String &err) {
  err = "";
  WiFiClient client;
  HTTPClient http;

  char url[96];
  snprintf(url, sizeof(url), "http://%s:%u%s", host.toString().c_str(), (unsigned)port,
           DASHBOARD_PATH);

  http.setConnectTimeout(2000);
  http.setTimeout(DASHBOARD_HTTP_TIMEOUT_MS);
  http.setReuse(false);
  if (!http.begin(client, url)) {
    err = "begin failed";
    return false;
  }

  http.addHeader("Accept", "application/json");
  // gauge accepts Bearer, X-Gauge-Token, or ?token=. Bearer keeps the token out
  // of the request line, so it stays out of any access log that records paths.
  if (strlen(gauge_config::kGaugeToken) > 0) {
    http.addHeader("Authorization", String("Bearer ") + gauge_config::kGaugeToken);
  }

  const int code = http.GET();
  if (code != HTTP_CODE_OK) {
    err = (code == HTTP_CODE_UNAUTHORIZED) ? "401 - check kGaugeToken" : String("HTTP ") + code;
    http.end();
    return false;
  }

  String body;
  const bool got = readBounded(http.getStreamPtr(), http.getSize(), body);
  http.end();
  if (!got) {
    err = "empty or oversized body";
    return false;
  }

  JsonDocument doc;
  const DeserializationError jerr = deserializeJson(doc, body);
  if (jerr) {
    err = String("json: ") + jerr.c_str();
    return false;
  }

  DashboardData dashboard;
  if (!dashboardparse::parse(doc.as<JsonVariantConst>(), dashboard)) {
    err = "unsupported dashboard schema";
    return false;
  }

  out = dashboard;
  return true;
}
