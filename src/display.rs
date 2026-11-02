//! ST7703 720x720 MIPI-DSI panel bring-up, ported from the stock BadgeVMS driver
//! (`badgevms/drivers/st7703.[ch]` in the badge firmware repository).

use core::ffi::c_void;
use core::ptr;

use embedded_graphics::pixelcolor::raw::ToBytes;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use esp_idf_svc::sys::display_sys::*;
use esp_idf_svc::sys::{self, esp, EspError};

pub const WIDTH: usize = 720;
pub const HEIGHT: usize = 720;

const LCD_RST_GPIO: i32 = 17;
const DSI_PHY_LDO_CHAN: i32 = 3;
const DSI_PHY_LDO_MV: i32 = 2500;

struct Cmd {
    cmd: u8,
    data: &'static [u8],
    delay_ms: u32,
}

const fn cmd(cmd: u8, data: &'static [u8], delay_ms: u32) -> Cmd {
    Cmd { cmd, data, delay_ms }
}

// Init sequence for the "Bono" panel (black ribbon border, carrier board v2.1).
// The older "Mountain" panel (blue border) needs a different sequence and timings.
const INIT_CMDS: &[Cmd] = &[
    cmd(0xB9, &[0xF1, 0x12, 0x83], 0),
    cmd(0xB1, &[0x00, 0x00, 0x00, 0xDA, 0x80], 0),
    cmd(0xB2, &[0x3C, 0x02, 0x30], 0),
    cmd(0xB3, &[0x10, 0x10, 0x28, 0x28, 0x03, 0xFF, 0x00, 0x00, 0x00, 0x00], 0),
    cmd(0xB4, &[0x80], 0),
    cmd(0xB5, &[0x0A, 0x0A], 0),
    cmd(0xB6, &[0x97, 0x97], 0),
    cmd(0xB8, &[0x26, 0x22, 0xF0, 0x63], 0),
    cmd(
        0xBA,
        &[
            0x31, 0x81, 0x05, 0xF9, 0x0E, 0x0E, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x44,
            0x25, 0x00, 0x90, 0x0A, 0x00, 0x00, 0x01, 0x4F, 0x01, 0x00, 0x00, 0x37,
        ],
        0,
    ),
    cmd(0xBC, &[0x47], 0),
    cmd(0xBF, &[0x02, 0x11, 0x00], 0),
    cmd(0xC0, &[0x73, 0x73, 0x50, 0x50, 0x00, 0x00, 0x12, 0x70, 0x00], 0),
    cmd(0xC1, &[0x43, 0x00, 0x32, 0x32, 0x77, 0xC1, 0xFF, 0xFF, 0xCC, 0xCC, 0x77, 0x77], 0),
    cmd(0xC6, &[0x82, 0x00, 0xBF, 0xFF, 0x00, 0xFF], 0),
    cmd(0xC7, &[0xB8, 0x00, 0x0A, 0x00, 0x00, 0x00], 0),
    cmd(0xC8, &[0x10, 0x40, 0x1E, 0x02], 0),
    cmd(0xCC, &[0x0B], 0),
    cmd(
        0xE0,
        &[
            0x00, 0x0B, 0x10, 0x2C, 0x3D, 0x3F, 0x42, 0x3A, 0x07, 0x0D, 0x0F, 0x13, 0x15, 0x13, 0x14,
            0x0F, 0x16, 0x00, 0x0B, 0x10, 0x2C, 0x3D, 0x3F, 0x42, 0x3A, 0x07, 0x0D, 0x0F, 0x13, 0x15,
            0x13, 0x14, 0x0F, 0x16,
        ],
        0,
    ),
    cmd(
        0xE3,
        &[0x07, 0x07, 0x0B, 0x0B, 0x0B, 0x0B, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xC0, 0x10],
        0,
    ),
    cmd(
        0xE9,
        &[
            0xC8, 0x10, 0x0A, 0x00, 0x00, 0x80, 0x81, 0x12, 0x31, 0x23, 0x4F, 0x86, 0xA0, 0x00, 0x47,
            0x08, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x98, 0x02,
            0x8B, 0xAF, 0x46, 0x02, 0x88, 0x88, 0x88, 0x88, 0x88, 0x98, 0x13, 0x8B, 0xAF, 0x57, 0x13,
            0x88, 0x88, 0x88, 0x88, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00,
        ],
        0,
    ),
    cmd(
        0xEA,
        &[
            0x97, 0x0C, 0x09, 0x09, 0x09, 0x78, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9F, 0x31, 0x8B,
            0xA8, 0x31, 0x75, 0x88, 0x88, 0x88, 0x88, 0x88, 0x9F, 0x20, 0x8B, 0xA8, 0x20, 0x64, 0x88,
            0x88, 0x88, 0x88, 0x88, 0x23, 0x00, 0x00, 0x02, 0x62, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00,
        ],
        0,
    ),
    cmd(0x11, &[0x00], 250),
    cmd(0x29, &[0x00], 50),
];

pub struct Display {
    panel: esp_lcd_panel_handle_t,
    buf: Vec<u16>,
}

// The panel handle is only ever used behind a Mutex.
unsafe impl Send for Display {}

fn delay_ms(ms: u32) {
    esp_idf_svc::hal::delay::FreeRtos::delay_ms(ms);
}

impl Display {
    pub fn new() -> Result<Self, EspError> {
        unsafe {
            // Power the DSI PHY
            let mut ldo: esp_ldo_channel_handle_t = ptr::null_mut();
            let ldo_cfg = esp_ldo_channel_config_t {
                chan_id: DSI_PHY_LDO_CHAN,
                voltage_mv: DSI_PHY_LDO_MV,
                ..core::mem::zeroed()
            };
            esp!(esp_ldo_acquire_channel(&ldo_cfg, &mut ldo))?;

            // DSI bus: 2 lanes at 1 Gbps
            let mut bus: esp_lcd_dsi_bus_handle_t = ptr::null_mut();
            let bus_cfg = esp_lcd_dsi_bus_config_t {
                bus_id: 0,
                num_data_lanes: 2,
                phy_clk_src: 0,
                lane_bit_rate_mbps: 1000,
            };
            esp!(esp_lcd_new_dsi_bus(&bus_cfg, &mut bus))?;

            // DBI is used to send commands to the panel controller
            let mut io: esp_lcd_panel_io_handle_t = ptr::null_mut();
            let io_cfg = esp_lcd_dbi_io_config_t {
                virtual_channel: 0,
                lcd_cmd_bits: 8,
                lcd_param_bits: 8,
            };
            esp!(esp_lcd_new_panel_io_dbi(bus, &io_cfg, &mut io))?;

            // DPI carries the pixel stream
            let mut dpi_cfg: esp_lcd_dpi_panel_config_t = core::mem::zeroed();
            dpi_cfg.dpi_clk_src = 0;
            dpi_cfg.dpi_clock_freq_mhz = 47;
            dpi_cfg.virtual_channel = 0;
            dpi_cfg.pixel_format = lcd_color_rgb_pixel_format_t_LCD_COLOR_PIXEL_FORMAT_RGB565 as _;
            dpi_cfg.num_fbs = 1;
            dpi_cfg.video_timing.h_size = WIDTH as u32;
            dpi_cfg.video_timing.v_size = HEIGHT as u32;
            dpi_cfg.video_timing.hsync_back_porch = 120;
            dpi_cfg.video_timing.hsync_pulse_width = 60;
            dpi_cfg.video_timing.hsync_front_porch = 106;
            dpi_cfg.video_timing.vsync_back_porch = 20;
            dpi_cfg.video_timing.vsync_pulse_width = 4;
            dpi_cfg.video_timing.vsync_front_porch = 20;
            dpi_cfg.flags.set_use_dma2d(1);
            let mut panel: esp_lcd_panel_handle_t = ptr::null_mut();
            esp!(esp_lcd_new_panel_dpi(bus, &dpi_cfg, &mut panel))?;

            // Hardware reset (reset line is active high on the badge)
            let rst = sys::gpio_config_t {
                pin_bit_mask: 1u64 << LCD_RST_GPIO,
                mode: sys::gpio_mode_t_GPIO_MODE_OUTPUT,
                ..core::mem::zeroed()
            };
            esp!(sys::gpio_config(&rst))?;
            esp!(sys::gpio_set_level(LCD_RST_GPIO, 0))?;
            delay_ms(5);
            esp!(sys::gpio_set_level(LCD_RST_GPIO, 1))?;
            delay_ms(10);
            esp!(sys::gpio_set_level(LCD_RST_GPIO, 0))?;
            delay_ms(120);

            // Start the DPI stream, then configure the controller
            esp!(esp_lcd_panel_init(panel))?;
            let tx = |c: u8, d: &[u8]| {
                esp!(esp_lcd_panel_io_tx_param(
                    io,
                    c as i32,
                    d.as_ptr() as *const c_void,
                    d.len()
                ))
            };
            tx(0x36, &[0x08])?; // MADCTL: BGR order, the badge wires it backwards
            tx(0x3A, &[0x55])?; // COLMOD: RGB565
            for c in INIT_CMDS {
                tx(c.cmd, c.data)?;
                delay_ms(c.delay_ms);
            }

            Ok(Self {
                panel,
                buf: vec![0; WIDTH * HEIGHT],
            })
        }
    }

    /// Pushes the back buffer to the panel.
    pub fn flush(&mut self) -> Result<(), EspError> {
        unsafe {
            esp!(esp_lcd_panel_draw_bitmap(
                self.panel,
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                self.buf.as_ptr() as *const c_void
            ))
        }
    }
}

impl OriginDimensions for Display {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Display {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, c) in pixels {
            if p.x >= 0 && p.y >= 0 && (p.x as usize) < WIDTH && (p.y as usize) < HEIGHT {
                self.buf[p.y as usize * WIDTH + p.x as usize] = u16::from_le_bytes(c.to_le_bytes());
            }
        }
        Ok(())
    }
}
