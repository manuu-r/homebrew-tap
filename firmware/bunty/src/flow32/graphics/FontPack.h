#pragma once

#include <Adafruit_GFX.h>

#include "flow32/graphics/AAFont.h"

// Font slot selector. Bunty vendors only the Flow32 graphics core, so the one
// enum the display API needs from the (dropped) UI layer lives here.
enum class FontRole : uint8_t {
  Small,
  Body,
  BodyBold,
  Title,
  Large,
  Default,
};

/** A single optional font face. Null pointers use Adafruit's built-in font. */
struct FontFace {
  const GFXfont *gfx = nullptr;
  const AAFont *aa = nullptr;
};

/**
 * Project-supplied font roles.
 *
 * Flow32 deliberately ships without branded font payloads. A project can use
 * ordinary Adafruit GFX fonts, 4-bit anti-aliased Flow32 fonts, or both. The
 * tools/ttf_to_aafont.py utility generates AAFont headers from any licensed
 * TTF supplied by the application.
 */
struct FontPack {
  FontFace small;
  FontFace body;
  FontFace bodyBold;
  FontFace title;
  FontFace large;
  FontFace fallback;

  const FontFace &face(FontRole role) const {
    switch (role) {
    case FontRole::Small:
      return small;
    case FontRole::Body:
      return body;
    case FontRole::BodyBold:
      return bodyBold;
    case FontRole::Title:
      return title;
    case FontRole::Large:
      return large;
    case FontRole::Default:
    default:
      return fallback;
    }
  }
};
