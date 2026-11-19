//! 2D rain / snow overlay (stand-in until FX draws 3D precipitation; switch it off with
//! `set_overlay_2d(false)`). Weather lighting, fog, sky, wet roads, snow cover and lightning
//! are rendered by `render::weather` + the shaders, per viewport.
use macroquad::prelude::*;
use rally_core::weather::Weather;

static OVERLAY_2D: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn the 2D rain/snow overlay off once real 3D precipitation particles exist (FX):
/// `crate::weather_view::set_overlay_2d(false)`.
#[allow(dead_code)]
pub fn set_overlay_2d(on: bool) {
    OVERLAY_2D.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Light rain streaks / snowflakes over a viewport.
pub fn draw_overlay(w: &Weather, x: f32, y: f32, wd: f32, ht: f32, k: f32) {
    if w.precipitation < 0.03 || !OVERLAY_2D.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let t = get_time() as f32;
    let count = (w.precipitation * 140.0 * (wd * ht / (1280.0 * 720.0)).max(0.3)) as i32;
    for i in 0..count {
        let h1 = rally_core::rng::hash2(i, 17, 3);
        let h2 = rally_core::rng::hash2(i, 29, 7);
        let speed = if w.snow { 0.18 } else { 1.6 };
        let px = x + ((h1 + t * (0.05 + w.wind[0] * 0.01)) % 1.0) * wd;
        let py = y + ((h2 + t * speed * (0.6 + h1 * 0.4)) % 1.0) * ht;
        if w.snow {
            draw_circle(px, py, (2.0 + h2 * 2.0) * k, Color::new(1.0, 1.0, 1.0, 0.75));
        } else {
            let len = (18.0 + h2 * 16.0) * k;
            draw_line(px, py, px - len * 0.18, py + len, 1.6 * k, Color::new(0.85, 0.9, 1.0, 0.35));
        }
    }
}
