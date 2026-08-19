# Gauge

A tiny Rust CLI that shows how much agent quota you have left.

```sh
$ gauge
Agent quota: Codex 20%, Claude 43%
```

The number is the *tightest* window per provider, so `Claude 43%` means 43% of
your most constrained limit is still available.

## Where the numbers come from

| Provider | Source |
| --- | --- |
| Codex | `codex app-server --stdio`, JSON-RPC `account/rateLimits/read` |
| Claude | `GET /api/oauth/usage`, the endpoint behind Claude Code's `/usage` screen |

Gauge is read-only. It reuses the OAuth token Claude Code stored at sign-in
(macOS keychain, or `~/.claude/.credentials.json`) and never writes, refreshes,
or stores credentials. The token is passed to curl over stdin, so it never
appears in `ps` output. If the token has expired, start `claude` once.

Requires macOS, `codex` on `PATH`, and `curl`.

## Install

```sh
brew install manuu-r/tap/gauge
```

This repository doubles as the Homebrew tap, so `manuu-r/tap` and the source
live in the same place.

## Build and run

```sh
cargo build --release

./target/release/gauge          # one-line summary
./target/release/gauge --json   # every window, machine-readable
./target/release/gauge --tray   # keep it in the menu bar
```

## Tests

```sh
cargo test
```

## Contributing

Keep changes focused, run `cargo test` and `cargo fmt`, and say in the PR which
parsing assumptions you changed.
