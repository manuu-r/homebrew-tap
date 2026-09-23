#pragma once

#include <ArduinoJson.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include "config.h"
#include "dashboard_types.h"

// Parses a Gauge `dev.gauge.dashboard` schema 1 snapshot. Header-only so the
// host tests run it against docs/fixtures/dashboard-v1.json.
namespace dashboardparse {

template <size_t N>
inline void copyText(char (&dest)[N], const char *source) {
  if (!source) source = "";
  size_t out = 0;
  for (size_t i = 0; source[i] && out + 1 < N; i++) {
    const unsigned char c = (unsigned char)source[i];
    dest[out++] = (c >= 32 && c <= 126) ? (char)c : '?';  // GFX fonts are ASCII
  }
  dest[out] = '\0';
}

inline float limitRemaining(JsonObjectConst limit) {
  JsonVariantConst remaining = limit["remaining_percent"];
  const float value = remaining.is<float>() ? remaining.as<float>()
                                            : 100.0f - (limit["used_percent"] | 100.0f);
  return value < 0.0f ? 0.0f : value > 100.0f ? 100.0f : value;
}

// Adds one quota group holding the provider's limits whose "Spark " prefix
// matches `spark`, and returns false when there is nothing to show.
inline bool addGroup(DashboardData &out, JsonObjectConst provider, bool spark) {
  if (out.providerCount >= DASH_MAX_PROVIDERS) return false;
  ProviderEntry entry;
  const char *name = provider["name"] | "Agent";
  if (spark) {
    char label[sizeof(entry.name)];
    snprintf(label, sizeof(label), "%s Spark", name);
    copyText(entry.name, label);
  } else {
    copyText(entry.name, name);
  }

  bool any = false;
  for (JsonObjectConst limit : provider["limits"].as<JsonArrayConst>()) {
    const char *label = limit["label"] | "";
    if ((strncmp(label, "Spark ", 6) == 0) != spark) continue;
    any = true;
    const float remaining = limitRemaining(limit);
    if (!entry.haveRemaining || remaining < entry.remaining) entry.remaining = remaining;
    entry.haveRemaining = true;
    if (entry.limitCount >= DASH_MAX_LIMITS) continue;
    LimitEntry &slot = entry.limits[entry.limitCount++];
    copyText(slot.label, spark ? label + 6 : label);
    slot.remaining = remaining;
    slot.resetsAt = limit["resets_at"] | (int64_t)0;
  }
  // The regular group uses Gauge's own headline, which already excludes Spark.
  JsonVariantConst headline = provider["remaining_percent"];
  if (!spark && headline.is<float>()) {
    entry.remaining = headline.as<float>();
    entry.haveRemaining = true;
  }
  // Every provider gets a page, even with no data; an empty Spark group does not.
  if (spark && !any) return false;
  out.providers[out.providerCount++] = entry;
  return true;
}

inline bool parse(JsonVariantConst root, DashboardData &out) {
  if (strcmp(root["protocol"] | "", DASHBOARD_PROTOCOL) != 0 ||
      (root["schema_version"] | 0) != 1) {
    return false;
  }

  DashboardData parsed;
  parsed.generatedAt = root["generated_at"] | (int64_t)0;
  parsed.refreshSeconds = root["refresh_seconds"] | 120;

  JsonObjectConst quota = root["quota"];
  for (JsonObjectConst provider : quota["providers"].as<JsonArrayConst>()) {
    addGroup(parsed, provider, false);
    addGroup(parsed, provider, true);
  }
  copyText(parsed.quotaError, quota["errors"][0] | "");

  JsonObjectConst calendar = root["calendar"];
  parsed.calendarEnabled = calendar["enabled"] | false;
  copyText(parsed.calendarError, calendar["error"] | "");
  for (JsonObjectConst event : calendar["events"].as<JsonArrayConst>()) {
    if (parsed.eventCount >= DASH_MAX_EVENTS) break;
    CalendarEntry &entry = parsed.events[parsed.eventCount++];
    copyText(entry.title, event["title"] | "Untitled event");
    entry.startsAt = event["starts_at"] | (int64_t)0;
    entry.endsAt = event["ends_at"] | (int64_t)0;
    entry.allDay = event["all_day"] | false;
  }

  JsonArrayConst todos = root["todos"];
  parsed.todoTotal = todos.size();
  for (JsonObjectConst todo : todos) {
    if (parsed.todoCount >= DASH_MAX_TODOS) break;
    TodoEntry &entry = parsed.todos[parsed.todoCount++];
    copyText(entry.title, todo["title"] | "Untitled task");
    entry.completed = todo["completed"] | false;
  }

  copyText(parsed.settingsError, root["errors"]["settings"] | "");
  out = parsed;
  return true;
}

}  // namespace dashboardparse
