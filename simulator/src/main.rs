use core::convert::Infallible;

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics_simulator::sdl2::Keycode;
use embedded_graphics_simulator::{
    OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};
use why2025_ui::{
    draw_widget_showcase, Dashboard, Network, UiDisplay, WifiSnapshot, WifiStatus, HEIGHT, WIDTH,
};

struct HostDisplay(SimulatorDisplay<Rgb565>);

impl OriginDimensions for HostDisplay {
    fn size(&self) -> Size {
        self.0.size()
    }
}

impl DrawTarget for HostDisplay {
    type Color = Rgb565;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        self.0.draw_iter(pixels)
    }
}

impl UiDisplay for HostDisplay {
    type FlushError = Infallible;

    fn flush(&mut self) -> Result<(), Self::FlushError> {
        Ok(())
    }
}

fn main() -> Result<(), Infallible> {
    let simulator_display = SimulatorDisplay::<Rgb565>::new(Size::new(WIDTH as u32, HEIGHT as u32));
    let mut dashboard = Dashboard::new(HostDisplay(simulator_display));
    dashboard.set_wifi(WifiSnapshot {
        status: WifiStatus::Connected,
        ssid: Some("WHY2025-LAB".to_owned()),
        ip: Some("192.168.1.42".to_owned()),
        networks: vec![
            Network {
                ssid: "WHY2025-LAB".to_owned(),
                signal_strength: -42,
                security: "WPA2",
                connectable: true,
            },
            Network {
                ssid: "BadgeHub".to_owned(),
                signal_strength: -68,
                security: "OPEN",
                connectable: true,
            },
            Network {
                ssid: "eduroam".to_owned(),
                signal_strength: -76,
                security: "WPA2-ENTERPRISE",
                connectable: false,
            },
        ],
        ..WifiSnapshot::default()
    });

    let output_settings = OutputSettingsBuilder::new().scale(1).build();
    let mut window = Window::new("WHY2025 UI simulator", &output_settings);
    let mut show_dashboard = false;
    let mut showcase_dirty = true;

    loop {
        if show_dashboard {
            dashboard.render_if_dirty()?;
        } else if showcase_dirty {
            draw_widget_showcase(dashboard.display_mut())?;
            showcase_dirty = false;
        }

        window.update(&dashboard.display().0);
        for event in window.events().collect::<Vec<_>>() {
            match event {
                SimulatorEvent::Quit => return Ok(()),
                SimulatorEvent::KeyDown {
                    keycode: Keycode::Tab,
                    ..
                } => {
                    show_dashboard = !show_dashboard;
                    showcase_dirty = true;
                    dashboard.invalidate();
                }
                SimulatorEvent::KeyDown { keycode, .. } if show_dashboard => {
                    if let Some((code, pressed)) = keypad_event(keycode, true) {
                        dashboard.handle_key(code, pressed);
                    }
                }
                SimulatorEvent::KeyUp { keycode, .. } if show_dashboard => {
                    if let Some((code, pressed)) = keypad_event(keycode, false) {
                        dashboard.handle_key(code, pressed);
                    }
                }
                _ => {}
            }
        }
    }
}

fn keypad_event(keycode: Keycode, pressed: bool) -> Option<(u8, bool)> {
    let code = match keycode {
        Keycode::Up => 55,
        Keycode::Down => 52,
        Keycode::Return | Keycode::KpEnter => 59,
        Keycode::R => 25,
        Keycode::D => 34,
        Keycode::F => 35,
        Keycode::Backspace => 8,
        Keycode::LShift => 41,
        Keycode::RShift => 56,
        Keycode::Num1 => 12,
        Keycode::Num2 => 13,
        Keycode::Num3 => 14,
        Keycode::Num4 => 15,
        Keycode::Num5 => 16,
        Keycode::Num6 => 17,
        Keycode::Num7 => 18,
        Keycode::Num8 => 19,
        Keycode::Num9 => 20,
        Keycode::Q => 22,
        Keycode::W => 23,
        Keycode::E => 24,
        Keycode::T => 26,
        Keycode::Y => 26,
        Keycode::U => 27,
        Keycode::I => 28,
        Keycode::O => 29,
        Keycode::A => 32,
        Keycode::S => 33,
        Keycode::G => 36,
        Keycode::H => 37,
        Keycode::J => 38,
        Keycode::K => 39,
        Keycode::L => 40,
        Keycode::Z => 42,
        Keycode::X => 43,
        Keycode::C => 44,
        Keycode::V => 45,
        Keycode::B => 46,
        Keycode::N => 47,
        Keycode::M => 48,
        Keycode::Space => 65,
        Keycode::Comma => 49,
        Keycode::Period => 50,
        Keycode::Slash => 54,
        _ => return None,
    };
    Some((code, pressed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showcase_renders_to_host_display() {
        let mut display = HostDisplay(SimulatorDisplay::<Rgb565>::new(Size::new(
            WIDTH as u32,
            HEIGHT as u32,
        )));

        draw_widget_showcase(&mut display).expect("showcase drawing is infallible");

        assert_eq!(
            display.0.get_pixel(Point::new(0, 0)),
            Rgb565::new(8, 52, 48)
        );
        assert_eq!(
            display.0.get_pixel(Point::new(30, 85)),
            Rgb565::new(5, 9, 15)
        );
    }

    #[test]
    fn dashboard_renders_to_host_display() {
        let display = HostDisplay(SimulatorDisplay::<Rgb565>::new(Size::new(
            WIDTH as u32,
            HEIGHT as u32,
        )));
        let mut dashboard = Dashboard::new(display);
        dashboard.set_wifi(WifiSnapshot {
            status: WifiStatus::Connected,
            ssid: Some("WHY2025-LAB".to_owned()),
            ..WifiSnapshot::default()
        });

        dashboard
            .render_if_dirty()
            .expect("host display flushing is infallible");

        assert_eq!(
            dashboard.display().0.get_pixel(Point::new(30, 85)),
            Rgb565::new(5, 9, 15)
        );
    }
}
