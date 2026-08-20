#pragma once
#include <Adafruit_GFX.h>

#include "mood.h"
#include "quota_types.h"

struct UiModel {
  ProviderKind provider = PROV_CLAUDE;
  bool  havePct   = false;
  float pct       = 0.0f;   // remaining, 0..100
  bool  haveDelta = false;
  float delta     = 0.0f;   // signed percentage points since the last change
  bool  stale     = false;  // last fetch failed; showing the previous value
};

// Forces the next uiRender() to repaint every zone.
void uiInvalidate();

// Full-screen status card, used for boot / Wi-Fi / sweep.
// `pct` < 0 hides the progress bar.
void uiStatus(Adafruit_GFX &g, const char *title, const char *detail, int pct);

// Updates just the detail line and bar of the current status card. Used for the
// LAN sweep, where a full repaint per host would strobe the panel.
void uiStatusProgress(Adafruit_GFX &g, const char *detail, int pct);

// The dashboard. Repaints only the zones whose content changed.
void uiRender(Adafruit_GFX &g, const UiModel &m);
