#pragma once
#include <Arduino.h>

class Preferences;

// The runtime half of the protocol: find the paired Gauge on the LAN by its
// Bonjour server ID, then talk to it with this device's bearer token.
namespace gauge {

enum class Fetch { Ok, Failed, Revoked };

// Reads the commissioned server ID, port, and token from NVS.
void begin(Preferences &prefs);

// GET /v1/dashboard. `Revoked` means Gauge answered 401: it forgot this device.
Fetch dashboard(String &body);

// Best-effort DELETE /v1/accessory so Gauge drops this device's token.
void revoke();

}  // namespace gauge
