# Packaging

`gauge.rb` is the Homebrew formula. It builds from source, so it needs no code
signing or notarization, and the same file serves the tap and any future
homebrew-core submission.

## Releasing

```sh
git push origin main
git tag -a v0.7.0 -m "Gauge 0.7.0" && git push origin v0.7.0
./packaging/release.sh 0.7.0
```

`release.sh` downloads the tag's source tarball, writes its `sha256` into
`gauge.rb`, and copies the result into the local tap. Commit and push the tap
and `brew install manuu-r/tap/gauge` works.

The tap lives at `$(brew --repository)/Library/Taps/manuu-r/homebrew-tap` and
pushes to `manuu-r/homebrew-tap` — create that repo on GitHub before the first
push. The repo name must start with `homebrew-`; `brew` maps `manuu-r/tap` to
it automatically.

## Checks

```sh
brew style manuu-r/tap
brew audit --formula --strict manuu-r/tap/gauge
brew install --build-from-source manuu-r/tap/gauge
```

`audit` and `style` both pass. `install` pulls Homebrew's `rust` as a build
dependency, which is a large download the first time.

## Submitting to homebrew-core

Not yet eligible. `brew audit --new` enforces the thresholds in
`Library/Homebrew/utils/shared_audits.rb`: 30 forks, 30 watchers or 75 stars,
tripled to **90 / 90 / 225** because submitting your own project counts as a
self-submission. The repository must also be at least 30 days old.

homebrew-cask is not an option regardless of popularity — `Acceptable-Casks`
states that "open-source command-line-only software normally belongs in
homebrew/core as a formula built from source".

When the thresholds are met:

```sh
brew tap --force homebrew/core
brew bump-formula-pr --url <tarball-url> --sha256 <checksum> gauge
```

Note that `depends_on :macos` makes this a macOS-only formula, which core
accepts but reviewers will ask about.
