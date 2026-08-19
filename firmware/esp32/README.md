# ESP32 Gauge clients

This PlatformIO project contains two independent C++ firmware clients:

| Environment | Source | Transport | Gauge server |
| --- | --- | --- | --- |
| `wifi` | `src/wifi_client.cpp` | HTTP over Wi-Fi | `gauge --wifi` |
| `ble` | `src/ble_client.cpp` | UDP over Wi-Fi | `gauge --ble` |

Gauge currently calls its UDP protocol “BLE-style,” but it is not Bluetooth LE
GATT. Both clients therefore need Wi-Fi credentials.

## Configure

```sh
cd firmware/esp32
cp include/gauge_config.example.h include/gauge_config.h
```

Edit `include/gauge_config.h` and set the Wi-Fi credentials, Gauge host, ports,
and API token. This private file is ignored by Git.

## Build and upload

Install [PlatformIO](https://platformio.org/install/cli), then choose a client:

```sh
# HTTP client
pio run -e wifi
pio run -e wifi --target upload

# BLE-style UDP client
pio run -e ble
pio run -e ble --target upload

pio device monitor --baud 115200
```

The default board is `esp32dev`; update `board` in `platformio.ini` if needed.

## Start Gauge

```sh
# For wifi_client.cpp
GAUGE_API_TOKEN=secret gauge --wifi --bind 0.0.0.0 --port 8080

# For ble_client.cpp
GAUGE_API_TOKEN=secret gauge --ble --bind 0.0.0.0 --port 8081
```

Use the same token in `gauge_config.h`. Both clients poll every 30 seconds.
