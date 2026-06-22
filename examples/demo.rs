#![no_std]
#![no_main]

use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{
    Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, StrokeAlignment, Triangle,
};
use embedded_graphics::text::Text;
use esp_println::println;

use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;

use waveshare_epaper::il0398::{FB_SIZE, Il0398};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

static mut FRAMEBUFFER: [u8; FB_SIZE] = [0u8; FB_SIZE];

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // waveshare 4.2" e-paper SPI pins
    // GPIO0 = DIN (MOSI), GPIO1 = CLK (SCK)
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(10))
            .with_mode(Mode::_0),
    )
    .expect("static spi config is valid")
    .with_sck(peripherals.GPIO1)
    .with_mosi(peripherals.GPIO0);

    // control pins: GPIO2=CS, GPIO3=DC, GPIO4=RST, GPIO5=BUSY
    let cs = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO3, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO4, Level::High, OutputConfig::default());
    let busy = Input::new(
        peripherals.GPIO5,
        InputConfig::default().with_pull(Pull::None),
    );
    let delay = Delay::new();

    let fb_ptr: *mut [u8; FB_SIZE] = &raw mut FRAMEBUFFER;
    let fb: &'static mut [u8; FB_SIZE] = unsafe { &mut *fb_ptr };

    let mut display = Il0398::new(spi, cs, dc, rst, busy, delay, fb);

    println!("waveshare-epaper: starting init");
    match display.init() {
        Ok(()) => println!("waveshare-epaper: init ok"),
        Err(e) => println!("waveshare-epaper: init failed: {:?}", e),
    }

    println!("waveshare-epaper: drawing to framebuffer");
    display.clear_white();
    let _ = draw_demo(&mut display);

    println!("waveshare-epaper: flushing to display");
    match display.flush() {
        Ok(()) => println!("waveshare-epaper: flush ok"),
        Err(e) => println!("waveshare-epaper: flush failed: {:?}", e),
    }

    loop {
        core::hint::spin_loop();
    }
}

fn draw_demo<D>(d: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    let ink = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    let border = PrimitiveStyleBuilder::new()
        .stroke_color(BinaryColor::On)
        .stroke_width(3)
        .stroke_alignment(StrokeAlignment::Inside)
        .build();

    let filled = PrimitiveStyle::with_fill(BinaryColor::On);
    let outline = PrimitiveStyleBuilder::new()
        .stroke_color(BinaryColor::On)
        .stroke_width(2)
        .build();

    // outer border
    Rectangle::new(Point::zero(), Size::new(400, 300))
        .into_styled(border)
        .draw(d)?;

    // title
    Text::new("Waveshare 4.2\" ePaper", Point::new(20, 40), ink).draw(d)?;
    Text::new("400x300 . IL0398 . ESP32-C3", Point::new(20, 65), ink).draw(d)?;
    Text::new("Rust no_std + esp-hal", Point::new(20, 90), ink).draw(d)?;

    // divider
    Rectangle::new(Point::new(15, 105), Size::new(370, 2))
        .into_styled(filled)
        .draw(d)?;

    // shapes
    Text::new("shapes:", Point::new(20, 140), ink).draw(d)?;

    Circle::new(Point::new(20, 155), 50)
        .into_styled(filled)
        .draw(d)?;

    Circle::new(Point::new(90, 155), 50)
        .into_styled(outline)
        .draw(d)?;

    Rectangle::new(Point::new(160, 155), Size::new(50, 50))
        .into_styled(filled)
        .draw(d)?;

    Rectangle::new(Point::new(230, 155), Size::new(50, 50))
        .into_styled(outline)
        .draw(d)?;

    Triangle::new(
        Point::new(325, 205),
        Point::new(300, 155),
        Point::new(350, 155),
    )
    .into_styled(filled)
    .draw(d)?;

    // checkerboard
    Text::new("pattern:", Point::new(20, 240), ink).draw(d)?;
    let sq = 15u32;
    for row in 0..3 {
        for col in 0..20 {
            if (row + col) % 2 == 0 {
                Rectangle::new(
                    Point::new(20 + (col * sq) as i32, 250 + (row * sq) as i32),
                    Size::new(sq, sq),
                )
                .into_styled(filled)
                .draw(d)?;
            }
        }
    }

    Ok(())
}
