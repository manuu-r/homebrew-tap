# Gauge Accessory Protocol v1

Gauge is a dashboard host. An accessory is an independent BLE + Wi-Fi device
that implements this document. Bunty is one reference implementation; its
display library, animations, audio, tap controls, sleep behavior, cloud gateway,
and storage choices are not part of the protocol.

The protocol has two independent phases:

1. authenticated Bluetooth commissioning; and
2. authenticated dashboard reads over the local network.

JSON objects are extensible. A v1 implementation must ignore unknown fields and
must reject an unknown value in a `protocol`, `version`, or `schema_version`
field it depends on.

## Bluetooth commissioning

An uncommissioned accessory advertises this primary GATT service:

| Purpose | UUID | Required access |
| --- | --- | --- |
| Pairing service | `c9cce6f3-bf10-4e6d-b719-f32911bbba89` | primary service |
| Identity | `1db9634c-a20f-43c6-8ed9-69ceda338178` | read, encrypted + MITM |
| Configuration | `289be295-d110-411b-888c-c80a601177fa` | write, encrypted + MITM |
| Status | `e763eccb-fa4c-4e3a-9211-850513371105` | read, encrypted + MITM |

The accessory and macOS use standard Bluetooth LE Secure Connections,
bonding, and the DisplayYesNo numeric-comparison association model. Gauge
triggers the system confirmation sheet by reading Identity; it does not define
a PIN or implement a second confirmation UI.

The accessory must show the same six-digit comparison value and provide a
physical accept/reject interaction. That interaction is device-defined: buttons,
touch, taps, or another local input are all valid and are invisible to Gauge.

### Identity

After successful numeric comparison, Identity returns UTF-8 JSON. New devices
must provide this shape:

```json
{
  "protocol": "dev.gauge.pairing",
  "version": 1,
  "device_id": "acme-desk-a1b2c3",
  "name": "Acme Desk Display",
  "kind": "display",
  "firmware_version": "2.3.1",
  "capabilities": ["dashboard.pull", "dashboard.cache", "display"]
}
```

| Field | Requirement |
| --- | --- |
| `protocol` | must be `dev.gauge.pairing` |
| `version` | must be `1` |
| `device_id` | stable across bond resets; 1–64 ASCII letters, digits, `.`, `-`, or `_` |
| `name` | human-readable device name; 1–80 printable characters |
| `kind` | vendor-neutral category such as `display`, `clock`, or `light`; 1–80 printable characters |
| `firmware_version` | optional printable firmware release identifier |
| `capabilities` | lowercase capability identifiers, each at most 48 characters |

Gauge still accepts early v1 identities without `name`, `kind`, or
`firmware_version`; it falls back to the BLE advertisement name and an inferred
kind. New implementations should not rely on that compatibility path.

`device_id` identifies the physical accessory in Gauge's registry. Re-pairing
the same ID replaces only that accessory's bearer credential. It must therefore
be unique and derived from stable device storage or hardware identity.

Standard capability identifiers are:

| Capability | Meaning |
| --- | --- |
| `dashboard.pull` | fetches Gauge dashboard snapshots; required for dashboard accessories |
| `dashboard.cache` | retains the last valid snapshot while Gauge is unavailable |
| `display` | renders information locally |
| `audio.output` | can produce audio |

Unknown capabilities are metadata and must not change authentication. Vendor
extensions should use a stable namespace, for example `com.example.haptics`.

The complete example is
[`fixtures/accessory-identity-v1.json`](fixtures/accessory-identity-v1.json).

### Configuration framing

Gauge splits the commissioning JSON into writes that work at the minimum ATT
MTU. Each GATT value is at most 20 bytes:

| Byte | Meaning |
| ---: | --- |
| `0` | frame marker `0x47` |
| `1` | zero-based sequence number |
| `2` | total frame count |
| `3…19` | up to 17 JSON bytes |

Frames must arrive in order. A message contains 1–255 frames. A receiver must
discard the partial message on a bad marker, count, sequence, overflow, or new
connection.

The reassembled document is:

```json
{
  "protocol": "dev.gauge.commission",
  "version": 1,
  "wifi": {
    "ssid": "Example Network",
    "password": "network password"
  },
  "accessory": {
    "protocol": "dev.gauge.accessory",
    "version": 1,
    "device_id": "acme-desk-a1b2c3",
    "server_id": "stable-random-gauge-id",
    "bearer_token": "64-lowercase-hex-characters",
    "service_type": "_gauge._tcp.local.",
    "dashboard_path": "/v1/dashboard",
    "server_port": 45831
  }
}
```

The accessory must verify that `accessory.device_id` equals the ID it exposed
through Identity before saving anything. Wi-Fi credentials, Gauge identity, and
the per-device bearer token are sensitive and must be stored as device secrets.

The complete example is
[`fixtures/commission-v1.json`](fixtures/commission-v1.json).

### Status

Status returns one of these UTF-8 values:

- `ready` — configuration can be sent;
- `receiving` — framed configuration is incomplete;
- `joining` — configuration was accepted and Wi-Fi is connecting;
- `connected` — commissioning completed; or
- `error:<reason>` — setup failed.

Gauge considers pairing complete only after `connected`.

## Local-network runtime

After joining Wi-Fi, the accessory discovers Gauge using DNS-Based Service
Discovery (Bonjour/mDNS):

- service type `_gauge._tcp.local.`;
- TXT `protocol=dev.gauge.accessory`;
- TXT `api=1`;
- TXT `id=<commissioned server_id>`;
- TXT `path=/v1/dashboard`;
- TXT `auth=bearer`; and
- TXT `pair=ble-sc-numeric-v1`.

The accessory must select the service whose `id` matches its commissioned
`server_id`. It must use the discovered address and port instead of persisting
the Mac's current IP address.

### HTTP

The dashboard request is HTTP/1.1 over the trusted local network:

```http
GET /v1/dashboard HTTP/1.1
Authorization: Bearer <per-device-token>
Accept: application/json
Connection: close
```

The bearer token is never valid in a URL, query string, or alternate header.
Gauge exposes these endpoints:

| Method and path | Authentication | Purpose |
| --- | --- | --- |
| `GET /health` | public | host and API liveness |
| `GET /v1/accessory` | public | non-secret service metadata |
| `GET /v1/dashboard` | bearer | current cached dashboard snapshot |
| `DELETE /v1/accessory` | bearer | revoke only the calling accessory |

Every paired accessory receives a separate random 256-bit bearer token.

### Dashboard

A valid response has `protocol=dev.gauge.dashboard`, `schema_version=1`, a Unix
`generated_at`, a requested `refresh_seconds`, and independently renderable
`quota`, `calendar`, and `todos` sections. Consumers must ignore unknown fields
and may ignore sections they do not render.

The complete reference response is
[`fixtures/dashboard-v1.json`](fixtures/dashboard-v1.json). Dashboard clients
should replace their cached state only after parsing and validating a complete
new response. Offline caching is recommended but not required.

### Unpairing

An accessory should expose an intentional local unpair action. The physical UX
is not defined here. When possible, it first sends authenticated
`DELETE /v1/accessory`, then clears its bond, Wi-Fi credentials, Gauge identity,
bearer token, and cached dashboard. Gauge can also revoke a device independently.

An uncommissioned accessory must not silently reuse a BLE bond left by an
interrupted setup. Before starting a new pairing session it must clear stale
link keys and, when it advertises with a persistent private address, rotate
that address. This ensures the host performs Secure Connections numeric
comparison again instead of reconnecting with a bond that no longer represents
a valid Gauge credential.

## Conformance checklist

A compatible accessory:

1. advertises the exact pairing service and characteristic UUIDs;
2. enforces encrypted MITM access on all sensitive characteristics;
3. completes standard numeric comparison on both devices;
4. returns a valid stable Identity document;
5. rejects malformed, reordered, mismatched, or oversized configuration frames;
6. saves configuration only after validating every protocol field and its own
   `device_id`;
7. reports `connected` only after joining Wi-Fi;
8. selects Gauge by the commissioned Bonjour server ID;
9. sends its bearer token only in the Authorization header; and
10. validates the dashboard protocol and schema before rendering or caching it.

Gauge runs the checked-in fixtures through `cargo test --test
accessory_protocol`. Third-party implementations can use those same JSON files
as interoperability vectors.

## Security boundary

Runtime HTTP v1 does not provide TLS and is intended only for a trusted local
Wi-Fi network. Do not expose the Gauge port to the internet. BLE Secure
Connections protects commissioning in transit, but accessories remain
responsible for protecting secrets at rest. A future protocol version should
add authenticated TLS before supporting untrusted or routed networks.

## Reference implementation

[Bunty](../firmware/bunty) implements this protocol on an ESP32-S3. It is an
example client, not a Gauge dependency and not the definition of the protocol.
