# Ideas backlog

Goal: recreate the spirit of the original BadgeVMS firmware in Rust. Take the ideas, not the code (the original is GPLv3 C).
The initial implementation slices are a one-shot boot-time I2C scan, TCA8418 key-event reporting, BMI270 accelerometer/gyroscope/tilt sampling, all verified on the badge. Per-key and IMU logs are opt-in via the `telemetry` Cargo feature.
The remaining ideas still need triage (keep / drop / later), then turn the keepers into tasks.

The upstream repos are cloned into `reference/` (gitignored, shallow): `firmware`, `Hardware`, `test-firmware`, `micropython`,
`mass-usb-firmware-flasher`, `badgevms-applications`, `badgehub-{api,frontend,infra}`.
Source of the facts below: schematics in `reference/Hardware`, drivers in `reference/firmware/badgevms`.

## A. Hardware inventory (from the schematics)

| Board | Part | What it does | Status for us |
|---|---|---|---|
| M2 (compute) | ESP32-P4NRW16 (+32MB PSRAM), 40 MHz + 32 kHz crystals | main CPU, MIPI DSI/CSI, SDMMC, PPA | running |
| M2 | XT25F128 (16MB SPI flash) | firmware storage | running |
| M2 | Micro SD slot (card detect) | removable storage | idea |
| M2 | CH340K USB-serial + auto-reset | flash/log over the side USB-C | used |
| M2 | TPS563202 bucks (3.3V, 1.2V) | power | n/a |
| Carrier | ESP32-C6-WROOM-1 | WiFi/BLE co-processor (`esp_hosted`) | Wi-Fi scan/connect groundwork implemented; hardware verification pending |
| Carrier | 720x720 ST7703 panel on 30-pin MIPI, AP3032 boost | display | running |
| Carrier | TCA8418 + 10 switches + LEDs, AP3032 boost | keyboard matrix (I2C) | implemented and verified |
| Carrier | BMI270 | accelerometer + gyro (I2C) | accelerometer, gyro and tilt verified |
| Carrier | BME680/BME690 | gas/air quality, temp, humidity, pressure (I2C) | idea |
| Carrier | 6x WS2812B | 6x addressable RGB LEDs; data input not connected to any GPIO in the carrier schematic | parked |
| Carrier | MAX98357A + speaker | I2S audio out (no microphone seen) | idea |
| Carrier | Vibrator (BC847 driver + motor) | haptics | follows upstream GPIO8/50 ms key-down pulse; hardware connection may be open at DNP R49 |
| Carrier | IP5306 + 18650 cell, charger, balancer, NTC | battery + charging (USB-C on carrier too) | idea |
| Carrier | Ai-Thinker Ra-01 / Ra-01H + SMA | LoRa (SX127x); antenna not connected, needs a rewire that costs WiFi/BT | parked |
| Carrier | 1-1734248-5 FFC connector | camera (MIPI-CSI) | idea |
| Carrier | Test pads, case screws | n/a | n/a |
| Addons | SAO v2, LED matrix, "Oscilli" (oscilloscope), PS2 + VGA | extension boards | idea |
| Case | slim and normal | 3D printable | n/a |

Open checks: which parts are actually populated on our unit; pin map (see `test-firmware/main_p4/main/board.h`); is there an RTC and a fuel gauge (battery level may only be an ADC divider); which I2C addresses answer.
Rust angle: `bosch-bme680` or `bme68x-rust`, `bmi270` crates exist; TCA8418 and ST7703 are probably ours to write on `embedded-hal`.

## B. What the original firmware does (architecture notes)

- **Processes**: apps are ELF files loaded at runtime (`components/elf_loader`), built with a riscv32 toolchain against an SDK sysroot. Each process gets its own linear address space (P4 MMU remapping + a per-process dlmalloc heap) and runs as a FreeRTOS task. The kernel kills processes cleanly (leak-free heap, thread tracking). Isolation protects apps from each other, not the OS.
- **Known weak spots** (its own `TODO.md`): events delivered once per frame (max 10), `select()` half works, lwIP/heap corruption across heaps, a race on process kill, killing a task that holds a FreeRTOS lock kills a random other task, flash access should go through a kernel task, wifi connect sequencing bugs, compositor needs restructuring, no letterboxing on scaled windows. These are good "what not to repeat" notes.
- **Hardware quirks to remember**: the P4 PPA crashes when `height > 32 && height % 32 == 1`; ESP-IDF owns FreeRTOS TLS slot 0; 200 MHz PSRAM can cause single-bit errors on some units (we already rely on 200 MHz).
- **Device model**: `device_get("NAME")` returns typed devices: block, LCD, keyboard, bus (I2C), wifi, orientation (BMI270), socket, filesystem, gas (BME690). One uniform registry.
- **VMS names**: logical names / search lists (`FLASH0:init.toml`, `DEVICE:[DIR]FILE`), resolved by `logical_names.c`. Neat for "where do apps live" aliasing, but not needed for us as is.
- **Compositor**: windows with flags (fullscreen, always on top, undecorated, maximized/left/right, double buffered, flips), framebuffers per window with pixel formats, decorations, scaling.
- **Event system**: key down/up, quit, window resize, per-process queues.
- **Apps**: manifest JSON (`unique_identifier`, name, author, version, interpreter, binary_path, source = BadgeHub), an app database, `application_launch/list/get/is_running`.
- **Boot**: `init.toml` on flash configures the system; OTA API (`ota_session_open/write/commit/abort`, running/invalid version).
- **Wifi**: modes enabled/disabled/ask, scan, cipher and auth types including WPA-Enterprise and OWE.
- **Launcher / settings / sponsors / namebadge** are normal apps, not part of the kernel. Doom, SDL2/SDL3, curl, sockets, threads, memtester, hardware_test, bench apps all exist as demos.
- **Dependencies**: cJSON, tomlc17, khash, dlmalloc, libcurl, esp-tls, esp_http_client. Rust equivalents: serde, toml, hashbrown, ureq/esp-idf-svc http.

## C. Rest of the ecosystem (also ideas)

- **BadgeHub** (api, frontend, infra): app store. Node REST API + Postgres + Keycloak (JWT) + Next.js/React frontend, Ansible deploy. Apps have metadata, versions, categories, "badges" (which hardware they support), zip download. API TODO mentions per-user API tokens so the badge can upload/edit apps itself. Idea: a native BadgeHub client (browse, install, update) and our own manifest format compatible with its metadata.
- **MicroPython port** (v1.25): runs as a BadgeVMS app (`mpy` binary); `mpydaemon` + `mpytool.sh` for REPL/file push; a **buildroot + QEMU** setup to run things on a host. Idea: scripting layer, and a host-side runner for testing.
- **test-firmware**: factory hardware test (LVGL on P4, plus the C6 `esp-hosted` slave firmware; prebuilt slave binary is copied into the P4 build). Useful as the reference for pins and I2C addresses, and for how the C6 slave gets built/flashed.
- **mass-usb-firmware-flasher**: Python script to flash many badges. Web flasher at https://why-prog.heplaphon.com/. Idea: release artifacts in a format those tools accept.
- **badgevms-applications**: effectively an empty index repo.
- **Hardware repo is MIT**: we can build our own carriers and addons (the P4 module is meant to be reused after camp).

## D. Idea list

### 1. UI / graphics
- [ ] Retained-mode widget toolkit: label, button, list, progress, icon, image, text input, toast, modal.
- [ ] Layout engine (stack/flex, padding, alignment) so screens are not pixel coordinates.
- [ ] Theme/style system, dark and light.
- [ ] Better fonts (bitmap + TTF, anti-aliasing, unicode).
- [x] UI stack decision: `embedded-graphics` with a shared host simulator; defer LVGL/Slint until a concrete need appears.
- [ ] Double buffering, dirty rects, PPA acceleration (mind the height % 32 bug).
- [ ] Animations/transitions.
- [x] Host SDL simulator and initial widget showcase (label, buttons, list, progress, icon, toast, modal).
- [ ] GUI demo app showing every widget (showcase + regression check).
- [ ] Launcher: app grid, status bar (clock, battery, wifi, SD), notifications.
- [ ] Console becomes one app/widget; the log mirror turns into a debug overlay.
- [ ] Compositor with several windows/surfaces, flags like the original (fullscreen, undecorated, scaling with letterboxing).
- [ ] Screenshots to SD/serial; backlight/dim control.
- [x] Framebuffer rotation for the badge's upright layout (needs on-device confirmation).
- [x] Wi-Fi status dashboard showing connection state, selected SSID, IP, nearby networks, and logs.

### 2. Input
- [x] Basic TCA8418 key-event mapping and keypad navigation/password entry for the Wi-Fi dashboard.
- [ ] Full TCA8418 driver (interrupt, repeat, general-purpose modifiers).
- [ ] Event model that is not frame-limited (the original delivers max 10 per frame).
- [ ] Focus/navigation for widgets, on-screen keyboard if needed.
- [ ] Keyboard backlight/LEDs `?`.

### 3. App model (the container/VM idea)
Original = ELF + per-process address space + syscall wrappers. Good concept, but the C approach has the races/leaks listed in B.
- [ ] Decide the model. Options:
  - [ ] Native Rust apps in one firmware (trait `App`, no isolation, fastest to build).
  - [ ] WASM (wasm3 / wamr / wasmtime pulley): real sandbox, memory limits, killable, language-agnostic. Likely the best fit for "containers" in Rust.
  - [ ] ELF loader like the original (ABI stability is a burden, unsafe).
  - [ ] Scripting: MicroPython (exists as a port), Lua, Rhai.
- [ ] Host API / syscall surface: window, input, fs, net, sensors, time, log, haptics, LEDs, audio.
- [ ] Lifecycle: start, suspend, resume, kill; quotas for memory and CPU; clean teardown (no leaked locks).
- [ ] Manifest compatible with BadgeHub metadata.
- [ ] Per-app permissions.
- [ ] Event bus + inter-app messaging.
- [ ] SDK/template repo and docs so others can write apps.

### 4. Storage / filesystem
- [ ] SD card on the M2 (SDMMC, FAT, hot-plug via card detect).
- [ ] Internal flash FS (FAT/LittleFS) for settings and apps; `init.toml`-style config.
- [ ] Path scheme: plain POSIX with mount aliases (`/flash`, `/sd`) vs VMS names. Decide.
- [ ] NVS settings with typed API.
- [ ] OTA with rollback; web flasher compatible release artifacts.
- [ ] A kernel-style storage task so apps never touch flash directly.

### 5. Connectivity
- [x] C6 `esp_hosted` Wi-Fi scan/connect for open and personal WPA-family networks, with saved profiles in P4 NVS (hardware verification pending).
- [ ] WPA-Enterprise (eduroam) and mode "ask".
- [x] On-screen Wi-Fi selection, personal-network password entry, and status controls.
- [ ] Settings UI for wifi, NTP, mDNS, HTTP(S) client.
- [ ] BLE via the C6 (HID, beacons, badge-to-badge).
- [ ] Flash/update the C6 slave firmware from the P4.
- [ ] USB device/host modes.
- [ ] LoRa (SX127x on SPI): parked. Needs a hardware rewire that costs WiFi/BT on the C6. Keep a radio trait so it can be added later.
- [ ] Badge-to-badge ideas: chat, pings, mesh, trading, multiplayer (BLE/ESP-NOW/LoRa).
- [ ] BadgeHub client: browse, install, update, publish.

### 6. Sensors / peripherals
- [ ] Shared I2C bus manager (original has `badgevms_i2c_bus`) so TCA8418/BMI270/BME690 share one bus safely.
- [ ] BMI270: orientation, gestures, step counter, shake; auto-rotate display. (Original only reported one axis, and returned stale data when busy.)
- [ ] BME690: IAQ, temp, humidity, pressure; sensor dashboard with history graphs.
- [ ] WS2812B x6: no data line is wired on the carrier (see section A); parked.
- [ ] Vibration motor: haptic patterns and feedback on events.
- [ ] Audio via MAX98357A: beeps, tones, WAV/MP3 playback, UI sounds.
- [ ] Battery: voltage, percentage, charging state, low battery action, deep sleep, power profile. `?` check what telemetry IP5306 gives (probably none).
- [ ] Camera (MIPI-CSI): preview, photo to SD. `?` check the FFC pinout and whether a module is available.
- [ ] Addons: SAO port, LED matrix, oscilloscope board, PS2/VGA board.
- [ ] Hardware inventory/test app (detects devices, shows live values). The factory test firmware is a template.
- [ ] Driver pattern: `embedded-hal` traits, one task per device, readings published on the event bus.

### 7. Demo apps (platform validation)
- [ ] GUI widget demo, sensor dashboard, settings, file browser, terminal/serial console.
- [ ] Namebadge with images/animation, wifi/BLE scanner, sponsors/info screens.
- [ ] Games: snake, tetris, a Doom port (as original), a tilt game using the IMU.
- [ ] Hardware test app, memory test, benchmarks.
- [ ] Chat, a music/tone app, a camera app.

### 8. Platform, tooling, quality
- [ ] Workspace of crates: `board` (drivers), `ui`, `runtime`, `apps`, `firmware` bin. Keep `ui` host-testable.
- [x] Desktop simulator (embedded-graphics-simulator/SDL); UI work without flashing.
- [ ] CI: build, clippy, fmt, host tests.
- [ ] Panic screen on the display; ring-buffer log viewable on screen.
- [ ] Heap/CPU stats; boot splash and boot-time work.
- [ ] Robust `flash.sh`; document restoring BadgeVMS.
- [ ] Panel variants (Bono vs Mountain) via feature or runtime detect.
- [ ] Architecture, hardware and app-author docs.

### 9. Probably skip
- VMS path syntax, full C SDK/SDL compatibility, loading arbitrary third party C ELF apps, PS2/VGA addon support.

## E. Open questions
1. App model: native first, WASM from the start, or both?
2. UI stack: own toolkit, LVGL, or Slint?
3. Which parts are populated and working on our unit (I2C scan, GPIO map)?
4. Keep P4-only code and use the C6 purely as radio over `esp_hosted`?
5. How compatible must we be with BadgeHub and the BadgeVMS app format?
6. License: original is GPLv3; reading it for ideas is fine, copying code is not.

## F. Suggested triage order (not decided)
1. Hardware bring-up inventory (I2C scan, keyboard, IMU, BME, LEDs, haptics, SD): section A/6. I2C scan, TCA8418 key-event reporting, and BMI270 accelerometer/gyroscope/tilt sampling are implemented and verified.
2. UI stack choice plus simulator and GUI demo: sections 1, 8. (embedded-graphics + SDL simulator chosen; initial showcase implemented)
3. Input events and shared bus: sections 2, 6.
4. App model decision: section 3.
5. Wifi/storage, then BadgeHub client.
