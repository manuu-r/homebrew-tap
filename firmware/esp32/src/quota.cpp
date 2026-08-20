#include "quota.h"

#include <ArduinoJson.h>
#include <HTTPClient.h>
#include <WiFiClient.h>

#include "config.h"
#include "gauge_parse.h"
#include "quota_parse.h"

// Hard ceiling on the response we will buffer. ArduinoJson v7 grows its pool on
// the heap, so without a cap a chatty endpoint could exhaust RAM.
static const size_t MAX_JSON_BYTES = 8192;

// Index into QUOTA_PATHS that last worked; tried first next time.
static int s_goodPath = -1;

void quotaResetPathCache() { s_goodPath = -1; }

// Reads at most MAX_JSON_BYTES from the stream. Returns false if the body is
// larger than the cap; better to fail loudly than to half-parse.
static bool readBounded(WiFiClient *stream, int contentLength, String &body) {
  if (contentLength > (int)MAX_JSON_BYTES) return false;

  body = "";
  if (contentLength > 0) body.reserve((size_t)contentLength + 1);

  const uint32_t deadline = millis() + 4000;
  // One spare byte so each chunk can be NUL-terminated and appended with the
  // plain String::operator+=; the length-taking concat() overload is not
  // reliably public across cores. JSON never contains embedded NULs.
  char buf[257];
  while (millis() < deadline) {
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

static bool tryPath(const IPAddress &host, uint16_t port, const char *path, QuotaReading &out,
                    String &err) {
  WiFiClient client;
  HTTPClient http;

  char url[96];
  snprintf(url, sizeof(url), "http://%s:%u%s", host.toString().c_str(), (unsigned)port, path);

  http.setConnectTimeout(2000);
  http.setTimeout(4000);
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

  QuotaReading r;
  // The real gauge schema first. If it recognises the payload its verdict is
  // final; the schema-agnostic parser only covers a non-gauge quota server.
  if (!parseGaugeQuota(doc.as<JsonVariantConst>(), r)) {
    parseQuotaJson(doc.as<JsonVariantConst>(), r);
  }
  if (!r.any()) {
    err = "no claude/codex quota in payload";
    return false;
  }

  out = r;
  return true;
}

bool quotaFetch(const IPAddress &host, uint16_t port, QuotaReading &out, String &err) {
  err = "";

  if (s_goodPath >= 0 && s_goodPath < (int)QUOTA_PATH_COUNT) {
    if (tryPath(host, port, QUOTA_PATHS[s_goodPath], out, err)) return true;
    Serial.printf("[quota] cached path %s failed (%s), re-probing\n", QUOTA_PATHS[s_goodPath],
                  err.c_str());
    s_goodPath = -1;
  }

  for (size_t i = 0; i < QUOTA_PATH_COUNT; i++) {
    String e;
    if (tryPath(host, port, QUOTA_PATHS[i], out, e)) {
      Serial.printf("[quota] endpoint: %s\n", QUOTA_PATHS[i]);
      s_goodPath = (int)i;
      return true;
    }
    Serial.printf("[quota] %s -> %s\n", QUOTA_PATHS[i], e.c_str());
    // A 401 is the most actionable failure, so let it win the reported error.
    if (err.isEmpty() || e.startsWith("401")) err = e;
  }
  return false;
}
