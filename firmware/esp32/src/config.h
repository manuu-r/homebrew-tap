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
static const uint8_t  TFT_ROTATION = 0;  // portrait, ribbon at top

// Hardware SPI vs bit-banged software SPI.
//
// Measured on this board with src/display_test.cpp: hardware SPI through the
// HSPI peripheral does NOT drive the panel on this pin set at any mode or
// speed, while software SPI on the identical pins works. GPIO2 is a strapping
// pin with the onboard LED on it and does not route cleanly through the SPI
// peripheral. Software SPI is fine here - the screen redraws once a minute and
// only the regions that changed.
//
// Set to 1 if you move SCLK to a native HSPI pin (GPIO14) and want the fast path.
#define TFT_USE_HW_SPI 0

// Hardware-SPI clock only; ignored when TFT_USE_HW_SPI is 0.
static const uint32_t TFT_SPI_HZ = 40000000UL;

// Servos: reserved only. Nothing in this firmware drives them yet.
static const int8_t PIN_SERVO_LEFT   = 22;
static const int8_t PIN_SERVO_RIGHT  = 23;
static const int8_t PIN_SERVO_MIDDLE = 12;  // MTDI strapping pin, see README

// ---------------------------------------------------------------------------
// Gauge endpoints
// ---------------------------------------------------------------------------

static const uint16_t GAUGE_PORT = gauge_config::kHttpPort;

// The only route gauge serves without a token, which is what makes it usable
// as a discovery probe (src/network.rs: `path != "/health" && !authorized`).
static const char HEALTH_PATH[] = "/health";

// Real gauge endpoint first; the rest let this firmware limp along against a
// different quota server, resolved by the generic parser in quota_parse.h.
static const char *const QUOTA_PATHS[] = {
    "/v1/quota", "/quota", "/api/quota", "/usage",
};
static const size_t QUOTA_PATH_COUNT = sizeof(QUOTA_PATHS) / sizeof(QUOTA_PATHS[0]);

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

// Per-host budget during the sweep. Unused addresses normally fail fast with a
// RST or an ARP timeout well inside this.
static const uint32_t TCP_CONNECT_TIMEOUT_MS = 120;
static const uint32_t HTTP_READ_TIMEOUT_MS   = 500;

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

static const uint32_t POLL_INTERVAL_MS = 60000UL;  // one fetch per minute
static const uint32_t WIFI_CONNECT_TIMEOUT_MS = 30000UL;

// Consecutive fetch failures before the cached host is dropped and re-swept.
static const uint8_t FAILURES_BEFORE_REDISCOVER = 3;

// Watchdog. Must exceed the longest blocking stretch between feeds; the LAN
// sweep feeds it per host, so this only has to cover one HTTP round trip.
static const uint32_t WDT_TIMEOUT_S = 30;
