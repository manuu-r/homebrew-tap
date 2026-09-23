// Host tests for the dashboard parser, run against Gauge's own conformance
// fixture so firmware and app cannot drift apart.
//
//   cd test/host && ./run.sh

#include <cmath>
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>

#include "../../src/dashboard_parse.h"

static int failures = 0;

static void check(const char *label, bool ok) {
  printf("  %s %s\n", ok ? "ok  " : "FAIL", label);
  if (!ok) failures++;
}

static bool parse(const std::string &json, DashboardData &out) {
  JsonDocument doc;
  return !deserializeJson(doc, json) && dashboardparse::parse(doc.as<JsonVariantConst>(), out);
}

int main() {
  std::ifstream file("../../../../docs/fixtures/dashboard-v1.json");
  std::stringstream fixture;
  fixture << file.rdbuf();

  DashboardData d;
  check("docs/fixtures/dashboard-v1.json parses", parse(fixture.str(), d));
  check("quota provider", d.providerCount == 1 && strcmp(d.providers[0].name, "Example") == 0 &&
                              d.providers[0].haveRemaining && d.providers[0].remaining == 75.0f);
  check("quota limit", d.providers[0].limitCount == 1 &&
                           strcmp(d.providers[0].limits[0].label, "Weekly") == 0 &&
                           d.providers[0].limits[0].resetsAt == 1700600000);
  check("calendar", d.calendarEnabled && d.eventCount == 1 && d.events[0].startsAt == 1700003600 &&
                        !d.events[0].allDay);
  check("todos", d.todoCount == 1 && d.todoTotal == 1 && !d.todos[0].completed);
  check("refresh_seconds", d.refreshSeconds == 120 && d.generatedAt == 1700000000);

  DashboardData spark;
  check("Codex Spark becomes its own group",
        parse(R"({"protocol":"dev.gauge.dashboard","schema_version":1,"quota":{"providers":[
          {"name":"Codex","remaining_percent":20,"limits":[
            {"label":"Weekly","used_percent":80,"remaining_percent":20},
            {"label":"Spark Weekly","used_percent":5,"remaining_percent":95}]}]},
          "calendar":{"enabled":false,"events":[]},"todos":[],"extra":{}})",
              spark) &&
            spark.providerCount == 2 && spark.providers[0].remaining == 20.0f &&
            spark.providers[0].limitCount == 1 &&
            strcmp(spark.providers[1].name, "Codex Spark") == 0 &&
            strcmp(spark.providers[1].limits[0].label, "Weekly") == 0 &&
            spark.providers[1].remaining == 95.0f);

  DashboardData rejected;
  check("rejects another protocol",
        !parse(R"({"protocol":"other","schema_version":1})", rejected));
  check("rejects schema 2",
        !parse(R"({"protocol":"dev.gauge.dashboard","schema_version":2})", rejected));

  printf("\n%s\n", failures ? "FAILED" : "all passed");
  return failures ? 1 : 0;
}
