mod display;

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use esp_idf_svc::log::{EspIdfLogFilter, EspLogger};
use log::{Level, LevelFilter, Log, Metadata, Record};

use display::{Display, HEIGHT, WIDTH};

const COLS: usize = WIDTH / 10;
const ROWS: usize = HEIGHT / 20;

struct Console {
    display: Display,
    lines: VecDeque<(Level, String)>,
}

impl Console {
    fn push(&mut self, level: Level, text: &str) {
        for line in text.lines() {
            // Wrap long lines to the screen width
            let chars: Vec<char> = line.chars().collect();
            for chunk in chars.chunks(COLS) {
                if self.lines.len() == ROWS {
                    self.lines.pop_front();
                }
                self.lines.push_back((level, chunk.iter().collect()));
            }
        }
        self.render();
    }

    fn render(&mut self) {
        let _ = self.display.clear(Rgb565::BLACK);
        for (row, (level, line)) in self.lines.iter().enumerate() {
            let color = match level {
                Level::Error => Rgb565::RED,
                Level::Warn => Rgb565::YELLOW,
                Level::Info => Rgb565::GREEN,
                _ => Rgb565::CSS_GRAY,
            };
            let style = MonoTextStyle::new(&FONT_10X20, color);
            let _ = Text::with_baseline(line, Point::new(0, row as i32 * 20), style, Baseline::Top)
                .draw(&mut self.display);
        }
        let _ = self.display.flush();
    }
}

static CONSOLE: Mutex<Option<Console>> = Mutex::new(None);

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
            if let Some(console) = guard.as_mut() {
                let mut text = String::new();
                let _ = write!(text, "{}: {}", record.target(), record.args());
                console.push(record.level(), &text);
            }
        }
    }

    fn flush(&self) {}
}

static LOGGER: DualLogger = DualLogger(EspLogger::new(EspIdfLogFilter::new()));

fn main() {
    // Required so runtime patches from esp-idf-sys are linked in.
    esp_idf_svc::sys::link_patches();
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LevelFilter::Info);

    log::info!("Booting WHY2025-RS");
    match Display::new() {
        Ok(display) => {
            *CONSOLE.lock().unwrap() = Some(Console {
                display,
                lines: VecDeque::new(),
            });
            log::info!("Display ready ({WIDTH}x{HEIGHT})");
        }
        Err(e) => log::error!("Display init failed: {e}"),
    }

    let mut count = 0u32;
    loop {
        log::info!("WHY2025-RS Base Firmware Online ({count})");
        count += 1;
        thread::sleep(Duration::from_secs(2));
    }
}
