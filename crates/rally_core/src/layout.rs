//! Rally-style layout planner.
//!
//! Instead of smoothing a random blob, the loop is *designed* as a sequence of main straights
//! and corner complexes (hairpins, square junction corners, tightening / opening corners,
//! S-bends, chicanes, kinks, sweepers). Every corner has an explicit curvature profile with
//! clothoid ramps, so curvature is continuous and the minimum radius is exactly what was asked
//! for. Headings are fixed by construction (net turning = one full turn) and the loop is closed
//! exactly by solving a tiny weighted least-squares problem for the main straight lengths.
//! Candidates that come too close to themselves are rejected (the planner is cheap; the caller
//! retries).
use crate::rng::Rng;
use glam::Vec2;
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    Straight,
    Hairpin,
    Square,
    Medium,
    Sweeper,
    Tightening,
    Opening,
    Chicane,
    Kink,
    SBend,
}

impl FeatureKind {
    pub fn is_corner(self) -> bool {
        self != FeatureKind::Straight
    }
}

/// A designed piece of the layout, in final arc-length coordinates (s0 may be > s1 when the
/// feature wraps over the start line).
#[derive(Clone, Copy, Debug)]
pub struct Feature {
    pub kind: FeatureKind,
    pub s0: f32,
    pub s1: f32,
    /// Main straight index (straights only); 0 = start/finish straight
    pub main: Option<usize>,
    /// Kink designed to come right after a jump landing
    pub jump_kink: bool,
}

impl Feature {
    pub fn len(&self, track_len: f32) -> f32 {
        (self.s1 - self.s0).rem_euclid(track_len)
    }
}

pub struct Layout {
    /// Closed centre line, uniform `spacing`
    pub pts: Vec<Vec2>,
    pub features: Vec<Feature>,
    /// Per sample 0 (fast/open) .. 1 (tight technical), smooth
    pub tech: Vec<f32>,
    /// Per sample camber factor: 1 banked, ~0.3 flat, negative = off-camber
    pub camber: Vec<f32>,
}

#[derive(Clone, Copy, Debug)]
enum Elem {
    Straight { len: f32, main: Option<usize>, frac: f32 },
    Curve { ka: f32, kb: f32, a: f32, b: f32, c: f32, kind: FeatureKind, camber: f32, tech: f32, jump_kink: bool },
}

impl Elem {
    fn len(&self, mains: &[f32]) -> f32 {
        match *self {
            Elem::Straight { len, main: Some(k), frac } => {
                let _ = len;
                mains[k] * frac
            }
            Elem::Straight { len, .. } => len,
            Elem::Curve { a, b, c, .. } => a + b + c,
        }
    }

    fn curvature(&self, s: f32) -> f32 {
        match *self {
            Elem::Straight { .. } => 0.0,
            Elem::Curve { ka, kb, a, b, c, .. } => {
                if s < a {
                    ka * s / a.max(1e-3)
                } else if s < a + c {
                    ka + (kb - ka) * (s - a) / c.max(1e-3)
                } else {
                    kb * (1.0 - (s - a - c) / b.max(1e-3)).max(0.0)
                }
            }
        }
    }
}

/// Corner with clothoid ramps: curvature 0 -> 1/r_in over `ramp_in`, then linearly to 1/r_out,
/// then back to 0 over `ramp_out`. `angle` (rad, > 0) is the total heading change; `sign` its
/// direction.
#[allow(clippy::too_many_arguments)]
fn curve(sign: f32, angle: f32, r_in: f32, r_out: f32, ramp_in: f32, ramp_out: f32, kind: FeatureKind, camber: f32, tech: f32) -> Elem {
    let ka = 1.0 / r_in;
    let kb = 1.0 / r_out;
    let mut a = ramp_in;
    let mut b = ramp_out;
    let ramps = ka * a * 0.5 + kb * b * 0.5;
    let c;
    if ramps >= angle {
        // too short for the ramps: shrink them, no constant part
        let f = angle / ramps;
        a *= f;
        b *= f;
        c = 0.0;
    } else {
        c = (angle - ramps) * 2.0 / (ka + kb);
    }
    Elem::Curve { ka: ka * sign, kb: kb * sign, a, b, c, kind, camber, tech, jump_kink: false }
}

fn deg(d: f32) -> f32 {
    d.to_radians()
}

struct Planner<'a> {
    rng: &'a mut Rng,
    d: usize,
}

impl Planner<'_> {
    fn camber(&mut self, kind: FeatureKind) -> f32 {
        let off_p = [0.05, 0.14, 0.24][self.d];
        match kind {
            FeatureKind::Hairpin => {
                if self.rng.chance(0.7) {
                    1.0
                } else {
                    0.3
                }
            }
            FeatureKind::Kink | FeatureKind::Chicane => 0.4,
            _ => {
                let r = self.rng.f32();
                if r < off_p {
                    -1.0
                } else if r < off_p + 0.3 {
                    0.3
                } else {
                    1.0
                }
            }
        }
    }

    fn hairpin(&mut self, sign: f32, angle: f32) -> Elem {
        let r = if self.d == 0 { self.rng.range(15.0, 18.0) } else { self.rng.range(12.0, 17.0) };
        let cam = self.camber(FeatureKind::Hairpin);
        curve(sign, angle, r * 1.15, r, self.rng.range(8.0, 12.0), self.rng.range(9.0, 13.0), FeatureKind::Hairpin, cam, 1.0)
    }

    fn square(&mut self, sign: f32, angle: f32) -> Elem {
        let r = [self.rng.range(18.0, 23.0), self.rng.range(15.0, 21.0), self.rng.range(14.0, 19.0)][self.d];
        let cam = self.camber(FeatureKind::Square);
        curve(sign, angle, r, r, self.rng.range(4.0, 7.0), self.rng.range(5.0, 8.0), FeatureKind::Square, cam, 0.9)
    }

    fn medium(&mut self, sign: f32, angle: f32) -> Elem {
        let r = [self.rng.range(36.0, 70.0), self.rng.range(26.0, 60.0), self.rng.range(22.0, 50.0)][self.d];
        let cam = self.camber(FeatureKind::Medium);
        let tech = (30.0 / r).min(1.0) * 0.8;
        curve(sign, angle, r, r, self.rng.range(10.0, 18.0), self.rng.range(10.0, 18.0), FeatureKind::Medium, cam, tech)
    }

    fn sweeper(&mut self, sign: f32, angle: f32) -> Elem {
        let r = self.rng.range(70.0, 150.0);
        let cam = self.camber(FeatureKind::Sweeper);
        curve(sign, angle, r, r, self.rng.range(22.0, 38.0), self.rng.range(22.0, 38.0), FeatureKind::Sweeper, cam, 0.0)
    }

    fn tightening(&mut self, sign: f32, angle: f32, opening: bool) -> Elem {
        let tight = [self.rng.range(30.0, 45.0), self.rng.range(22.0, 38.0), self.rng.range(17.0, 32.0)][self.d];
        let wide = tight * self.rng.range(2.2, 3.4);
        let cam = self.camber(FeatureKind::Tightening);
        if opening {
            curve(sign, angle, tight, wide, self.rng.range(6.0, 10.0), self.rng.range(18.0, 28.0), FeatureKind::Opening, cam, 0.6)
        } else {
            curve(sign, angle, wide, tight, self.rng.range(18.0, 28.0), self.rng.range(6.0, 10.0), FeatureKind::Tightening, cam, 0.7)
        }
    }

    fn kink(&mut self, sign: f32, angle: f32) -> Elem {
        let r = self.rng.range(55.0, 110.0);
        curve(sign, angle, r, r, self.rng.range(7.0, 11.0), self.rng.range(7.0, 11.0), FeatureKind::Kink, 0.4, 0.0)
    }

    fn sbend_corner(&mut self, sign: f32, angle: f32) -> Elem {
        let r = [self.rng.range(30.0, 48.0), self.rng.range(24.0, 42.0), self.rng.range(20.0, 36.0)][self.d];
        let cam = self.camber(FeatureKind::SBend);
        curve(sign, angle, r, r, self.rng.range(7.0, 12.0), self.rng.range(7.0, 12.0), FeatureKind::SBend, cam, 0.85)
    }

    fn link(&mut self, lo: f32, hi: f32) -> Elem {
        Elem::Straight { len: self.rng.range(lo, hi), main: None, frac: 0.0 }
    }

    /// Any "normal" single corner of the given angle, chosen by difficulty
    fn single(&mut self, sign: f32, angle: f32) -> Elem {
        let w = [[0.35, 0.40, 0.10, 0.15], [0.35, 0.18, 0.27, 0.20], [0.33, 0.08, 0.37, 0.22]][self.d];
        let r = self.rng.f32();
        if r < w[0] || angle < deg(25.0) {
            self.medium(sign, angle)
        } else if r < w[0] + w[1] {
            self.sweeper(sign, angle)
        } else if r < w[0] + w[1] + w[2] {
            self.tightening(sign, angle, false)
        } else {
            self.tightening(sign, angle, true)
        }
    }

    /// Corner complex with net heading change `delta` (rad, > 0) in direction `dir`.
    fn junction(&mut self, kind: JunctionKind, dir: f32, delta: f32, out: &mut Vec<Elem>) {
        match kind {
            JunctionKind::Single => {
                let e = self.single(dir, delta);
                out.push(e);
            }
            JunctionKind::Hairpin => {
                let h = self.rng.range(deg(160.0), deg(181.0)).clamp(delta + deg(60.0), delta + deg(125.0));
                let counter = h - delta;
                let hp = self.hairpin(dir, h);
                let c = if counter > deg(75.0) && counter < deg(105.0) && self.rng.chance(0.6) {
                    self.square(-dir, counter)
                } else if self.rng.chance(0.3) {
                    {
                    let o = self.rng.chance(0.5);
                    self.tightening(-dir, counter, o)
                }
                } else {
                    self.medium(-dir, counter)
                };
                let l = self.link(16.0, 34.0);
                if self.rng.chance(0.55) {
                    out.extend([hp, l, c]);
                } else {
                    out.extend([c, l, hp]);
                }
            }
            JunctionKind::Square => {
                if delta > deg(68.0) && delta < deg(112.0) {
                    let e = self.square(dir, delta);
                    out.push(e);
                } else {
                    let a = deg(self.rng.range(84.0, 96.0));
                    let sq = self.square(dir, a);
                    let Elem::Curve { ka, a, b, c, .. } = sq else { unreachable!() };
                    let sq_angle = ka.abs() * (a * 0.5 + b * 0.5 + c);
                    let rem = delta - sq_angle;
                    let l = self.link(22.0, 50.0);
                    let other = if rem < 0.0 {
                        if -rem < deg(22.0) { self.kink(-dir, -rem) } else { self.medium(-dir, -rem) }
                    } else if rem < deg(22.0) {
                        self.kink(dir, rem.max(deg(6.0)))
                    } else {
                        self.medium(dir, rem)
                    };
                    if self.rng.chance(0.5) {
                        out.extend([sq, l, other]);
                    } else {
                        out.extend([other, l, sq]);
                    }
                }
            }
            JunctionKind::SBend => {
                let b = deg(self.rng.range(30.0, 55.0));
                let a = (delta + b) * self.rng.range(0.4, 0.6);
                let c = delta + b - a;
                let e1 = self.sbend_corner(dir, a);
                let l1 = self.link(0.0, 12.0);
                let e2 = self.sbend_corner(-dir, b);
                let l2 = self.link(0.0, 12.0);
                let e3 = self.sbend_corner(dir, c);
                out.extend([e1, l1, e2, l2, e3]);
            }
            JunctionKind::Double => {
                let f = self.rng.range(0.35, 0.65);
                let e1 = self.single(dir, delta * f);
                let l = self.link(8.0, 45.0);
                let e2 = self.single(dir, delta * (1.0 - f));
                out.extend([e1, l, e2]);
            }
        }
    }

    /// Net-zero complex inserted in the middle of a main straight.
    fn insert(&mut self, out: &mut Vec<Elem>) {
        let sign = if self.rng.chance(0.5) { 1.0 } else { -1.0 };
        let chicane_p = [0.25, 0.5, 0.55][self.d];
        if self.rng.chance(chicane_p) {
            let a = deg(self.rng.range(24.0, 42.0) + 6.0 * self.d as f32);
            let r = self.rng.range(18.0, 30.0);
            let cam = 0.4;
            let mut e1 = curve(sign, a, r, r, 6.0, 6.0, FeatureKind::Chicane, cam, 0.9);
            let l = self.link(0.0, 6.0);
            let mut e2 = curve(-sign, a, r, r, 6.0, 6.0, FeatureKind::Chicane, cam, 0.9);
            if let Elem::Curve { tech, .. } = &mut e1 {
                *tech = 0.9;
            }
            if let Elem::Curve { tech, .. } = &mut e2 {
                *tech = 0.9;
            }
            out.extend([e1, l, e2]);
        } else {
            let a = deg(self.rng.range(8.0, 17.0));
            let mut k1 = self.kink(sign, a);
            let jk_p = [0.0, 0.45, 0.75][self.d];
            if let Elem::Curve { jump_kink, .. } = &mut k1 {
                *jump_kink = self.rng.chance(jk_p);
            }
            let l = self.link(25.0, 60.0);
            let k2 = self.kink(-sign, a);
            out.extend([k1, l, k2]);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JunctionKind {
    Single,
    Hairpin,
    Square,
    SBend,
    Double,
}

const STEP: f32 = 0.25;

struct Integrated {
    /// Fine points at STEP spacing (the last one is the loop end)
    pts: Vec<Vec2>,
    /// Element start arc lengths
    starts: Vec<f32>,
    total: f32,
    end_heading: f32,
    /// heading (unit) of each main straight
    main_dir: Vec<Vec2>,
}

fn integrate(elems: &[Elem], mains: &[f32]) -> Integrated {
    let mut p = Vec2::ZERO;
    let mut th = 0.0f32;
    let mut pts = vec![p];
    let mut starts = Vec::with_capacity(elems.len());
    let mut main_dir = vec![Vec2::ZERO; mains.len()];
    let mut s_total = 0.0;
    for e in elems {
        starts.push(s_total);
        let len = e.len(mains);
        if let Elem::Straight { main: Some(k), .. } = e {
            if main_dir[*k] == Vec2::ZERO {
                main_dir[*k] = Vec2::new(th.cos(), th.sin());
            }
        }
        let steps = (len / STEP).ceil().max(1.0) as usize;
        let h = len / steps as f32;
        for i in 0..steps {
            let k = e.curvature((i as f32 + 0.5) * h);
            let th_mid = th + k * h * 0.5;
            th += k * h;
            p += Vec2::new(th_mid.cos(), th_mid.sin()) * h;
            pts.push(p);
        }
        s_total += len;
    }
    Integrated { pts, starts, total: s_total, end_heading: th, main_dir }
}

/// Resample an open polyline (first == last for a loop) to `count` points at uniform arc length.
fn resample_loop(pts: &[Vec2], count: usize) -> Vec<Vec2> {
    let mut cum = Vec::with_capacity(pts.len());
    let mut t = 0.0;
    cum.push(0.0);
    for w in pts.windows(2) {
        t += w[0].distance(w[1]);
        cum.push(t);
    }
    let step = t / count as f32;
    let mut out = Vec::with_capacity(count);
    let mut j = 0;
    for i in 0..count {
        let target = i as f32 * step;
        while j + 1 < cum.len() - 1 && cum[j + 1] < target {
            j += 1;
        }
        let seg = (cum[j + 1] - cum[j]).max(1e-6);
        let f = ((target - cum[j]) / seg).clamp(0.0, 1.0);
        out.push(pts[j].lerp(pts[j + 1], f));
    }
    out
}

/// Pairs of samples far apart along the loop must be far apart in space too: no overlapping
/// roads and no tempting shortcuts.
fn clearance_ok(pts: &[Vec2], spacing: f32, near: f32, far: f32) -> bool {
    let n = pts.len();
    let cell = 16.0;
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for p in pts {
        min = min.min(*p);
        max = max.max(*p);
    }
    let nx = ((max.x - min.x) / cell) as i32 + 1;
    let nz = ((max.y - min.y) / cell) as i32 + 1;
    let mut cells: Vec<Vec<u32>> = vec![Vec::new(); (nx * nz) as usize];
    for (i, p) in pts.iter().enumerate() {
        let cx = ((p.x - min.x) / cell) as i32;
        let cz = ((p.y - min.y) / cell) as i32;
        cells[(cz * nx + cx) as usize].push(i as u32);
    }
    let r = (far / cell).ceil() as i32;
    for i in (0..n).step_by(2) {
        let p = pts[i];
        let cx = ((p.x - min.x) / cell) as i32;
        let cz = ((p.y - min.y) / cell) as i32;
        for z in (cz - r).max(0)..=(cz + r).min(nz - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(nx - 1) {
                for &j in &cells[(z * nx + x) as usize] {
                    let j = j as usize;
                    let di = (i as isize - j as isize).unsigned_abs();
                    let ds = di.min(n - di) as f32 * spacing;
                    if ds < 36.0 {
                        continue;
                    }
                    // required spacing grows from `near` to `far` with arc distance
                    let req = if ds < 120.0 { near } else { (near + (ds - 120.0) * 0.15).min(far) };
                    if p.distance_squared(pts[j]) < req * req {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Design a closed loop. Returns None when this candidate fails (caller retries with the rng).
pub fn design(rng: &mut Rng, difficulty: u8, length_class: u8, spacing: f32, max_half_width: f32, shoulder: f32) -> Option<Layout> {
    let d = (difficulty as usize).min(2);
    let lc = (length_class as usize).min(2);
    let target = [1250.0, 1750.0, 2450.0][lc] * rng.range(0.92, 1.08);
    let k_main = match lc {
        0 => rng.range_i(5, 7),
        1 => rng.range_i(6, 8),
        _ => rng.range_i(7, 10),
    } as usize
        + (d == 2) as usize;
    let dir = if rng.chance(0.5) { 1.0 } else { -1.0 };

    // net heading change per junction (sums to one full turn)
    let mut deltas: Vec<f32> = (0..k_main).map(|_| rng.range(0.55, 1.5)).collect();
    for _ in 0..4 {
        let sum: f32 = deltas.iter().sum();
        for dl in deltas.iter_mut() {
            *dl = (*dl / sum * TAU).clamp(deg(22.0), deg(125.0));
        }
    }
    let sum: f32 = deltas.iter().sum();
    deltas.iter_mut().for_each(|dl| *dl *= TAU / sum);

    // junction kinds: mandatory technical corners first, the rest by weight
    let mut kinds = vec![JunctionKind::Single; k_main];
    let (min_hp, max_hp, min_sq) = match d {
        0 => (0, 1, 1),
        1 => (1, 2 + (lc == 2) as usize, 1),
        _ => (2, 3 + (lc == 2) as usize, 1),
    };
    let mut order: Vec<usize> = (0..k_main).collect();
    for i in (1..order.len()).rev() {
        let j = rng.range_i(0, i as i32 + 1) as usize;
        order.swap(i, j);
    }
    let mut oi = 0;
    for _ in 0..min_hp {
        kinds[order[oi]] = JunctionKind::Hairpin;
        oi += 1;
    }
    for _ in 0..min_sq {
        kinds[order[oi]] = JunctionKind::Square;
        oi += 1;
    }
    let w = [[0.52, 0.06, 0.14, 0.10, 0.18], [0.32, 0.18, 0.20, 0.15, 0.15], [0.20, 0.28, 0.22, 0.18, 0.12]][d];
    let mut hp_count = min_hp;
    for &j in &order[oi..] {
        let r = rng.f32();
        let mut k = if r < w[0] {
            JunctionKind::Single
        } else if r < w[0] + w[1] {
            JunctionKind::Hairpin
        } else if r < w[0] + w[1] + w[2] {
            JunctionKind::Square
        } else if r < w[0] + w[1] + w[2] + w[3] {
            JunctionKind::SBend
        } else {
            JunctionKind::Double
        };
        if k == JunctionKind::Hairpin {
            if hp_count >= max_hp {
                k = JunctionKind::Single;
            } else {
                hp_count += 1;
            }
        }
        kinds[j] = k;
    }

    // build the element list: S_0 J_0 S_1 J_1 ... ; main straight 0 is the start straight
    let mut pl = Planner { rng, d };
    let mut elems: Vec<Elem> = Vec::new();
    let mut weights = vec![0.0f32; k_main];
    let insert_p = [0.3, 0.45, 0.6][d];
    for k in 0..k_main {
        if k > 0 && pl.rng.chance(insert_p) {
            let f = pl.rng.range(0.35, 0.65);
            elems.push(Elem::Straight { len: 0.0, main: Some(k), frac: f });
            pl.insert(&mut elems);
            elems.push(Elem::Straight { len: 0.0, main: Some(k), frac: 1.0 - f });
        } else {
            elems.push(Elem::Straight { len: 0.0, main: Some(k), frac: 1.0 });
        }
        let mut wk = pl.rng.range(0.5, 1.5);
        if matches!(kinds[k], JunctionKind::Hairpin | JunctionKind::Square) {
            wk *= 1.7; // build speed before heavy braking
        }
        if k == 0 {
            wk *= 1.8;
        }
        weights[k] = wk;
        pl.junction(kinds[k], dir, deltas[k], &mut elems);
    }

    let min_len = |k: usize| if k == 0 { 140.0 } else { [45.0, 35.0, 28.0][d] };
    let zero = vec![0.0; k_main];
    let fixed: f32 = elems.iter().map(|e| e.len(&zero)).sum();
    let remaining = (target - fixed).max(k_main as f32 * 50.0);
    let wsum: f32 = weights.iter().sum();
    let mut mains: Vec<f32> = (0..k_main).map(|k| (remaining * weights[k] / wsum).max(min_len(k))).collect();

    // ---- close the loop: solve for straight length changes (weighted min-norm)
    let mut free = vec![true; k_main];
    let mut closed = false;
    for _ in 0..6 {
        let it = integrate(&elems, &mains);
        let f = *it.pts.last().unwrap();
        if f.length() < 0.05 {
            closed = true;
            break;
        }
        let mut m = [0.0f32; 4];
        for k in 0..k_main {
            if !free[k] {
                continue;
            }
            let u = it.main_dir[k];
            let wk = mains[k];
            m[0] += wk * u.x * u.x;
            m[1] += wk * u.x * u.y;
            m[3] += wk * u.y * u.y;
        }
        m[2] = m[1];
        let det = m[0] * m[3] - m[1] * m[2];
        if det.abs() < 1e-3 {
            return None;
        }
        // lambda = -M^-1 f
        let lx = -(m[3] * f.x - m[1] * f.y) / det;
        let ly = -(-m[2] * f.x + m[0] * f.y) / det;
        let mut clamped = false;
        for k in 0..k_main {
            if !free[k] {
                continue;
            }
            let u = it.main_dir[k];
            let nl = mains[k] + mains[k] * (u.x * lx + u.y * ly);
            if nl < min_len(k) {
                mains[k] = min_len(k);
                free[k] = false;
                clamped = true;
            } else {
                mains[k] = nl;
            }
        }
        let _ = clamped;
    }
    if !closed {
        let it = integrate(&elems, &mains);
        if it.pts.last().unwrap().length() > 0.3 {
            return None;
        }
    }
    let it = integrate(&elems, &mains);
    // heading must close too
    let turn = it.end_heading.abs();
    if (turn - TAU).abs() > 0.02 {
        return None;
    }
    let total = it.total;
    if total < target * 0.75 || total > target * 1.45 {
        return None;
    }
    // compact enough (terrain size)
    let mut bmin = Vec2::splat(f32::MAX);
    let mut bmax = Vec2::splat(f32::MIN);
    for p in &it.pts {
        bmin = bmin.min(*p);
        bmax = bmax.max(*p);
    }
    let ext = bmax - bmin;
    if ext.max_element() > total * 0.42 + 160.0 {
        return None;
    }

    let mut fine = it.pts.clone();
    // distribute the tiny residual linearly
    let resid = *fine.last().unwrap();
    let nf = fine.len() as f32;
    for (i, p) in fine.iter_mut().enumerate() {
        *p -= resid * (i as f32 / (nf - 1.0));
    }
    let count = (total / spacing).round() as usize;
    let pts0 = resample_loop(&fine, count);
    let scale = count as f32 * spacing / total;

    // start: 60% along main straight 0 (never split)
    let start_s = (it.starts[0] + mains[0] * 0.6) * scale;
    let shift = (start_s / spacing).round() as usize % count;
    let mut pts = pts0;
    pts.rotate_left(shift);
    let shift_s = shift as f32 * spacing;
    let len_final = count as f32 * spacing;

    let near = 2.0 * (max_half_width + shoulder) + 6.0;
    if !clearance_ok(&pts, spacing, near, near + 16.0) {
        return None;
    }

    // features + per-sample technical / camber
    let mut features = Vec::new();
    let mut tech = vec![0.0f32; count];
    let mut camber = vec![1.0f32; count];
    for (ei, e) in elems.iter().enumerate() {
        let s0 = (it.starts[ei] * scale - shift_s).rem_euclid(len_final);
        let l = e.len(&mains) * scale;
        let s1 = (s0 + l).rem_euclid(len_final);
        match *e {
            Elem::Straight { main, .. } => {
                if main.is_some() {
                    features.push(Feature { kind: FeatureKind::Straight, s0, s1, main, jump_kink: false });
                }
            }
            Elem::Curve { kind, camber: cam, tech: t, jump_kink, .. } => {
                features.push(Feature { kind, s0, s1, main: None, jump_kink });
                let i0 = (s0 / spacing).floor() as isize;
                let i1 = i0 + (l / spacing).ceil() as isize;
                for i in i0 - 6..=i1 + 6 {
                    let j = i.rem_euclid(count as isize) as usize;
                    let edge = if i < i0 { (i0 - i) as f32 / 6.0 } else if i > i1 { (i - i1) as f32 / 6.0 } else { 0.0 };
                    tech[j] = tech[j].max(t * (1.0 - edge));
                    if (i0..=i1).contains(&i) {
                        camber[j] = cam;
                    }
                }
            }
        }
    }
    Some(Layout { pts, features, tech, camber })
}

// ---------------------------------------------------------------------------------------------
// Elevation and surface design on top of the layout

fn fwd(a: f32, b: f32, len: f32) -> f32 {
    (b - a).rem_euclid(len)
}

fn arc_dist(a: f32, b: f32, len: f32) -> f32 {
    let d = (a - b).rem_euclid(len);
    d.min(len - d)
}

fn straight_window(curv: &[f32], spacing: f32, s: f32, before: f32, after: f32, max_k: f32) -> bool {
    let n = curv.len() as isize;
    let i = (s / spacing).round() as isize;
    let b = (before / spacing) as isize;
    let a = (after / spacing) as isize;
    (-b..=a).all(|k| curv[(i + k).rem_euclid(n) as usize].abs() < max_k)
}

/// Distance from s to the start of the next real corner (kinks don't count).
fn next_corner(features: &[Feature], s: f32, len: f32) -> f32 {
    features
        .iter()
        .filter(|f| f.kind.is_corner() && f.kind != FeatureKind::Kink)
        .map(|f| fwd(s, f.s0, len))
        .fold(f32::MAX, f32::min)
}

/// Adds blind crests before/over corners, compressions and jumps. Returns jump take-off indices.
pub fn shape_elevation(elev: &mut [f32], features: &[Feature], curv: &[f32], rng: &mut Rng, difficulty: u8, spacing: f32) -> Vec<usize> {
    let n = elev.len();
    let len = n as f32 * spacing;
    let d = (difficulty as usize).min(2);
    let near_start = |s: f32| s < 90.0 || len - s < 120.0;
    let max_k = 1.0 / 90.0;

    // ---- jumps
    let mut jump_s: Vec<f32> = Vec::new();
    let n_jumps = [1, rng.range_i(1, 3), rng.range_i(2, 4)][d] as usize + (len > 2200.0) as usize;
    for f in features.iter().filter(|f| f.jump_kink) {
        if jump_s.len() >= n_jumps {
            break;
        }
        let s = (f.s0 - rng.range(15.0, 22.0)).rem_euclid(len);
        if !near_start(s) && straight_window(curv, spacing, s, 36.0, 4.0, max_k) && jump_s.iter().all(|&j| arc_dist(j, s, len) > 150.0) {
            jump_s.push(s);
        }
    }
    let straights: Vec<&Feature> = features.iter().filter(|f| f.kind == FeatureKind::Straight && f.len(len) > 75.0).collect();
    let mut tries = 0;
    while jump_s.len() < n_jumps && tries < 60 && !straights.is_empty() {
        tries += 1;
        let f = straights[rng.range_i(0, straights.len() as i32) as usize];
        let fl = f.len(len);
        let s = (f.s0 + rng.range(38.0, (fl - 28.0).max(39.0))).rem_euclid(len);
        if near_start(s) || !straight_window(curv, spacing, s, 38.0, 24.0, max_k) {
            continue;
        }
        if next_corner(features, s, len) < 60.0 || jump_s.iter().any(|&j| arc_dist(j, s, len) < 150.0) {
            continue;
        }
        jump_s.push(s);
    }

    // ---- crests and compressions (raised-cosine bumps)
    let mut bumps: Vec<(f32, f32, f32)> = Vec::new();
    let crest_p = [0.22, 0.42, 0.55][d];
    let clear_of_jumps = |s: f32, w: f32, jumps: &[f32]| jumps.iter().all(|&j| arc_dist(j, s, len) > w + 45.0);
    for f in features.iter().filter(|f| f.kind.is_corner() && f.kind != FeatureKind::Kink) {
        if !rng.chance(crest_p) {
            continue;
        }
        let over = rng.chance(0.35);
        let c = if over { f.s0 + rng.range(3.0, 12.0) } else { f.s0 - rng.range(6.0, 22.0) }.rem_euclid(len);
        let (amp, w) = if f.kind == FeatureKind::Hairpin { (rng.range(0.9, 1.4), rng.range(26.0, 34.0)) } else { (rng.range(1.3, 2.3), rng.range(24.0, 34.0)) };
        if near_start(c) || !clear_of_jumps(c, w, &jump_s) || bumps.iter().any(|b| arc_dist(b.0, c, len) < b.2 + w) {
            continue;
        }
        bumps.push((c, amp, w));
    }
    let dip_p = [0.2, 0.35, 0.45][d];
    for f in features.iter().filter(|f| f.kind == FeatureKind::Straight && f.main != Some(0)) {
        let fl = f.len(len);
        if fl < 80.0 || !rng.chance(dip_p) {
            continue;
        }
        let c = (f.s0 + fl * rng.range(0.3, 0.7)).rem_euclid(len);
        let w = rng.range(20.0, 28.0);
        if near_start(c) || !clear_of_jumps(c, w, &jump_s) || bumps.iter().any(|b| arc_dist(b.0, c, len) < b.2 + w) {
            continue;
        }
        bumps.push((c, -rng.range(1.1, 1.9), w));
    }
    for &(c, amp, w) in &bumps {
        let r = (w / spacing).ceil() as isize;
        let ci = (c / spacing).round() as isize;
        for k in -r..=r {
            let x = (k as f32 * spacing / w).clamp(-1.0, 1.0);
            let i = (ci + k).rem_euclid(n as isize) as usize;
            elev[i] += amp * 0.5 * (1.0 + (x * std::f32::consts::PI).cos());
        }
    }
    // keep grades sane after adding features
    let max_step = 0.17 * spacing;
    for _ in 0..3 {
        for i in 0..n {
            let prev = elev[(i + n - 1) % n];
            elev[i] = elev[i].clamp(prev - max_step, prev + max_step);
        }
        for i in (0..n).rev() {
            let next = elev[(i + 1) % n];
            elev[i] = elev[i].clamp(next - max_step, next + max_step);
        }
    }

    // ---- jump ramps: up over 16 m then a short drop
    let (hlo, hhi) = [(1.1, 1.5), (1.3, 1.9), (1.5, 2.0)][d];
    let mut jumps = Vec::new();
    for &s in &jump_s {
        let j = (s / spacing).round() as isize;
        let up = 8isize;
        let down = 3isize;
        let h = rng.range(hlo, hhi);
        for k in -up..=down {
            let i = (j + k).rem_euclid(n as isize) as usize;
            let f = if k <= 0 {
                let t = (k + up) as f32 / up as f32;
                t * t * (3.0 - 2.0 * t)
            } else {
                1.0 - (k as f32 / down as f32)
            };
            elev[i] += h * f;
        }
        jumps.push(j.rem_euclid(n as isize) as usize);
    }
    jumps
}
