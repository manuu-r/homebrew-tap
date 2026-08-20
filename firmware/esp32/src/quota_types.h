#pragma once
#include <stdint.h>

enum ProviderKind : uint8_t { PROV_CLAUDE = 0, PROV_CODEX = 1, PROV_COUNT = 2 };

// The window each provider is reported against, per spec: Claude hourly,
// Codex weekly. Used both for parser scoring and for the on-screen label.
inline const char *providerName(ProviderKind k) {
  return (k == PROV_CLAUDE) ? "CLAUDE" : "CODEX";
}
inline const char *providerWindow(ProviderKind k) {
  return (k == PROV_CLAUDE) ? "HOURLY" : "WEEKLY";
}

struct QuotaReading {
  bool  valid[PROV_COUNT];
  float pct[PROV_COUNT];  // remaining, 0..100

  QuotaReading() : valid{false, false}, pct{0.0f, 0.0f} {}

  bool any() const { return valid[PROV_CLAUDE] || valid[PROV_CODEX]; }
};
