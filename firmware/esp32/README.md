# ESP32 Gauge display

A small Gauge accessory for a classic ESP32 devkit, a 240x320 ST7789 panel, and
an optional two-servo head. It implements the
[Gauge Accessory Protocol](../../docs/accessory-protocol.md) end to end:

- **Pairing** over Bluetooth LE Secure Connections. The panel shows the same
  six-digit number as your Mac; press **BOOT** to confirm. Gauge then sends
  Wi-Fi and a per-device token over the protected link. Nothing is compiled in.
- **Discovery** of the paired Mac over Bonjour by its server ID, so a new DHCP
  lease on the Mac does not break it.
- **Dashboard** pages from `GET /v1/dashboard`, every 8 seconds:
  one page per quota group (Claude, Codex, Codex Spark, ...), then Calendar,
  then To-do. Gauge's own settings decide what appears: a provider switched off
  in Gauge disappears here too, and the refresh interval follows Gauge.
- **Offline cache**: the last valid snapshot is kept in NVS and shown after a
  reboot or while the Mac is asleep, with a red dot marking it stale.
- **Unpairing** from either side: hold **BOOT** for 5 seconds, or click
  **Forget** in Gauge. Both return the device to pairing mode.

| Environment | Source | Purpose |
| --- | --- | --- |
| `display` | `src/display_client.cpp` | The accessory |
| `displaytest` | `src/display_test.cpp` | Panel bring-up diagnostic |

## Wiring

| Part | Signal | GPIO |
| --- | --- | --- |
| ST7789 | `CS` / `DC` / `RST` | 5 / 4 / 15 |
| ST7789 | `MOSI` (`SDA`) / `SCLK` (`SCL`) | 18 / 2 |
| ST7789 | `BLK` | 3V3, or set `PIN_TFT_BLK` |
| Button | the devkit's **BOOT** button | 0 |
| Servo | head up/down | 13 |
| Servo | head left/right | 14 |

All pins and servo angles live in `src/config.h`. The servos are optional;
power them from a separate 5 V supply with its ground joined to ESP32 GND.

The panel runs on bit-banged SPI. On this pin set hardware SPI does not drive
it at any mode or clock: GPIO2 is a strapping pin with the devkit LED on it.
GPIO2 must be low or floating at boot, and GPIO15 high. If the panel stays
dark, run `pio run -e displaytest -t upload`, which walks software SPI and
several hardware SPI modes and labels each attempt on screen.

## Build and flash

```sh
cd firmware/esp32
pio run -e display -t upload
pio device monitor
```

The build uses the `huge_app` partition table: Bluetooth, Wi-Fi, mDNS, and
HTTP together do not fit the default 1.3 MB app slot.

## Pair with Gauge

1. Flash the board. An unpaired board shows **PAIR** and its name, for
   example `Gauge Display a1b2`.
2. Connect the Mac to the Wi-Fi network the board should join. Gauge sends the
   Mac's current network, and the ESP32 only joins 2.4 GHz, so that network
   must offer 2.4 GHz. Keep the board near the Mac.
3. In Gauge, open **Settings… › Accessories › Pair Accessory…** (or the pairing
   link at the bottom of the popover) and choose the board from the list.
4. macOS shows a Bluetooth number. Check that the panel shows the same one,
   confirm on the Mac, and press **BOOT** on the board within 30 seconds.
   Not pressing it rejects the pairing.
5. The board joins Wi-Fi, reports `connected`, and restarts. Gauge lists it
   under **Accessories** with its last-seen time once it reads the dashboard.

Gauge must stay open for the board to update: accessories read Gauge's
snapshot over the local network and never contact providers themselves.

## Screens

| Screen | Shows |
| --- | --- |
| Quota group | Mood for the tightest remaining window, then up to three windows with remaining % and time to reset |
| Calendar | Up to three upcoming events, "IN 25m" / "NOW" / "ALL DAY" |
| To-do | Up to five to-dos, completed ones struck through, and how many more exist |
| Quota | Gauge's settings or quota error, when no provider is reporting |

Moods follow the remaining percentage: Well-Fed (80+), Getting-Peckish (60+),
Hungry (40+), Starving (20+), Feral (5+), Near-Death (1+), DEAD (0). Icons are
drawn with GFX primitives because Adafruit_GFX fonts are ASCII-only.

Times are relative to the snapshot's `generated_at`, so the board needs no
clock or time zone. Gauge's token history and agent attention alerts are local
to the Mac and are not part of the accessory dashboard.

## Troubleshooting

| Panel | Meaning |
| --- | --- |
| `NO GAUGE` | Wi-Fi works but Gauge was not found. Open Gauge on a Mac on the same network; some guest networks block Bonjour between clients. |
| Red dot, top right | Showing the cached snapshot; the last refresh failed. It retries every 15 seconds. |
| `UNPAIRED` | Gauge forgot this board (401) or BOOT was held. It restarts in pairing mode. |
| Pairing fails in Gauge | Re-check the Wi-Fi password Gauge used; the board reports `error:wifi connection failed` over Bluetooth. |

To erase everything without Gauge, hold BOOT for 5 seconds while the dashboard
is showing, or `pio run -e display -t erase` and flash again.

## Host tests

No board or ESP32 toolchain is needed; the scripts download their
dependencies on first run.

```sh
cd test/host
./run.sh        # parser against docs/fixtures/dashboard-v1.json
./preview.sh    # renders every screen with the real ui.cpp to preview.ppm/png
```
