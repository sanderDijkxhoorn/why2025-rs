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

Build, flash and monitor (no code fences when pasting into fish):
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
