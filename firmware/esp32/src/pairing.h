#pragma once
#include <Arduino.h>

class Preferences;

// Gauge Accessory Protocol commissioning over Bluetooth LE Secure Connections.
//
// Gauge reads the MITM-protected Identity characteristic, which makes macOS
// and this device show the same six-digit number. `confirm` shows it and
// returns the user's decision. Gauge then writes framed Wi-Fi and server
// credentials; they are validated, saved to NVS, and Wi-Fi is joined.
namespace pairing {

using ConfirmFn = bool (*)(uint32_t number);

void start(Preferences &prefs, const String &deviceId, const String &name, ConfirmFn confirm);

// Call from loop(). Returns true once Status has reported "connected" long
// enough for Gauge to read it; the caller then restarts into the dashboard.
bool service();

}  // namespace pairing
