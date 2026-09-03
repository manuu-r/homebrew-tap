# Security

## Reporting a vulnerability

Report security issues through a private
[GitHub security advisory](https://github.com/manuu-r/homebrew-tap/security/advisories/new).

## Data handling

- Gauge reads state that already exists on the machine and sends no analytics.
- Gauge never writes, refreshes, or copies the Codex or Claude credentials it
  uses to read quota.
- The Claude OAuth token is read from the macOS keychain (or
  `~/.claude/.credentials.json`) and passed to curl over stdin, so it does not
  appear in `ps` output. It is used only for `GET /api/oauth/usage`.
- Codex usage is read over the local `codex app-server` stdio protocol.
- A Wi-Fi password retrieved from the user's Keychain or entered during
  accessory setup exists only in memory while Gauge sends it through the
  encrypted Bluetooth provisioning session. Gauge does not make another copy
  on the Mac. The ESP32 Wi-Fi stack saves its station configuration in NVS so
  the device can reconnect automatically.
- macOS Location access is used only to let CoreWLAN return the name of the
  currently connected Wi-Fi network. Gauge does not request coordinates or
  start location updates.

## Accessory security

- Gauge has no standalone Wi-Fi/BLE server mode, command-line token, or global
  accessory secret. Local-network sharing exists only inside the running
  menu-bar app and is enabled automatically when the user starts pairing.
- Commissioning uses standard Bluetooth LE Secure Connections with bonding,
  MITM protection, and the DisplayYesNo numeric-comparison association model.
  macOS and the accessory display the same six-digit value and both require
  physical confirmation before Wi-Fi or Gauge credentials cross the encrypted
  link. The accessory's physical input mechanism is outside Gauge's protocol.
- Every paired device gets an independent random 256-bit bearer credential.
  The Mac stores it in Keychain under `dev.gauge.accessory`; the ESP32 stores
  its copy in NVS. Removing a device deletes only that device's Keychain item.
- Runtime authentication accepts exactly one `Authorization: Bearer <token>`
  header. Tokens in query parameters and alternate headers are rejected.
- `/v1/dashboard` is served from the app's cached snapshot and is never
  exposed without a valid paired-device credential. `/health` and
  `/v1/accessory` expose protocol metadata only.
- A compatible accessory should provide an intentional local unpair action
  that clears its bond, Wi-Fi credentials, bearer credential, and dashboard
  cache. When Gauge is reachable, it can use the device credential to revoke
  its Mac-side record as well.
- Runtime HTTP is intended only for a trusted local Wi-Fi network and does not
  provide TLS. Do not forward its port or expose it to the internet. A future
  protocol version should use TLS with device trust anchoring before supporting
  untrusted or routed networks.
