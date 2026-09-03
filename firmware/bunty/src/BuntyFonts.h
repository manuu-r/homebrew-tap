#pragma once

#include <cstdint>
#include <gfxfont.h>

// Keep every bitmap font in one translation unit so its glyph data is linked
// exactly once.
namespace BuntyFonts {

extern const GFXfont *const kBody;
extern const GFXfont *const kHeading;
extern const GFXfont *const kComparison;

}  // namespace BuntyFonts
