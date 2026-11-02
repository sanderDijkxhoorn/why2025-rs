#!/usr/bin/env bash
# Build, flash and monitor using the pinned nightly, ignoring any RUSTUP_TOOLCHAIN/mise override.
set -euo pipefail
cd "$(dirname "$0")"
TC=nightly-2026-06-01
rustup toolchain install "$TC" --component rust-src --profile minimal
export RUSTUP_TOOLCHAIN=$TC
export PATH="$HOME/.rustup/toolchains/$TC-x86_64-unknown-linux-gnu/bin:$PATH"
# The P4 is on the CH340 serial port; override with ESPFLASH_PORT
exec cargo espflash flash --release --monitor --port "${ESPFLASH_PORT:-/dev/ttyUSB0}" --baud 460800 "$@"
