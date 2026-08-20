#!/usr/bin/env bash
# Renders the real ui.cpp / emoji.cpp drawing code to preview.png so the layout
# can be checked without flashing hardware. No ESP32 toolchain needed.
set -euo pipefail
cd "$(dirname "$0")"

GFX=https://raw.githubusercontent.com/adafruit/Adafruit-GFX-Library/master
if [ ! -d gfx ]; then
  echo "fetching Adafruit_GFX sources..."
  mkdir -p gfx/Fonts
  for f in Adafruit_GFX.h Adafruit_GFX.cpp gfxfont.h glcdfont.c; do
    curl -sSLf -o "gfx/$f" "$GFX/$f"
  done
  for f in FreeSans9pt7b.h FreeSansBold12pt7b.h FreeSansBold18pt7b.h; do
    curl -sSLf -o "gfx/Fonts/$f" "$GFX/Fonts/$f"
  done
fi

# ARDUINO must be defined on the command line: Adafruit_GFX.h tests it with
# `#if ARDUINO >= 100` before our shim Arduino.h gets a chance to be included.
g++ -std=c++17 -DARDUINO=100 -Ishim -Igfx -I../../include \
    -o preview preview.cpp gfx/Adafruit_GFX.cpp ../../src/emoji.cpp ../../src/ui.cpp

./preview

if command -v magick >/dev/null 2>&1; then
  magick preview.ppm preview.png && rm -f preview.ppm && echo "wrote preview.png"
elif command -v convert >/dev/null 2>&1; then
  convert preview.ppm preview.png && rm -f preview.ppm && echo "wrote preview.png"
else
  echo "install ImageMagick to get a PNG; preview.ppm is written either way"
fi
