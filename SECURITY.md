# Security

## Reporting a vulnerability

Report security issues through a private
[GitHub security advisory](https://github.com/manuu-r/homebrew-tap/security/advisories/new).

## Data handling

- Gauge reads state that already exists on the machine and sends no analytics.
- It never writes, refreshes, or stores credentials.
- The Claude OAuth token is read from the macOS keychain (or
  `~/.claude/.credentials.json`) and passed to curl over stdin, so it does not
  appear in `ps` output. It is used only for `GET /api/oauth/usage`.
- Codex usage is read over the local `codex app-server` stdio protocol.

## Network API security

- Network modes (`--wifi` and `--ble`) are disabled from startup without
  `GAUGE_API_TOKEN` unless an explicit `--token` is passed. Prefer the
  environment variable because command-line arguments may appear in process
  listings.
- Authentication accepts:
  - `Authorization: Bearer <token>`
  - `X-Gauge-Token: <token>`
  - `token=<...>` query parameter (headers are preferred to avoid logging tokens)
- Bind to a trusted interface and use host firewall rules. The built-in HTTP
  server does not provide TLS and should not be exposed directly to the internet.
