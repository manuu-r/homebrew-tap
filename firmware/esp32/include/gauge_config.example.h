#pragma once

#include <cstdint>

namespace gauge_config {

// --- Wi-Fi -----------------------------------------------------------------

constexpr char kWifiSsid[] = "your-ssid";
constexpr char kWifiPassword[] = "your-password";

// --- Gauge server ----------------------------------------------------------

// With auto-discovery the firmware sweeps the local /24 for an unauthenticated
// `GET /health` on kHttpPort and caches the winner in NVS, so it survives the
// server moving to a new DHCP lease. Set false to pin kGaugeHost instead.
constexpr bool kAutoDiscover = true;

// Used when kAutoDiscover is false, and tried first when it is true.
constexpr char kGaugeHost[] = "192.168.1.10";
constexpr std::uint16_t kHttpPort = 8080;

// `/health` is the only unauthenticated route; `/v1/quota` needs this token.
// It must match GAUGE_API_TOKEN (or --token) on the gauge server. Leave empty
// only if your instance runs without a token.
constexpr char kGaugeToken[] = "replace-with-token";

// The UDP ("BLE-style") transport served by `gauge --ble` on port 8081 is not
// implemented by this client. See firmware/esp32/README.md for the protocol
// docs if you want to add it.

}  // namespace gauge_config
