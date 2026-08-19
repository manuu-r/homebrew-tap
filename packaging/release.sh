#!/bin/bash
# Point the Homebrew formula at a released tag.
#
#   git tag -a v0.7.0 -m "Gauge 0.7.0" && git push origin v0.7.0
#   ./packaging/release.sh 0.7.0
#
# The tag must be pushed first: GitHub builds the source tarball from it, and
# its checksum cannot be known until it exists.
set -euo pipefail

version="${1:?usage: release.sh <version>}"
tag="v${version}"
formula="$(cd "$(dirname "$0")" && pwd)/gauge.rb"
url="https://github.com/manuu-r/gauge/archive/refs/tags/${tag}.tar.gz"

echo "Fetching ${url}"
checksum="$(curl --fail --silent --show-error --location "${url}" | shasum -a 256 | cut -d' ' -f1)"

/usr/bin/sed -i '' \
  -e "s|/archive/refs/tags/v[^\"]*|/archive/refs/tags/${tag}|" \
  -e "s|^  sha256 \".*\"$|  sha256 \"${checksum}\"|" \
  "${formula}"

echo "Updated ${formula} (sha256 ${checksum})"

tap="$(brew --repository 2>/dev/null)/Library/Taps/manuu-r/homebrew-tap"
if [ -d "${tap}/Formula" ]; then
  cp "${formula}" "${tap}/Formula/gauge.rb"
  echo "Copied into ${tap}/Formula/gauge.rb"
  echo "Commit and push the tap to publish it."
else
  echo "Tap not found at ${tap}; copy ${formula} into it manually."
fi
