#pragma once
#include <Arduino.h>
#include <IPAddress.h>

// Called during the sweep so the UI can show progress. `pct` is 0..100.
typedef void (*SweepProgressFn)(const IPAddress &probing, int pct);

// Single-host check: TCP connect, unauthenticated GET HEALTH_PATH, expect 2xx.
// A 401/403 means the endpoint wants auth, which disqualifies the host.
bool probeHealth(const IPAddress &ip);

// Sweeps the local /24 and returns the first host answering /health.
bool discoverHealthHost(IPAddress &found, SweepProgressFn onProgress);
