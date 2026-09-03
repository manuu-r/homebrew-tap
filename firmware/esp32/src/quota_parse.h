#pragma once
#include <ArduinoJson.h>
#include <math.h>
#include <string.h>

#include "quota_types.h"

// ---------------------------------------------------------------------------
// Schema-agnostic quota extraction.
//
// We do not know the endpoint's schema, so instead of a fixed path we walk the
// whole document and score every numeric leaf on how much it looks like "the
// remaining percentage for provider X in window W". Highest score wins.
//
// Handles, among others:
//   {"claude":{"five_hour":{"remaining_pct":62}},"codex":{"week":{"used_pct":30}}}
//   {"providers":[{"name":"codex","weekly":{"used":120,"limit":400}}]}
//   {"data":{"claude_code":{"session":{"utilization":0.42}}}}
// ---------------------------------------------------------------------------

namespace quotaparse {

inline char lower(char c) { return (c >= 'A' && c <= 'Z') ? (char)(c + 32) : c; }

// case-insensitive substring test
inline bool ciFind(const char *hay, const char *needle) {
  if (!hay || !needle) return false;
  for (const char *h = hay; *h; h++) {
    const char *a = h, *b = needle;
    while (*a && *b && lower(*a) == lower(*b)) { a++; b++; }
    if (!*b) return true;
  }
  return false;
}

inline bool ciFindAny(const char *hay, const char *const *needles, size_t n) {
  for (size_t i = 0; i < n; i++)
    if (ciFind(hay, needles[i])) return true;
  return false;
}

// --- vocabulary ------------------------------------------------------------

static const char *const kClaudeWords[] = {"claude", "anthropic"};
static const char *const kCodexWords[]  = {"codex", "openai"};

static const char *const kHourWords[] = {"hour", "session", "5h", "five_hour",
                                         "fivehour", "current", "short"};
static const char *const kWeekWords[] = {"week", "7d", "seven_day", "sevenday"};
static const char *const kOtherWindow[] = {"day", "daily", "month", "monthly",
                                           "minute", "lifetime", "total_all"};

static const char *const kRemainWords[] = {"remain", "left", "avail", "free", "budget"};
static const char *const kUsedWords[]   = {"used", "usage", "consumed", "spent",
                                           "utilization", "utilisation", "burn"};
// Explicit percent markers: a value under one of these keys is already 0..100
// and must never be rescaled.
static const char *const kPctWords[]  = {"percent", "pct", "_%"};
// Explicit 0..1 markers.
static const char *const kFracWords[] = {"ratio", "fraction", "frac", "normalized"};
static const char *const kLimitWords[] = {"limit", "total", "max", "cap", "allowance"};

#define NELEM(a) (sizeof(a) / sizeof((a)[0]))

inline bool matchesProvider(const char *key, ProviderKind kind) {
  return (kind == PROV_CLAUDE) ? ciFindAny(key, kClaudeWords, NELEM(kClaudeWords))
                               : ciFindAny(key, kCodexWords, NELEM(kCodexWords));
}

// +6 for the window we want, -6 for a window belonging to the other provider's
// spec, -3 for an unrelated window. Accumulated down the key path.
inline int windowScore(const char *key, ProviderKind kind) {
  const bool isHour = ciFindAny(key, kHourWords, NELEM(kHourWords));
  const bool isWeek = ciFindAny(key, kWeekWords, NELEM(kWeekWords));
  const bool isOther = ciFindAny(key, kOtherWindow, NELEM(kOtherWindow));
  const bool want = (kind == PROV_CLAUDE) ? isHour : isWeek;
  const bool wrong = (kind == PROV_CLAUDE) ? isWeek : isHour;
  if (want) return 6;
  if (wrong) return -6;
  if (isOther) return -3;
  return 0;
}

struct Candidate {
  float value = 0.0f;
  int   score = -1000;
  bool  valid = false;

  void offer(float v, int s) {
    if (!isfinite(v)) return;
    if (v < 0.0f) v = 0.0f;
    if (v > 100.0f) v = 100.0f;
    if (!valid || s > score) {
      value = v;
      score = s;
      valid = true;
    }
  }
};

inline bool isNumber(JsonVariantConst v) {
  return !v.isNull() && v.is<float>() && !v.is<bool>() && !v.is<const char *>();
}

// Score a single numeric leaf given its own key.
inline void offerLeaf(Candidate &best, const char *key, float raw, int winScore) {
  const bool pctish    = ciFindAny(key, kPctWords, NELEM(kPctWords));
  const bool fracish   = ciFindAny(key, kFracWords, NELEM(kFracWords));
  const bool remainish = ciFindAny(key, kRemainWords, NELEM(kRemainWords));
  const bool usedish   = ciFindAny(key, kUsedWords, NELEM(kUsedWords));
  if (!pctish && !fracish && !remainish && !usedish) return;

  float v = raw;

  // Fraction convention: anything not explicitly marked as a percent, sitting
  // in [0,1], is read as 0..1 ("utilization": 0.42 -> 42% used). Keys that do
  // say percent are trusted verbatim, so a real "1 percent left" survives.
  if (!pctish && v >= 0.0f && v <= 1.0f) v *= 100.0f;

  // Past 100 with no percent marker this is a raw count, not a percentage.
  // offerPair() is the path that turns counts into percentages.
  if (!pctish && !fracish && v > 100.0f) return;

  if (usedish && !remainish) v = 100.0f - v;

  int s = winScore;
  if (pctish) s += 4;
  if (fracish) s += 4;
  if (remainish) s += 3;
  if (usedish) s += 2;
  best.offer(v, s);
}

// Score an object that carries a used/remaining count alongside a limit.
inline void offerPair(Candidate &best, JsonObjectConst obj, int winScore) {
  float used = NAN, remain = NAN, limit = NAN;
  for (JsonPairConst kv : obj) {
    if (!isNumber(kv.value())) continue;
    const char *k = kv.key().c_str();
    const float v = kv.value().as<float>();
    // Order matters: "total_used" is a used count, not a limit, and
    // "quota_remaining" is a remainder. Most specific role wins first.
    if (ciFindAny(k, kRemainWords, NELEM(kRemainWords)) && isnan(remain)) remain = v;
    else if (ciFindAny(k, kUsedWords, NELEM(kUsedWords)) && isnan(used)) used = v;
    else if (ciFindAny(k, kLimitWords, NELEM(kLimitWords)) && isnan(limit)) limit = v;
  }
  if (isnan(limit) || limit <= 0.0f) return;
  if (!isnan(remain))    best.offer(remain / limit * 100.0f, winScore + 7);
  else if (!isnan(used)) best.offer((1.0f - used / limit) * 100.0f, winScore + 6);
}

inline void scan(JsonVariantConst v, const char *key, ProviderKind kind, int winScore,
                 Candidate &best, int depth) {
  if (depth > 8) return;

  if (v.is<JsonObjectConst>()) {
    JsonObjectConst obj = v.as<JsonObjectConst>();
    offerPair(best, obj, winScore);
    for (JsonPairConst kv : obj) {
      const char *k = kv.key().c_str();
      scan(kv.value(), k, kind, winScore + windowScore(k, kind), best, depth + 1);
    }
    return;
  }
  if (v.is<JsonArrayConst>()) {
    for (JsonVariantConst e : v.as<JsonArrayConst>())
      scan(e, key, kind, winScore, best, depth + 1);
    return;
  }
  if (isNumber(v) && key) offerLeaf(best, key, v.as<float>(), winScore);
}

// Locate the subtree describing `kind`: either a key naming the provider, or an
// object with a name/id/provider field naming it.
inline bool findProviderRoot(JsonVariantConst v, ProviderKind kind, JsonVariantConst &out,
                             int depth = 0) {
  if (depth > 8) return false;

  if (v.is<JsonObjectConst>()) {
    JsonObjectConst obj = v.as<JsonObjectConst>();
    static const char *const idKeys[] = {"name", "id", "provider", "service", "vendor", "key"};
    for (size_t i = 0; i < NELEM(idKeys); i++) {
      JsonVariantConst idv = obj[idKeys[i]];
      if (idv.is<const char *>() && matchesProvider(idv.as<const char *>(), kind)) {
        out = v;
        return true;
      }
    }
    for (JsonPairConst kv : obj) {
      if (matchesProvider(kv.key().c_str(), kind)) {
        out = kv.value();
        return true;
      }
    }
    for (JsonPairConst kv : obj)
      if (findProviderRoot(kv.value(), kind, out, depth + 1)) return true;
    return false;
  }
  if (v.is<JsonArrayConst>()) {
    for (JsonVariantConst e : v.as<JsonArrayConst>())
      if (findProviderRoot(e, kind, out, depth + 1)) return true;
  }
  return false;
}

#undef NELEM

}  // namespace quotaparse

// Returns true if at least one provider was resolved.
inline bool parseQuotaJson(JsonVariantConst root, QuotaReading &out) {
  bool any = false;
  for (uint8_t i = 0; i < PROV_COUNT; i++) {
    const ProviderKind kind = (ProviderKind)i;
    JsonVariantConst sub;
    if (!quotaparse::findProviderRoot(root, kind, sub)) continue;

    quotaparse::Candidate best;
    quotaparse::scan(sub, nullptr, kind, 0, best, 0);
    if (!best.valid) continue;

    out.valid[i] = true;
    out.pct[i] = best.value;
    any = true;
  }
  return any;
}
