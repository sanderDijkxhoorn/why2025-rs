# why2025-rs
Code work WHY? Code not work WHY?

## Setup
```sh
cargo install cargo-espflash espflash ldproxy cargo-generate
```
Arch/CachyOS: ESP's clang needs the legacy libxml2: `sudo pacman -S libxml2-legacy`.
If `RUSTUP_TOOLCHAIN` is set (or cargo comes from a mise shim), make sure the nightly
toolchain is used: `rustup toolchain install nightly --component rust-src`, then put
`~/.rustup/toolchains/nightly-*/bin` first in `PATH`.

Build, flash and monitor:
./flash.sh

`flash.sh` uses the pinned nightly (2026-06-01). Newer nightlies currently fail to build `std` for ESP-IDF.

The side USB-C port (`/dev/ttyUSB0`) is the P4 port used by `./flash.sh`. The separate
`/dev/ttyACM*` port is the C6 radio co-processor; this project does not flash it. The P4 and C6
run different firmware, and the C6 must retain a compatible `esp_hosted` slave image. The first
build downloads ESP-IDF (needs git, python3, cmake, ninja, libudev headers).

## References
- Badge wiki: https://wiki.why2025.org/Badge
- Firmware: https://gitlab.com/why2025/team-badge/firmware
- Team badge group: https://gitlab.com/why2025/team-badge
- Hardware: https://gitlab.com/why2025/team-badge/Hardware
- Programming: https://why-prog.heplaphon.com/

## Hardware notes
- The badge has two chips: an **ESP32-P4** (main CPU, runs BadgeVMS, drives the display) and an
  **ESP32-C6** (radio co-processor running `esp_hosted`).
- The side USB-C port is a CH340 serial bridge to the **P4** (`/dev/ttyUSB0`, group `uucp`:
  `sudo usermod -aG uucp $USER`). The other port is the C6's native USB (`/dev/ttyACM*`).
- This project targets the P4 (`esp32p4`, `riscv32imafc-esp-espidf`, ESP-IDF v5.5.1). It replaces BadgeVMS on the P4;
  restore it with https://why-prog.heplaphon.com/ ("BadgeVMS post_event").
- Display: 720x720 ST7703 over MIPI-DSI, ported from `badgevms/drivers/st7703.[ch]` in the firmware repo.
  `src/display.rs` uses the **Bono** panel (black ribbon border, carrier v2.1) init and timings; the older
  Mountain panel (blue border) needs the other sequence from that driver. The framebuffer rotates
  drawing counterclockwise for the badge's upright layout; confirm the orientation on the device.
- Logs are mirrored to the screen (`src/main.rs`, `DualLogger`).
- The framebuffer lives in PSRAM; 200 MHz PSRAM (`IDF_EXPERIMENTAL_FEATURES`) is required, at 20 MHz the DPI stream underruns.

## I2C bring-up
At boot, the firmware scans the I2C bus once at 100 kHz using GPIO 18 (SDA) and GPIO 20 (SCL),
the WHY2025 v2/v2.1 pin mapping. Discovered addresses and scan errors appear on both serial and
the display console. A scan that finds no devices is reported explicitly; any non-address-NACK
error stops the scan and is logged. It then configures the TCA8418 at `0x34` for its 8x10 keypad
matrix and polls its event FIFO, logging key-down and key-up events with matrix row/column.
The BMI270 at `0x69` is initialized from its configuration (accelerometer ±2 g and gyroscope
±2000 dps, both 100 Hz). Its sample is scaled to m/s² and degrees per second, with roll/pitch
estimated from gravity. At rest roll reads about ±178° because the badge's Z axis reports -g.
Hardware testing confirmed stable gravity while stationary, and changing accel, gyro and tilt
values when rotating the badge.

Per-key and per-second IMU log lines are off by default so they do not flood the display
console. Enable them with the `telemetry` feature, e.g. `./flash.sh --features telemetry`.
Initialization and error messages are always logged.
On-device verification found I2C devices at `0x34`, `0x69`, and `0x76`; manually testing the
keyboard produced 68 press/release pairs across 67 distinct matrix codes.

## Wi-Fi and status dashboard
The ESP32-P4 has no built-in radio. Wi-Fi uses the carrier's ESP32-C6 over the SDIO `esp_hosted`
link (P4 GPIOs 28-33 and reset GPIO13), using the component versions and pin map from the badge
test firmware. The C6 must be running a compatible `esp_hosted` 2.0.17 slave image; `./flash.sh` only flashes
the P4 and does not install or replace the C6 image. The reference
[C6 slave-firmware instructions](reference/test-firmware/README.md) describe building the image.
If the P4 log repeatedly says `Not able to connect with ESP-Hosted slave device` and no `Wi-Fi AP`
scan lines appear, the radio link has not come up; check the C6 slave image and SDIO/reset wiring
before troubleshooting the scan list.

The display shows Wi-Fi state, the selected SSID and IP address, nearby scan results, and a small
log view. It only redraws after a state or log change. Use the keypad arrows to select a network
and Enter to connect. Open networks connect immediately; for secured networks, type the password
(the display masks it), use either Shift key for capitals/symbols, Backspace to edit, Enter to
submit, and Esc to cancel. Press `R` to rescan, `D` to disconnect, and `F` to forget the saved
profile. The scan list shows each named SSID once, using its strongest access point; selection
scrolls through the full list.

From the P4 serial monitor, use:

```text
wifi scan
wifi connect 1 [password]
wifi disconnect
wifi forget
```

The scan list is numbered from 1 and sorted by signal strength, with one entry per named SSID;
the screen shows eight at a time and scrolls through all results. Open networks need no password;
personal WPA/WPA2/WPA3 networks accept an 8-63 byte password. Unsupported security modes are
shown but cannot be selected. A selected profile is stored in the P4's default NVS partition and
retried at boot. The profile includes the password and NVS encryption is not enabled, so physical
flash access can expose it. Passwords are not shown on screen or included in logs.

Wi-Fi is build-integrated but still needs on-device verification against the installed C6 slave
image. If the hosted link does not start, confirm the C6 image version and the P4/C6 SDIO wiring.

## Desktop UI simulator

The UI uses `embedded-graphics` on both the badge and desktop. This keeps the existing Rust drawing
path and avoids adding a separate LVGL or Slint runtime while the UI is still small. The display-
independent dashboard is in `ui-core`; the desktop simulator uses mock Wi-Fi data, so it can be
worked on without the badge or ESP-IDF toolchain.

Install SDL2 development libraries (for example, `sudo apt install libsdl2-dev` on Debian/Ubuntu),
then run:

```sh
cargo run --manifest-path simulator/Cargo.toml --target x86_64-unknown-linux-gnu
```

The simulator opens on the widget showcase; press Tab to switch to the Wi-Fi dashboard. In the
dashboard, use Up/Down and Enter to select/connect, R to rescan, and D/F to disconnect/forget.
Escape closes the simulator window. Its host build and showcase rendering test can be checked with:

```sh
cargo test --manifest-path simulator/Cargo.toml --target x86_64-unknown-linux-gnu
```

## Haptics and LEDs
- The vibration motor follows the upstream test firmware: ESP32 GPIO 8 is configured as an output
  and driven high for 50 ms for every key-down event. GPIO initialization and pulse errors are
  logged. In the carrier PCB the `PWM_VIB` net only reaches that pin through `R49` (0R), which is
  marked do-not-populate, so on a stock board the motor may be disconnected until R49 is bridged.
- The 6 on-board WS2812 LEDs have their data inputs tied together with nothing driving them in the
  carrier schematic, which matches upstream's `WS2812_LED_DISABLE`, so there is nothing to light.
