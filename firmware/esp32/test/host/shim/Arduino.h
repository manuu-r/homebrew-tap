// Minimal Arduino.h, host-side only. Just enough for Adafruit_GFX plus the
// firmware's drawing code to compile off-target.
#pragma once

#define ARDUINO 100

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

#include "Print.h"

#ifndef PROGMEM
#define PROGMEM
#endif

// Templates, not macros: the std headers this shim pulls in also spell `max(`,
// and a function-like macro would rewrite them into syntax errors.
template <class A, class B>
constexpr auto min(A a, B b) -> decltype(a < b ? a : b) {
  return a < b ? a : b;
}
template <class A, class B>
constexpr auto max(A a, B b) -> decltype(a < b ? b : a) {
  return a < b ? b : a;
}

#ifndef DEG_TO_RAD
#define DEG_TO_RAD 0.017453292519943295
#endif
constexpr float radians(float deg) { return deg * (float)DEG_TO_RAD; }

typedef uint8_t byte;
typedef bool boolean;
typedef const char *PGM_P;

// Only ever used as a pointer type in the declarations we compile.
class __FlashStringHelper;

// Adafruit_GFX declares String overloads of getTextBounds/print.
class String {
 public:
  String(const char *s = "") : s_(s ? s : "") {}
  const char *c_str() const { return s_.c_str(); }
  unsigned length() const { return (unsigned)s_.size(); }

 private:
  std::string s_;
};
