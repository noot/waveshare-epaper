# waveshare-epaper

Rust no_std driver for the **Waveshare 4.2" e-Paper Module V2 Rev 2.2** (GDEY042T81, SSD1683) on ESP32-C3.

400×300 pixels, black/white, SPI interface.

## Wiring

ESP32-C3 SuperMini → Waveshare 4.2" module:

| GPIO | Function |
|------|----------|
| 0    | DIN (MOSI) |
| 1    | CLK (SCK) |
| 2    | CS |
| 3    | DC |
| 4    | RST |
| 5    | BUSY |
| 3V3  | VCC |
| GND  | GND |

Make sure the **BS** switch on the back of the module is set to **0** (4-wire SPI).

## Build & Flash

Requires the ESP32-C3 Rust toolchain ([espup](https://github.com/esp-rs/espup)):

```sh
# install toolchain (one-time)
cargo install espup espflash
espup install

# flash and monitor
cargo run --example demo
```

If `cargo run` doesn't auto-detect the port, specify it:

```sh
espflash flash --monitor --chip esp32c3 target/riscv32imc-unknown-none-elf/dev/examples/demo --port /dev/ttyACM0
```

## Examples

- **demo** — draws shapes, text, and a checkerboard pattern

```sh
cargo run --example demo
```

## Driver API

```rust
use waveshare_epaper::ssd1683::{Ssd1683, FB_SIZE};

let mut display = Ssd1683::new(spi, cs, dc, rst, busy, delay, &mut framebuffer);
display.init()?;
display.clear_white();

// draw with embedded-graphics
use embedded_graphics::prelude::*;
// ... draw stuff ...

display.flush()?;         // full refresh (~3-4s, clean)
display.flush_fast()?;    // fast full refresh (~2s)
display.flush_partial()?; // partial refresh (~1s, may ghost)
display.sleep()?;         // deep sleep, wake with reset
```

Implements `DrawTarget<Color = BinaryColor>` from [embedded-graphics](https://docs.rs/embedded-graphics).

## Hardware Notes

- **Rev 2.2** uses the SSD1683 controller (GDEY042T81 panel), not the older IL0398. Older init sequences will not work.
- Busy pin is **active HIGH** (HIGH = busy, LOW = idle).
- SSD1683 has dual RAM buffers (current + previous) enabling differential partial refresh.
- No custom LUTs needed — uses built-in OTP waveforms.
