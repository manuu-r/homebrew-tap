// Host-side tests for the quota parsers.
//
//   cd test/host && ./run.sh
//
// ArduinoJson.h here is the upstream single-header amalgamation, fetched by
// run.sh so the parsers can be exercised without an ESP32 toolchain.

#include <cmath>
#include <cstdio>
#include <string>

#include "../../src/gauge_parse.h"
#include "../../src/quota_parse.h"

static int g_fail = 0;
static int g_total = 0;

// Mirrors quota.cpp: the gauge parser's verdict is final once it recognises
// the payload; the generic parser only covers a non-gauge server.
static bool parseLikeFirmware(JsonVariantConst root, QuotaReading &out) {
  if (!parseGaugeQuota(root, out)) parseQuotaJson(root, out);
  return out.any();
}

static std::string show(bool valid, float pct) {
  char b[24];
  if (!valid) snprintf(b, sizeof(b), "--");
  else snprintf(b, sizeof(b), "%.1f", (double)pct);
  return b;
}

// exp < 0 means "this provider must not resolve"
static void check(const char *label, const char *json, float expClaude, float expCodex) {
  g_total++;
  JsonDocument doc;
  const DeserializationError err = deserializeJson(doc, json);
  if (err) {
    printf("  FAIL %-36s json error: %s\n", label, err.c_str());
    g_fail++;
    return;
  }

  QuotaReading r;
  parseLikeFirmware(doc.as<JsonVariantConst>(), r);

  const float exp[2] = {expClaude, expCodex};
  bool ok = true;
  for (int i = 0; i < PROV_COUNT; i++) {
    if (exp[i] < 0.0f) ok &= !r.valid[i];
    else ok &= r.valid[i] && fabsf(r.pct[i] - exp[i]) <= 0.6f;
  }

  if (ok) {
    printf("  ok   %-36s claude=%-6s codex=%-6s\n", label, show(r.valid[0], r.pct[0]).c_str(),
           show(r.valid[1], r.pct[1]).c_str());
  } else {
    printf("  FAIL %-36s claude=%-6s codex=%-6s  (want %.1f / %.1f)\n", label,
           show(r.valid[0], r.pct[0]).c_str(), show(r.valid[1], r.pct[1]).c_str(),
           (double)expClaude, (double)expCodex);
    g_fail++;
  }
}

int main() {
  printf("\ngauge /v1/quota, exact schema\n");

  check("canonical payload",
        R"J({"generated_at":1710000000,
            "providers":[
              {"name":"Claude","remaining_percent":43,
               "limits":[{"label":"5-hour","used_percent":57.0},
                         {"label":"Weekly","used_percent":12.0},
                         {"label":"Weekly (Opus)","used_percent":80.0}]},
              {"name":"Codex","remaining_percent":20,
               "limits":[{"label":"Weekly","used_percent":80.0},
                         {"label":"5-hour","used_percent":10.0},
                         {"label":"Spark Weekly","used_percent":95.0}]}],
            "errors":[]})J",
        43.0f, 20.0f);

  // Spark is a separate allowance; gauge keeps it out of the headline number.
  check("codex ignores Spark buckets",
        R"J({"providers":[{"name":"Codex","remaining_percent":5,
             "limits":[{"label":"Weekly","used_percent":40.0},
                       {"label":"Spark Weekly","used_percent":95.0}]}]})J",
        -1.0f, 60.0f);

  check("null remaining_percent, window present",
        R"J({"providers":[{"name":"Claude","remaining_percent":null,
             "limits":[{"label":"5-hour","used_percent":2.5}]}]})J",
        97.5f, -1.0f);

  check("no hourly window, falls back",
        R"J({"providers":[{"name":"Claude","remaining_percent":66,
             "limits":[{"label":"Weekly","used_percent":34.0}]}]})J",
        66.0f, -1.0f);

  check("null remaining and no window",
        R"J({"providers":[{"name":"Claude","remaining_percent":null,
             "limits":[{"label":"Weekly","used_percent":34.0}]}]})J",
        -1.0f, -1.0f);

  check("tightest of several hourly windows",
        R"J({"providers":[{"name":"Claude","remaining_percent":90,
             "limits":[{"label":"5-hour","used_percent":10.0},
                       {"label":"1-hour","used_percent":70.0}]}]})J",
        30.0f, -1.0f);

  check("provider fetch failed entirely",
        R"J({"generated_at":1,"providers":[],"errors":["claude: token expired"]})J",
        -1.0f, -1.0f);

  check("zero and full",
        R"J({"providers":[{"name":"Claude","remaining_percent":0,
                          "limits":[{"label":"5-hour","used_percent":100.0}]},
                         {"name":"Codex","remaining_percent":100,
                          "limits":[{"label":"Weekly","used_percent":0.0}]}]})J",
        0.0f, 100.0f);

  printf("\ngeneric fallback, unknown schema\n");

  check("explicit pct, both windows",
        R"J({"claude":{"five_hour":{"remaining_pct":62}},
            "codex":{"week":{"used_pct":30}}})J",
        62.0f, 70.0f);

  check("array of providers, used/limit",
        R"J({"providers":[{"name":"codex","weekly":{"used":120,"limit":400}},
                         {"name":"claude","session":{"used":10,"limit":50}}]})J",
        80.0f, 70.0f);

  check("0..1 utilization fractions",
        R"J({"data":{"claude_code":{"session":{"utilization":0.42}},
                    "openai_codex":{"week":{"utilization":0.9}}}})J",
        58.0f, 10.0f);

  check("flat remaining percent",
        R"J({"claude":{"percent_remaining":100},"codex":{"percent_remaining":0}})J",
        100.0f, 0.0f);

  check("picks hourly for claude",
        R"J({"claude":{"week":{"remaining_pct":90},"hour":{"remaining_pct":25}},
            "codex":{"hour":{"remaining_pct":5},"week":{"remaining_pct":77}}})J",
        25.0f, 77.0f);

  check("anthropic/openai aliases",
        R"J({"anthropic":{"session":{"pct_left":33}},
            "openai":{"weekly":{"pct_left":88}}})J",
        33.0f, 88.0f);

  check("1 percent is not a fraction",
        R"J({"claude":{"hour":{"remaining_percent":1}},
            "codex":{"week":{"remaining_percent":4}}})J",
        1.0f, 4.0f);

  check("nested under envelope",
        R"J({"ok":true,"result":{"quotas":{"claude":{"session":{"used_pct":95}},
                                          "codex":{"week":{"used_pct":100}}}}})J",
        5.0f, 0.0f);

  check("counts w/ limit beat bare counts",
        R"J({"claude":{"hourly":{"used":1,"limit":400}},
            "codex":{"weekly":{"used":399,"limit":400}}})J",
        99.75f, 0.25f);

  check("only claude present",
        R"J({"claude":{"hour":{"remaining_pct":50}}})J",
        50.0f, -1.0f);

  check("no providers at all",
        R"J({"status":"ok","uptime":1234})J",
        -1.0f, -1.0f);

  check("clamps out-of-range",
        R"J({"claude":{"hour":{"remaining_pct":140}},
            "codex":{"week":{"remaining_pct":-20}}})J",
        100.0f, 0.0f);

  printf("\n%d/%d passed\n", g_total - g_fail, g_total);
  return g_fail ? 1 : 0;
}
