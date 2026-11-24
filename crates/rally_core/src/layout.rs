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
    /// Camber factor (1 banked, ~0.3 flat, < 0 off-camber); 1 for straights
    pub camber: f32,
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
            // loops: main straights get their length from the closure solve; stages (no
            // `mains`) use the stored length
            Elem::Straight { main: Some(k), frac, .. } if k < mains.len() => mains[k] * frac,
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
        let off_p = [0.0, 0.12, 0.22][self.d];
        match kind {
            // hairpins are banked like real mountain hairpins (helps everyone round)
            FeatureKind::Hairpin => 1.25,
            FeatureKind::Square => {
                if self.d == 2 && self.rng.chance(0.4) {
                    0.3
                } else {
                    1.0
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
                    features.push(Feature { kind: FeatureKind::Straight, s0, s1, main, jump_kink: false, camber: 1.0 });
                }
            }
            Elem::Curve { kind, camber: cam, tech: t, jump_kink, .. } => {
                features.push(Feature { kind, s0, s1, main: None, jump_kink, camber: cam });
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
    // open roads: the ends simply count as straight (clamped)
    (-b..=a).all(|k| {
        let j = i + k;
        let j = if j < 0 || j >= n { j.rem_euclid(n) } else { j };
        curv[j as usize].abs() < max_k
    })
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
    shape_elevation_ex(elev, features, curv, rng, difficulty, spacing, None)
}

/// `open` = Some((start_line_s, finish_line_s)) for point-to-point stages: no wrap-around, no
/// features near the start area / finish / run-off, jump count scales with stage length.
pub fn shape_elevation_ex(elev: &mut [f32], features: &[Feature], curv: &[f32], rng: &mut Rng, difficulty: u8, spacing: f32, open: Option<(f32, f32)>) -> Vec<usize> {
    let n = elev.len();
    let road_len = n as f32 * spacing;
    // distances on an open road: a "virtual loop" far longer than the road never wraps
    let len = if open.is_some() { road_len * 2.0 + 2000.0 } else { road_len };
    let idx = |i: isize| -> usize { if open.is_some() { i.clamp(0, n as isize - 1) as usize } else { i.rem_euclid(n as isize) as usize } };
    let d = (difficulty as usize).min(2);
    let near_start = |s: f32| match open {
        Some((a, b)) => s < a + 70.0 || s > b - 70.0,
        None => s < 90.0 || len - s < 120.0,
    };
    let max_k = 1.0 / 90.0;

    // ---- jumps
    let mut jump_s: Vec<f32> = Vec::new();
    let n_jumps = match open {
        Some((a, b)) => (((b - a) / 1000.0) * [0.3, 0.45, 0.6][d] + rng.f32()) as usize,
        None => [1, rng.range_i(1, 3), rng.range_i(2, 4)][d] as usize + (len > 2200.0) as usize,
    };
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
        let lead = if over { -rng.range(3.0, 12.0) } else { rng.range(6.0, 22.0) };
        let c = (f.s0 - lead).rem_euclid(len);
        let (amp, w) = if f.kind == FeatureKind::Hairpin { (rng.range(0.9, 1.4), rng.range(26.0, 34.0)) } else { (rng.range(1.3, 2.3), rng.range(24.0, 34.0)) };
        // keep the crest drivable at the speed a car arrives with: vertical acceleration over
        // the top (amp * pi^2 / (2 w^2) * v^2) stays below ~0.65 g
        let i0 = (f.s0 / spacing) as usize;
        let i1 = i0 + (f.len(len) / spacing) as usize;
        let k_peak = (i0..=i1).map(|i| curv[idx(i as isize)].abs()).fold(1e-4, f32::max);
        let v_corner = (0.8 * 9.81 / k_peak).sqrt().min(40.0);
        let v = (v_corner * v_corner + 2.0 * 6.0 * lead.max(0.0)).sqrt().min(40.0);
        let amp_max = 0.65 * 9.81 * 2.0 * w * w / (std::f32::consts::PI.powi(2) * v * v);
        let amp = amp.min(amp_max);
        if amp < 0.6 {
            continue;
        }
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
        // dips at full speed: keep the compression below ~1.1 g extra
        let amp_max = 1.1 * 9.81 * 2.0 * w * w / (std::f32::consts::PI.powi(2) * 40.0 * 40.0);
        bumps.push((c, -rng.range(1.1, 1.9).min(amp_max), w));
    }
    for &(c, amp, w) in &bumps {
        let r = (w / spacing).ceil() as isize;
        let ci = (c / spacing).round() as isize;
        for k in -r..=r {
            let x = (k as f32 * spacing / w).clamp(-1.0, 1.0);
            let i = idx(ci + k);
            elev[i] += amp * 0.5 * (1.0 + (x * std::f32::consts::PI).cos());
        }
    }
    // keep grades sane after adding features
    let max_step = 0.17 * spacing;
    for _ in 0..3 {
        for i in 0..n {
            let prev = elev[idx(i as isize - 1)];
            elev[i] = elev[i].clamp(prev - max_step, prev + max_step);
        }
        for i in (0..n).rev() {
            let next = elev[idx(i as isize + 1)];
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
            let i = idx(j + k);
            let f = if k <= 0 {
                let t = (k + up) as f32 / up as f32;
                t * t * (3.0 - 2.0 * t)
            } else {
                1.0 - (k as f32 / down as f32)
            };
            elev[i] += h * f;
        }
        jumps.push(idx(j));
    }
    jumps
}

// =============================================================================================
// Point-to-point stages
// =============================================================================================

/// An open (A→B) stage layout. Arc length 0 is the very beginning of the road (start area);
/// the start line is at `start_s`, the finish line at `finish_s`, followed by a straight run-off.
pub struct StageLayout {
    pub pts: Vec<Vec2>,
    pub features: Vec<Feature>,
    pub tech: Vec<f32>,
    pub camber: Vec<f32>,
    pub start_s: f32,
    pub finish_s: f32,
    /// Designed elevation changes (s0, s1, rise m): switchbacks climbing / descending a hill
    pub climbs: Vec<(f32, f32, f32)>,
}

/// Start area length before the start line, and run-off after the finish line (m)
pub const STAGE_PRE_START: f32 = 80.0;
pub const STAGE_RUN_OFF: f32 = 220.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Fast,
    Technical,
    Mountain,
    Junctions,
}

impl Planner<'_> {
    /// Zig-zag of `count` alternating hairpins joined by short climbing legs.
    fn switchback(&mut self, first: f32, count: usize, out: &mut Vec<Elem>) {
        for k in 0..count {
            let sign = if k % 2 == 0 { first } else { -first };
            let h = deg(self.rng.range(150.0, 170.0));
            let e = self.hairpin(sign, h);
            out.push(e);
            if k + 1 < count {
                let l = self.link(45.0, 85.0);
                out.push(l);
            }
        }
    }
}

fn wrap_pi(a: f32) -> f32 {
    let mut a = a.rem_euclid(TAU);
    if a > std::f32::consts::PI {
        a -= TAU;
    }
    a
}

/// Incremental clearance grid for the stage walk (points every ~2 m with their arc length).
struct WalkGrid {
    cell: f32,
    cells: std::collections::HashMap<(i32, i32), Vec<u32>>,
    pts: Vec<(Vec2, f32)>,
}

impl WalkGrid {
    fn key(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.y / self.cell).floor() as i32)
    }

    fn push(&mut self, p: Vec2, s: f32) {
        let k = self.key(p);
        self.cells.entry(k).or_default().push(self.pts.len() as u32);
        self.pts.push((p, s));
    }

    fn truncate(&mut self, len: usize) {
        while self.pts.len() > len {
            let (p, _) = self.pts.pop().unwrap();
            let k = self.key(p);
            let i = self.pts.len() as u32;
            if let Some(v) = self.cells.get_mut(&k) {
                v.retain(|&j| j != i);
            }
        }
    }

    /// Is `p` (at arc length s) clear of every committed point far enough along the road?
    fn clear(&self, p: Vec2, s: f32, near: f32, far: f32) -> bool {
        let r = (far / self.cell).ceil() as i32;
        let (cx, cz) = self.key(p);
        for z in cz - r..=cz + r {
            for x in cx - r..=cx + r {
                if let Some(v) = self.cells.get(&(x, z)) {
                    for &j in v {
                        let (q, sq) = self.pts[j as usize];
                        let ds = (s - sq).abs();
                        if ds < 36.0 {
                            continue;
                        }
                        let req = if ds < 120.0 { near } else { (near + (ds - 120.0) * 0.2).min(far) };
                        if p.distance_squared(q) < req * req {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

/// Integrate `elems` from (pos, heading, s); returns fine points every ~0.5 m with arc length,
/// each element's start s, the end pose.
fn integrate_open(elems: &[Elem], mut p: Vec2, mut th: f32, mut s: f32) -> (Vec<(Vec2, f32)>, Vec<f32>, Vec2, f32, f32) {
    let mut pts = Vec::new();
    let mut starts = Vec::with_capacity(elems.len());
    for e in elems {
        starts.push(s);
        let len = e.len(&[]);
        let steps = (len / 0.5).ceil().max(1.0) as usize;
        let h = len / steps as f32;
        for i in 0..steps {
            let k = e.curvature((i as f32 + 0.5) * h);
            let th_mid = th + k * h * 0.5;
            th += k * h;
            p += Vec2::new(th_mid.cos(), th_mid.sin()) * h;
            s += h;
            pts.push((p, s));
        }
    }
    (pts, starts, p, th, s)
}

/// Design an open rally stage of roughly `stage_len` metres between start and finish line.
/// `natural(x, z)` is the untouched terrain height (switchbacks prefer to climb real hills).
#[allow(clippy::too_many_arguments)]
pub fn design_stage(rng: &mut Rng, difficulty: u8, stage_len: f32, spacing: f32, max_half_width: f32, shoulder: f32, natural: &dyn Fn(f32, f32) -> f32) -> Option<StageLayout> {
    let d = (difficulty as usize).min(2);
    let near = 2.0 * (max_half_width + shoulder) + 6.0;
    let far = near + 26.0;

    // ---- sections along the stage
    let mut sections: Vec<(Section, f32)> = Vec::new(); // (kind, end s relative to start line)
    let w = [[0.42, 0.30, 0.10, 0.18], [0.32, 0.36, 0.16, 0.16], [0.22, 0.42, 0.22, 0.14]][d];
    let mut acc = 0.0;
    let mut has_mountain = false;
    while acc < stage_len {
        let r = rng.f32();
        let mut kind = if r < w[0] {
            Section::Fast
        } else if r < w[0] + w[1] {
            Section::Technical
        } else if r < w[0] + w[1] + w[2] {
            Section::Mountain
        } else {
            Section::Junctions
        };
        // the first stretch after the start is always a fast one; no two mountains in a row
        if sections.is_empty() {
            kind = Section::Fast;
        }
        if kind == Section::Mountain && sections.last().is_some_and(|l| l.0 == Section::Mountain) {
            kind = Section::Technical;
        }
        let l = match kind {
            Section::Fast => rng.range(450.0, 950.0),
            Section::Technical => rng.range(400.0, 900.0),
            Section::Mountain => rng.range(350.0, 650.0),
            Section::Junctions => rng.range(250.0, 450.0),
        };
        has_mountain |= kind == Section::Mountain;
        acc += l;
        sections.push((kind, acc));
    }
    if !has_mountain && d >= 1 && stage_len >= 3500.0 && sections.len() > 2 {
        let k = rng.range_i(1, sections.len() as i32 - 1) as usize;
        sections[k].0 = Section::Mountain;
    }

    // ---- macro route: the general direction the stage wanders in
    let phi0 = rng.range(0.0, TAU);
    let (a1, l1, p1) = (rng.range(0.6, 1.15), rng.range(2200.0, 3800.0), rng.range(0.0, TAU));
    let (a2, l2, p2) = (rng.range(0.25, 0.6), rng.range(800.0, 1300.0), rng.range(0.0, TAU));
    let route = |s: f32| phi0 + a1 * (TAU * s / l1 + p1).sin() + a2 * (TAU * s / l2 + p2).sin();

    let mut grid = WalkGrid { cell: 16.0, cells: Default::default(), pts: Vec::new() };
    let mut elems: Vec<Elem> = Vec::new();
    let mut elem_s: Vec<f32> = Vec::new();
    // undo stack: (elems len, grid len, pose)
    let mut stack: Vec<(usize, usize, Vec2, f32, f32)> = Vec::new();
    let mut climbs: Vec<(f32, f32, f32, usize)> = Vec::new(); // + elems len at creation
    let (mut pos, mut th, mut s) = (Vec2::ZERO, phi0, 0.0f32);
    grid.push(pos, 0.0);

    // start area + first straight (the start line sits STAGE_PRE_START into it)
    let first = Elem::Straight { len: STAGE_PRE_START + rng.range(110.0, 170.0), main: Some(0), frac: 1.0 };
    {
        let (pts, starts, p, t, s1) = integrate_open(&[first], pos, th, s);
        for (q, sq) in pts.iter().step_by(4) {
            grid.push(*q, *sq);
        }
        elems.push(first);
        elem_s.extend(starts);
        pos = p;
        th = t;
        s = s1;
    }
    let start_s = STAGE_PRE_START;
    let target_finish = start_s + stage_len;
    let mut failures = 0;
    let mut fails_here = 0;
    let mut finish_s = 0.0;
    let mut finished = false;
    let mut main_idx = 1usize;
    while !finished {
        if failures > 600 {
            return None;
        }
        let rel = s - start_s;
        let section = sections.iter().find(|x| rel < x.1).map(|x| x.0).unwrap_or(Section::Fast);
        let mut group: Vec<Elem> = Vec::new();
        let mut climb: Option<f32> = None;
        let remaining = target_finish - s;
        let mut pl = Planner { rng: &mut *rng, d };
        if remaining < 260.0 {
            // finish: a straight up to the line, then the run-off
            let l = remaining.max(90.0);
            group.push(Elem::Straight { len: l, main: Some(main_idx), frac: 1.0 });
            group.push(Elem::Straight { len: STAGE_RUN_OFF, main: Some(usize::MAX), frac: 1.0 });
        } else {
            let want = wrap_pi(route(rel + 200.0) - th);
            let jitter = pl.rng.range(-0.55, 0.55);
            let delta = (want * 0.85 + jitter).clamp(-deg(115.0), deg(115.0));
            let sign = if delta.abs() < 1e-3 { if pl.rng.chance(0.5) { 1.0 } else { -1.0 } } else { delta.signum() };
            let mag = delta.abs();
            match section {
                Section::Fast => {
                    let l = pl.rng.range(120.0, 320.0) * [1.0, 0.9, 0.8][d];
                    if pl.rng.chance(0.35) {
                        let f = pl.rng.range(0.35, 0.65);
                        group.push(Elem::Straight { len: l * f, main: Some(main_idx), frac: 1.0 });
                        pl.insert(&mut group);
                        group.push(Elem::Straight { len: l * (1.0 - f), main: Some(main_idx), frac: 1.0 });
                    } else {
                        group.push(Elem::Straight { len: l, main: Some(main_idx), frac: 1.0 });
                    }
                    if mag < deg(18.0) {
                        let e = pl.kink(sign, mag.max(deg(7.0)));
                        group.push(e);
                    } else if pl.rng.chance(0.55) {
                        let e = pl.sweeper(sign, mag);
                        group.push(e);
                    } else {
                        let e = pl.single(sign, mag);
                        group.push(e);
                    }
                }
                Section::Technical => {
                    let l = pl.rng.range(20.0, 75.0);
                    group.push(Elem::Straight { len: l, main: Some(main_idx), frac: 1.0 });
                    let r = pl.rng.f32();
                    let mag = mag.max(deg(30.0));
                    let kind = if r < 0.28 {
                        JunctionKind::SBend
                    } else if r < 0.5 {
                        JunctionKind::Double
                    } else if r < 0.62 && d >= 1 {
                        JunctionKind::Hairpin
                    } else if r < 0.74 {
                        JunctionKind::Square
                    } else {
                        JunctionKind::Single
                    };
                    pl.junction(kind, sign, mag.min(deg(110.0)), &mut group);
                    if pl.rng.chance([0.15, 0.25, 0.35][d]) {
                        group.push(Elem::Straight { len: pl.rng.range(25.0, 50.0), main: Some(main_idx), frac: 1.0 });
                        pl.insert(&mut group);
                    }
                }
                Section::Mountain => {
                    group.push(Elem::Straight { len: pl.rng.range(40.0, 90.0), main: Some(main_idx), frac: 1.0 });
                    let count = if d == 2 && pl.rng.chance(0.4) { 4 } else { 2 };
                    let n0 = group.len();
                    pl.switchback(sign, count, &mut group);
                    let len: f32 = group[n0..].iter().map(|e| e.len(&[])).sum();
                    // climb the real hill if there is one nearby, otherwise mostly up
                    let ahead = pos + Vec2::new(th.cos(), th.sin()) * 120.0;
                    let side = Vec2::new(-th.sin(), th.cos()) * sign * 60.0;
                    let up = natural((ahead + side).x, (ahead + side).y) >= natural(pos.x, pos.y) - 2.0;
                    let rise = (len * pl.rng.range(0.06, 0.09)).min(26.0) * if up || pl.rng.chance(0.3) { 1.0 } else { -1.0 };
                    climb = Some(rise);
                    // leave the zig-zag with a short straight
                    group.push(Elem::Straight { len: pl.rng.range(30.0, 60.0), main: Some(main_idx), frac: 1.0 });
                }
                Section::Junctions => {
                    group.push(Elem::Straight { len: pl.rng.range(60.0, 170.0), main: Some(main_idx), frac: 1.0 });
                    let a = deg(pl.rng.range(82.0, 98.0));
                    let e = pl.square(sign, a);
                    group.push(e);
                }
            }
        }
        // ---- try to commit the group
        let (pts, starts, p, t, s1) = integrate_open(&group, pos, th, s);
        let mut ok = true;
        let mut fresh = WalkGrid { cell: 16.0, cells: Default::default(), pts: Vec::new() };
        for (q, sq) in pts.iter().step_by(4) {
            if !grid.clear(*q, *sq, near, far) || !fresh.clear(*q, *sq, near, far) {
                ok = false;
                break;
            }
            fresh.push(*q, *sq);
        }
        if ok {
            stack.push((elems.len(), grid.pts.len(), pos, th, s));
            if let Some(rise) = climb {
                // the climb spans the hairpins (not the approach / exit straights)
                let a = starts[1];
                let b = *starts.last().unwrap();
                climbs.push((a, b, rise, elems.len()));
            }
            for (q, sq) in pts.iter().step_by(4) {
                grid.push(*q, *sq);
            }
            if remaining < 260.0 {
                finish_s = starts[1];
                finished = true;
            }
            elems.extend(group);
            elem_s.extend(starts);
            pos = p;
            th = t;
            s = s1;
            main_idx += 1;
            fails_here = 0;
        } else {
            failures += 1;
            fails_here += 1;
            if fails_here > 8 {
                // backtrack one group
                fails_here = 0;
                if let Some((el, gl, p0, t0, s0)) = stack.pop() {
                    elems.truncate(el);
                    elem_s.truncate(el);
                    grid.truncate(gl);
                    climbs.retain(|c| c.3 < el);
                    pos = p0;
                    th = t0;
                    s = s0;
                } else {
                    return None;
                }
            }
        }
    }

    // ---- resample the whole road uniformly
    let (fine, starts, _, _, total) = integrate_open(&elems, Vec2::ZERO, phi0, 0.0);
    let mut poly: Vec<Vec2> = vec![Vec2::ZERO];
    poly.extend(fine.iter().map(|(p, _)| *p));
    let count = (total / spacing).round() as usize + 1;
    // resample_loop samples `count` points over [0, total) – add one more step to reach the end
    let pts = resample_loop(&poly, count - 1);
    let mut pts = pts;
    pts.push(*poly.last().unwrap());
    let n = pts.len();
    let scale = (n - 1) as f32 * spacing / total;

    // features + per-sample technical / camber
    let mut features = Vec::new();
    let mut tech = vec![0.0f32; n];
    let mut camber = vec![1.0f32; n];
    for (ei, e) in elems.iter().enumerate() {
        let s0 = starts[ei] * scale;
        let l = e.len(&[]) * scale;
        let s1 = s0 + l;
        match *e {
            Elem::Straight { main, .. } => {
                let main = match main {
                    Some(usize::MAX) => Some(0), // run-off: no features
                    m => m,
                };
                features.push(Feature { kind: FeatureKind::Straight, s0, s1, main, jump_kink: false, camber: 1.0 });
            }
            Elem::Curve { kind, camber: cam, tech: t, jump_kink, .. } => {
                features.push(Feature { kind, s0, s1, main: None, jump_kink, camber: cam });
                let i0 = (s0 / spacing).floor() as isize;
                let i1 = i0 + (l / spacing).ceil() as isize;
                for i in i0 - 6..=i1 + 6 {
                    if i < 0 || i >= n as isize {
                        continue;
                    }
                    let j = i as usize;
                    let edge = if i < i0 { (i0 - i) as f32 / 6.0 } else if i > i1 { (i - i1) as f32 / 6.0 } else { 0.0 };
                    tech[j] = tech[j].max(t * (1.0 - edge));
                    if (i0..=i1).contains(&i) {
                        camber[j] = cam;
                    }
                }
            }
        }
    }
    let climbs = climbs.iter().map(|c| (c.0 * scale, c.1 * scale, c.2)).collect();
    let _ = elem_s;
    Some(StageLayout { pts, features, tech, camber, start_s: start_s * scale, finish_s: finish_s * scale, climbs })
}

// =============================================================================================
// Tarmac racing circuits
// =============================================================================================

/// Circuit size classes (TrackParams::length_class for Discipline::Circuit)
pub const CIRCUIT_SIZE_NAMES: [&str; 4] = ["Sprint", "Grand Prix", "Ring", "Nordring"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CJ {
    Sweeper,
    Medium,
    HeavyBrake,
    Hairpin,
    Esses,
    DoubleApex,
    Tightening,
    Carousel,
}

/// Corner with circuit proportions (long clothoids for high speed)
fn ccurve(sign: f32, angle: f32, r_in: f32, r_out: f32, ramp: f32, kind: FeatureKind, camber: f32, tech: f32) -> Elem {
    curve(sign, angle, r_in, r_out, ramp, ramp * 1.15, kind, camber, tech)
}

fn circuit_junction(rng: &mut Rng, kind: CJ, dir: f32, delta: f32, out: &mut Vec<Elem>) {
    let link = |rng: &mut Rng, lo: f32, hi: f32| Elem::Straight { len: rng.range(lo, hi), main: None, frac: 0.0 };
    match kind {
        CJ::Sweeper => {
            let r = rng.range(150.0, 330.0);
            out.push(ccurve(dir, delta, r, r, rng.range(40.0, 70.0), FeatureKind::Sweeper, 0.6, 0.0));
        }
        CJ::Medium => {
            let r = rng.range(55.0, 115.0);
            out.push(ccurve(dir, delta, r, r, rng.range(22.0, 38.0), FeatureKind::Medium, 0.6, 0.3));
        }
        CJ::Tightening => {
            let r = rng.range(40.0, 70.0);
            if rng.chance(0.5) {
                out.push(ccurve(dir, delta, r * 2.4, r, rng.range(25.0, 40.0), FeatureKind::Tightening, 0.6, 0.4));
            } else {
                out.push(ccurve(dir, delta, r, r * 2.4, rng.range(20.0, 30.0), FeatureKind::Opening, 0.6, 0.4));
            }
        }
        CJ::Carousel => {
            // steeply banked long bowl
            let r = rng.range(38.0, 55.0);
            out.push(ccurve(dir, delta, r, r, rng.range(20.0, 30.0), FeatureKind::Medium, 2.2, 0.5));
        }
        CJ::DoubleApex => {
            let f = rng.range(0.4, 0.6);
            let r = rng.range(42.0, 75.0);
            out.push(ccurve(dir, delta * f, r, r, rng.range(18.0, 28.0), FeatureKind::Medium, 0.6, 0.5));
            out.push(link(rng, 15.0, 45.0));
            out.push(ccurve(dir, delta * (1.0 - f), r * rng.range(0.8, 1.2), r, rng.range(18.0, 28.0), FeatureKind::Medium, 0.6, 0.5));
        }
        CJ::HeavyBrake => {
            if delta > deg(65.0) && delta < deg(115.0) {
                let r = rng.range(22.0, 38.0);
                out.push(ccurve(dir, delta, r, r, rng.range(10.0, 16.0), FeatureKind::Square, 0.6, 0.8));
            } else {
                let a = deg(rng.range(84.0, 96.0));
                let r = rng.range(22.0, 38.0);
                out.push(ccurve(dir, a, r, r, rng.range(10.0, 16.0), FeatureKind::Square, 0.6, 0.8));
                let rem = delta - a;
                if rem.abs() > deg(8.0) {
                    out.push(link(rng, 90.0, 220.0));
                    let r2 = rng.range(80.0, 180.0);
                    out.push(ccurve(rem.signum() * dir, rem.abs(), r2, r2, 35.0, FeatureKind::Medium, 0.6, 0.2));
                }
            }
        }
        CJ::Hairpin => {
            let h = rng.range(deg(132.0), deg(165.0)).clamp(delta + deg(50.0), delta + deg(125.0));
            let r = rng.range(15.0, 24.0);
            out.push(ccurve(dir, h, r * 1.2, r, rng.range(12.0, 18.0), FeatureKind::Hairpin, 0.8, 1.0));
            out.push(link(rng, 90.0, 180.0));
            let counter = h - delta;
            let r2 = rng.range(60.0, 140.0);
            out.push(ccurve(-dir, counter, r2, r2, rng.range(25.0, 40.0), FeatureKind::Medium, 0.6, 0.3));
        }
        CJ::Esses => {
            let b = deg(rng.range(30.0, 50.0));
            let a = (delta + b) * rng.range(0.4, 0.6);
            let c = delta + b - a;
            let r = rng.range(70.0, 140.0);
            out.push(ccurve(dir, a, r, r, 25.0, FeatureKind::SBend, 0.6, 0.5));
            out.push(link(rng, 10.0, 40.0));
            out.push(ccurve(-dir, b, r * rng.range(0.8, 1.2), r, 25.0, FeatureKind::SBend, 0.6, 0.5));
            out.push(link(rng, 10.0, 40.0));
            out.push(ccurve(dir, c, r * rng.range(0.8, 1.2), r, 25.0, FeatureKind::SBend, 0.6, 0.5));
        }
    }
}

/// Chicane inserted into a straight (net zero heading)
fn circuit_chicane(rng: &mut Rng, out: &mut Vec<Elem>) {
    let sign = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let a = deg(rng.range(35.0, 55.0));
    let r = rng.range(20.0, 30.0);
    out.push(ccurve(sign, a, r, r, 8.0, FeatureKind::Chicane, 0.4, 0.9));
    out.push(Elem::Straight { len: rng.range(0.0, 8.0), main: None, frac: 0.0 });
    out.push(ccurve(-sign, a, r, r, 8.0, FeatureKind::Chicane, 0.4, 0.9));
}

/// Design a closed racing circuit. `size` 0 = Sprint (~2.5 km), 1 = GP (~4.5 km), 2 = Ring
/// (~9-11 km, many corners, built for crests and blind bends).
pub fn design_circuit(rng: &mut Rng, size: u8, difficulty: u8, spacing: f32, max_half_width: f32, shoulder: f32) -> Option<Layout> {
    let dbg = std::env::var("CIRCUIT_DEBUG").is_ok();
    let size = (size as usize).min(3);
    // size 3, the Nordring: ~20 km of forest road that wriggles left AND right (corners may
    // turn against the loop direction), a long flat-out straight, little run-off
    let nord = size == 3;
    let d = (difficulty as usize).min(2);
    let target = [2500.0, 4500.0, 10000.0, 20500.0][size] * rng.range(0.9, 1.1);
    let k_main = match size {
        0 => rng.range_i(6, 9),
        1 => rng.range_i(9, 13),
        2 => rng.range_i(18, 25),
        _ => rng.range_i(34, 42),
    } as usize;
    let dir = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let mut deltas: Vec<f32> = (0..k_main).map(|_| rng.range(0.5, 1.6)).collect();
    if nord {
        // signed turns: about a third against the loop direction; the rest scaled so the lap
        // still turns exactly once around
        deltas = (0..k_main).map(|k| { let m = deg(rng.range(28.0, 105.0)); if k > 0 && rng.chance(0.36) { -m } else { m } }).collect();
        for _ in 0..6 {
            let pos: f32 = deltas.iter().filter(|x| **x > 0.0).sum();
            let neg: f32 = -deltas.iter().filter(|x| **x < 0.0).sum::<f32>();
            let s = (TAU + neg) / pos.max(1e-3);
            for dl in deltas.iter_mut().filter(|x| **x > 0.0) {
                *dl = (*dl * s).clamp(deg(20.0), deg(140.0));
            }
        }
        let pos: f32 = deltas.iter().filter(|x| **x > 0.0).sum();
        let neg: f32 = -deltas.iter().filter(|x| **x < 0.0).sum::<f32>();
        let s = (TAU + neg) / pos.max(1e-3);
        deltas.iter_mut().filter(|x| **x > 0.0).for_each(|dl| *dl *= s);
    } else {
        for _ in 0..4 {
            let sum: f32 = deltas.iter().sum();
            for dl in deltas.iter_mut() {
                *dl = (*dl / sum * TAU).clamp(deg(18.0), deg(125.0));
            }
        }
        let sum: f32 = deltas.iter().sum();
        deltas.iter_mut().for_each(|dl| *dl *= TAU / sum);
    }

    // mandatory corners first, the rest by weight
    let mut kinds = vec![CJ::Medium; k_main];
    let mut order: Vec<usize> = (1..k_main).collect(); // junction 0 ends the pit straight: heavy braking
    for i in (1..order.len()).rev() {
        let j = rng.range_i(0, i as i32 + 1) as usize;
        order.swap(i, j);
    }
    kinds[0] = if rng.chance(0.6) { CJ::HeavyBrake } else { CJ::Hairpin };
    let mandatory: Vec<CJ> = match size {
        0 => vec![CJ::Hairpin, CJ::Esses],
        1 => vec![CJ::Hairpin, CJ::HeavyBrake, CJ::Esses, CJ::DoubleApex, CJ::Sweeper],
        3 => vec![
            CJ::Hairpin, CJ::Hairpin, CJ::Hairpin, CJ::HeavyBrake, CJ::HeavyBrake, CJ::HeavyBrake,
            CJ::Esses, CJ::Esses, CJ::Esses, CJ::Esses, CJ::Esses, CJ::Esses,
            CJ::Carousel, CJ::Carousel, CJ::DoubleApex, CJ::DoubleApex, CJ::DoubleApex,
            CJ::Tightening, CJ::Tightening, CJ::Tightening, CJ::Sweeper, CJ::Sweeper, CJ::Sweeper,
        ],
        _ => vec![CJ::Hairpin, CJ::Hairpin, CJ::HeavyBrake, CJ::HeavyBrake, CJ::Esses, CJ::Esses, CJ::Esses, CJ::Carousel, CJ::DoubleApex, CJ::Sweeper, CJ::Sweeper],
    };
    let mut oi = 0;
    for m in mandatory {
        if oi < order.len() {
            kinds[order[oi]] = m;
            oi += 1;
        }
    }
    let w = [[0.34, 0.26, 0.12, 0.04, 0.1, 0.08, 0.06], [0.26, 0.24, 0.14, 0.07, 0.12, 0.1, 0.07], [0.2, 0.22, 0.15, 0.1, 0.14, 0.1, 0.09]][d];
    for &j in &order[oi..] {
        let r = rng.f32();
        let mut acc = 0.0;
        let opts = [CJ::Sweeper, CJ::Medium, CJ::Esses, CJ::Hairpin, CJ::DoubleApex, CJ::HeavyBrake, CJ::Tightening];
        let mut pick = CJ::Medium;
        for (o, p) in opts.iter().zip(w.iter()) {
            acc += p;
            if r < acc {
                pick = *o;
                break;
            }
        }
        if pick == CJ::Hairpin && size == 0 {
            pick = CJ::HeavyBrake;
        }
        kinds[j] = pick;
    }

    // elements: S_0 J_0 S_1 J_1 ... ; S_0 is the pit straight; one long back straight
    let back = 1 + rng.range_i(0, (k_main - 1) as i32) as usize;
    let mut elems: Vec<Elem> = Vec::new();
    let mut weights = vec![0.0f32; k_main];
    let chicane_p = [0.15, 0.22, 0.25, 0.16][size];
    for k in 0..k_main {
        if k > 0 && rng.chance(chicane_p) {
            let f = rng.range(0.3, 0.7);
            elems.push(Elem::Straight { len: 0.0, main: Some(k), frac: f });
            circuit_chicane(rng, &mut elems);
            elems.push(Elem::Straight { len: 0.0, main: Some(k), frac: 1.0 - f });
        } else {
            elems.push(Elem::Straight { len: 0.0, main: Some(k), frac: 1.0 });
        }
        let mut wk = rng.range(0.4, 1.4);
        if matches!(kinds[k], CJ::Hairpin | CJ::HeavyBrake) {
            wk *= 1.8; // speed before heavy braking
        }
        if k == 0 {
            wk *= 3.0;
        }
        if k == back {
            wk *= 3.0;
        }
        weights[k] = wk;
        // a negative turn is a corner against the loop direction
        let (jdir, jdelta) = if deltas[k] < 0.0 { (-dir, -deltas[k]) } else { (dir, deltas[k]) };
        circuit_junction(rng, kinds[k], jdir, jdelta, &mut elems);
    }
    let min_main = [500.0, 650.0, 700.0, 700.0][size];
    let min_back = [330.0, 550.0, 700.0, 1500.0][size];
    let min_other = [70.0, 90.0, 80.0, 60.0][size];
    let min_len = |k: usize| if k == 0 { min_main } else if k == back { min_back } else { min_other };
    let zero = vec![0.0; k_main];
    let fixed: f32 = elems.iter().map(|e| e.len(&zero)).sum();
    let remaining = (target - fixed).max(k_main as f32 * 120.0);
    let wsum: f32 = weights.iter().sum();
    let mut mains: Vec<f32> = (0..k_main).map(|k| (remaining * weights[k] / wsum).max(min_len(k))).collect();
    mains[0] = mains[0].min(1150.0);

    // close the loop: weighted min-norm change of the straight lengths
    let mut free = vec![true; k_main];
    for _ in 0..8 {
        let it = integrate(&elems, &mains);
        let f = *it.pts.last().unwrap();
        if f.length() < 0.05 {
            break;
        }
        let mut m = [0.0f32; 3];
        for k in 0..k_main {
            if free[k] {
                let u = it.main_dir[k];
                let wk = mains[k];
                m[0] += wk * u.x * u.x;
                m[1] += wk * u.x * u.y;
                m[2] += wk * u.y * u.y;
            }
        }
        let det = m[0] * m[2] - m[1] * m[1];
        if det.abs() < 1e-3 {
            if dbg { eprintln!("reject: singular"); }
            return None;
        }
        let lx = -(m[2] * f.x - m[1] * f.y) / det;
        let ly = -(-m[1] * f.x + m[0] * f.y) / det;
        for k in 0..k_main {
            if free[k] {
                let u = it.main_dir[k];
                let nl = mains[k] + mains[k] * (u.x * lx + u.y * ly);
                if nl < min_len(k) {
                    mains[k] = min_len(k);
                    free[k] = false;
                } else {
                    mains[k] = nl;
                }
            }
        }
    }
    let it = integrate(&elems, &mains);
    if it.pts.last().unwrap().length() > 0.3 || (it.end_heading.abs() - TAU).abs() > 0.02 {
        if dbg { eprintln!("reject: not closed {:.1}", it.pts.last().unwrap().length()); }
        return None;
    }
    let max_straight = if nord { 2400.0 } else { 1400.0 };
    if mains[0] > 1300.0 || mains.iter().any(|&m| m > max_straight) {
        if dbg { eprintln!("reject: long straight {:?}", mains.iter().map(|m| *m as i32).collect::<Vec<_>>()); }
        return None; // no absurd drag strips
    }
    let total = it.total;
    if total < target * 0.75 || total > target * 1.4 {
        if dbg { eprintln!("reject: length {total:.0} vs {target:.0}"); }
        return None;
    }
    let mut bmin = Vec2::splat(f32::MAX);
    let mut bmax = Vec2::splat(f32::MIN);
    for p in &it.pts {
        bmin = bmin.min(*p);
        bmax = bmax.max(*p);
    }
    if (bmax - bmin).max_element() > total * 0.45 + 200.0 {
        if dbg { eprintln!("reject: bbox"); }
        return None;
    }
    let mut fine = it.pts.clone();
    let resid = *fine.last().unwrap();
    let nf = fine.len() as f32;
    for (i, p) in fine.iter_mut().enumerate() {
        *p -= resid * (i as f32 / (nf - 1.0));
    }
    let count = (total / spacing).round() as usize;
    let pts0 = resample_loop(&fine, count);
    let scale = count as f32 * spacing / total;
    // start/finish line 65% along the pit straight (room for the grid behind it)
    let start_s = (it.starts[0] + mains[0] * 0.65) * scale;
    let shift = (start_s / spacing).round() as usize % count;
    let mut pts = pts0;
    pts.rotate_left(shift);
    let shift_s = shift as f32 * spacing;
    let len_final = count as f32 * spacing;
    // sections (incl. a share of the run-off) must not overlap
    let near = 2.0 * (max_half_width + shoulder) + 8.0;
    if !clearance_ok(&pts, spacing, near, near + 22.0) {
        if dbg { eprintln!("reject: clearance"); }
        return None;
    }
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
                    features.push(Feature { kind: FeatureKind::Straight, s0, s1, main, jump_kink: false, camber: 1.0 });
                }
            }
            Elem::Curve { kind, camber: cam, tech: t, .. } => {
                features.push(Feature { kind, s0, s1, main: None, jump_kink: false, camber: cam });
                let i0 = (s0 / spacing).floor() as isize;
                let i1 = i0 + (l / spacing).ceil() as isize;
                for i in i0 - 8..=i1 + 8 {
                    let j = i.rem_euclid(count as isize) as usize;
                    let edge = if i < i0 { (i0 - i) as f32 / 8.0 } else if i > i1 { (i - i1) as f32 / 8.0 } else { 0.0 };
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
