# Bunty ESP32-S3 firmware

Bunty is the ESP32-S3 reference implementation of the vendor-neutral
[Gauge Accessory Protocol](../../docs/accessory-protocol.md). It pairs using
standard Bluetooth LE Secure Connections, joins Wi-Fi, fetches the Gauge
dashboard, and retains the last valid snapshot for offline display. Flow32,
eyes, audio, taps, sleep, and the optional IoT Gateway belong to Bunty rather
than the protocol.

## Pairing

1. An unpaired Bunty advertises its Gauge GATT service and shows **PAIR
   BUNTY**.
2. Click **Pair Accessory…** in the Mac app.
3. macOS and Bunty show the same six-digit numeric-comparison value.
4. Confirm on the Mac, then tap Bunty once. Two taps on Bunty reject pairing.
5. Gauge sends Wi-Fi and its per-device dashboard credential over the bonded,
   MITM-protected link.
6. Bunty joins Wi-Fi, says hello, and begins showing Gauge data.

Keep only the intended compatible accessory in pairing mode. Gauge deliberately
commissions one candidate at a time and reports an error if several are
advertising the protocol service.

## Tap controls

The MEMS microphone is sampled continuously on `I2S_NUM_1`, independently of
network requests and screen rendering. The adaptive detector logs each accepted
impulse and its current noise floor over serial.

- During numeric comparison: one tap confirms, two taps reject.
- While viewing the dashboard: three **firm, evenly spaced** taps request
  unpairing. Soft impulses, fast echoes, and irregular timing are ignored.
- On the unpair screen: pause for one second, then make one more firm tap to
  confirm. Waiting six seconds cancels.
- After two minutes without interaction, the stats give way to Bunty's dimmed
  sleeping emotion at roughly 3% backlight brightness. One firm tap restores
  full brightness, plays the wake emotion, and returns to the latest dashboard
  page.

Confirmed unpairing asks an available Gauge Mac to revoke the device token,
then clears local Wi-Fi, pairing, bond, and dashboard state. Bunty rotates its
static random BLE address before advertising again so a stale macOS bond cannot
block repair. Tap sensitivity and debounce constants live in `src/TapInput.cpp`
and should be calibrated on the assembled enclosure.

## Dashboard and offline behavior

On every paired boot, Bunty reconnects with the saved Wi-Fi credentials and
uses Bonjour to find the Gauge service whose server ID matches the pairing
bundle. It requests `/v1/dashboard` with its per-device bearer token, validates
the protocol/schema, and stores the complete response in NVS only when it
changes. The display rotates through:

- AI quota overview;
- each provider's individual limit windows;
- upcoming Calendar events;
- to-dos.

When Wi-Fi or the Mac is unavailable, the last validated snapshot remains on
screen as **CACHED**. Failed discovery/fetch attempts retry every 15 seconds;
successful refreshes honor Gauge's interval, clamped to 30–900 seconds.
Display sleep does not stop Wi-Fi or dashboard refreshes, so the cached data is
kept current while the stats are hidden. Bunty disables ESP32 modem sleep for
this always-on LAN connection, refreshes Bonjour after a failed socket, and
reconnects its Wi-Fi station after three consecutive connection failures.

## Build and flash

The gateway is not required for ordinary builds or uploads. Once the gateway is
live, install its tooling and authenticate Wrangler before the first
**provisioned** flash:

```sh
cd iot-gateway
npm install
npx wrangler login
npm run db:migrate:remote
```

Ensure `iot-gateway/wrangler.jsonc` contains the real remote D1
`database_id`. Provisioning is deliberately opt-in. An ordinary upload performs
no Cloudflare or network operations and leaves an existing NVS credential
untouched:

```sh
cd ../firmware/bunty
pio run -e bunty --target upload
```

When the IoT Gateway is live, request a provisioned flash explicitly:

```sh
BUNTY_PROVISION_IOT=1 pio run -e bunty --target upload
```

Only the provisioned form:

1. stages a new random token for the D1 device named `bunty`;
2. embeds it into the firmware from Bunty's ignored `.pio/` directory;
3. flashes the board;
4. activates the new token after a successful upload.

The previous token remains valid if compilation or upload fails. If the flash
succeeds but the final activation call fails, the staged token is still
accepted. PlatformIO leaves a non-secret marker under `.pio/`, retries that
activation before the next flash, and also prints a manual recovery command.

Normal non-upload builds are also offline:

```sh
pio run -e bunty
pio device monitor --baud 115200
```

At boot, Bunty copies the compiled credential into its `bunty` NVS namespace
under `iot_token` and records `iot_device=bunty`. This credential is separate
from the local Gauge dashboard token stored under `token`; pairing and dashboard
authentication are unchanged. The plaintext IoT credential is never written to
the repository or printed during the flash, but it is present in the firmware
image and on the device. Use ESP32 flash encryption or a secure element if
physical extraction is in scope.

The cloud device ID is intentionally fixed to `bunty` for now. Before flashing
more than one physical Bunty, change the hook to derive a stable unique ID per
board; otherwise the most recent flash will rotate the credential used by the
previous board.

For a manual factory reset, clear the NVS partition. `--target erase` wipes the
whole chip and requires a full reflash:

```sh
esptool.py --chip esp32s3 erase_region 0x9000 0x5000
```

## Audio

`src/bunty_audio.pcm` is `bunty_audio.mp3` pre-decoded to 22.05 kHz mono
16-bit, embedded by `src/bunty_audio.S` and streamed to the MAX98357 on
`I2S_NUM_0`. Shipping decoded samples keeps playback to a plain I2S write loop
instead of an MP3 decoder, at the cost of roughly 360 KB of flash. The tap
detector drains and suppresses microphone events during playback so the speaker
cannot trigger an accidental unpair gesture.

## Display

Rendering goes through Flow32's `Display`, an `Adafruit_GFX` that paints into an
off-screen framebuffer and presents complete frames without tearing. The
framebuffer is allocated before Wi-Fi and Bluetooth start because this board
has no PSRAM. Speaking, sleeping, and waking frames live in
`src/BuntyAnimations.*`, separate from pairing and dashboard logic in
`main.cpp`. Emotions use only chunky cyan robo-eye silhouettes rendered with
Flow32's anti-aliased rounded shapes; sleep adds a small drifting `ZZZ`.
Dashboard pages remain visible for 11 seconds before rotating.

## Hardware map

The SmartElex 2-inch 240x320 IPS panel uses its ST7789P3 controller in
hardware-SPI mode. Connect the module's `EN` pin to `TFT_BL`.

| Function | GPIO |
| --- | ---: |
| TFT CS | 10 |
| TFT DC | 9 |
| TFT RST | 14 |
| TFT EN / backlight | 21 |
| TFT MOSI / SDA | 11 |
| TFT SCLK / SCL | 12 |
| Turn / pan servo signal | 7 |
| Up/down tilt servo signal | 8 |
| MAX98357 SD | 13 |
| MAX98357 BCLK | 4 |
| MAX98357 LRC | 5 |
| MAX98357 DIN | 6 |
| MEMS mic BCLK | 15 |
| MEMS mic WS | 16 |
| MEMS mic DATA | 17 |

The display, amplifier, microphone tap detector, BLE pairing, Wi-Fi runtime,
dashboard client, and GPIO 7 turn servo are active. GPIO 8 servo control and
OTA are not implemented.

Power the servo from a suitable 5 V supply rather than the ESP32's 3.3 V rail,
and connect the servo supply ground to ESP32 ground. The GPIO 7 home angle is
the `kTurnServoHomeAngle` constant in `src/main.cpp`; it is currently calibrated
to 30° to correct a 60° clockwise offset from the 90° midpoint.
