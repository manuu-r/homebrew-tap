#!/bin/bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
app="${root}/dist/Gauge.app"

cargo build --manifest-path "${root}/Cargo.toml" --release
mkdir -p "${app}/Contents/MacOS" "${app}/Contents/Resources"
cp "${root}/target/release/gauge" "${app}/Contents/MacOS/gauge"
cp "${root}/packaging/Info.plist" "${app}/Contents/Info.plist"
cp "${root}/packaging/Gauge.icns" "${app}/Contents/Resources/Gauge.icns"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${root}/Cargo.toml" | head -1)"
plutil -replace CFBundleShortVersionString -string "${version}" "${app}/Contents/Info.plist"

# Ad-hoc signing gives local builds one stable identity for Bluetooth,
# Calendar, Keychain, and local-network privacy grants. Release builds can set
# GAUGE_SIGNING_IDENTITY to a Developer ID Application identity.
identity="${GAUGE_SIGNING_IDENTITY:--}"
codesign --force --deep --sign "${identity}" "${app}"
echo "Built ${app}"
