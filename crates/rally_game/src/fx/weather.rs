//! Weather particles driven by the stage conditions timeline (`Track::conditions`) at each
//! local player's position: rain streaks, snowfall, blowing sand and rain ripples.
//!
//! None of this is simulated: every streak / flake / ripple is a point in a world-anchored
//! lattice that repeats every few metres, animated purely from `time` (fall + wind drift) and
//! rebuilt around the camera at draw time. So there is no per-particle CPU state, it looks the
//! same from both split-screen views, and it costs a few thousand quads in the existing batches
//! (no extra draw calls).
use super::render::{col, Basis, Batches};
use macroquad::math::Vec3;
use rally_core::biome::Biome;
use rally_core::track::Track;
use rally_core::weather::Weather;
use std::f32::consts::TAU;

const TABLE: usize = 4096;
/// Max rain streaks per view (Storm)
const MAX_RAIN: usize = 2600;
const MAX_SNOW: usize = 2200;
const MAX_SAND: usize = 1500;
const MAX_RIPPLES: usize = 220;

pub struct WeatherFx {
    /// Random lattice offsets (x, y, z, w) in 0..1
    table: Vec<[f32; 4]>,
    /// Smoothed weather per local player (index into `Fx::locals`)
    pub smooth: Vec<Weather>,
    /// The track has a conditions timeline (otherwise everything stays off)
    pub active: bool,
}

/// Dust-storm strength for a weather sample (desert fog without rain reads as blowing sand)
pub fn sand_amount(biome: Biome, w: &Weather) -> f32 {
    if biome == Biome::Desert && w.precipitation < 0.1 {
        ((w.fog - 0.2) / 0.5).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[inline]
fn wrap(v: f32, period: f32) -> f32 {
    v - (v / period).floor() * period
}

impl WeatherFx {
    pub fn new(track: &Track) -> WeatherFx {
        let mut x: u32 = 0x1234_5678 ^ (track.params.seed as u32);
        let mut f = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as f32 / 16_777_216.0
        };
        let table = (0..TABLE).map(|_| [f(), f(), f(), f()]).collect();
        WeatherFx { table, smooth: Vec::new(), active: !track.conditions.keys.is_empty() }
    }

    /// Ease each local player's weather towards the conditions at their arc length, so driving
    /// into a rain front builds up over a second or two instead of switching.
    pub fn update(&mut self, targets: &[Weather], dt: f32) {
        if self.smooth.len() != targets.len() {
            self.smooth = targets.to_vec();
        }
        let k = 1.0 - (-dt / 1.2).exp();
        for (s, t) in self.smooth.iter_mut().zip(targets) {
            *s = s.lerp(t, k);
            s.snow = t.snow;
        }
    }

    /// Falling rain / snow and blowing sand (unlit batch).
    pub fn build_air(&self, b: &mut Batches, bs: &Basis, w: &Weather, biome: Biome, time: f32) {
        let fwd = Vec3::new(-bs.toward.x, 0.0, -bs.toward.z).normalize_or(Vec3::X);
        let wind = Vec3::new(w.wind[0], 0.0, w.wind[1]);
        if w.precipitation > 0.02 {
            if w.snow {
                self.snow(b, bs, w, fwd, wind, time);
            } else {
                self.rain(b, bs, w, fwd, wind, time);
            }
        }
        let sand = sand_amount(biome, w);
        if sand > 0.02 {
            self.sand(b, bs, sand, fwd, wind, time);
        }
    }

    fn rain(&self, b: &mut Batches, bs: &Basis, w: &Weather, fwd: Vec3, wind: Vec3, time: f32) {
        let (bx, bh) = (24.0f32, 14.0f32);
        let centre = bs.eye + fwd * 9.0;
        let n = ((w.precipitation * w.precipitation.sqrt()) * MAX_RAIN as f32) as usize;
        let fall = 9.5;
        let vel = Vec3::new(wind.x * 0.85, -fall, wind.z * 0.85);
        let dir = vel.normalize();
        let colr = [0.84, 0.89, 0.96];
        for i in 0..n.min(TABLE) {
            let h = self.table[i];
            let len = 0.45 + 0.35 * h[3];
            // world-anchored lattice: drift with wind and fall over time, wrap around the camera
            let x = wrap(h[0] * bx + vel.x * time - (centre.x - bx * 0.5), bx) + centre.x - bx * 0.5;
            let z = wrap(h[2] * bx + vel.z * time - (centre.z - bx * 0.5), bx) + centre.z - bx * 0.5;
            let y = wrap(h[1] * bh + vel.y * time * (0.85 + 0.3 * h[3]) - (bs.eye.y - bh * 0.45), bh) + bs.eye.y - bh * 0.45;
            let p = Vec3::new(x, y, z);
            let d = p.distance(bs.eye);
            // fade very near (no screen-sized streaks) and towards the box edge
            let a = 0.4 * ((d - 1.2) / 1.5).clamp(0.0, 1.0) * (1.0 - d / (bx * 0.75)).clamp(0.0, 1.0);
            if a < 0.02 {
                continue;
            }
            let side = dir.cross(bs.toward).normalize_or_zero() * (0.009 + d * 0.0011);
            let tail = p - dir * len;
            let batch = b.room(4);
            let c = col(colr, a);
            let c0 = col(colr, 0.0);
            let n = bs.toward;
            let i0 = batch.v.len() as u16;
            for (q, cc) in [(p + side, c), (p - side, c), (tail - side, c0), (tail + side, c0)] {
                batch.v.push(crate::render::mesh::Vertex { pos: q.to_array(), normal: n.to_array(), color: cc });
            }
            batch.i.extend_from_slice(&[i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
        }
    }

    fn snow(&self, b: &mut Batches, bs: &Basis, w: &Weather, fwd: Vec3, wind: Vec3, time: f32) {
        let (bx, bh) = (26.0f32, 13.0f32);
        let centre = bs.eye + fwd * 9.0;
        let n = (w.precipitation * MAX_SNOW as f32) as usize;
        let vel = Vec3::new(wind.x * 0.7, -1.1, wind.z * 0.7);
        for i in 0..n.min(TABLE) {
            let h = self.table[i];
            let sway = (time * (0.9 + h[3]) + h[0] * TAU).sin() * 0.45;
            let x = wrap(h[0] * bx + vel.x * time + sway - (centre.x - bx * 0.5), bx) + centre.x - bx * 0.5;
            let z = wrap(h[2] * bx + vel.z * time + sway * 0.6 - (centre.z - bx * 0.5), bx) + centre.z - bx * 0.5;
            let y = wrap(h[1] * bh + vel.y * time * (0.8 + 0.4 * h[3]) - (bs.eye.y - bh * 0.4), bh) + bs.eye.y - bh * 0.4;
            let p = Vec3::new(x, y, z);
            let d = p.distance(bs.eye);
            let a = 0.9 * ((d - 0.8) / 1.2).clamp(0.0, 1.0) * (1.0 - d / (bx * 0.75)).clamp(0.0, 1.0);
            if a < 0.03 {
                continue;
            }
            let s = (0.035 + 0.03 * h[3]).max(d * 0.0022);
            let batch = b.room(7);
            let n = bs.toward;
            let i0 = batch.v.len() as u16;
            batch.v.push(crate::render::mesh::Vertex { pos: p.to_array(), normal: n.to_array(), color: col([1.0, 1.0, 1.0], a) });
            let rim = col([0.92, 0.95, 1.0], a * 0.35);
            for k in 0..6 {
                let ang = k as f32 / 6.0 * TAU + h[3];
                let q = p + (bs.right * ang.cos() + bs.up * ang.sin()) * s;
                batch.v.push(crate::render::mesh::Vertex { pos: q.to_array(), normal: n.to_array(), color: rim });
            }
            for k in 0..6u16 {
                batch.i.extend_from_slice(&[i0, i0 + 1 + k, i0 + 1 + (k + 1) % 6]);
            }
        }
    }

    fn sand(&self, b: &mut Batches, bs: &Basis, sand: f32, fwd: Vec3, wind: Vec3, time: f32) {
        // fast, nearly horizontal tan streaks hugging the ground band around the camera
        let (bx, bh) = (30.0f32, 5.0f32);
        let centre = bs.eye + fwd * 10.0;
        let n = (sand * MAX_SAND as f32) as usize;
        let wdir = wind.normalize_or(Vec3::X);
        let speed = wind.length().max(4.0) * 1.8;
        let vel = wdir * speed + Vec3::new(0.0, -0.4, 0.0);
        let dir = vel.normalize();
        for i in 0..n.min(TABLE) {
            let h = self.table[(i + 1777) % TABLE];
            let x = wrap(h[0] * bx + vel.x * time * (0.8 + 0.4 * h[3]) - (centre.x - bx * 0.5), bx) + centre.x - bx * 0.5;
            let z = wrap(h[2] * bx + vel.z * time * (0.8 + 0.4 * h[3]) - (centre.z - bx * 0.5), bx) + centre.z - bx * 0.5;
            let y = bs.eye.y - 2.8 + h[1] * bh + (time * 2.0 + h[3] * 9.0).sin() * 0.25;
            let p = Vec3::new(x, y, z);
            let d = p.distance(bs.eye);
            let a = 0.5 * ((d - 1.5) / 2.0).clamp(0.0, 1.0) * (1.0 - d / (bx * 0.7)).clamp(0.0, 1.0);
            if a < 0.02 {
                continue;
            }
            let len = 0.9 + 1.2 * h[3];
            let side = dir.cross(bs.toward).normalize_or_zero() * (0.016 + d * 0.0016);
            let tail = p - dir * len;
            let colr = [1.0 - 0.06 * h[1], 0.92 - 0.08 * h[1], 0.78 - 0.08 * h[1]];
            let batch = b.room(4);
            let nrm = bs.toward;
            let i0 = batch.v.len() as u16;
            for (q, cc) in [(p + side, col(colr, a)), (p - side, col(colr, a)), (tail - side, col(colr, 0.0)), (tail + side, col(colr, 0.0))] {
                batch.v.push(crate::render::mesh::Vertex { pos: q.to_array(), normal: nrm.to_array(), color: cc });
            }
            batch.i.extend_from_slice(&[i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
        }
    }

    /// Rain ripples on wet ground and puddles (ground batch).
    pub fn build_ground(&self, b: &mut Batches, bs: &Basis, w: &Weather, track: &Track, time: f32) {
        if w.snow || w.precipitation < 0.05 || w.wetness < 0.2 {
            return;
        }
        let fwd = Vec3::new(-bs.toward.x, 0.0, -bs.toward.z).normalize_or(Vec3::X);
        let centre = bs.eye + fwd * 8.0;
        let bx = 16.0f32;
        let n = (w.precipitation * MAX_RIPPLES as f32) as usize;
        let period = 0.55;
        for i in 0..n.min(TABLE) {
            let h = self.table[(i * 7 + 311) % TABLE];
            let ph = time / period + h[3];
            let cycle = ph.floor();
            let t = ph - cycle;
            // a fresh spot every cycle
            let jx = (cycle * 0.618_034 + h[1]).fract();
            let jz = (cycle * 0.414_214 + h[2]).fract();
            let x = wrap((h[0] + jx) * bx - (centre.x - bx * 0.5), bx) + centre.x - bx * 0.5;
            let z = wrap((h[2] + jz) * bx - (centre.z - bx * 0.5), bx) + centre.z - bx * 0.5;
            let y = track.visual_height(x, z) + 0.075;
            let c = Vec3::new(x, y, z);
            let d = c.distance(bs.eye);
            let a = 0.32 * (1.0 - t) * (1.0 - d / (bx * 0.8)).clamp(0.0, 1.0) * ((d - 1.5) / 2.0).clamp(0.0, 1.0);
            if a < 0.02 {
                continue;
            }
            let r = 0.06 + t * 0.32;
            let wdt = 0.025 + 0.02 * (1.0 - t);
            let batch = b.room(24);
            let i0 = batch.v.len() as u16;
            let colr = [0.88, 0.92, 0.98];
            for k in 0..12 {
                let ang = k as f32 / 12.0 * TAU;
                let dd = Vec3::new(ang.cos(), 0.0, ang.sin());
                batch.v.push(crate::render::mesh::Vertex { pos: (c + dd * r).to_array(), normal: [0.0, 1.0, 0.0], color: col(colr, a * 0.3) });
                batch.v.push(crate::render::mesh::Vertex { pos: (c + dd * (r + wdt)).to_array(), normal: [0.0, 1.0, 0.0], color: col(colr, a) });
            }
            for k in 0..12u16 {
                let a0 = i0 + k * 2;
                let b0 = i0 + ((k + 1) % 12) * 2;
                batch.i.extend_from_slice(&[a0, a0 + 1, b0 + 1, a0, b0 + 1, b0]);
            }
        }
    }
}
