# Gauge

A tiny Rust CLI that shows how much agent quota you have left and can expose it
to trusted devices on your local network.

```sh
$ gauge
Codex 20%, Claude 43%
```

The number is the tightest window per provider, so `Claude 43%` means 43% of
your most constrained limit remains.

## Where the numbers come from

| Provider | Source |
| --- | --- |
| Codex | `codex app-server --stdio`, JSON-RPC `account/rateLimits/read` |
| Claude | `GET /api/oauth/usage`, the endpoint behind Claude Code's `/usage` screen |

Gauge is read-only. It reuses the OAuth token Claude Code stored at sign-in
(macOS keychain or `~/.claude/.credentials.json`) and never writes, refreshes,
or stores credentials. The token is passed to curl over stdin, so it never
appears in `ps` output. If it has expired, start `claude` once.

Gauge requires macOS, `codex` on `PATH`, and `curl`.

## Install

```sh
brew install manuu-r/tap/gauge
```

This repository doubles as the Homebrew tap.

## Usage

```sh
gauge                    # one-line summary
gauge --json             # machine-readable snapshot
gauge --tray             # menu bar utility
gauge --version          # print version
```

To build from source:

```sh
cargo build --release
```

## Network API

Network modes require `GAUGE_API_TOKEN` or `--token TOKEN`. Prefer the
environment variable because command-line arguments may be visible to other
local processes.

Start the HTTP server:

```sh
GAUGE_API_TOKEN=secret gauge --wifi --bind 0.0.0.0 --port 8080
```

| Method | Path | Authentication |
| --- | --- | --- |
| `GET` | `/v1/quota` | Required |
| `GET` | `/v1/summary` | Required |
| `GET` | `/health` | None |
| `GET` | `/` | Required |

All responses are JSON. Authenticate with a bearer token, the Gauge header, or
the `token` query parameter:

```sh
curl -H "Authorization: Bearer $GAUGE_API_TOKEN" http://localhost:8080/v1/quota
curl -H "X-Gauge-Token: $GAUGE_API_TOKEN" http://localhost:8080/v1/summary
curl "http://localhost:8080/v1/quota?token=$GAUGE_API_TOKEN"
```

Headers are preferred because query strings can appear in logs.

### BLE-style UDP transport

`--ble` is a small UDP text protocol for constrained clients and gateways; it
is not Bluetooth LE GATT.

```sh
GAUGE_API_TOKEN=secret gauge --ble --bind 0.0.0.0 --port 8081
printf 'GET /v1/quota\nAuthorization: Bearer secret\n' | nc -u -w1 127.0.0.1 8081
```

The same paths and authentication forms are supported. Each datagram contains
one request and receives one JSON datagram.

An ESP32 example is available in [firmware/esp32](firmware/esp32).

## Menu bar

`gauge --tray` keeps the menu bar compact with Claude's hourly quota and the
regular Codex quota. Open it to see every available hourly and weekly window,
including Codex Spark. Installing does not start it automatically. To run it at
login:

```sh
brew services start gauge
```

Quitting from the menu stays quit; launchd only relaunches it after a crash.
`brew services stop gauge` disables it entirely.

## Tests

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

Keep changes focused and document any parsing assumptions changed in a PR.
