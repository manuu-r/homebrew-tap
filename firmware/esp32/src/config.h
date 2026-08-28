#pragma once
#include <Arduino.h>

#if __has_include("gauge_config.h")
#include "gauge_config.h"
#else
#include "gauge_config.example.h"
#endif

// ---------------------------------------------------------------------------
// Hardware map
// ---------------------------------------------------------------------------

// ST7789 TFT. These are NOT the ESP32 default VSPI pins, so we drive the panel
// through HSPI and let the GPIO matrix route the signals.
static const int8_t PIN_TFT_CS   = 5;
static const int8_t PIN_TFT_DC   = 4;
static const int8_t PIN_TFT_RST  = 15;  // strapping pin, see README
static const int8_t PIN_TFT_MOSI = 18;
static const int8_t PIN_TFT_SCLK = 2;   // strapping pin + onboard LED, see README
static const int8_t PIN_TFT_MISO = -1;  // panel is write-only

// Most ST7789 breakouts have a BLK/LED backlight pin. If it is not tied to 3V3
// the panel stays dark even when SPI is working perfectly. Set this to the GPIO
// you wired BLK to; -1 means "not driven by the firmware".
static const int8_t PIN_TFT_BLK = -1;

static const uint16_t TFT_W = 240;
static const uint16_t TFT_H = 320;
static const uint8_t  TFT_ROTATION = 2;  // portrait, ribbon at top

// Hardware SPI vs bit-banged software SPI.
//
// Measured on this board with src/display_test.cpp: hardware SPI through the
// HSPI peripheral does NOT drive the panel on this pin set at any mode or
// speed, while software SPI on the identical pins works. GPIO2 is a strapping
// pin with the onboard LED on it and does not route cleanly through the SPI
// peripheral. Software SPI is fine here - the screen changes every 8 seconds.
//
// The bottom servo now occupies native HSPI pin GPIO14, so hardware SPI would
// require remapping either that servo or the display clock first.
#define TFT_USE_HW_SPI 0

// Hardware-SPI clock only; ignored when TFT_USE_HW_SPI is 0.
static const uint32_t TFT_SPI_HZ = 40000000UL;

// Four-servo head wiring. Left/right are ear servos and intentionally remain
// detached; only middle (head pitch) and bottom (head yaw) are driven.
// Power the servos from a separate 5 V supply and join its ground to ESP32 GND.
#define PIN_SERVO_LEFT 22
#define PIN_SERVO_RIGHT 23
#define PIN_SERVO_MIDDLE 13
#define PIN_SERVO_BOTTOM 14

// Calibrate these for the physical bracket. Swap MIDDLE_UP and MIDDLE_DOWN if
// the head moves in the opposite direction on your linkage.
static const int SERVO_MIDDLE_UP = 110;
static const int SERVO_MIDDLE_DOWN = 40;
static const int SERVO_BOTTOM_CENTER = 90;
static const int SERVO_BOTTOM_SWING = 22;
static const uint16_t SERVO_SHAKE_HOLD_MS = 105;
static const uint16_t SERVO_HEAD_DOWN_HOLD_MS = 260;
static const uint16_t SERVO_RISE_STEP_MS = 18;
static const int SERVO_MIN_US = 500;
static const int SERVO_MAX_US = 2400;

// ---------------------------------------------------------------------------
// Gauge endpoints
// ---------------------------------------------------------------------------

static const uint16_t GAUGE_PORT = gauge_config::kHttpPort;

// The only route gauge serves without a token, which is what makes it usable
// as a discovery probe (src/network.rs: `path != "/health" && !authorized`).
static const char HEALTH_PATH[] = "/health";

// One universal protocol and payload for every screen on the device.
static const char DASHBOARD_PATH[] = "/v1/dashboard";

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

// Per-host budget during the sweep. Unused addresses normally fail fast with a
// RST or an ARP timeout well inside this.
static const uint32_t TCP_CONNECT_TIMEOUT_MS = 120;
static const uint32_t HTTP_READ_TIMEOUT_MS   = 500;
static const uint32_t DASHBOARD_HTTP_TIMEOUT_MS = 35000;

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

static const uint32_t PAGE_DURATION_MS = 8000UL;
static const uint32_t WIFI_CONNECT_TIMEOUT_MS = 30000UL;

// Consecutive fetch failures before the cached host is dropped and re-swept.
static const uint8_t FAILURES_BEFORE_REDISCOVER = 3;

// Watchdog. Must exceed the longest blocking stretch between feeds; the LAN
// sweep feeds it per host, so this only has to cover one HTTP round trip.
static const uint32_t WDT_TIMEOUT_S = 45;
