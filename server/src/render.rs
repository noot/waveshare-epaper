use ab_glyph::{FontRef, PxScale};
use image::{GrayImage, Luma, imageops};
use imageproc::drawing::draw_text_mut;

use crate::backend::NowPlaying;

const WIDTH: u32 = 400;
const HEIGHT: u32 = 300;
const FB_SIZE: usize = (WIDTH as usize / 8) * HEIGHT as usize;

const ART_SIZE: u32 = 150;
const ART_X: u32 = 20;
const ART_Y: u32 = 40;

const TEXT_X: i32 = 185;

static FONT_BYTES: &[u8] = include_bytes!("../assets/font.ttf");

pub fn render_now_playing(np: &NowPlaying) -> Vec<u8> {
    let mut img = GrayImage::from_pixel(WIDTH, HEIGHT, Luma([255u8]));
    let font = FontRef::try_from_slice(FONT_BYTES).expect("bundled font is valid");

    if let Some(ref art_bytes) = np.cover_art {
        draw_album_art(&mut img, art_bytes);
    }

    let track_scale = PxScale::from(24.0);
    let detail_scale = PxScale::from(18.0);
    let small_scale = PxScale::from(14.0);

    draw_text_mut(
        &mut img,
        Luma([0u8]),
        TEXT_X,
        ART_Y as i32 + 10,
        track_scale,
        &font,
        &truncate(&np.track, 18),
    );

    draw_text_mut(
        &mut img,
        Luma([60u8]),
        TEXT_X,
        ART_Y as i32 + 45,
        detail_scale,
        &font,
        &truncate(&np.artist, 20),
    );

    draw_text_mut(
        &mut img,
        Luma([80u8]),
        TEXT_X,
        ART_Y as i32 + 72,
        detail_scale,
        &font,
        &truncate(&np.album, 20),
    );

    if let Some(duration) = np.duration_secs {
        let time_str = if let Some(progress) = np.progress_secs {
            format!("{} / {}", format_time(progress), format_time(duration))
        } else {
            format_time(duration)
        };
        draw_text_mut(
            &mut img,
            Luma([0u8]),
            TEXT_X,
            ART_Y as i32 + 110,
            small_scale,
            &font,
            &time_str,
        );

        draw_progress_bar(&mut img, np.progress_secs, duration);
    }

    dither_and_pack(&img)
}

pub fn render_idle() -> Vec<u8> {
    let mut img = GrayImage::from_pixel(WIDTH, HEIGHT, Luma([255u8]));
    let font = FontRef::try_from_slice(FONT_BYTES).expect("bundled font is valid");

    draw_text_mut(
        &mut img,
        Luma([80u8]),
        130,
        135,
        PxScale::from(22.0),
        &font,
        "nothing playing",
    );

    dither_and_pack(&img)
}

fn draw_album_art(img: &mut GrayImage, art_bytes: &[u8]) {
    let Ok(art) = image::load_from_memory(art_bytes) else {
        return;
    };
    let art = art.resize_exact(ART_SIZE, ART_SIZE, imageops::FilterType::Lanczos3);
    let art_gray = art.to_luma8();

    for y in 0..ART_SIZE {
        for x in 0..ART_SIZE {
            let px = art_gray.get_pixel(x, y);
            img.put_pixel(ART_X + x, ART_Y + y, *px);
        }
    }

    // 1px border around album art
    for x in ART_X..ART_X + ART_SIZE {
        img.put_pixel(x, ART_Y, Luma([0u8]));
        img.put_pixel(x, ART_Y + ART_SIZE - 1, Luma([0u8]));
    }
    for y in ART_Y..ART_Y + ART_SIZE {
        img.put_pixel(ART_X, y, Luma([0u8]));
        img.put_pixel(ART_X + ART_SIZE - 1, y, Luma([0u8]));
    }
}

fn draw_progress_bar(img: &mut GrayImage, progress: Option<u32>, duration: u32) {
    let bar_x = 20u32;
    let bar_y = 220u32;
    let bar_w = 360u32;
    let bar_h = 6u32;

    // border
    for x in bar_x..bar_x + bar_w {
        img.put_pixel(x, bar_y, Luma([0u8]));
        img.put_pixel(x, bar_y + bar_h, Luma([0u8]));
    }
    for y in bar_y..=bar_y + bar_h {
        img.put_pixel(bar_x, y, Luma([0u8]));
        img.put_pixel(bar_x + bar_w - 1, y, Luma([0u8]));
    }

    // fill
    if let Some(progress) = progress {
        let fill_w = if duration > 0 {
            ((progress as u64 * (bar_w - 2) as u64) / duration as u64) as u32
        } else {
            0
        };
        for y in (bar_y + 1)..=(bar_y + bar_h - 1) {
            for x in (bar_x + 1)..=(bar_x + fill_w) {
                img.put_pixel(x, y, Luma([0u8]));
            }
        }
    }
}

fn format_time(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 1).collect();
        format!("{}…", truncated)
    }
}

/// Floyd-Steinberg dither to 1-bit, then pack into 1bpp framebuffer.
/// 1 = white, 0 = black (matching SSD1683 convention).
fn dither_and_pack(img: &GrayImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut pixels: Vec<f32> = img.pixels().map(|p| p.0[0] as f32).collect();

    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let old = pixels[idx];
            let new = if old > 127.0 { 255.0 } else { 0.0 };
            pixels[idx] = new;
            let err = old - new;

            if x + 1 < w {
                pixels[idx + 1] += err * 7.0 / 16.0;
            }
            if y + 1 < h {
                if x > 0 {
                    pixels[(idx + w as usize) - 1] += err * 3.0 / 16.0;
                }
                pixels[idx + w as usize] += err * 5.0 / 16.0;
                if x + 1 < w {
                    pixels[idx + w as usize + 1] += err * 1.0 / 16.0;
                }
            }
        }
    }

    let row_bytes = w as usize / 8;
    let mut fb = vec![0xFFu8; FB_SIZE];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let idx = y * w as usize + x;
            if pixels[idx] < 128.0 {
                fb[y * row_bytes + x / 8] &= !(0x80u8 >> (x % 8));
            }
        }
    }

    fb
}
