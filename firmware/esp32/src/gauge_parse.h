#pragma once
#include <ArduinoJson.h>
#include <math.h>
#include <string.h>

#include "quota_parse.h"
#include "quota_types.h"

// ---------------------------------------------------------------------------
// Exact parser for gauge's GET /v1/quota (see src/main.rs: quota_json).
//
//   {
//     "generated_at": 1710000000,
//     "providers": [
//       { "name": "Claude",
//         "remaining_percent": 43,          // tightest window, may be null
//         "limits": [ {"label": "5-hour",   "used_percent": 57.0},
//                     {"label": "Weekly",   "used_percent": 12.0} ] },
//       { "name": "Codex",
//         "remaining_percent": 20,
//         "limits": [ {"label": "Weekly",       "used_percent": 80.0},
//                     {"label": "Spark Weekly", "used_percent": 5.0} ] }
//     ],
//     "errors": []
//   }
//
// gauge's own `remaining_percent` is the *tightest* window across the provider.
// The display wants a specific window per provider - Claude hourly, Codex
// weekly - so we read `limits[]` by label and only fall back to
// `remaining_percent` when that window is missing.
// ---------------------------------------------------------------------------

namespace gaugeparse {

// Claude emits "5-hour", "Weekly", "Weekly (Opus)", "Weekly (Sonnet)".
// Codex emits "5-hour", "Weekly", "<n>-hour", each optionally "Spark "-prefixed.
inline const char *wantedLabel(ProviderKind kind) {
  return (kind == PROV_CLAUDE) ? "hour" : "week";
}

// gauge keeps Codex's Spark buckets out of the headline number because they are
// a separate allowance rather than the main quota; match that here.
inline bool isSparkBucket(const char *label) { return strncmp(label, "Spark ", 6) == 0; }

inline bool providerKindFromName(const char *name, ProviderKind &kind) {
  if (quotaparse::ciFind(name, "claude") || quotaparse::ciFind(name, "anthropic")) {
    kind = PROV_CLAUDE;
    return true;
  }
  if (quotaparse::ciFind(name, "codex") || quotaparse::ciFind(name, "openai")) {
    kind = PROV_CODEX;
    return true;
  }
  return false;
}

}  // namespace gaugeparse

// Returns true if the document is recognisably a gauge payload, whether or not
// any provider resolved. Callers use that as "my verdict is final, do not fall
// back to the generic parser" - otherwise a Claude entry with no hourly window
// would silently be filled in from its weekly number and mislabelled HOURLY.
inline bool parseGaugeQuota(JsonVariantConst root, QuotaReading &out) {
  JsonArrayConst providers = root["providers"].as<JsonArrayConst>();
  if (providers.isNull()) return false;

  // A bare {"name": ...} array is not gauge; require a field gauge actually
  // emits per provider. Key presence, not value: a provider that failed to
  // report serialises as "remaining_percent": null and is still gauge.
  bool recognised = false;
  for (JsonObjectConst p : providers) {
    for (JsonPairConst kv : p) {
      const char *k = kv.key().c_str();
      if (strcmp(k, "limits") == 0 || strcmp(k, "remaining_percent") == 0) {
        recognised = true;
        break;
      }
    }
    if (recognised) break;
  }
  if (!recognised) return false;

  for (JsonObjectConst p : providers) {
    const char *name = p["name"] | "";
    ProviderKind kind;
    if (!gaugeparse::providerKindFromName(name, kind)) continue;

    const char *want = gaugeparse::wantedLabel(kind);
    float pct = NAN;

    // 1. The window the display is specified against. If a provider reports
    //    several matching windows, the tightest one is the one that bites.
    JsonArrayConst limits = p["limits"].as<JsonArrayConst>();
    if (!limits.isNull()) {
      for (JsonObjectConst lim : limits) {
        const char *label = lim["label"] | "";
        if (!label[0]) continue;
        if (kind == PROV_CODEX && gaugeparse::isSparkBucket(label)) continue;
        if (!quotaparse::ciFind(label, want)) continue;

        JsonVariantConst used = lim["used_percent"];
        if (!quotaparse::isNumber(used)) continue;

        const float remaining = 100.0f - used.as<float>();
        if (isnan(pct) || remaining < pct) pct = remaining;
      }
    }

    // 2. Fall back to gauge's own tightest-window figure. It is Option<u64>,
    //    so a provider that failed to report serialises as null.
    if (isnan(pct)) {
      JsonVariantConst rp = p["remaining_percent"];
      if (quotaparse::isNumber(rp)) pct = rp.as<float>();
    }

    if (isnan(pct)) continue;
    if (pct < 0.0f) pct = 0.0f;
    if (pct > 100.0f) pct = 100.0f;

    out.valid[kind] = true;
    out.pct[kind] = pct;
  }
  return true;
}
