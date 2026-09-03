#pragma once
#include <Arduino.h>
#include "theme.h"

// The emoji font problem: Adafruit_GFX ships ASCII/CP437 bitmaps only, so a
// literal 🥩 renders as garbage. Every "emoji" here is drawn with GFX
// primitives instead (see emoji.cpp).
enum Mood : uint8_t {
  MOOD_WELLFED = 0,
  MOOD_PECKISH,
  MOOD_HUNGRY,
  MOOD_STARVING,
  MOOD_FERAL,
  MOOD_NEARDEATH,
  MOOD_DEAD,
};

// Bands are on the *remaining* percentage, rounded to the nearest integer:
//   100-80 Well-Fed | 79-60 Getting-Peckish | 59-40 Hungry | 39-20 Starving
//   19-5   Feral    | 4-1   Near-Death      | 0     DEAD
inline Mood moodFromPct(float pct) {
  if (pct < 0.0f) pct = 0.0f;
  if (pct > 100.0f) pct = 100.0f;
  const int p = (int)lroundf(pct);
  if (p >= 80) return MOOD_WELLFED;
  if (p >= 60) return MOOD_PECKISH;
  if (p >= 40) return MOOD_HUNGRY;
  if (p >= 20) return MOOD_STARVING;
  if (p >= 5)  return MOOD_FERAL;
  if (p >= 1)  return MOOD_NEARDEATH;
  return MOOD_DEAD;
}

// One word each, as asked.
inline const char *moodWord(Mood m) {
  switch (m) {
    case MOOD_WELLFED:   return "Well-Fed";
    case MOOD_PECKISH:   return "Getting-Peckish";
    case MOOD_HUNGRY:    return "Hungry";
    case MOOD_STARVING:  return "Starving";
    case MOOD_FERAL:     return "Feral";
    case MOOD_NEARDEATH: return "Near-Death";
    case MOOD_DEAD:      return "DEAD";
  }
  return "?";
}

inline uint16_t moodColor(Mood m) {
  switch (m) {
    case MOOD_WELLFED:   return C_WELLFED;
    case MOOD_PECKISH:   return C_PECKISH;
    case MOOD_HUNGRY:    return C_HUNGRY;
    case MOOD_STARVING:  return C_STARVING;
    case MOOD_FERAL:     return C_FERAL;
    case MOOD_NEARDEATH: return C_NEARDEATH;
    case MOOD_DEAD:      return C_DEAD;
  }
  return C_MUTED;
}
