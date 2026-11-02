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

Note: the badge's USB port (`/dev/ttyACM0`) currently identifies as an **ESP32-C6**, so the
project targets `esp32c6` / `riscv32imac-esp-espidf`. Flashing overwrites the C6's firmware.
The first build downloads ESP-IDF (needs git, python3, cmake, ninja, libudev headers).

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
  Mountain panel (blue border) needs the other sequence from that driver.
- Logs are mirrored to the screen (`src/main.rs`, `DualLogger`).
- The framebuffer lives in PSRAM; 200 MHz PSRAM (`IDF_EXPERIMENTAL_FEATURES`) is required, at 20 MHz the DPI stream underruns.

## I2C bring-up
At boot, the firmware scans the I2C bus once at 100 kHz using GPIO 18 (SDA) and GPIO 20 (SCL),
the WHY2025 v2/v2.1 pin mapping. Discovered addresses and scan errors appear on both serial and
the display console. A scan that finds no devices is reported explicitly; any non-address-NACK
error stops the scan and is logged. It then configures the TCA8418 at `0x34` for its 8x10 keypad
matrix and polls its event FIFO, logging key-down and key-up events with matrix row/column.
On-device verification found I2C devices at `0x34`, `0x69`, and `0x76`; manually testing the
keyboard produced 68 press/release pairs across 67 distinct matrix codes.
