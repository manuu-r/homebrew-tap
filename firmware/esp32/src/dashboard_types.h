#pragma once

#include <stdint.h>

#include "quota_types.h"

static const uint8_t DASH_MAX_EVENTS = 3;
static const uint8_t DASH_MAX_TICKERS = 4;
static const uint8_t DASH_MAX_TODOS = 5;

struct CalendarEntry {
  char title[72] = {};
  int64_t startsAt = 0;
  int64_t endsAt = 0;
  bool allDay = false;
};

struct TickerEntry {
  char symbol[16] = {};
  char label[24] = {};
  float price = 0.0f;
  float changePct = 0.0f;
  bool haveChange = false;
};

struct TodoEntry {
  char title[72] = {};
  bool completed = false;
};

struct DashboardData {
  QuotaReading quota;
  uint32_t generatedAt = 0;
  uint32_t refreshSeconds = 120;

  bool calendarEnabled = false;
  CalendarEntry events[DASH_MAX_EVENTS];
  uint8_t eventCount = 0;
  uint16_t eventTotal = 0;
  char calendarError[72] = {};

  bool tickersEnabled = false;
  TickerEntry tickers[DASH_MAX_TICKERS];
  uint8_t tickerCount = 0;
  uint16_t tickerTotal = 0;
  char tickerError[72] = {};

  TodoEntry todos[DASH_MAX_TODOS];
  uint8_t todoCount = 0;
  uint16_t todoTotal = 0;
};
