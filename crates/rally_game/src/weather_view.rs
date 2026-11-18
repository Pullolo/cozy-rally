//! Minimal visual hookup of the stage conditions timeline (`Track::conditions`):
//! fog density / colour, overcast dimming, and a light 2D rain / snow overlay.
//!
//! This is a placeholder so dynamic stages read correctly today; the rendering / FX agents
//! can replace it with real 3D precipitation, wet-road gloss (`Track::puddle_at`) etc.
use crate::render::gpu::Environment;
use macroquad::prelude::*;
use rally_core::track::Track;
use rally_core::weather::Weather;
use std::cell::RefCell;

thread_local! {
    /// (track pointer, environment as built by the scene)
    static BASE: RefCell<Option<(usize, Environment)>> = const { RefCell::new(None) };
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Blend the scene environment towards the weather at arc length `s` (call once per frame).
pub fn apply(env: &mut Environment, track: &std::sync::Arc<Track>, s: f32) {
    if track.conditions.keys.is_empty() {
        return;
    }
    let key = std::sync::Arc::as_ptr(track) as usize;
    let base = BASE.with(|b| {
        let mut b = b.borrow_mut();
        if b.as_ref().is_none_or(|x| x.0 != key) {
            *b = Some((key, *env));
        }
        b.as_ref().unwrap().1
    });
    let w = track.conditions.at(s);
    let grey = if track.biome == rally_core::biome::Biome::Desert && w.fog > 0.2 && w.precipitation < 0.1 { [0.86, 0.72, 0.55] } else { [0.70, 0.73, 0.78] };
    let murk = (w.fog * 0.9 + w.overcast * 0.35 + w.precipitation * 0.3).min(1.0);
    env.fog_color = mix3(base.fog_color, grey, murk);
    env.fog_density = base.fog_density * (1.0 + 7.0 * w.fog + 1.5 * w.precipitation);
    let dim = 1.0 - 0.55 * w.overcast;
    env.sun_color = [base.sun_color[0] * dim, base.sun_color[1] * dim, base.sun_color[2] * dim];
    env.ambient = mix3(base.ambient, [0.5, 0.53, 0.6], w.overcast * 0.5);
}

/// Light rain streaks / snowflakes over a viewport.
pub fn draw_overlay(w: &Weather, x: f32, y: f32, wd: f32, ht: f32, k: f32) {
    if w.precipitation < 0.03 {
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
