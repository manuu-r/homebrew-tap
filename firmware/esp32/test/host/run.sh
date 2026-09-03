#!/usr/bin/env bash
# Exercises the quota parsers on the host. No ESP32 toolchain needed.
set -euo pipefail
cd "$(dirname "$0")"

AJ_VERSION=v7.2.0
if [ ! -f ArduinoJson.h ]; then
  echo "fetching ArduinoJson $AJ_VERSION single header..."
  curl -sSLf -o ArduinoJson.h \
    "https://github.com/bblanchon/ArduinoJson/releases/download/${AJ_VERSION}/ArduinoJson-${AJ_VERSION}.h"
fi

g++ -std=c++17 -I. -Wall -Wextra -Wno-unused-parameter -o test_parse test_parse.cpp
./test_parse
