mod display;
mod ui;
mod wifi;

use std::fmt::Write as _;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use esp_idf_hal::delay::{Ets, BLOCK};
use esp_idf_hal::gpio::{Output, PinDriver};
use esp_idf_hal::i2c::{APBTickType, I2cConfig, I2cDriver, I2cError};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::units::*;
use esp_idf_svc::log::{EspIdfLogFilter, EspLogger};
use esp_idf_svc::sys::{EspError, ESP_FAIL};
use log::{LevelFilter, Log, Metadata, Record};

use bmi2::config::BMI270_CONFIG_FILE;
use bmi2::types::{
    AccBwp, AccConf, AccRange, Burst, Data, GyrBwp, GyrConf, GyrRange, GyrRangeVal, Odr, OisRange,
    PerfMode, PwrCtrl, BMI270_CHIP_ID,
};
use bmi2::{Bmi2, I2cAddr};

use display::{Display, HEIGHT, WIDTH};
use ui::Dashboard;
use wifi::{WifiCommand, WifiSnapshot};

static CONSOLE: Mutex<Option<Dashboard>> = Mutex::new(None);

/// Sends every log record to the serial console and to the screen.
struct DualLogger(EspLogger);

impl Log for DualLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.0.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        self.0.log(record);
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Ok(mut guard) = CONSOLE.try_lock() {
            if let Some(dashboard) = guard.as_mut() {
                let mut text = String::new();
                let _ = write!(text, "{}: {}", record.target(), record.args());
                dashboard.push_log(record.level(), &text);
            }
        }
    }

    fn flush(&self) {}
}

static LOGGER: DualLogger = DualLogger(EspLogger::new(EspIdfLogFilter::new()));

const TCA8418_ADDRESS: u8 = 0x34;
const TCA8418_KEY_EVENT_COUNT: u8 = 0x03;
const TCA8418_KEY_EVENT_A: u8 = 0x04;
const BMI270_ADDRESS: u8 = 0x69;
const BMI270_BUFFER_SIZE: usize = 512;
const GRAVITY_MPS2: f32 = 9.80665;
const BMI270_GYRO_SENSITIVITY_LSB_PER_DPS: f32 = 16.4;

type Bmi270Error = bmi2::types::Error<I2cError>;

const VIBRATION_DURATION: Duration = Duration::from_millis(50);

#[derive(Clone, Copy)]
struct KeyboardEvent {
    key_code: u8,
    pressed: bool,
}

fn vibrate(pin: &mut PinDriver<'_, Output>) -> Result<(), EspError> {
    pin.set_high()?;
    thread::sleep(VIBRATION_DURATION);
    pin.set_low()
}

fn init_i2c_bus(
    i2c0: esp_idf_hal::i2c::I2C0<'static>,
    sda: esp_idf_hal::gpio::Gpio18<'static>,
    scl: esp_idf_hal::gpio::Gpio20<'static>,
) -> Result<I2cDriver<'static>, EspError> {
    let config = I2cConfig::new()
        .baudrate(100.kHz().into())
        .timeout(APBTickType::from(Duration::from_millis(100)));
    I2cDriver::new(i2c0, sda, scl, &config)
}

fn scan_i2c_bus(i2c: &mut I2cDriver<'_>) {
    let mut found = Vec::new();
    for address in 0x08..=0x77 {
        match i2c.write(address, &[], BLOCK) {
            Ok(()) => found.push(address),
            Err(error) if error.code() == ESP_FAIL => {}
            Err(error) => {
                log::error!("I2C scan stopped at 0x{address:02X}: {error}");
                return;
            }
        }
    }

    if found.is_empty() {
        log::warn!("I2C scan complete: no devices found on GPIO18/20");
    } else {
        let mut addresses = String::new();
        for address in &found {
            let _ = write!(addresses, " 0x{address:02X}");
        }
        log::info!(
            "I2C scan complete: {} device(s) found:{}",
            found.len(),
            addresses
        );
    }
}

fn init_keyboard(i2c: &mut I2cDriver<'_>) -> Result<(), EspError> {
    for (register, value) in [
        (0x23, 0x00),
        (0x24, 0x00),
        (0x25, 0x00),
        (0x1D, 0xFF),
        (0x1E, 0xFF),
        (0x1F, 0x03),
        (0x29, 0x00),
        (0x2A, 0x00),
        (0x2B, 0x00),
    ] {
        i2c.write(TCA8418_ADDRESS, &[register, value], BLOCK)?;
    }

    log::info!("TCA8418 keyboard ready at 0x{TCA8418_ADDRESS:02X}");
    Ok(())
}

fn poll_keyboard(i2c: &mut I2cDriver<'_>) -> Result<Vec<KeyboardEvent>, EspError> {
    let mut key_events = Vec::new();
    let mut event_count = [0];
    i2c.write_read(
        TCA8418_ADDRESS,
        &[TCA8418_KEY_EVENT_COUNT],
        &mut event_count,
        BLOCK,
    )?;

    for _ in 0..(event_count[0] & 0x0F) {
        let mut event = [0];
        i2c.write_read(TCA8418_ADDRESS, &[TCA8418_KEY_EVENT_A], &mut event, BLOCK)?;

        let key = event[0] & 0x7F;
        let pressed = event[0] & 0x80 != 0;
        let action = if pressed { "pressed" } else { "released" };
        key_events.push(KeyboardEvent {
            key_code: key,
            pressed,
        });
        if !cfg!(feature = "telemetry") {
            continue;
        }
        if (1..=80).contains(&key) {
            let position = key - 1;
            log::info!(
                "Keyboard key {action}: code {key}, row {}, column {}",
                position / 10,
                position % 10
            );
        } else {
            log::info!("Keyboard key {action}: code {key}");
        }
    }

    Ok(key_events)
}

fn init_bmi270(i2c: &mut I2cDriver<'static>) -> Result<(), Bmi270Error> {
    let mut bmi = Bmi2::<_, _, BMI270_BUFFER_SIZE>::new_i2c(
        &mut *i2c,
        Ets,
        I2cAddr::Alternative,
        Burst::new(255),
    );
    let chip_id = bmi.get_chip_id()?;
    log::info!("BMI270 chip ID at 0x{BMI270_ADDRESS:02X}: 0x{chip_id:02X}");
    if chip_id != BMI270_CHIP_ID {
        return Err(Bmi270Error::InvalidChipId);
    }

    bmi.init(&BMI270_CONFIG_FILE)?;
    bmi.set_acc_conf(AccConf {
        odr: Odr::Odr100,
        bwp: AccBwp::NormAvg4,
        filter_perf: PerfMode::Perf,
    })?;
    bmi.set_acc_range(AccRange::Range2g)?;
    bmi.set_gyr_conf(GyrConf {
        odr: Odr::Odr100,
        bwp: GyrBwp::Norm,
        noise_perf: PerfMode::Power,
        filter_perf: PerfMode::Perf,
    })?;
    bmi.set_gyr_range(GyrRange {
        range: GyrRangeVal::Range2000,
        ois_range: OisRange::Range250,
    })?;
    bmi.set_pwr_ctrl(PwrCtrl {
        aux_en: false,
        gyr_en: true,
        acc_en: true,
        temp_en: false,
    })?;
    log::info!("BMI270 accelerometer and gyroscope ready (100 Hz)");
    Ok(())
}

fn read_bmi270_data(i2c: &mut I2cDriver<'static>) -> Result<Data, Bmi270Error> {
    let mut bmi = Bmi2::<_, _, BMI270_BUFFER_SIZE>::new_i2c(
        &mut *i2c,
        Ets,
        I2cAddr::Alternative,
        Burst::new(255),
    );
    bmi.get_data()
}

fn main() {
    // Required so runtime patches from esp-idf-sys are linked in.
    esp_idf_svc::sys::link_patches();
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LevelFilter::Info);

    log::info!("Booting WHY2025-RS");
    match Display::new() {
        Ok(display) => {
            *CONSOLE.lock().unwrap() = Some(Dashboard::new(display));
            log::info!("Display ready ({WIDTH}x{HEIGHT})");
        }
        Err(e) => log::error!("Display init failed: {e}"),
    }

    log::info!("WHY2025-RS Base Firmware Online");
    let peripherals = Peripherals::take().expect("peripherals already taken");
    let modem = peripherals.modem;
    let mut vibrator: Option<PinDriver<'static, Output>> =
        match PinDriver::output(peripherals.pins.gpio8) {
            Ok(pin) => {
                log::info!("Vibrator output ready on GPIO8");
                Some(pin)
            }
            Err(error) => {
                log::error!("Vibrator GPIO8 initialization failed: {error}");
                None
            }
        };
    let mut i2c = match init_i2c_bus(
        peripherals.i2c0,
        peripherals.pins.gpio18,
        peripherals.pins.gpio20,
    ) {
        Ok(i2c) => Some(i2c),
        Err(error) => {
            log::error!("I2C bus initialization failed: {error}");
            None
        }
    };
    let mut keyboard_ready = false;
    let mut bmi270_ready = false;
    if let Some(i2c) = i2c.as_mut() {
        scan_i2c_bus(i2c);
        match init_keyboard(i2c) {
            Ok(()) => keyboard_ready = true,
            Err(error) => log::error!("TCA8418 keyboard initialization failed: {error}"),
        }
        match init_bmi270(i2c) {
            Ok(()) => bmi270_ready = true,
            Err(error) => log::error!("BMI270 initialization failed: {error:?}"),
        }
    }

    let (wifi_command_tx, wifi_command_rx) = mpsc::channel();
    let (wifi_update_tx, wifi_update_rx) = mpsc::channel::<WifiSnapshot>();
    if let Err(error) = wifi::spawn_console(wifi_command_tx.clone()) {
        log::error!("Wi-Fi serial command input could not start: {error}");
    }
    let manager_updates_tx = wifi_update_tx.clone();
    let init_failure_updates_tx = wifi_update_tx.clone();
    if let Err(error) = thread::Builder::new()
        .name("wifi-manager".into())
        .stack_size(12_288)
        .spawn(move || {
            if let Err(error) = wifi::run(modem, wifi_command_rx, manager_updates_tx) {
                log::error!("Wi-Fi manager initialization failed: {error}");
                let mut snapshot = WifiSnapshot::default();
                snapshot.status = wifi::WifiStatus::Unavailable;
                snapshot.detail = Some(error);
                if let Err(send_error) = init_failure_updates_tx.send(snapshot) {
                    log::error!(
                        "Wi-Fi initialization status could not reach the display: {send_error}"
                    );
                }
            }
        })
    {
        log::error!("Wi-Fi manager task could not start: {error}");
        let mut snapshot = WifiSnapshot::default();
        snapshot.status = wifi::WifiStatus::Unavailable;
        snapshot.detail = Some(error.to_string());
        if let Err(send_error) = wifi_update_tx.send(snapshot) {
            log::error!("Wi-Fi task failure status could not reach the display: {send_error}");
        }
    }

    let mut last_imu_sample = std::time::Instant::now();
    loop {
        while let Ok(snapshot) = wifi_update_rx.try_recv() {
            if let Ok(mut guard) = CONSOLE.lock() {
                if let Some(dashboard) = guard.as_mut() {
                    dashboard.set_wifi(snapshot);
                }
            }
        }

        let display_result = if let Ok(mut guard) = CONSOLE.lock() {
            guard.as_mut().map(Dashboard::render_if_dirty)
        } else {
            None
        };
        if let Some(Err(error)) = display_result {
            log::error!("Display refresh failed: {error}");
        }

        if keyboard_ready {
            if let Some(i2c) = i2c.as_mut() {
                match poll_keyboard(i2c) {
                    Ok(key_events) => {
                        let mut ui_commands: Vec<WifiCommand> = Vec::new();
                        if let Ok(mut guard) = CONSOLE.lock() {
                            if let Some(dashboard) = guard.as_mut() {
                                for event in &key_events {
                                    if let Some(command) =
                                        dashboard.handle_key(event.key_code, event.pressed)
                                    {
                                        ui_commands.push(command);
                                    }
                                }
                            }
                        }
                        for command in ui_commands {
                            if wifi_command_tx.send(command).is_err() {
                                log::error!("Wi-Fi manager is not accepting keypad commands");
                                break;
                            }
                        }
                        if let Some(pin) = vibrator.as_mut() {
                            for _ in key_events.iter().filter(|event| event.pressed) {
                                if let Err(error) = vibrate(pin) {
                                    log::error!("Vibrator pulse failed on GPIO8: {error}");
                                    vibrator = None;
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        log::error!("TCA8418 keyboard polling stopped: {error}");
                        keyboard_ready = false;
                    }
                }
            }
        }
        if bmi270_ready && last_imu_sample.elapsed() >= Duration::from_secs(1) {
            last_imu_sample = std::time::Instant::now();
            if let Some(i2c) = i2c.as_mut() {
                match read_bmi270_data(i2c) {
                    Ok(data) => {
                        let accel_x = f32::from(data.acc.x) * GRAVITY_MPS2 / 16384.0;
                        let accel_y = f32::from(data.acc.y) * GRAVITY_MPS2 / 16384.0;
                        let accel_z = f32::from(data.acc.z) * GRAVITY_MPS2 / 16384.0;
                        let gyro_x = f32::from(data.gyr.x) / BMI270_GYRO_SENSITIVITY_LSB_PER_DPS;
                        let gyro_y = f32::from(data.gyr.y) / BMI270_GYRO_SENSITIVITY_LSB_PER_DPS;
                        let gyro_z = f32::from(data.gyr.z) / BMI270_GYRO_SENSITIVITY_LSB_PER_DPS;
                        let roll = accel_y.atan2(accel_z).to_degrees();
                        let pitch = (-accel_x).atan2(accel_y.hypot(accel_z)).to_degrees();
                        if cfg!(feature = "telemetry") {
                            log::info!(
                                "BMI270 accel m/s^2: x={accel_x:.2} y={accel_y:.2} z={accel_z:.2}"
                            );
                            log::info!(
                                "BMI270 gyro deg/s: x={gyro_x:.1} y={gyro_y:.1} z={gyro_z:.1}"
                            );
                            log::info!("BMI270 tilt: roll={roll:.1} deg pitch={pitch:.1} deg");
                        }
                    }
                    Err(error) => {
                        log::error!("BMI270 sampling stopped: {error:?}");
                        bmi270_ready = false;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}
