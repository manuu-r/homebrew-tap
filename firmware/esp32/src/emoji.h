#pragma once
#include <Adafruit_GFX.h>
#include "mood.h"

// Draws the mood's icon centred on (cx, cy) inside an s x s box.
// Everything is vector-drawn from GFX primitives - no bitmaps, no emoji font.
void drawMoodIcon(Adafruit_GFX &g, Mood m, int cx, int cy, int s);
