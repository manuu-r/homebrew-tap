# ESP32 Gauge display client

Example firmware that turns a Gauge server into a desk gadget: an ESP32 finds
Gauge on the local network, polls `/v1/quota` once a minute, and shows how
hungry your agents are on a 240x320 ST7789 panel.

| Environment | Source | Purpose |
| --- | --- | --- |
| `display` | `src/display_client.cpp` | HTTP over Wi-Fi, ST7789 output |
| `displaytest` | `src/display_test.cpp` | Panel bring-up diagnostic |

The UDP ("BLE-style") transport served by `gauge --ble` on port 8081 is **not**
implemented here. Follow the protocol notes in the repo root `README.md` if you
want to add a second environment for it.

## Wiring

| Signal | GPIO |
| --- | --- |
| `CS` | 5 |
| `DC` | 4 |
| `RST` | 15 |
| `MOSI` | 18 |
| `SCLK` | 2 |

### SPI: this build uses software SPI

`TFT_USE_HW_SPI` in `src/config.h` is **0**, so the panel is driven by
bit-banged SPI rather than the HSPI peripheral.

That is not arbitrary. On this pin set, hardware SPI does not drive the panel at
any mode or clock — `SCLK` is on **GPIO2**, a strapping pin with the devkit's
onboard LED attached, and it does not route cleanly through the SPI peripheral.
Software SPI on the identical pins works. `src/display_test.cpp` is the
diagnostic that established this; it walks software SPI, then hardware SPI
across `SPI_MODE0`/`SPI_MODE3` and 10/40 MHz, labelling each attempt on screen.

The cost is irrelevant here: the dashboard repaints once a minute and only the
regions that changed. If you move `SCLK` to a native HSPI pin (**GPIO14**), set
`TFT_USE_HW_SPI` to 1 to get the fast path, and `TFT_SPI_HZ` starts mattering.

If your breakout's `BLK`/`LED` backlight pin is wired to a GPIO rather than
3V3, set `PIN_TFT_BLK` and the firmware will drive it high at boot.

Servo headers are reserved in `src/config.h` (`left 22`, `right 23`,
`middle 12`) but nothing drives them yet.

### Strapping-pin warnings

Three of these are ESP32 strapping pins, read once at reset:

- **GPIO2 (SCLK)** must be low or floating at boot. It also carries the onboard
  LED on most devkits, which is why hardware SPI does not work on this pin —
  see the SPI note above.
- **GPIO15 (RST)** must be high at boot.
- **GPIO12 (servo middle)** must be **low** at boot. It selects the flash
  voltage, and a servo lead with a pull-up here can leave the board unable to
  boot. Keep it disconnected until you actually wire the servos up.

## Configure

```sh
cd firmware/esp32
cp include/gauge_config.example.h include/gauge_config.h
```

Set the Wi-Fi credentials and the API token in `include/gauge_config.h`. That
file is gitignored. Note that these are compiled into the image in plaintext —
anyone who can `esptool read_flash` the board can recover them.

`kAutoDiscover` (default on) sweeps the local /24 for an unauthenticated
`GET /health` on `kHttpPort` and caches the winning address in NVS, so the
display survives Gauge picking up a new DHCP lease. `/health` is the only route
Gauge serves without a token, which is exactly what makes it a usable probe.
Set `kAutoDiscover = false` to pin `kGaugeHost` instead.

`kGaugeToken` must match `GAUGE_API_TOKEN` on the server, or `/v1/quota`
returns 401 and the screen sits on `TOKEN?`.

## Build and upload

```sh
pio run -e display
pio run -e display --target upload
pio device monitor --baud 115200
```

## Start Gauge

```sh
GAUGE_API_TOKEN=secret gauge --wifi --bind 0.0.0.0 --port 8080
```

## What it shows

One provider per minute, alternating Claude and Codex. Claude is reported on
its hourly window and Codex on its weekly one, read out of `limits[]` by label;
Gauge's own `remaining_percent` (the tightest window across all of a provider's
limits) is only used as a fallback. Codex `Spark ` buckets are skipped, matching
how Gauge computes its own headline number.

Remaining quota picks a mood, drawn as a large icon over a single word, with a
stock-ticker delta and an availability bar below:

| Remaining | Mood |
| --- | --- |
| 100–80% | Well-Fed (steak) |
| 79–60% | Getting-Peckish (cookie) |
| 59–40% | Hungry (burger) |
| 39–20% | Starving (wilted flower) |
| 19–5% | Feral (bone) |
| 4–1% | Near-Death (skull and crossbones) |
| 0% | DEAD (skull) |

Adafruit_GFX ships ASCII bitmaps only, so a literal emoji would render as
garbage. Every icon in `src/emoji.cpp` is vector-drawn from GFX primitives
instead.

The ticker shows the change since the last time the number actually *moved*,
rather than resetting to 0.0% on every unchanged poll. At 0% the ticker is
suppressed so the screen sits still on `DEAD` for the whole minute.

## Host tests

Both scripts run on a normal machine — no ESP32 toolchain, no board. They
fetch their third-party dependencies on first run.

```sh
cd test/host
./run.sh        # parser tests: real gauge payloads + unknown-schema fallback
./preview.sh    # renders the real UI code to preview.png
```

`preview.sh` compiles `src/ui.cpp` and `src/emoji.cpp` against the upstream
Adafruit_GFX text engine and a small Arduino shim, then draws all seven moods
side by side, which is the quickest way to check a layout change.
