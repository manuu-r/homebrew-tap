#pragma once
#include <Arduino.h>

constexpr uint16_t rgb565(uint8_t r, uint8_t g, uint8_t b) {
  return (uint16_t)(((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3));
}

// Dark mode is the only display theme. A true-black canvas and very dark cards
// keep the always-on TFT from turning into a light source in a dim room.
static const uint16_t C_BG     = rgb565(0, 0, 0);
static const uint16_t C_TEXT   = rgb565(216, 220, 226);
static const uint16_t C_MUTED  = rgb565(96, 105, 119);
static const uint16_t C_TRACK  = rgb565(18, 22, 29);
static const uint16_t C_CARD   = rgb565(6, 9, 13);
static const uint16_t C_BORDER = rgb565(25, 31, 40);
static const uint16_t C_BLUE   = rgb565(62, 126, 220);
static const uint16_t C_VIOLET = rgb565(143, 93, 218);

// Ticker
static const uint16_t C_UP     = rgb565(46, 204, 113);
static const uint16_t C_DOWN   = rgb565(231, 76, 60);
static const uint16_t C_FLAT   = rgb565(118, 128, 143);

// Mood accents, walked from healthy to dead
static const uint16_t C_WELLFED   = rgb565(46, 204, 113);
static const uint16_t C_PECKISH   = rgb565(163, 201, 74);
static const uint16_t C_HUNGRY    = rgb565(241, 196, 15);
static const uint16_t C_STARVING  = rgb565(230, 126, 34);
static const uint16_t C_FERAL     = rgb565(211, 84, 0);
static const uint16_t C_NEARDEATH = rgb565(192, 57, 43);
static const uint16_t C_DEAD      = rgb565(120, 124, 130);
