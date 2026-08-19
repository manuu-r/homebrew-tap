#!/bin/bash
# Point Formula/gauge.rb at a released tag.
#
#   git tag -a v0.1.1 -m "Gauge 0.1.1" && git push origin v0.1.1
#   ./packaging/release.sh 0.1.1
#
# The tag must be pushed first: GitHub builds the source tarball from it, and
# its checksum cannot be known until it exists.
set -euo pipefail

version="${1:?usage: release.sh <version>}"
tag="v${version}"
root="$(cd "$(dirname "$0")/.." && pwd)"
formula="${root}/Formula/gauge.rb"
url="https://github.com/manuu-r/homebrew-tap/archive/refs/tags/${tag}.tar.gz"

echo "Fetching ${url}"
checksum="$(curl --fail --silent --show-error --location "${url}" | shasum -a 256 | cut -d' ' -f1)"

/usr/bin/sed -i '' \
  -e "s|^  url \".*\"$|  url \"${url}\"|" \
  -e "s|^  sha256 \".*\"$|  sha256 \"${checksum}\"|" \
  "${formula}"

# Bottle checksums belong to the previous version until CI rebuilds them.
/usr/bin/sed -i '' '/^  bottle do$/,/^  end$/d' "${formula}"

echo "Updated Formula/gauge.rb (sha256 ${checksum})"
echo "Commit and push it, then: brew install manuu-r/tap/gauge"
