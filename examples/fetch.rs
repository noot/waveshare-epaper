#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;

use embassy_executor::Spawner;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::{Runner, StackResources};
use embassy_time::{Instant, Timer};
use embedded_io_async::Read as _;
use esp_alloc as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rng::Rng;
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::wifi::{Config, ControllerConfig, Interface, WifiController, sta::StationConfig};
use reqwless::client::HttpClient;
use reqwless::request::Method;
use static_cell::StaticCell;

use waveshare_epaper::ssd1683::{FB_SIZE, Ssd1683};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {}", info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const SERVER_URL: &str = env!("SERVER_URL");

const POLL_INTERVAL_SECS: u64 = 5;
const FULL_REFRESH_EVERY: u32 = 60;

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: StaticCell<$t> = StaticCell::new();
        STATIC_CELL.uninit().write($val)
    }};
}

static mut FRAMEBUFFER: [u8; FB_SIZE] = [0u8; FB_SIZE];

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // ── display setup ───────────────────────────────────────────
    // GPIO0=MOSI, GPIO1=SCK
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(10))
            .with_mode(Mode::_0),
    )
    .expect("spi config valid")
    .with_sck(peripherals.GPIO1)
    .with_mosi(peripherals.GPIO0);

    // GPIO2=CS, GPIO5=DC, GPIO4=RST, GPIO10=BUSY
    let cs = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO4, Level::High, OutputConfig::default());
    let busy = Input::new(
        peripherals.GPIO10,
        InputConfig::default().with_pull(Pull::None),
    );
    let delay = Delay::new();

    // GPIO6 = touch sensor (AT42QT1010, HIGH on touch)
    let touch = Input::new(
        peripherals.GPIO6,
        InputConfig::default().with_pull(Pull::None),
    );

    let fb_ptr: *mut [u8; FB_SIZE] = &raw mut FRAMEBUFFER;
    let fb: &'static mut [u8; FB_SIZE] = unsafe { &mut *fb_ptr };

    let mut display = Ssd1683::new(spi, cs, dc, rst, busy, delay, fb);

    println!("display: init");
    if let Err(e) = display.init() {
        println!("display: init failed: {:?}", e);
    }

    // ── wifi setup ──────────────────────────────────────────────
    println!("wifi: connecting to {}", SSID);
    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );

    let wifi_interface = Interface::station();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("wifi config valid");

    let net_config = embassy_net::Config::dhcpv4(Default::default());
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(connection(controller).expect("single connection task"));
    spawner.spawn(net_task(runner).expect("single net task"));

    println!("wifi: waiting for dhcp...");
    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
        println!("wifi: ip {}", config.address);
    }

    // ── fetch + refresh loop ──────────────────────────────────
    let tcp_client = TcpClient::new(
        stack,
        mk_static!(
            TcpClientState<2, 1500, 1500>,
            TcpClientState::<2, 1500, 1500>::new()
        ),
    );
    let dns_client = DnsSocket::new(stack);

    let mut client = HttpClient::new(&tcp_client, &dns_client);
    let mut rx_buf = [0u8; 4096];
    let mut cycle: u32 = 0;

    let base = server_base(SERVER_URL);
    let play_pause_url = format!("{}/play-pause", base);
    let next_url = format!("{}/next", base);

    loop {
        let fb_mut = display.framebuffer_mut();

        println!("fetch: requesting {} (cycle {})", SERVER_URL, cycle);
        match fetch_framebuffer(&mut client, &mut rx_buf, fb_mut).await {
            Ok(()) => {
                let full = cycle == 0 || cycle % FULL_REFRESH_EVERY == 0;
                let result = if full {
                    println!("display: full refresh");
                    display.flush()
                } else {
                    println!("display: partial refresh");
                    display.flush_partial()
                };
                match result {
                    Ok(()) => println!("display: updated"),
                    Err(e) => println!("display: flush error: {:?}", e),
                }
            }
            Err(e) => println!("fetch: failed: {}", e),
        }

        cycle += 1;

        // poll touch during wait period
        let wait_end = Instant::now() + embassy_time::Duration::from_secs(POLL_INTERVAL_SECS);
        while Instant::now() < wait_end {
            if let Some(gesture) = detect_gesture(&touch).await {
                let url = match gesture {
                    Gesture::SingleTap => {
                        println!("touch: single tap → play/pause");
                        &play_pause_url
                    }
                    Gesture::DoubleTap => {
                        println!("touch: double tap → next");
                        &next_url
                    }
                };
                send_command(&mut client, &mut rx_buf, url).await;
                Timer::after(embassy_time::Duration::from_millis(500)).await;
                break;
            }
            Timer::after(embassy_time::Duration::from_millis(20)).await;
        }
    }
}

fn server_base(url: &str) -> &str {
    match url.rfind('/') {
        Some(pos) => &url[..pos],
        None => url,
    }
}

enum Gesture {
    SingleTap,
    DoubleTap,
}

async fn detect_gesture(pin: &Input<'_>) -> Option<Gesture> {
    if pin.is_low() {
        return None;
    }

    // touched — wait for release
    while pin.is_high() {
        Timer::after(embassy_time::Duration::from_millis(20)).await;
    }

    // 300ms window for a second tap
    let deadline = Instant::now() + embassy_time::Duration::from_millis(300);
    while Instant::now() < deadline {
        if pin.is_high() {
            while pin.is_high() {
                Timer::after(embassy_time::Duration::from_millis(20)).await;
            }
            return Some(Gesture::DoubleTap);
        }
        Timer::after(embassy_time::Duration::from_millis(20)).await;
    }

    Some(Gesture::SingleTap)
}

async fn send_command<'a>(
    client: &mut HttpClient<'a, TcpClient<'a, 2, 1500, 1500>, DnsSocket<'a>>,
    rx_buf: &mut [u8],
    url: &str,
) {
    match client.request(Method::POST, url).await {
        Ok(mut builder) => match builder.send(rx_buf).await {
            Ok(resp) => println!("command: {} → {}", url, resp.status.0),
            Err(_) => println!("command: send failed"),
        },
        Err(_) => println!("command: request failed"),
    }
}

async fn fetch_framebuffer<'a>(
    client: &mut HttpClient<'a, TcpClient<'a, 2, 1500, 1500>, DnsSocket<'a>>,
    rx_buf: &mut [u8],
    fb: &mut [u8; FB_SIZE],
) -> Result<(), &'static str> {
    let mut builder = client
        .request(Method::GET, SERVER_URL)
        .await
        .map_err(|_| "request create failed")?;

    let response = builder.send(rx_buf).await.map_err(|_| "send failed")?;

    let status = response.status.0;
    if status != 200 {
        println!("fetch: server returned {}", status);
        return Err("non-200");
    }

    let body = response.body();
    let mut reader = body.reader();
    let mut offset = 0;

    while offset < FB_SIZE {
        let n = reader
            .read(&mut fb[offset..])
            .await
            .map_err(|_| "read error")?;
        if n == 0 {
            break;
        }
        offset += n;
    }

    println!("fetch: {} bytes", offset);
    if offset != FB_SIZE {
        println!("fetch: warning: expected {}, got {}", FB_SIZE, offset);
    }
    Ok(())
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    loop {
        println!("wifi: connecting...");
        match controller.connect_async().await {
            Ok(info) => {
                println!("wifi: connected {:?}", info);
                let info = controller.wait_for_disconnect_async().await.ok();
                println!("wifi: disconnected {:?}", info);
            }
            Err(e) => println!("wifi: connect failed: {:?}", e),
        }
        Timer::after(embassy_time::Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}
