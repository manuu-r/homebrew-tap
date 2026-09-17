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
gauge --stats            # token history and pending requests as JSON
gauge --install-hooks    # connect agent attention events
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
quota grid, Calendar, and to-dos. The popover automatically refreshes;
**Refresh**, **Settings…**, and **Quit** stay inline at the top, and the bottom
line names the paired accessory, or offers to pair one when there is none.


## Token history and attention

The popover shows **Today / Week / Month** token totals for each enabled agent.
**Usage history & breakdown…** opens a native, scrollable window with exact
input, output, cache-read, cache-write, and request counts for today, this
calendar week (Monday onward), this calendar month, and all available history.
Daily rows break usage down by provider and model, including reasoning tokens.
Dates use this Mac's local timezone. `gauge --stats` exports the complete data as
JSON, including daily cache counters and attention requests.

Usage is read locally from Codex's `sessions` and `archived_sessions` folders
and Claude Code's `projects` folder, including subagent logs. Gauge honors
`CODEX_HOME` and `CLAUDE_CONFIG_DIR` when those variables are present in its
launch environment. These are recorded tokens on this Mac, not account-wide
billing totals: deleted logs, remote sessions without local logs, and activity
in other Claude products are excluded. Input includes cache reads and writes;
reasoning is a subset of output. Neither is added a second time to the total.

The initial history scan runs in the background. Afterward Gauge checks for
new token records every 15 seconds and reads appended data incrementally.
It deduplicates Claude message/request IDs and modern Codex response IDs;
older Codex logs use deltas between cumulative usage snapshots. Mirrored Codex
quota updates do not count again. Transcript formats can change; unreadable
records appear as read issues in the detail window. An incomplete final line
is deferred until its newline arrives. Legacy logs without stable request IDs
provide approximate request counts, and legacy forks with rewritten history
may not be fully deduplicated.

### Connect attention alerts once

Open Gauge's popover and click **Enable attention monitoring…**, or run:

```sh
gauge --install-hooks
```

Setup adds Gauge's command hooks to `~/.codex/hooks.json` and
`~/.claude/settings.json`, preserving other settings and hooks and saving
backup files alongside them. It uses the executable performing setup, so run
setup from your installed app or stable binary location. Restart Claude Code
sessions after setup. In Codex, open **`/hooks`** and review/trust the Gauge
hooks; untrusted hooks do not run. Older agent versions without these hook
events need an update. Run setup again after moving the Gauge executable.

When an agent requests approval or calls a supported user-input tool, the
menu bar gains a **● N need attention** badge. The popover previews the latest
requests; **Details…** shows every request, its project, session ID, and text.
Respond in the original agent session. Hook events are checked every two
seconds, independently of the token history scan. Existing quota refreshes
can temporarily delay the UI while the provider is being contacted.

Gauge listens to `PermissionRequest` and to `PreToolUse` for Claude's
`AskUserQuestion` / `ExitPlanMode` and Codex's `request_user_input` tools.
The installer uses a conservative Claude hook set; newer MCP elicitation and
permission-denial events are not installed. Requests clear on a matching
tool completion/failure, a new user prompt, or session/turn termination.
An approval may stay visible while its approved tool is still running.
Asynchronous questions clear on the next user prompt or turn termination.
Ordinary questions written only in assistant prose are not inferred from text.
Sessions killed without a final hook can leave a stale entry; entries expire
after 24 hours. Hooks must run in the agent host to observe it—this does not
scrape hidden tabs or read prompts from unrelated apps.

Events are stored with owner-only permissions in Gauge's Application Support
`attention` folder. The handler emits no approval decisions and always allows
the normal agent approval flow to continue. Turning off a provider hides its
stats and requests and stops reading its transcripts; installed hooks remain
in the agent configuration. To uninstall monitoring, remove the handler entries
whose commands end in `gauge' --hook codex` or `gauge' --hook claude` from those
two hook files, preserving any other handlers. Token and attention details are
local to Gauge and are not added to the accessory dashboard protocol.

Integration references: [Codex hooks](https://learn.chatgpt.com/docs/hooks)
and [Claude Code hooks](https://code.claude.com/docs/en/hooks).

## Settings

**Settings…** opens a single window. Every control is a switch on one visible
fact, applied the moment it is set — there is no Apply button and no second
level to go looking in.

| Section | Controls |
| --- | --- |
| Updates | How often Gauge refreshes; Open at login |
| Show | Codex quota, Claude quota, next calendar event, today's tasks |
| Accessories | Share this dashboard; every paired device, with Forget; Pair Accessory… |

Switching a provider off removes it from the menu bar, the popover, and every
paired accessory, and stops Gauge contacting it at all. Open at login writes a
plain LaunchAgent at `~/Library/LaunchAgents/dev.gauge.tray.plist`; removing the
switch removes the file.

Settings are stored at `~/Library/Application Support/Gauge/config.json`.
The window covers everything except the fields it deliberately leaves to the
file — which calendars to include, and the accessory port and service name.
Open it with **Configuration file…** in the settings window, or with:

```sh
gauge --settings
```

```json
{
  "refresh_seconds": 120,
  "providers": { "codex": true, "claude": true },
  "calendar": {
    "enabled": true,
    "calendar_names": [],
    "max_events": 1,
    "look_ahead_hours": 24
  },
  "tasks": { "enabled": true },
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

Gauge is a generic dashboard host rather than a Bunty controller. Put a
compatible device in pairing mode and click **Pair Accessory…**. Gauge uses the
Mac's current Wi-Fi name and saved password when available; a fallback Wi-Fi
form appears only when macOS cannot provide them. On first use, macOS may ask
for Location access to reveal the current network name and for Keychain access
to its password. Gauge does not request or read physical location data.

Gauge then lists every accessory it can hear, nearest first, and connects to the
one that is chosen — never to whatever answered first. Paired devices stay
visible in Settings with what each one is and when it last read the dashboard,
and **Forget** revokes its credential.

Pairing over Bluetooth is authenticated but deliberately not bonded: it is a
single act that hands over Wi-Fi and a bearer token, after which the accessory
talks to Gauge over the LAN. Nothing is meant to outlive it on either side. An
accessory should keep one Bluetooth identity for its lifetime; minting a new one
per attempt fills the Mac's Bluetooth list with dead entries.

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

Bunty, an ESP32-S3 reference implementation, lives in the separate
`bunty-firmware` repository (firmware plus its IoT Gateway). Its
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
```

Keep changes focused and document any parsing assumptions changed in a PR.
