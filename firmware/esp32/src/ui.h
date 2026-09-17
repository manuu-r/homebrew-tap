#pragma once
#include <Adafruit_GFX.h>

#include "dashboard_types.h"

// Every screen is a full repaint; pages change at most every 8 seconds.
// `now` is Unix seconds (the snapshot's generated_at plus time since fetch),
// used only for relative times such as "resets 2h 5m".

// Centred title, detail line, and optional progress bar (`pct` < 0 hides it).
void uiStatus(Adafruit_GFX &g, const char *title, const char *detail, int pct);

// Pairing mode: the name Gauge will list under Pair Accessory...
void uiPairing(Adafruit_GFX &g, const char *name);

// Bluetooth Secure Connections number, identical to the one on the Mac.
void uiCompare(Adafruit_GFX &g, uint32_t number);

// Dashboard pages. `page` of `pages` drives the pager dots; `stale` marks a
// snapshot that could not be refreshed.
void uiProvider(Adafruit_GFX &g, const ProviderEntry &p, int64_t now, bool stale, uint8_t page,
                uint8_t pages);
void uiQuotaError(Adafruit_GFX &g, const DashboardData &d, bool stale, uint8_t page,
                  uint8_t pages);
void uiCalendar(Adafruit_GFX &g, const DashboardData &d, int64_t now, bool stale, uint8_t page,
                uint8_t pages);
void uiTodos(Adafruit_GFX &g, const DashboardData &d, bool stale, uint8_t page, uint8_t pages);
