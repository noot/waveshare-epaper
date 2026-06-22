//! Minimal IL0398 / SSD1683 driver for the Waveshare 4.2" e-Paper (400×300).
//!
//! Command sequence ported from Waveshare's epd4in2 reference code and
//! the epd-waveshare Rust crate.

use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::spi::SpiBus;
use esp_println::println;

pub const WIDTH: u32 = 400;
pub const HEIGHT: u32 = 300;
const ROW_BYTES: usize = (WIDTH as usize) / 8; // 50
pub const FB_SIZE: usize = ROW_BYTES * HEIGHT as usize; // 15000

// commands (IL0398 / SSD1683)
const CMD_PANEL_SETTING: u8 = 0x00;
const CMD_POWER_SETTING: u8 = 0x01;
const CMD_POWER_ON: u8 = 0x04;
const CMD_BOOSTER_SOFT_START: u8 = 0x06;
const CMD_DEEP_SLEEP: u8 = 0x07;
const CMD_DATA_START_1: u8 = 0x10; // old data (B/W mode)
const CMD_DATA_START_2: u8 = 0x13; // new data (B/W mode)
const CMD_DISPLAY_REFRESH: u8 = 0x12;
const CMD_LUT_VCOM: u8 = 0x20;
const CMD_LUT_WW: u8 = 0x21;
const CMD_LUT_BW: u8 = 0x22;
const CMD_LUT_WB: u8 = 0x23;
const CMD_LUT_BB: u8 = 0x24;
const CMD_PLL_CONTROL: u8 = 0x30;
const CMD_VCOM_DATA_INTERVAL: u8 = 0x50;
const CMD_TCON_SETTING: u8 = 0x60;
const CMD_RESOLUTION_SETTING: u8 = 0x61;
const CMD_VCOM_DC_SETTING: u8 = 0x82;
const CMD_PARTIAL_WINDOW: u8 = 0x90;
const CMD_PARTIAL_IN: u8 = 0x91;
const CMD_PARTIAL_OUT: u8 = 0x92;

// full refresh LUTs from epd-waveshare / Waveshare reference
#[rustfmt::skip]
const LUT_VCOM0: [u8; 44] = [
    0x00, 0x17, 0x00, 0x00, 0x00, 0x02,
    0x00, 0x17, 0x17, 0x00, 0x00, 0x02,
    0x00, 0x0A, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x0E, 0x0E, 0x00, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_WW: [u8; 42] = [
    0x40, 0x17, 0x00, 0x00, 0x00, 0x02,
    0x90, 0x17, 0x17, 0x00, 0x00, 0x02,
    0x40, 0x0A, 0x01, 0x00, 0x00, 0x01,
    0xA0, 0x0E, 0x0E, 0x00, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_BW: [u8; 42] = [
    0x40, 0x17, 0x00, 0x00, 0x00, 0x02,
    0x90, 0x17, 0x17, 0x00, 0x00, 0x02,
    0x40, 0x0A, 0x01, 0x00, 0x00, 0x01,
    0xA0, 0x0E, 0x0E, 0x00, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_WB: [u8; 42] = [
    0x80, 0x17, 0x00, 0x00, 0x00, 0x02,
    0x90, 0x17, 0x17, 0x00, 0x00, 0x02,
    0x80, 0x0A, 0x01, 0x00, 0x00, 0x01,
    0x50, 0x0E, 0x0E, 0x00, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_BB: [u8; 42] = [
    0x80, 0x17, 0x00, 0x00, 0x00, 0x02,
    0x90, 0x17, 0x17, 0x00, 0x00, 0x02,
    0x80, 0x0A, 0x01, 0x00, 0x00, 0x01,
    0x50, 0x0E, 0x0E, 0x00, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

// partial (quick) refresh LUTs — single short drive phase
#[rustfmt::skip]
const LUT_VCOM0_QUICK: [u8; 44] = [
    0x00, 0x0E, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_WW_QUICK: [u8; 42] = [
    0xA0, 0x0E, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_BW_QUICK: [u8; 42] = [
    0xA0, 0x0E, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_WB_QUICK: [u8; 42] = [
    0x50, 0x0E, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
#[rustfmt::skip]
const LUT_BB_QUICK: [u8; 42] = [
    0x50, 0x0E, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

#[derive(Debug)]
pub enum Error<S, P> {
    Spi(S),
    Pin(P),
}

pub struct Il0398<'a, SPI, CS, DC, RST, BUSY, DELAY> {
    spi: SPI,
    cs: CS,
    dc: DC,
    rst: RST,
    busy: BUSY,
    delay: DELAY,
    fb: &'a mut [u8; FB_SIZE],
}

impl<'a, SPI, CS, DC, RST, BUSY, DELAY> Il0398<'a, SPI, CS, DC, RST, BUSY, DELAY>
where
    SPI: SpiBus,
    CS: OutputPin,
    DC: OutputPin<Error = CS::Error>,
    RST: OutputPin<Error = CS::Error>,
    BUSY: InputPin<Error = CS::Error>,
    DELAY: DelayNs,
{
    pub fn new(
        spi: SPI,
        cs: CS,
        dc: DC,
        rst: RST,
        busy: BUSY,
        delay: DELAY,
        fb: &'a mut [u8; FB_SIZE],
    ) -> Self {
        Self {
            spi,
            cs,
            dc,
            rst,
            busy,
            delay,
            fb,
        }
    }

    fn cmd(&mut self, c: u8) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.dc.set_low().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        self.spi.write(&[c]).map_err(Error::Spi)?;
        self.spi.flush().map_err(Error::Spi)?;
        self.cs.set_high().map_err(Error::Pin)?;
        Ok(())
    }

    fn data(&mut self, d: &[u8]) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.dc.set_high().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        self.spi.write(d).map_err(Error::Spi)?;
        self.spi.flush().map_err(Error::Spi)?;
        self.cs.set_high().map_err(Error::Pin)?;
        Ok(())
    }

    fn cmd_with(&mut self, c: u8, d: &[u8]) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.cmd(c)?;
        self.data(d)
    }

    // IL0398 busy: LOW = busy, HIGH = idle
    fn wait_busy(&mut self, label: &str, timeout_ms: u32) {
        self.delay.delay_ms(10);
        let mut elapsed = 10u32;

        while self.busy.is_low().unwrap_or(true) {
            self.delay.delay_ms(100);
            elapsed += 100;
            if elapsed >= timeout_ms {
                println!("  busy ({}): timeout after {}ms", label, elapsed);
                return;
            }
        }
        println!("  busy ({}): ready after {}ms", label, elapsed);
    }

    fn reset(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.rst.set_low().map_err(Error::Pin)?;
        self.delay.delay_ms(2);
        self.rst.set_high().map_err(Error::Pin)?;
        self.delay.delay_ms(20);
        self.rst.set_low().map_err(Error::Pin)?;
        self.delay.delay_ms(2);
        self.rst.set_high().map_err(Error::Pin)?;
        self.delay.delay_ms(20);
        self.rst.set_low().map_err(Error::Pin)?;
        self.delay.delay_ms(2);
        self.rst.set_high().map_err(Error::Pin)?;
        self.delay.delay_ms(20);
        Ok(())
    }

    fn send_init_sequence(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        // power setting
        self.cmd_with(CMD_POWER_SETTING, &[0x03, 0x00, 0x2b, 0x2b])?;
        // booster soft start
        self.cmd_with(CMD_BOOSTER_SOFT_START, &[0x17, 0x17, 0x17])?;
        // power on
        self.cmd(CMD_POWER_ON)?;
        self.wait_busy("power-on", 10000);
        // panel setting: B/W mode, LUT from OTP
        self.cmd_with(CMD_PANEL_SETTING, &[0xbf])?;
        // PLL: 100Hz
        self.cmd_with(CMD_PLL_CONTROL, &[0x3c])?;
        // resolution: 400×300
        self.cmd_with(
            CMD_RESOLUTION_SETTING,
            &[
                (WIDTH >> 8) as u8,
                (WIDTH & 0xFF) as u8,
                (HEIGHT >> 8) as u8,
                (HEIGHT & 0xFF) as u8,
            ],
        )?;
        // VCOM DC
        self.cmd_with(CMD_VCOM_DC_SETTING, &[0x12])?;
        // VCOM and data interval: white border
        self.cmd_with(CMD_VCOM_DATA_INTERVAL, &[0x97])?;
        Ok(())
    }

    fn set_lut_full(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.cmd_with(CMD_LUT_VCOM, &LUT_VCOM0)?;
        self.cmd_with(CMD_LUT_WW, &LUT_WW)?;
        self.cmd_with(CMD_LUT_BW, &LUT_BW)?;
        self.cmd_with(CMD_LUT_WB, &LUT_WB)?;
        self.cmd_with(CMD_LUT_BB, &LUT_BB)?;
        Ok(())
    }

    fn set_lut_quick(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.cmd_with(CMD_LUT_VCOM, &LUT_VCOM0_QUICK)?;
        self.cmd_with(CMD_LUT_WW, &LUT_WW_QUICK)?;
        self.cmd_with(CMD_LUT_BW, &LUT_BW_QUICK)?;
        self.cmd_with(CMD_LUT_WB, &LUT_WB_QUICK)?;
        self.cmd_with(CMD_LUT_BB, &LUT_BB_QUICK)?;
        Ok(())
    }

    pub fn init(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        println!("il0398: hardware reset");
        self.reset()?;
        self.wait_busy("reset", 5000);
        println!("il0398: init done");
        Ok(())
    }

    pub fn clear_white(&mut self) {
        self.fb.fill(0xFF);
    }

    pub fn framebuffer_mut(&mut self) -> &mut [u8; FB_SIZE] {
        self.fb
    }

    /// Full refresh — clean, no ghosting, ~2-4s
    pub fn flush(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        println!("il0398: full flush start");
        self.send_init_sequence()?;
        self.set_lut_full()?;
        self.write_framebuffer()?;
        self.cmd(CMD_DISPLAY_REFRESH)?;
        self.wait_busy("refresh", 30000);
        println!("il0398: full flush done");
        Ok(())
    }

    /// Partial (quick) refresh — faster but may ghost over time
    pub fn flush_partial(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        println!("il0398: partial flush start");
        self.send_init_sequence()?;
        // switch to register LUT mode
        self.cmd_with(CMD_PANEL_SETTING, &[0x3f])?;
        self.set_lut_quick()?;
        self.write_framebuffer()?;
        self.cmd(CMD_DISPLAY_REFRESH)?;
        self.wait_busy("refresh", 30000);
        println!("il0398: partial flush done");
        Ok(())
    }

    /// Enter deep sleep mode — wake with hardware reset
    pub fn sleep(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.cmd_with(CMD_DEEP_SLEEP, &[0xA5])?;
        Ok(())
    }

    fn write_framebuffer(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.cmd(CMD_DATA_START_2)?;
        self.dc.set_high().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        for chunk in self.fb.chunks(ROW_BYTES) {
            self.spi.write(chunk).map_err(Error::Spi)?;
            self.spi.flush().map_err(Error::Spi)?;
        }
        self.cs.set_high().map_err(Error::Pin)?;
        Ok(())
    }

    fn set_pixel(&mut self, x: u32, y: u32, black: bool) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let idx = y as usize * ROW_BYTES + (x as usize >> 3);
        let mask = 0x80u8 >> (x & 7);
        if black {
            self.fb[idx] &= !mask;
        } else {
            self.fb[idx] |= mask;
        }
    }
}

impl<SPI, CS, DC, RST, BUSY, DELAY> Dimensions for Il0398<'_, SPI, CS, DC, RST, BUSY, DELAY> {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), Size::new(WIDTH, HEIGHT))
    }
}

impl<SPI, CS, DC, RST, BUSY, DELAY> DrawTarget for Il0398<'_, SPI, CS, DC, RST, BUSY, DELAY>
where
    SPI: SpiBus,
    CS: OutputPin,
    DC: OutputPin<Error = CS::Error>,
    RST: OutputPin<Error = CS::Error>,
    BUSY: InputPin<Error = CS::Error>,
    DELAY: DelayNs,
{
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0 && point.y >= 0 {
                self.set_pixel(point.x as u32, point.y as u32, color.is_on());
            }
        }
        Ok(())
    }
}
