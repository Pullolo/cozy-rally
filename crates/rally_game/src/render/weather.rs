//! Stage weather -> renderer: maps `rally_core::weather::Weather` (sampled at each player's
//! own arc length, already smoothly blended along the stage) to a `WeatherLook` per viewport.
use super::gpu::WeatherLook;
use rally_core::biome::Biome;
use rally_core::rng::hash2;
use rally_core::track::Track;
use rally_core::weather::Weather;

/// Gentle lightning for storms: rare double flickers, brightness 0..1 at `time`.
pub fn lightning(w: &Weather, time: f32) -> f32 {
    let storm = ((w.precipitation - 0.8) / 0.2).clamp(0.0, 1.0) * ((w.overcast - 0.85) / 0.15).clamp(0.0, 1.0);
    if storm <= 0.0 {
        return 0.0;
    }
    // one candidate strike per 2.5 s slot, ~22% of slots strike at a random offset
    let slot = (time / 2.5).floor();
    let r = hash2(slot as i32, 71, 5);
    if r > 0.22 * storm {
        return 0.0;
    }
    let t0 = slot * 2.5 + hash2(slot as i32, 13, 9) * 1.8;
    let dt = time - t0;
    if !(0.0..0.6).contains(&dt) {
        return 0.0;
    }
    let pulse = |c: f32, w: f32| (-((dt - c) / w).powi(2)).exp();
    (pulse(0.03, 0.04) * 0.9 + pulse(0.2, 0.06) * 0.6) * 0.6 * storm
}

/// Renderer weather for a viewport whose player is at arc length `s`
pub fn look_at(track: &Track, s: f32, time: f32) -> WeatherLook {
    if track.conditions.keys.is_empty() {
        return WeatherLook::default();
    }
    let w = track.weather_at(s);
    let snowing = if w.snow { w.precipitation } else { 0.0 };
    WeatherLook {
        overcast: w.overcast.clamp(0.0, 1.0),
        fog: w.fog.clamp(0.0, 1.0),
        precip: w.precipitation.clamp(0.0, 1.0),
        wetness: if w.snow { 0.0 } else { w.wetness.clamp(0.0, 1.0) },
        // snowfall whitens the world (biome snow already is white)
        snow: if track.biome == Biome::Snow { snowing * 0.35 } else { (snowing * 1.3).min(0.85) },
        flash: lightning(&w, time),
        dusty: track.biome == Biome::Desert && w.precipitation < 0.1,
    }
}
