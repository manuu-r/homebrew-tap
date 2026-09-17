#pragma once

#include <stdint.h>

static const uint8_t DASH_MAX_PROVIDERS = 4;
static const uint8_t DASH_MAX_LIMITS = 3;
static const uint8_t DASH_MAX_EVENTS = 3;
static const uint8_t DASH_MAX_TODOS = 5;

struct LimitEntry {
  char label[24] = {};
  float remaining = 0.0f;  // percent, 0..100
  int64_t resetsAt = 0;    // Unix seconds, 0 when unknown
};

// One quota group, as the Gauge popover shows it. Codex's "Spark " windows are
// split into their own "Codex Spark" group.
struct ProviderEntry {
  char name[20] = {};
  bool haveRemaining = false;
  float remaining = 0.0f;  // tightest window, percent
  LimitEntry limits[DASH_MAX_LIMITS];
  uint8_t limitCount = 0;
};

struct CalendarEntry {
  char title[72] = {};
  int64_t startsAt = 0;
  int64_t endsAt = 0;
  bool allDay = false;
};

struct TodoEntry {
  char title[72] = {};
  bool completed = false;
};

struct DashboardData {
  int64_t generatedAt = 0;
  uint32_t refreshSeconds = 120;

  ProviderEntry providers[DASH_MAX_PROVIDERS];
  uint8_t providerCount = 0;
  char quotaError[72] = {};

  bool calendarEnabled = false;
  CalendarEntry events[DASH_MAX_EVENTS];
  uint8_t eventCount = 0;
  char calendarError[72] = {};

  TodoEntry todos[DASH_MAX_TODOS];
  uint8_t todoCount = 0;
  uint16_t todoTotal = 0;

  char settingsError[72] = {};
};
