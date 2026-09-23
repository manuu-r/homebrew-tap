#pragma once
#include <stdint.h>

// ---------------------------------------------------------------------------
// Hardware map
// ---------------------------------------------------------------------------

// ST7789 240x320, bit-banged SPI. On this pin set hardware SPI does not drive
// the panel at any mode or clock (GPIO2 is a strapping pin with the devkit LED
// on it); src/display_test.cpp is the diagnostic that established this.
static const int8_t PIN_TFT_CS   = 5;
static const int8_t PIN_TFT_DC   = 4;
static const int8_t PIN_TFT_RST  = 15;  // strapping pin: must be high at boot
static const int8_t PIN_TFT_MOSI = 18;
static const int8_t PIN_TFT_SCLK = 2;   // strapping pin: low/floating at boot
static const int8_t PIN_TFT_MISO = -1;  // panel is write-only
// Set to the GPIO wired to the panel's BLK/LED pin, or -1 if it is tied to 3V3.
static const int8_t PIN_TFT_BLK  = -1;

static const uint16_t TFT_W = 240;
static const uint16_t TFT_H = 320;
static const uint8_t  TFT_ROTATION = 2;  // portrait, ribbon at top

// The devkit's BOOT button. Press it to accept a pairing number; hold it for
// UNPAIR_HOLD_MS while the dashboard is showing to unpair.
static const int8_t PIN_BUTTON = 0;

// Two-servo head. Power the servos from a separate 5 V supply and join its
// ground to ESP32 GND. Swap SERVO_MIDDLE_UP/DOWN if the linkage is mirrored.
static const int8_t PIN_SERVO_MIDDLE = 13;  // head up/down
static const int8_t PIN_SERVO_BOTTOM = 14;  // head left/right
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
// Gauge Accessory Protocol v1 (docs/accessory-protocol.md)
// ---------------------------------------------------------------------------

static const char FIRMWARE_VERSION[] = "2.0.0";

static const char PAIRING_SERVICE_UUID[] = "c9cce6f3-bf10-4e6d-b719-f32911bbba89";
static const char IDENTITY_UUID[]        = "1db9634c-a20f-43c6-8ed9-69ceda338178";
static const char CONFIG_UUID[]          = "289be295-d110-411b-888c-c80a601177fa";
static const char STATUS_UUID[]          = "e763eccb-fa4c-4e3a-9211-850513371105";

static const char PAIRING_PROTOCOL[]    = "dev.gauge.pairing";
static const char COMMISSION_PROTOCOL[] = "dev.gauge.commission";
static const char ACCESSORY_PROTOCOL[]  = "dev.gauge.accessory";
static const char DASHBOARD_PROTOCOL[]  = "dev.gauge.dashboard";
static const char SERVICE_TYPE[]        = "_gauge._tcp.local.";
static const char DASHBOARD_PATH[]      = "/v1/dashboard";
static const char ACCESSORY_PATH[]      = "/v1/accessory";

static const uint8_t FRAME_MAGIC = 0x47;
static const size_t  MAX_COMMISSION_BYTES = 1024;
static const size_t  MAX_DASHBOARD_BYTES = 16384;

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

static const uint32_t PAIR_CONFIRM_TIMEOUT_MS = 30000;  // no press = reject
static const uint32_t WIFI_JOIN_TIMEOUT_MS = 30000;
// Gauge polls Status every 700 ms; keep "connected" readable before rebooting.
static const uint32_t PAIRED_RESTART_DELAY_MS = 3000;
static const uint32_t UNPAIR_HOLD_MS = 5000;
static const uint32_t PAGE_DURATION_MS = 8000;
static const uint32_t RETRY_MS = 15000;
static const uint32_t MIN_REFRESH_S = 30;
static const uint32_t MAX_REFRESH_S = 900;
static const uint32_t MDNS_QUERY_MS = 3000;
static const uint32_t HTTP_TIMEOUT_MS = 5000;
