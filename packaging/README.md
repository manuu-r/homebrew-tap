# Packaging

This repository is both the Gauge source and its Homebrew tap: `Formula/gauge.rb`
builds from a tarball of this repo's own tags. Building from source means no
code signing or notarization is involved.

## Releasing

```sh
git push origin main
git tag -a v0.7.0 -m "Gauge 0.7.0" && git push origin v0.7.0
./packaging/release.sh 0.7.0
git commit -am "Release 0.7.0" && git push origin main
```

`release.sh` downloads the tag's source tarball and writes its `sha256` into
`Formula/gauge.rb`. That checksum cannot be computed before the tag is pushed.

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
