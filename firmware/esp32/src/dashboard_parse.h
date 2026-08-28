#pragma once

#include <ArduinoJson.h>
#include <stddef.h>
#include <string.h>

#include "dashboard_types.h"
#include "gauge_parse.h"

namespace dashboardparse {

template <size_t N>
inline void copyDisplayText(char (&dest)[N], const char *source) {
  if (!source) source = "";
  size_t out = 0;
  for (size_t i = 0; source[i] && out + 1 < N; i++) {
    const unsigned char c = (unsigned char)source[i];
    dest[out++] = (c >= 32 && c <= 126) ? (char)c : '?';
  }
  dest[out] = '\0';
}

inline bool parse(JsonVariantConst root, DashboardData &out) {
  if ((root["schema_version"] | 0) != 1) return false;

  DashboardData parsed;
  parsed.generatedAt = root["generated_at"] | 0;
  parsed.refreshSeconds = root["refresh_seconds"] | 120;

  JsonVariantConst quota = root["quota"];
  if (!quota.isNull()) parseGaugeQuota(quota, parsed.quota);

  JsonObjectConst calendar = root["calendar"].as<JsonObjectConst>();
  if (!calendar.isNull()) {
    parsed.calendarEnabled = calendar["enabled"] | false;
    copyDisplayText(parsed.calendarError, calendar["error"] | "");
    JsonArrayConst events = calendar["events"].as<JsonArrayConst>();
    parsed.eventTotal = events.size();
    for (JsonObjectConst event : events) {
      if (parsed.eventCount >= DASH_MAX_EVENTS) break;
      CalendarEntry &entry = parsed.events[parsed.eventCount++];
      copyDisplayText(entry.title, event["title"] | "Untitled event");
      entry.startsAt = event["starts_at"] | 0;
      entry.endsAt = event["ends_at"] | 0;
      entry.allDay = event["all_day"] | false;
    }
  }

  JsonObjectConst tickers = root["tickers"].as<JsonObjectConst>();
  if (!tickers.isNull()) {
    parsed.tickersEnabled = tickers["enabled"] | false;
    JsonArrayConst errors = tickers["errors"].as<JsonArrayConst>();
    if (!errors.isNull() && errors.size() > 0) {
      copyDisplayText(parsed.tickerError, errors[0] | "Ticker unavailable");
    }
    JsonArrayConst quotes = tickers["quotes"].as<JsonArrayConst>();
    parsed.tickerTotal = quotes.size();
    for (JsonObjectConst quote : quotes) {
      if (parsed.tickerCount >= DASH_MAX_TICKERS) break;
      TickerEntry &entry = parsed.tickers[parsed.tickerCount++];
      copyDisplayText(entry.symbol, quote["symbol"] | "");
      copyDisplayText(entry.label, quote["label"] | entry.symbol);
      entry.price = quote["price"] | 0.0f;
      JsonVariantConst change = quote["change_percent"];
      entry.haveChange = !change.isNull();
      entry.changePct = entry.haveChange ? change.as<float>() : 0.0f;
    }
  }

  JsonArrayConst todos = root["todos"].as<JsonArrayConst>();
  parsed.todoTotal = todos.size();
  for (JsonObjectConst todo : todos) {
    if (parsed.todoCount >= DASH_MAX_TODOS) break;
    TodoEntry &entry = parsed.todos[parsed.todoCount++];
    copyDisplayText(entry.title, todo["title"] | "Untitled task");
    entry.completed = todo["completed"] | false;
  }

  out = parsed;
  return true;
}

}  // namespace dashboardparse
