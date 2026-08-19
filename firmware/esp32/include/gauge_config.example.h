#pragma once

#include <cstdint>

namespace gauge_config {

constexpr char kWifiSsid[] = "your-ssid";
constexpr char kWifiPassword[] = "your-password";

constexpr char kGaugeHost[] = "192.168.1.10";
constexpr std::uint16_t kHttpPort = 8080;
constexpr std::uint16_t kUdpPort = 8081;
constexpr char kGaugeToken[] = "replace-with-token";

}  // namespace gauge_config
