# Packaging

This repository is both the Gauge source and its Homebrew tap. The formula
builds a real `Gauge.app`, ad-hoc signs local/Homebrew builds, and retains a
`gauge` symlink for CLI snapshots. Release distribution should use a Developer
ID Application identity and notarization so Bluetooth, Calendar, Keychain, and
local-network and Wi-Fi-detection permissions stay attached to a stable trusted
app identity.

For a local app bundle:

```sh
./packaging/build-app.sh
open dist/Gauge.app
```

Set `GAUGE_SIGNING_IDENTITY` to a Developer ID Application identity to replace
the script's ad-hoc signature.

## Releasing

```sh
VERSION=0.1.5
git push origin main
git tag -a "v$VERSION" -m "Gauge $VERSION" && git push origin "v$VERSION"
./packaging/release.sh "$VERSION"
git commit -am "Update formula for $VERSION" && git push origin main
```

`release.sh` downloads the tag's source tarball and writes its `sha256` into
`Formula/gauge.rb`, removing bottle metadata from the previous version. That
checksum cannot be computed before the tag is pushed.

After releasing, run the **Build Homebrew bottles** workflow with the new tag.
It builds and tests Apple Silicon and Intel bottles, uploads them to the GitHub
release, and commits their checksums to `Formula/gauge.rb`. Matching bottle
installs skip the build-only Rust and LLVM toolchain.

Because `brew` maps `manuu-r/tap` onto the repo name `homebrew-tap`, users
install with:

```sh
brew install manuu-r/tap/gauge
```

## Checks

```sh
brew style manuu-r/tap
brew audit --formula --strict manuu-r/tap/gauge
brew install --build-from-source manuu-r/tap/gauge
```

`style` and `audit` pass. `install` pulls Homebrew's `rust` as a build
dependency, which is a large download the first time.

## Submitting to homebrew-core

Not currently viable. `brew audit --new` enforces the thresholds in
`Library/Homebrew/utils/shared_audits.rb`: 30 forks, 30 watchers or 75 stars,
tripled to **90 / 90 / 225** for a self-submission, plus 30 days of repo age.
Core also expects upstream to be the project's own repository rather than a tap.

homebrew-cask is not an option at any popularity — `Acceptable-Casks` states
that "open-source command-line-only software normally belongs in homebrew/core
as a formula built from source".
