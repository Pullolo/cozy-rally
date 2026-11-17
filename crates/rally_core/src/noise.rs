//! Value noise + fBm, deterministic.
use crate::rng::hash2;

fn smooth(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 2D value noise in [-1, 1]
pub fn value2(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let fx = smooth(x - xi);
    let fy = smooth(y - yi);
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    let v = a + (b - a) * fx + (c - a) * fy + (a - b - c + d) * fx * fy;
    v * 2.0 - 1.0
}

/// Fractal Brownian motion, roughly in [-1, 1]
pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for o in 0..octaves {
        sum += amp * value2(x * freq, y * freq, seed.wrapping_add(o * 1013));
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

/// Ridged noise in [0,1], good for mountains
pub fn ridged(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for o in 0..octaves {
        let n = 1.0 - value2(x * freq, y * freq, seed.wrapping_add(o * 7919)).abs();
        sum += amp * n * n;
        norm += amp;
        amp *= 0.5;
        freq *= 2.1;
    }
    sum / norm
}
