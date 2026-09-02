# Gauge

A native macOS menu-bar app that combines agent quota, Calendar, and a small
to-do list. It can also share that same cached dashboard with displays and desk
robots that you explicitly pair.

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
open "$(brew --prefix gauge)/Gauge.app"
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
./packaging/build-app.sh
open dist/Gauge.app
```

The command-line snapshot remains available as `gauge`, but it is not needed
to run the menu-bar app or its accessory service.

## Menu bar

Gauge keeps the menu-bar title compact with Claude's hourly quota and the
regular Codex quota. Click it to open a narrow native popover with the complete
quota grid, Calendar, and to-dos. The popover automatically refreshes
(two minutes by default); **Refresh**, **Settings…**, and **Quit** stay inline
at the top.

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
  "todos": [
    { "title": "Review pull request", "completed": false },
    { "title": "Send invoice", "completed": false }
  ],
  "accessories": {
    "enabled": false,
    "port": 45831,
    "display_name": "Gauge on this Mac"
  }
}
```

Calendar data is read through macOS EventKit, so it uses the accounts already
configured in the Calendar app (iCloud, Google, Outlook/Exchange, CalDAV, and
so on). Gauge asks macOS for Calendar permission the first time it needs it.
An empty `calendar_names` array includes every available calendar; otherwise
list the exact Calendar-app names to show only those calendars.

To-dos live in the same settings file; click **+ Add to-do…** for a focused
one-field editor, click a task to toggle its strikethrough, or use the adjacent
**Edit** and **×** controls.

## Accessories

Gauge is a generic dashboard host rather than a Bunty controller. Put one
compatible device in pairing mode and click **Pair Accessory…**. Gauge uses the
Mac's current Wi-Fi name and saved password when available; a fallback Wi-Fi
form appears only when macOS cannot provide them. On first use, macOS may ask
for Location access to reveal the current network name and for Keychain access
to its password. Gauge does not request or read physical location data.

Gauge scans for the standard pairing service and reads the device's protected
identity. macOS and the accessory then show the same Bluetooth Secure
Connections number. The accessory decides how its user confirms or rejects
that number—buttons, touch, taps, and other local controls do not leak into
Gauge. Wi-Fi and Gauge credentials cross Bluetooth only after both sides
confirm the standard authenticated link.

Each accessory supplies its stable ID, name, kind, firmware version, and
capabilities. After commissioning, it finds Gauge through Bonjour
(`_gauge._tcp`) and reads `/v1/dashboard` with its own random 256-bit bearer
credential. Gauge stores that credential in macOS Keychain. Devices may render
or cache any supported dashboard sections and must ignore unknown fields.

Gauge starts and advertises the runtime service automatically after pairing
while the app is open. There is no server command to run. The vendor-neutral
wire contract and conformance fixtures are in
[docs/accessory-protocol.md](docs/accessory-protocol.md).

[Bunty](firmware/bunty) is the included ESP32-S3 reference implementation. Its
Flow32 UI, robo eyes, audio, tap gestures, sleep behavior, offline NVS cache,
and IoT Gateway are Bunty features—not Gauge protocol requirements.

For other ESP32 display projects, [firmware/flow32](firmware/flow32) contains a
board-neutral UI library extracted from the upstream Flow32 project. Its
display, canvas, controls, input, storage, asset, persistence, and app-runtime
modules are independently usable; no Gauge or Bunty behavior is built in. Its
lean page kernel can verify a declarative pack on SD and render one page at a
time from caller-owned RAM/PSRAM, so large interface libraries do not have to
live in ESP32 firmware or memory together. Its manifest-driven build profiles
can also export a curated Arduino library containing only the modules a device
uses; see [the Flow32 build-profile guide](firmware/flow32/docs/BUILD_PROFILES.md).

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
./packaging/build-app.sh

cd firmware/bunty
pio run -e bunty
cd test/host && ./run.sh && ./preview.sh
```

Keep changes focused and document any parsing assumptions changed in a PR.
