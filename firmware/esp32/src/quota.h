#pragma once
#include <Arduino.h>
#include <IPAddress.h>

#include "dashboard_types.h"

// Fetches the versioned HTTP + JSON dashboard in one request.
bool dashboardFetch(const IPAddress &host, uint16_t port, DashboardData &out, String &err);
