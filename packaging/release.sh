#!/bin/bash
# Cut a release and refresh the formula's sha256.
#
#   ./packaging/release.sh 0.7.0
#
# Tag and push first; GitHub generates the source tarball from the tag, and its
# checksum cannot be known until it exists.
set -euo pipefail

version="${1:?usage: release.sh <version>}"
tag="v${version}"
formula="$(dirname "$0")/gauge.rb"
url="https://github.com/manuu-r/gauge/archive/refs/tags/${tag}.tar.gz"

if ! git rev-parse "${tag}" >/dev/null 2>&1; then
  echo "Tag ${tag} does not exist locally. Create it with:" >&2
  echo "  git tag -a ${tag} -m 'Gauge ${version}' && git push origin ${tag}" >&2
  exit 1
fi

echo "Fetching ${url}"
checksum="$(curl --fail --silent --show-error --location "${url}" | shasum -a 256 | cut -d' ' -f1)"

/usr/bin/sed -i '' \
  -e "s|/archive/refs/tags/v[^\"]*\.tar\.gz|/archive/refs/tags/${tag}.tar.gz|" \
  -e "s|^  sha256 \".*\"$|  sha256 \"${checksum}\"|" \
  "${formula}"

echo "Updated ${formula}"
echo "  version ${version}"
echo "  sha256  ${checksum}"
echo
echo "Now copy it into the tap:"
echo "  cp ${formula} ../homebrew-tap/Formula/gauge.rb"
