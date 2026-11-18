//! Precomputed racing line for a track plus per-car speed profiles.
//!
//! The line is an offset from the centre line per track sample, found by projected
//! Gauss-Seidel relaxation of a blend of "minimum curvature" (4th-order stencil) and
//! "shortest path" (midpoint) energies, constrained to the road width minus a margin.
//! It runs coarse-to-fine (large strides first) so it converges in a few ms.
//!
//! The speed profile turns line curvature (horizontal + vertical) into a cornering speed
//! limit, then a backward pass adds braking zones (friction-circle aware) and a forward
//! pass predicts acceleration (used for lap-time estimates).
use crate::surface::Surface;
use crate::track::{Track, SAMPLE_SPACING};
use crate::vehicle::{CarSpec, Drivetrain, GRAVITY};
use glam::{Vec2, Vec3};
use std::sync::{Arc, Mutex};

/// Distance the line keeps from the road edge (m), measured to the car centre.
pub const LINE_MARGIN: f32 = 1.55;
/// Samples after a jump take-off treated as airborne (no braking possible)
const JUMP_AIR_SAMPLES: usize = 22;

#[derive(Clone, Debug)]
pub struct RacingLine {
    /// Lateral offset from the centre line per sample (m, + = right of travel)
    pub offset: Vec<f32>,
    /// Usable lateral limit per sample (|offset| <= limit)
    pub limit: Vec<f32>,
    /// World position of the line per sample
    pub pos: Vec<Vec3>,
    /// Signed horizontal curvature of the line (1/m, + = left turn)
    pub curvature: Vec<f32>,
    /// Vertical curvature (1/m): + = compression (dip), - = crest
    pub vcurv: Vec<f32>,
    /// Line length from sample i to i+1 (m)
    pub seg: Vec<f32>,
    /// true for samples inside a jump's take-off / flight zone
    pub jump_zone: Vec<bool>,
    pub length: f32,
}

fn wrap(i: isize, n: usize) -> usize {
    i.rem_euclid(n as isize) as usize
}

fn smooth_closed(v: &mut [f32], radius: usize, passes: usize) {
    let n = v.len();
    let mut tmp = v.to_vec();
    for _ in 0..passes {
        for i in 0..n {
            let mut s = 0.0;
            for k in -(radius as isize)..=(radius as isize) {
                s += v[wrap(i as isize + k, n)];
            }
            tmp[i] = s / (2 * radius + 1) as f32;
        }
        v.copy_from_slice(&tmp);
    }
}

type CacheKey = (usize, usize, u32, u32, u32);
static CACHE: Mutex<Vec<(CacheKey, Arc<RacingLine>)>> = Mutex::new(Vec::new());

fn key_of(track: &Track) -> CacheKey {
    let p0 = track.samples.first().map(|s| s.pos).unwrap_or(Vec3::ZERO);
    (track.samples.as_ptr() as usize, track.samples.len(), track.length.to_bits(), p0.x.to_bits(), p0.z.to_bits())
}

impl RacingLine {
    /// Racing line for `track`, cached (the last few tracks are kept).
    pub fn shared(track: &Track) -> Arc<RacingLine> {
        let key = key_of(track);
        if let Ok(c) = CACHE.lock() {
            if let Some((_, l)) = c.iter().find(|(k, _)| *k == key) {
                return l.clone();
            }
        }
        let line = Arc::new(RacingLine::compute(track, LINE_MARGIN));
        if let Ok(mut c) = CACHE.lock() {
            c.push((key, line.clone()));
            if c.len() > 4 {
                c.remove(0);
            }
        }
        line
    }

    pub fn compute(track: &Track, margin: f32) -> RacingLine {
        let n = track.samples.len();
        let centre: Vec<Vec2> = track.samples.iter().map(|s| Vec2::new(s.pos.x, s.pos.z)).collect();
        let right: Vec<Vec2> = track.samples.iter().map(|s| Vec2::new(s.right.x, s.right.z).normalize_or_zero()).collect();
        let mut limit: Vec<f32> = track.samples.iter().map(|s| (s.half_width - margin).max(0.2)).collect();
        // jump zones: keep the line near the middle and straight over ramps
        let mut jump_zone = vec![false; n];
        for &j in &track.jumps {
            for k in -14isize..=(JUMP_AIR_SAMPLES as isize) {
                let i = wrap(j as isize + k, n);
                jump_zone[i] = true;
                limit[i] = limit[i].min(1.2);
            }
            // taper back to the full width so the line has no kink at the zone edges
            for t in 1..=20isize {
                let extra = t as f32 * 0.25;
                for i in [wrap(j as isize - 14 - t, n), wrap(j as isize + JUMP_AIR_SAMPLES as isize + t, n)] {
                    limit[i] = limit[i].min(1.2 + extra);
                }
            }
        }
        // start grid / line: stay a little tamer so the launch isn't chaotic
        let mut offset = vec![0.0f32; n];
        let pt = |o: &[f32], i: usize| centre[i] + right[i] * o[i];
        // (stride, sweeps)
        let schedule: [(usize, usize); 6] = [(24, 40), (12, 60), (6, 80), (3, 100), (2, 100), (1, 160)];
        for &(k, sweeps) in &schedule {
            let ki = k as isize;
            for _ in 0..sweeps {
                for i in 0..n {
                    let im2 = wrap(i as isize - 2 * ki, n);
                    let im1 = wrap(i as isize - ki, n);
                    let ip1 = wrap(i as isize + ki, n);
                    let ip2 = wrap(i as isize + 2 * ki, n);
                    // minimum curvature: p_i = (-p[-2] + 4p[-1] + 4p[+1] - p[+2]) / 6
                    let mc = (-pt(&offset, im2) + 4.0 * pt(&offset, im1) + 4.0 * pt(&offset, ip1) - pt(&offset, ip2)) / 6.0;
                    // shortest path: midpoint
                    let sp = (pt(&offset, im1) + pt(&offset, ip1)) * 0.5;
                    let target = mc * 0.82 + sp * 0.18;
                    let o = (target - centre[i]).dot(right[i]);
                    let v = offset[i] + (o - offset[i]) * 0.7;
                    offset[i] = v.clamp(-limit[i], limit[i]);
                }
            }
        }
        // tiny smoothing to remove sample noise, then re-clamp
        smooth_closed(&mut offset, 1, 1);
        for i in 0..n {
            offset[i] = offset[i].clamp(-limit[i], limit[i]);
        }
        let pos: Vec<Vec3> = (0..n).map(|i| track.samples[i].pos + track.samples[i].right * offset[i]).collect();
        let p2: Vec<Vec2> = pos.iter().map(|p| Vec2::new(p.x, p.z)).collect();
        let seg: Vec<f32> = (0..n).map(|i| pos[i].distance(pos[(i + 1) % n]).max(0.05)).collect();
        let mut curvature: Vec<f32> = (0..n)
            .map(|i| {
                let a = p2[wrap(i as isize - 1, n)];
                let b = p2[i];
                let c = p2[(i + 1) % n];
                let t0 = (b - a).normalize_or_zero();
                let t1 = (c - b).normalize_or_zero();
                let cross_y = t0.y * t1.x - t0.x * t1.y;
                let ang = cross_y.atan2(t0.dot(t1));
                let ds = 0.5 * (a.distance(b) + b.distance(c));
                ang / ds.max(1e-3)
            })
            .collect();
        smooth_closed(&mut curvature, 2, 2);
        let mut vcurv: Vec<f32> = (0..n)
            .map(|i| {
                let ya = pos[wrap(i as isize - 1, n)].y;
                let yb = pos[i].y;
                let yc = pos[(i + 1) % n].y;
                let ds = SAMPLE_SPACING;
                (yc - 2.0 * yb + ya) / (ds * ds)
            })
            .collect();
        smooth_closed(&mut vcurv, 2, 2);
        let length = seg.iter().sum();
        RacingLine { offset, limit, pos, curvature, vcurv, seg, jump_zone, length }
    }

    /// Interpolated line offset at track arc length s
    pub fn offset_at(&self, s: f32, track: &Track) -> f32 {
        let n = self.offset.len();
        let f = s.rem_euclid(track.length) / SAMPLE_SPACING;
        let i = (f.floor() as usize) % n;
        let j = (i + 1) % n;
        let t = f.fract();
        self.offset[i] + (self.offset[j] - self.offset[i]) * t
    }

    pub fn limit_at(&self, s: f32, track: &Track) -> f32 {
        let n = self.limit.len();
        let i = ((s.rem_euclid(track.length) / SAMPLE_SPACING) as usize) % n;
        self.limit[i].min(self.limit[(i + 1) % n])
    }

    /// Max curvature magnitude over the next `dist` metres from sample index i
    pub fn max_curv_ahead(&self, i: usize, dist: f32) -> f32 {
        let n = self.curvature.len();
        let steps = (dist / SAMPLE_SPACING) as usize;
        (0..steps).map(|k| self.curvature[(i + k) % n]).fold(0.0, |a: f32, c| if c.abs() > a.abs() { c } else { a })
    }
}

/// Car capability used to build a speed profile.
#[derive(Clone, Copy, Debug)]
pub struct SpeedParams {
    /// Lateral friction coefficient used for cornering
    pub mu: f32,
    /// Braking deceleration (m/s^2)
    pub brake: f32,
    /// Traction-limited acceleration (m/s^2)
    pub traction: f32,
    /// Effective engine power / mass (W/kg)
    pub power_per_kg: f32,
    /// Aero drag / mass
    pub drag_per_kg: f32,
    /// Downforce / mass
    pub down_per_kg: f32,
    pub v_max: f32,
    /// Speed cap at jump take-offs
    pub jump_speed: f32,
}

impl SpeedParams {
    /// Capability of `spec` on `surface`. `grip_use` (0..1) = fraction of the grip a driver
    /// dares to use when cornering, `brake_use` the same for braking.
    pub fn for_car(spec: &CarSpec, surface: Surface, grip_use: f32, brake_use: f32) -> SpeedParams {
        let mu_raw = spec.tire_grip * surface.props().grip;
        let m = spec.mass;
        let brake = (spec.brake_force / m).min(mu_raw * GRAVITY) * brake_use;
        let driven = match spec.drivetrain {
            Drivetrain::Fwd => 0.58,
            Drivetrain::Rwd => 0.50,
            Drivetrain::Awd => 0.95,
        };
        let peak_power = spec.peak_torque * spec.peak_rpm * std::f32::consts::TAU / 60.0;
        SpeedParams {
            mu: mu_raw * grip_use,
            brake,
            traction: mu_raw * GRAVITY * driven * 0.85,
            power_per_kg: peak_power * 0.88 * 0.82 / m,
            drag_per_kg: spec.drag / m,
            down_per_kg: spec.downforce / m,
            v_max: 54.0,
            jump_speed: 33.0,
        }
    }
}

/// Speed profile along a racing line.
#[derive(Clone, Debug)]
pub struct SpeedProfile {
    /// Speed limit per sample including braking zones (what an AI should not exceed)
    pub limit: Vec<f32>,
    /// Predicted speed per sample including acceleration (for lap-time estimates)
    pub predicted: Vec<f32>,
    /// Predicted flying-lap time (s)
    pub lap_time: f32,
}

impl SpeedProfile {
    pub fn build(line: &RacingLine, p: &SpeedParams) -> SpeedProfile {
        let n = line.curvature.len();
        let g = GRAVITY;
        // cornering limit: v^2 * |k| <= mu * (g + v^2 * kv + v^2 * down)
        //  -> v^2 <= mu g / (|k| - mu kv - mu down)
        let mut corner = vec![p.v_max; n];
        for i in 0..n {
            let k = line.curvature[i].abs();
            let kv = if line.jump_zone[i] { 0.0 } else { line.vcurv[i].clamp(-0.08, 0.08) };
            let denom = k - p.mu * kv - p.mu * p.down_per_kg;
            let mut v = if denom > 1e-5 { (p.mu * g / denom).sqrt() } else { p.v_max };
            // crests: don't take off on a hump that isn't a jump (keeps normal load positive)
            if kv < -1e-4 && !line.jump_zone[i] {
                v = v.min((g * 1.15 / -kv).sqrt());
            }
            corner[i] = v.min(p.v_max).max(6.0);
        }
        // jumps: take-off speed cap at the ramp
        for i in 0..n {
            if line.jump_zone[i] {
                corner[i] = corner[i].min(p.jump_speed);
            }
        }
        // backward pass: braking (twice around the loop so it wraps)
        let mut limit = corner.clone();
        for _ in 0..2 {
            for ii in (0..n).rev() {
                let i = ii;
                let j = (i + 1) % n;
                let ds = line.seg[i];
                let v = limit[j];
                // friction circle: braking grip left after the lateral demand
                let lat = v * v * line.curvature[i].abs();
                let lat_frac = (lat / (p.mu * g).max(0.1)).min(1.0);
                let avail = p.brake * (1.0 - lat_frac * lat_frac).max(0.12).sqrt();
                // can't brake in the air
                let avail = if line.jump_zone[i] && !line.jump_zone[wrap(i as isize - 14, n)] { avail } else if line.jump_zone[i] { avail * 0.15 } else { avail };
                let vb = (v * v + 2.0 * avail * ds).sqrt() + p.drag_per_kg * v * v * ds / v.max(1.0);
                if vb < limit[i] {
                    limit[i] = vb;
                }
            }
        }
        // forward pass: acceleration prediction
        let mut pred = limit.clone();
        for _ in 0..2 {
            for i in 0..n {
                let j = (i + 1) % n;
                let v = pred[i].max(1.0);
                let lat = v * v * line.curvature[i].abs();
                let lat_frac = (lat / (p.mu * g).max(0.1)).min(1.0);
                let trac = p.traction * (1.0 - lat_frac * lat_frac).max(0.1).sqrt();
                let a = trac.min(p.power_per_kg / v) - p.drag_per_kg * v * v - 0.02 * g;
                let va = (v * v + 2.0 * a.max(0.0) * line.seg[i]).sqrt();
                if va < pred[j] {
                    pred[j] = va;
                }
            }
        }
        let lap_time = (0..n).map(|i| line.seg[i] / (0.5 * (pred[i] + pred[(i + 1) % n])).max(1.0)).sum();
        SpeedProfile { limit, predicted: pred, lap_time }
    }

    pub fn limit_at(&self, s: f32, track: &Track) -> f32 {
        let n = self.limit.len();
        let f = s.rem_euclid(track.length) / SAMPLE_SPACING;
        let i = (f.floor() as usize) % n;
        let j = (i + 1) % n;
        let t = f.fract();
        self.limit[i] + (self.limit[j] - self.limit[i]) * t
    }
}
