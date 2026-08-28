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
| `GET` | `/v1/dashboard` | Required |
| `GET` | `/v1/quota` | Required |
| `GET` | `/v1/summary` | Required |
| `GET` | `/health` | None |
| `GET` | `/` | Required |

All responses are JSON. Authenticate with a bearer token, the Gauge header, or
the `token` query parameter:

```sh
curl -H "Authorization: Bearer $GAUGE_API_TOKEN" http://localhost:8080/v1/dashboard
curl -H "Authorization: Bearer $GAUGE_API_TOKEN" http://localhost:8080/v1/quota
curl -H "X-Gauge-Token: $GAUGE_API_TOKEN" http://localhost:8080/v1/summary
curl "http://localhost:8080/v1/quota?token=$GAUGE_API_TOKEN"
```

Headers are preferred because query strings can appear in logs.

`/v1/dashboard` is the universal edge-device endpoint. It uses one protocol,
HTTP with a JSON response, and returns `schema_version`, `generated_at`, the
recommended `refresh_seconds`, detailed quota windows, upcoming Calendar
events, ticker quotes, and the complete to-do list. Event and reset times are
Unix timestamps in seconds. Section-level errors are included in the payload
so a display can keep rendering the other sections when one source is
temporarily unavailable.

### Legacy BLE-style UDP transport

`--ble` is a small UDP text protocol for constrained clients and gateways; it
is not Bluetooth LE GATT. It remains available for the original quota routes,
but the dashboard contract is intentionally HTTP + JSON only.

```sh
GAUGE_API_TOKEN=secret gauge --ble --bind 0.0.0.0 --port 8081
printf 'GET /v1/quota\nAuthorization: Bearer secret\n' | nc -u -w1 127.0.0.1 8081
```

The same paths and authentication forms are supported. Each datagram contains
one request and receives one JSON datagram.

An ESP32 example is available in [firmware/esp32](firmware/esp32).

## Menu bar

`gauge --tray` keeps the menu bar compact with Claude's hourly quota and the
regular Codex quota. Click it to open a narrow native popover with optional
Calendar, stocks, and to-do sections. Click **Gauge** to expand every available
hourly and weekly quota window in place, including Codex Spark. The popover
automatically refreshes (two minutes by default); **Refresh**, **Settings…**,
and **Quit** stay inline at the top.

On its first tray launch, Gauge creates a local settings file at
`~/Library/Application Support/Gauge/config.json`. Open it from **Settings…**
or with:

```sh
gauge --settings
```

```json
{
  "refresh_seconds": 120,
  "calendar": {
    "enabled": true,
    "calendar_names": [],
    "max_events": 1,
    "look_ahead_hours": 24
  },
  "stocks": {
    "enabled": true,
    "symbols": ["^NSEI", "^BSESN"],
    "max_items": 4,
    "quote_url_template": "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?range=1d&interval=1m"
  },
  "todos": [
    { "title": "Review pull request", "completed": false },
    { "title": "Send invoice", "completed": false }
  ]
}
```

Calendar data is read through macOS EventKit, so it uses the accounts already
configured in the Calendar app (iCloud, Google, Outlook/Exchange, CalDAV, and
so on). Gauge asks macOS for Calendar permission the first time it needs it.
An empty `calendar_names` array includes every available calendar; otherwise
list the exact Calendar-app names to show only those calendars.

Stocks default to NIFTY 50 (`^NSEI`) and SENSEX (`^BSESN`), read from the NSE
and BSE index endpoints rather than Yahoo Finance (which frequently rate-limits
menu-bar clients). Additional symbols use the configured HTTPS URL template;
`{symbol}` is replaced per symbol and the default template expects Yahoo's
chart response shape. To-dos live in the same settings file; click **+ Add
to-do…** for a small one-field editor, click a task to toggle its completion,
and click a completed task again to choose **Restore** or **Delete**.

Installing does not start it automatically. To run it at login:

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
