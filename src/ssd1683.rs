//! Minimal SSD1683 driver for the Waveshare 4.2" e-Paper V2 Rev 2.2 (GDEY042T81).
//!
//! Command sequence follows the Waveshare EPD_4in2_V2 reference implementation.
//! This is NOT the older IL0398 — Rev 2.2 uses SSD1683 with SSD1681-style commands.

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

// SSD1683 commands
const CMD_SW_RESET: u8 = 0x12;
const CMD_DATA_ENTRY_MODE: u8 = 0x11;
const CMD_SET_RAM_X_ADDR: u8 = 0x44;
const CMD_SET_RAM_Y_ADDR: u8 = 0x45;
const CMD_SET_RAM_X_COUNT: u8 = 0x4E;
const CMD_SET_RAM_Y_COUNT: u8 = 0x4F;
const CMD_WRITE_RAM_BW: u8 = 0x24; // current (new) B/W data
const CMD_WRITE_RAM_RED: u8 = 0x26; // previous (old) data for partial
const CMD_BORDER_WAVEFORM: u8 = 0x3C;
const CMD_WRITE_TEMP: u8 = 0x1A;
const CMD_DISPLAY_UPDATE_CTRL: u8 = 0x21;
const CMD_DISPLAY_UPDATE_SEQ: u8 = 0x22;
const CMD_MASTER_ACTIVATE: u8 = 0x20;
const CMD_DEEP_SLEEP: u8 = 0x10;

#[derive(Debug)]
pub enum Error<S, P> {
    Spi(S),
    Pin(P),
}

pub struct Ssd1683<'a, SPI, CS, DC, RST, BUSY, DELAY> {
    spi: SPI,
    cs: CS,
    dc: DC,
    rst: RST,
    busy: BUSY,
    delay: DELAY,
    fb: &'a mut [u8; FB_SIZE],
    initialized: bool,
}

impl<'a, SPI, CS, DC, RST, BUSY, DELAY> Ssd1683<'a, SPI, CS, DC, RST, BUSY, DELAY>
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
            initialized: false,
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

    // BUSY: LOW = busy, HIGH = idle (active low, confirmed by logic trace)
    fn wait_busy(&mut self, label: &str, timeout_ms: u32) {
        self.delay.delay_ms(10);
        let mut elapsed = 10u32;

        while self.busy.is_low().unwrap_or(true) {
            self.delay.delay_ms(10);
            elapsed += 10;
            if elapsed >= timeout_ms {
                println!("  busy ({}): timeout after {}ms", label, elapsed);
                return;
            }
        }
        println!("  busy ({}): ready after {}ms", label, elapsed);
    }

    fn reset(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.rst.set_high().map_err(Error::Pin)?;
        self.delay.delay_ms(20);
        self.rst.set_low().map_err(Error::Pin)?;
        self.delay.delay_ms(2);
        self.rst.set_high().map_err(Error::Pin)?;
        self.delay.delay_ms(20);
        Ok(())
    }

    /// Set the partial RAM area for writing
    fn set_ram_area(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    ) -> Result<(), Error<SPI::Error, CS::Error>> {
        // data entry mode: x increase, y increase (normal)
        self.cmd_with(CMD_DATA_ENTRY_MODE, &[0x03])?;
        // x address range (in bytes)
        self.cmd_with(
            CMD_SET_RAM_X_ADDR,
            &[(x / 8) as u8, ((x + w - 1) / 8) as u8],
        )?;
        // y address range
        self.cmd_with(
            CMD_SET_RAM_Y_ADDR,
            &[
                (y % 256) as u8,
                (y / 256) as u8,
                ((y + h - 1) % 256) as u8,
                ((y + h - 1) / 256) as u8,
            ],
        )?;
        // set counters to start position
        self.cmd_with(CMD_SET_RAM_X_COUNT, &[(x / 8) as u8])?;
        self.cmd_with(CMD_SET_RAM_Y_COUNT, &[(y % 256) as u8, (y / 256) as u8])?;
        Ok(())
    }

    /// Initialize the display — matches Waveshare EPD_4IN2_V2_Init
    fn init_display(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.cmd(CMD_SW_RESET)?;
        self.wait_busy("sw_reset", 5000);
        // display update control: bypass RED channel
        self.cmd_with(CMD_DISPLAY_UPDATE_CTRL, &[0x40, 0x00])?;
        // border waveform
        self.cmd_with(CMD_BORDER_WAVEFORM, &[0x05])?;
        // data entry mode + RAM window + cursor
        self.set_ram_area(0, 0, WIDTH as u16, HEIGHT as u16)?;
        self.wait_busy("init", 5000);
        self.initialized = true;
        Ok(())
    }

    pub fn init(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        println!("ssd1683: hardware reset");
        self.reset()?;
        self.wait_busy("reset", 5000);
        self.init_display()?;
        println!("ssd1683: init done");
        Ok(())
    }

    pub fn clear_white(&mut self) {
        self.fb.fill(0xFF);
    }

    pub fn framebuffer_mut(&mut self) -> &mut [u8; FB_SIZE] {
        self.fb
    }

    /// Write framebuffer to both current and previous RAM (needed for full refresh)
    fn write_framebuffer_full(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        // write to "previous" RAM (0x26)
        self.set_ram_area(0, 0, WIDTH as u16, HEIGHT as u16)?;
        self.cmd(CMD_WRITE_RAM_RED)?;
        self.dc.set_high().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        for chunk in self.fb.chunks(ROW_BYTES) {
            self.spi.write(chunk).map_err(Error::Spi)?;
            self.spi.flush().map_err(Error::Spi)?;
        }
        self.cs.set_high().map_err(Error::Pin)?;

        // write to "current" RAM (0x24)
        self.set_ram_area(0, 0, WIDTH as u16, HEIGHT as u16)?;
        self.cmd(CMD_WRITE_RAM_BW)?;
        self.dc.set_high().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        for chunk in self.fb.chunks(ROW_BYTES) {
            self.spi.write(chunk).map_err(Error::Spi)?;
            self.spi.flush().map_err(Error::Spi)?;
        }
        self.cs.set_high().map_err(Error::Pin)?;

        Ok(())
    }

    /// Write framebuffer to current RAM only (for partial refresh)
    fn write_framebuffer_current(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.set_ram_area(0, 0, WIDTH as u16, HEIGHT as u16)?;
        self.cmd(CMD_WRITE_RAM_BW)?;
        self.dc.set_high().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        for chunk in self.fb.chunks(ROW_BYTES) {
            self.spi.write(chunk).map_err(Error::Spi)?;
            self.spi.flush().map_err(Error::Spi)?;
        }
        self.cs.set_high().map_err(Error::Pin)?;
        Ok(())
    }

    /// Copy current RAM to previous RAM (after partial refresh)
    fn copy_current_to_previous(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.set_ram_area(0, 0, WIDTH as u16, HEIGHT as u16)?;
        self.cmd(CMD_WRITE_RAM_BW)?;
        self.dc.set_high().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        for chunk in self.fb.chunks(ROW_BYTES) {
            self.spi.write(chunk).map_err(Error::Spi)?;
            self.spi.flush().map_err(Error::Spi)?;
        }
        self.cs.set_high().map_err(Error::Pin)?;

        self.set_ram_area(0, 0, WIDTH as u16, HEIGHT as u16)?;
        self.cmd(CMD_WRITE_RAM_RED)?;
        self.dc.set_high().map_err(Error::Pin)?;
        self.cs.set_low().map_err(Error::Pin)?;
        for chunk in self.fb.chunks(ROW_BYTES) {
            self.spi.write(chunk).map_err(Error::Spi)?;
            self.spi.flush().map_err(Error::Spi)?;
        }
        self.cs.set_high().map_err(Error::Pin)?;
        Ok(())
    }

    /// Full refresh — clean, no ghosting, ~3-4s
    pub fn flush(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        println!("ssd1683: full flush start");
        if !self.initialized {
            self.init_display()?;
        }
        // write to both current and previous ram so a following partial refresh
        // has a correct base image to diff against
        self.write_framebuffer_full()?;

        // bypass red channel — also resets the mode if a prior partial set it normal
        self.cmd_with(CMD_DISPLAY_UPDATE_CTRL, &[0x40, 0x00])?;
        self.cmd_with(CMD_DISPLAY_UPDATE_SEQ, &[0xf7])?;
        self.cmd(CMD_MASTER_ACTIVATE)?;
        self.wait_busy("refresh", 30000);
        println!("ssd1683: full flush done");
        Ok(())
    }

    /// Fast full refresh — slightly faster, uses temperature override
    pub fn flush_fast(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        println!("ssd1683: fast flush start");
        if !self.initialized {
            self.init_display()?;
        }
        self.write_framebuffer_full()?;

        // display update control: bypass RED channel
        self.cmd_with(CMD_DISPLAY_UPDATE_CTRL, &[0x40, 0x00])?;
        // temperature override for faster waveform
        self.cmd_with(CMD_WRITE_TEMP, &[0x6E])?;
        // display update sequence: fast full
        self.cmd_with(CMD_DISPLAY_UPDATE_SEQ, &[0xd7])?;
        self.cmd(CMD_MASTER_ACTIVATE)?;
        self.wait_busy("refresh", 30000);
        println!("ssd1683: fast flush done");
        Ok(())
    }

    /// Partial refresh — fast, only changed pixels driven. May ghost over time.
    pub fn flush_partial(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        println!("ssd1683: partial flush start");
        if !self.initialized {
            self.init_display()?;
        }

        // write current data only (previous stays in RAM from last write)
        self.write_framebuffer_current()?;

        // border waveform: follow LUT to avoid border flicker during partial
        self.cmd_with(CMD_BORDER_WAVEFORM, &[0x80])?;
        // display update control: RED normal (needed for differential update)
        self.cmd_with(CMD_DISPLAY_UPDATE_CTRL, &[0x00, 0x00])?;
        // partial update sequence
        self.cmd_with(CMD_DISPLAY_UPDATE_SEQ, &[0xfc])?;
        self.cmd(CMD_MASTER_ACTIVATE)?;
        self.wait_busy("refresh", 30000);

        // copy current to previous for next partial
        self.copy_current_to_previous()?;

        println!("ssd1683: partial flush done");
        Ok(())
    }

    /// Enter deep sleep — wake with hardware reset
    pub fn sleep(&mut self) -> Result<(), Error<SPI::Error, CS::Error>> {
        self.cmd_with(CMD_DEEP_SLEEP, &[0x01])?;
        self.initialized = false;
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

impl<SPI, CS, DC, RST, BUSY, DELAY> Dimensions for Ssd1683<'_, SPI, CS, DC, RST, BUSY, DELAY> {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), Size::new(WIDTH, HEIGHT))
    }
}

impl<SPI, CS, DC, RST, BUSY, DELAY> DrawTarget for Ssd1683<'_, SPI, CS, DC, RST, BUSY, DELAY>
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
