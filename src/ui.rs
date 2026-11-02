use std::collections::VecDeque;

use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};
use log::Level;

use crate::display::{Display, HEIGHT, WIDTH};
use crate::wifi::{Network, WifiCommand, WifiSnapshot, WifiStatus};

const COLUMNS: usize = WIDTH / 10;
const LOG_ROWS: usize = 8;
const VISIBLE_NETWORKS: usize = 8;
const BACKGROUND: Rgb565 = Rgb565::new(2, 4, 8);
const SURFACE: Rgb565 = Rgb565::new(5, 9, 15);
const ACCENT: Rgb565 = Rgb565::new(8, 52, 48);
const SELECTED: Rgb565 = Rgb565::new(10, 28, 27);

struct PasswordPrompt {
    ssid: String,
    value: String,
}

pub struct Dashboard {
    display: Display,
    lines: VecDeque<(Level, String)>,
    wifi: WifiSnapshot,
    selected_network: usize,
    first_visible_network: usize,
    password_prompt: Option<PasswordPrompt>,
    shift_pressed: bool,
    notice: Option<String>,
    dirty: bool,
}

impl Dashboard {
    pub fn new(display: Display) -> Self {
        Self {
            display,
            lines: VecDeque::new(),
            wifi: WifiSnapshot::default(),
            selected_network: 0,
            first_visible_network: 0,
            password_prompt: None,
            shift_pressed: false,
            notice: None,
            dirty: true,
        }
    }

    pub fn push_log(&mut self, level: Level, text: &str) {
        for line in text.lines() {
            let chars: Vec<char> = line.chars().collect();
            for chunk in chars.chunks(COLUMNS) {
                if self.lines.len() == LOG_ROWS {
                    self.lines.pop_front();
                }
                self.lines.push_back((level, chunk.iter().collect()));
            }
        }
        self.dirty = true;
    }

    pub fn set_wifi(&mut self, wifi: WifiSnapshot) {
        if self.wifi.status != wifi.status && self.password_prompt.is_none() {
            self.notice = None;
        }
        if self.wifi.networks != wifi.networks {
            let selected_ssid = self
                .wifi
                .networks
                .get(self.selected_network)
                .map(|network| network.ssid.as_str());
            self.selected_network = selected_ssid
                .and_then(|ssid| {
                    wifi.networks
                        .iter()
                        .position(|network| network.ssid == ssid)
                })
                .unwrap_or_else(|| {
                    self.selected_network
                        .min(wifi.networks.len().saturating_sub(1))
                });
            self.keep_selection_visible(wifi.networks.len());
        }
        if self.wifi != wifi {
            self.wifi = wifi;
            self.dirty = true;
        }
    }

    pub fn handle_key(&mut self, key_code: u8, pressed: bool) -> Option<WifiCommand> {
        if matches!(key_code, 41 | 56) {
            self.shift_pressed = pressed;
            return None;
        }
        if !pressed {
            return None;
        }

        if self.password_prompt.is_some() {
            return self.handle_password_key(key_code);
        }

        match key_code {
            55 => self.move_selection(-1),
            52 => self.move_selection(1),
            59 => return self.connect_selected(),
            25 => {
                self.notice = Some("Scanning for networks...".to_owned());
                self.dirty = true;
                return Some(WifiCommand::Scan);
            }
            34 => {
                self.notice = Some("Disconnecting...".to_owned());
                self.dirty = true;
                return Some(WifiCommand::Disconnect);
            }
            35 => {
                self.notice = Some("Forgetting saved network...".to_owned());
                self.dirty = true;
                return Some(WifiCommand::Forget);
            }
            _ => {}
        }
        None
    }

    pub fn render_if_dirty(&mut self) -> Result<(), esp_idf_svc::sys::EspError> {
        if !self.dirty {
            return Ok(());
        }
        self.dirty = false;

        let _ = self.display.clear(BACKGROUND);
        self.fill(0, 0, WIDTH as u32, 6, ACCENT);
        self.text("WHY2025  /  DEVICE STATUS", 26, 22, Rgb565::WHITE);
        self.text("P4 + C6  |  LIVE", 26, 48, Rgb565::CSS_GRAY);

        self.fill(24, 80, 672, 120, SURFACE);
        self.fill(24, 80, 6, 120, ACCENT);
        self.text("WIFI", 44, 96, Rgb565::CSS_GRAY);
        self.text(
            self.wifi.status.label(),
            44,
            122,
            status_color(&self.wifi.status),
        );
        let network = self
            .wifi
            .ssid
            .as_deref()
            .map(display_text)
            .unwrap_or_else(|| "No network selected".into());
        self.text(&network, 44, 150, Rgb565::WHITE);
        let ip = self
            .wifi
            .ip
            .as_deref()
            .map(|ip| format!("IP  {ip}"))
            .or_else(|| self.wifi.detail.as_deref().map(display_text))
            .unwrap_or_else(|| "Waiting for a network".into());
        self.text(&ip, 44, 174, Rgb565::CSS_GRAY);

        self.text("NEARBY NETWORKS", 26, 220, Rgb565::WHITE);
        self.text(
            &format!("{} found", self.wifi.networks.len()),
            590,
            220,
            Rgb565::CSS_GRAY,
        );
        let network_rows: Vec<(usize, String)> = self
            .wifi
            .networks
            .iter()
            .enumerate()
            .skip(self.first_visible_network)
            .take(VISIBLE_NETWORKS)
            .map(|(index, network)| (index, format_network(index + 1, network)))
            .collect();
        for (row, (index, text)) in network_rows.iter().enumerate() {
            let y = 250 + row as i32 * 25;
            if *index == self.selected_network && self.password_prompt.is_none() {
                self.fill(24, y - 2, 672, 24, SELECTED);
            }
            let color = if *index == self.selected_network {
                Rgb565::WHITE
            } else {
                Rgb565::CSS_GRAY
            };
            self.text(text, 34, y, color);
        }
        if self.wifi.networks.is_empty() {
            self.text(
                "No networks found. Press R to scan.",
                34,
                252,
                Rgb565::CSS_GRAY,
            );
        }

        if let Some((ssid, password_length)) = self
            .password_prompt
            .as_ref()
            .map(|prompt| (prompt.ssid.clone(), prompt.value.len()))
        {
            self.fill(24, 232, 672, 218, SURFACE);
            self.fill(24, 232, 6, 218, ACCENT);
            self.text("CONNECT TO NETWORK", 44, 250, Rgb565::CSS_GRAY);
            self.text(&display_text(&ssid), 44, 278, Rgb565::WHITE);
            self.text("PASSWORD", 44, 310, Rgb565::CSS_GRAY);
            self.text(&"*".repeat(password_length), 44, 336, Rgb565::WHITE);
            self.text(
                "Type password; hold either SHIFT for capitals/symbols.",
                44,
                370,
                Rgb565::CSS_GRAY,
            );
            self.text(
                "ENTER connect  BACKSPACE delete  ESC cancel",
                44,
                398,
                Rgb565::CSS_GRAY,
            );
            if let Some(notice) = self.notice.as_deref() {
                self.text(&display_text(notice), 44, 422, Rgb565::YELLOW);
            }
        } else {
            let notice = self
                .notice
                .as_deref()
                .unwrap_or("UP/DOWN select  ENTER connect  R rescan  D disconnect  F forget");
            self.text(&display_text(notice), 34, 452, Rgb565::CSS_GRAY);
        }

        self.fill(24, 472, 672, 2, ACCENT);
        self.text("SYSTEM LOG", 26, 486, Rgb565::WHITE);
        let first_y = 516;
        let row_height = ((HEIGHT as i32 - first_y - 26) / LOG_ROWS as i32).max(20);
        let log_rows: Vec<(Level, String)> = self.lines.iter().cloned().collect();
        for (row, (level, line)) in log_rows.iter().enumerate() {
            self.text(
                line,
                26,
                first_y + row as i32 * row_height,
                log_color(*level),
            );
        }
        self.text(
            "Serial: wifi scan | wifi connect <number> [password] | wifi forget",
            26,
            696,
            Rgb565::CSS_GRAY,
        );
        self.display.flush()
    }

    fn move_selection(&mut self, direction: isize) {
        let count = self.wifi.networks.len();
        if count == 0 {
            return;
        }
        self.selected_network = if direction < 0 {
            if self.selected_network == 0 {
                count - 1
            } else {
                self.selected_network - 1
            }
        } else {
            (self.selected_network + 1) % count
        };
        self.keep_selection_visible(count);
        self.dirty = true;
    }

    fn keep_selection_visible(&mut self, network_count: usize) {
        if self.selected_network < self.first_visible_network {
            self.first_visible_network = self.selected_network;
        } else if self.selected_network >= self.first_visible_network + VISIBLE_NETWORKS {
            self.first_visible_network = self.selected_network + 1 - VISIBLE_NETWORKS;
        }
        self.first_visible_network = self
            .first_visible_network
            .min(network_count.saturating_sub(VISIBLE_NETWORKS));
    }

    fn connect_selected(&mut self) -> Option<WifiCommand> {
        let Some(network) = self.wifi.networks.get(self.selected_network).cloned() else {
            self.notice = Some("No network selected; press R to scan.".to_owned());
            self.dirty = true;
            return None;
        };
        if !network.connectable {
            self.notice = Some(format!("{} security is not supported.", network.security));
            self.dirty = true;
            return None;
        }
        if network.ssid.is_empty() {
            self.notice = Some("Hidden networks cannot be selected from a scan.".to_owned());
            self.dirty = true;
            return None;
        }

        if network.security == "OPEN" {
            self.notice = Some(format!("Connecting to {}...", display_text(&network.ssid)));
            self.dirty = true;
            Some(WifiCommand::Connect {
                index: self.selected_network + 1,
                password: String::new(),
            })
        } else {
            self.password_prompt = Some(PasswordPrompt {
                ssid: network.ssid,
                value: String::new(),
            });
            self.notice = None;
            self.dirty = true;
            None
        }
    }

    fn handle_password_key(&mut self, key_code: u8) -> Option<WifiCommand> {
        match key_code {
            1 => {
                self.password_prompt = None;
                self.notice = Some("Connection cancelled.".to_owned());
                self.dirty = true;
            }
            8 => {
                if let Some(prompt) = self.password_prompt.as_mut() {
                    prompt.value.pop();
                    self.dirty = true;
                }
            }
            59 => {
                let Some(prompt) = self.password_prompt.take() else {
                    return None;
                };
                if prompt.value.len() < 8 {
                    self.notice =
                        Some("WPA passwords must contain at least 8 characters.".to_owned());
                    self.password_prompt = Some(prompt);
                    self.dirty = true;
                    return None;
                }
                let Some(index) = self
                    .wifi
                    .networks
                    .iter()
                    .position(|network| network.ssid == prompt.ssid)
                else {
                    self.notice =
                        Some("Network disappeared; scan again before connecting.".to_owned());
                    self.dirty = true;
                    return None;
                };
                self.notice = Some(format!("Connecting to {}...", display_text(&prompt.ssid)));
                self.dirty = true;
                return Some(WifiCommand::Connect {
                    index: index + 1,
                    password: prompt.value,
                });
            }
            _ => {
                if let Some(character) = key_character(key_code, self.shift_pressed) {
                    if let Some(prompt) = self.password_prompt.as_mut() {
                        if prompt.value.len() < 63 {
                            prompt.value.push(character);
                            self.dirty = true;
                        }
                    }
                }
            }
        }
        None
    }

    fn fill(&mut self, x: i32, y: i32, width: u32, height: u32, color: Rgb565) {
        let _ = Rectangle::new(Point::new(x, y), Size::new(width, height))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(&mut self.display);
    }

    fn text(&mut self, text: &str, x: i32, y: i32, color: Rgb565) {
        let style = MonoTextStyle::new(&FONT_10X20, color);
        let _ = Text::with_baseline(text, Point::new(x, y), style, Baseline::Top)
            .draw(&mut self.display);
    }
}

fn format_network(index: usize, network: &Network) -> String {
    let name = if network.ssid.is_empty() {
        "Hidden network".to_owned()
    } else {
        display_text(&network.ssid)
    };
    let connectability = if network.connectable { "" } else { "  -" };
    format!(
        "{index:02}  {name:<30} {:>4} dBm  {}{}",
        network.signal_strength, network.security, connectability
    )
}

fn key_character(key_code: u8, shift: bool) -> Option<char> {
    let character = match key_code {
        9 => {
            if shift {
                ')'
            } else {
                '0'
            }
        }
        10 => {
            if shift {
                '_'
            } else {
                '-'
            }
        }
        11 => {
            if shift {
                '~'
            } else {
                '`'
            }
        }
        12..=20 => {
            const DIGITS: &[u8] = b"123456789";
            const SHIFTED: &[u8] = b"!@#$%^&*(";
            let index = usize::from(key_code - 12);
            if shift {
                SHIFTED[index] as char
            } else {
                DIGITS[index] as char
            }
        }
        22..=30 => shifted_letter(b"qwertyuio", key_code - 22, shift),
        32..=40 => shifted_letter(b"asdfghjkl", key_code - 32, shift),
        42..=50 => match key_code {
            42..=48 => shifted_letter(b"zxcvbnm", key_code - 42, shift),
            49 => {
                if shift {
                    '<'
                } else {
                    ','
                }
            }
            50 => {
                if shift {
                    '>'
                } else {
                    '.'
                }
            }
            _ => return None,
        },
        54 => {
            if shift {
                '?'
            } else {
                '/'
            }
        }
        57 => ':',
        58 => '"',
        60 => '+',
        64 => '\\',
        65..=67 => ' ',
        69 => {
            if shift {
                'P'
            } else {
                'p'
            }
        }
        70 => '{',
        80 => '}',
        _ => return None,
    };
    Some(character)
}

fn shifted_letter(keys: &[u8], offset: u8, shift: bool) -> char {
    let character = keys[usize::from(offset)] as char;
    if shift {
        character.to_ascii_uppercase()
    } else {
        character
    }
}

fn status_color(status: &WifiStatus) -> Rgb565 {
    match status {
        WifiStatus::Connected => Rgb565::GREEN,
        WifiStatus::Scanning | WifiStatus::Connecting | WifiStatus::Starting => Rgb565::YELLOW,
        WifiStatus::Ready | WifiStatus::Disconnected => Rgb565::CYAN,
        WifiStatus::Error | WifiStatus::Unavailable => Rgb565::RED,
    }
}

fn log_color(level: Level) -> Rgb565 {
    match level {
        Level::Error => Rgb565::RED,
        Level::Warn => Rgb565::YELLOW,
        Level::Info => Rgb565::GREEN,
        _ => Rgb565::CSS_GRAY,
    }
}

fn display_text(text: &str) -> String {
    text.chars()
        .take(COLUMNS - 8)
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                '?'
            }
        })
        .collect()
}
