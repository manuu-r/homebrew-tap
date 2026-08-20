#pragma once
#include <Arduino.h>
#include <IPAddress.h>

#include "quota_types.h"

// GETs the quota endpoint on `host` and fills `out`.
// Tries the previously successful path first, then every QUOTA_PATHS entry.
// Returns false and sets `err` if nothing usable came back.
bool quotaFetch(const IPAddress &host, uint16_t port, QuotaReading &out, String &err);

// Forget the cached endpoint path (call when re-discovering the host).
void quotaResetPathCache();
