//! Road surface layout: long surface *sections* (tarmac / gravel / snow...) separated by smooth
//! transitions, plus organic *patches* (ice, mud, sand drifts) with feathered edges.
//!
//! Everything is continuous: a point on the road is a blend of at most two surfaces with a
//! smoothstep weight, so grip never steps. `Track::ground_at` turns this into blended
//! `SurfaceProps` for physics; renderers can use `Track::surface_mix_at` + `road_color`.
use crate::biome::Biome;
use crate::layout::{Feature, FeatureKind};
use crate::rng::Rng;
use crate::surface::Surface;
use serde::{Deserialize, Serialize};

/// Which surfaces a track uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SurfaceMode {
    /// The biome's own road surface (+ biome patches / coastal gravel sections)
    #[default]
    BiomeDefault,
    /// Gravel road everywhere (biome patches only at higher difficulty)
    Gravel,
    /// Tarmac everywhere (biome patches only at higher difficulty)
    Tarmac,
    /// Snow road everywhere (ice patches only at higher difficulty)
    Snow,
    /// Real-rally mixed stage: long alternating tarmac / loose sections + biome patches
    Mixed,
}

pub const SURFACE_MODES: [SurfaceMode; 5] = [SurfaceMode::BiomeDefault, SurfaceMode::Gravel, SurfaceMode::Tarmac, SurfaceMode::Snow, SurfaceMode::Mixed];

impl SurfaceMode {
    pub fn name(self) -> &'static str {
        match self {
            SurfaceMode::BiomeDefault => "Biome",
            SurfaceMode::Gravel => "Gravel",
            SurfaceMode::Tarmac => "Tarmac",
            SurfaceMode::Snow => "Snow",
            SurfaceMode::Mixed => "Mixed",
        }
    }
}

/// Smooth change from one section surface to the next, starting at `s` and lasting `len` m.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceTransition {
    pub s: f32,
    pub len: f32,
    pub from: Surface,
    pub to: Surface,
    /// Wobble of the boundary across the road (organic edge), metres and phase
    pub wobble: f32,
    pub phase: f32,
}

/// Organic blob of another surface lying on the road.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfacePatch {
    pub surface: Surface,
    /// Centre arc length
    pub s: f32,
    pub half_len: f32,
    /// Centre lateral offset (m, + = right)
    pub lat: f32,
    pub half_width: f32,
    /// Fraction of the radius used for the soft edge (0.15 sharp .. 0.5 very soft)
    pub feather: f32,
    pub phase: [f32; 2],
}

impl SurfacePatch {
    /// Coverage 0..1 at (s, lateral). `len` = track length for wrapping.
    pub fn coverage(&self, s: f32, lat: f32, len: f32) -> f32 {
        let mut ds = (s - self.s).rem_euclid(len);
        if ds > len * 0.5 {
            ds -= len;
        }
        if ds.abs() > self.half_len * 1.35 {
            return 0.0;
        }
        let u = ds / self.half_len;
        let v = (lat - self.lat) / self.half_width;
        let r = (u * u + v * v).sqrt();
        let th = v.atan2(u);
        let edge = 1.0 + 0.16 * (3.0 * th + self.phase[0]).sin() + 0.09 * (5.0 * th + self.phase[1]).sin();
        let d = r / edge;
        1.0 - smoothstep(1.0 - self.feather, 1.0, d)
    }

    pub fn s_start(&self, len: f32) -> f32 {
        (self.s - self.half_len).rem_euclid(len)
    }
}

/// Up to two surfaces and the weight of the second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceMix {
    pub a: Surface,
    pub b: Surface,
    /// Weight of `b` (0..1)
    pub t: f32,
}

impl SurfaceMix {
    pub fn single(s: Surface) -> Self {
        SurfaceMix { a: s, b: s, t: 0.0 }
    }

    /// (dominant, other, weight of other in 0..=0.5)
    pub fn dominant(&self) -> (Surface, Surface, f32) {
        if self.t > 0.5 { (self.b, self.a, 1.0 - self.t) } else { (self.a, self.b, self.t) }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMap {
    /// Surface when there are no transitions
    pub base: Surface,
    /// Sorted by `s`
    pub transitions: Vec<SurfaceTransition>,
    pub patches: Vec<SurfacePatch>,
    pub track_length: f32,
}

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl SurfaceMap {
    pub fn uniform(s: Surface, len: f32) -> Self {
        SurfaceMap { base: s, transitions: Vec::new(), patches: Vec::new(), track_length: len }
    }

    /// Section surface (ignoring patches) at (s, lateral)
    pub fn section_mix(&self, s: f32, lat: f32) -> SurfaceMix {
        let len = self.track_length;
        if self.transitions.is_empty() {
            return SurfaceMix::single(self.base);
        }
        for tr in &self.transitions {
            let pad = tr.wobble.abs() + 0.01;
            let rel = (s - (tr.s - pad)).rem_euclid(len);
            if rel <= tr.len + 2.0 * pad {
                let x = rel - pad + tr.wobble * (lat * 0.85 + tr.phase).sin();
                let t = smoothstep(0.0, tr.len, x);
                return SurfaceMix { a: tr.from, b: tr.to, t };
            }
        }
        // between transitions: the surface after the most recent one
        let last = self.transitions.iter().min_by(|a, b| (s - a.s).rem_euclid(len).partial_cmp(&(s - b.s).rem_euclid(len)).unwrap()).unwrap();
        SurfaceMix::single(last.to)
    }

    /// Full surface mix at (s, lateral) including patches.
    pub fn mix_at(&self, s: f32, lat: f32) -> SurfaceMix {
        let m = self.section_mix(s, lat);
        if m.t > 0.0 && m.t < 1.0 {
            return m;
        }
        let base = if m.t >= 1.0 { m.b } else { m.a };
        let mut best = SurfaceMix::single(base);
        for p in &self.patches {
            let c = p.coverage(s, lat, self.track_length);
            if c > best.t {
                best = SurfaceMix { a: base, b: p.surface, t: c };
            }
        }
        best
    }
}

/// Visual road colour for a surface in a biome (the biome's own road surface uses its palette).
pub fn road_color(biome: Biome, s: Surface) -> [f32; 3] {
    let st = biome.style();
    if s == st.road_surface {
        return st.road;
    }
    match s {
        Surface::Asphalt => [0.40, 0.40, 0.44],
        Surface::Gravel => [0.74, 0.66, 0.52],
        Surface::Dirt => [0.58, 0.43, 0.30],
        Surface::Grass => [0.50, 0.68, 0.35],
        Surface::Sand => [0.93, 0.80, 0.58],
        Surface::Snow => [0.90, 0.92, 0.96],
        Surface::Ice => [0.70, 0.86, 0.97],
        Surface::Mud => [0.40, 0.29, 0.20],
    }
}

/// Shoulder surface next to a road surface
pub fn shoulder_of(s: Surface) -> Surface {
    match s {
        Surface::Asphalt => Surface::Gravel,
        Surface::Ice => Surface::Snow,
        other => other,
    }
}

fn is_loose(s: Surface) -> bool {
    !matches!(s, Surface::Asphalt | Surface::Ice)
}

fn arc_dist(a: f32, b: f32, len: f32) -> f32 {
    let d = (a - b).rem_euclid(len);
    d.min(len - d)
}

pub struct SurfacePlan {
    pub map: SurfaceMap,
    /// Surface the AI / audio should assume as the "main" road surface
    pub main: Surface,
}

/// Plan the surfaces of a track. `hw` = per-sample half widths.
#[allow(clippy::too_many_arguments)]
pub fn plan(biome: Biome, mode: SurfaceMode, difficulty: u8, features: &[Feature], jumps: &[usize], hw: &[f32], spacing: f32, rng: &mut Rng) -> SurfacePlan {
    let d = (difficulty as usize).min(2);
    let n = hw.len();
    let len = n as f32 * spacing;
    let biome_road = biome.style().road_surface;
    let jump_s: Vec<f32> = jumps.iter().map(|&j| j as f32 * spacing).collect();

    // ---- section surfaces
    let (main, alt, n_changes): (Surface, Option<Surface>, usize) = match mode {
        SurfaceMode::BiomeDefault => {
            if biome == Biome::Coast {
                // tarmac with gravel sections
                (Surface::Asphalt, Some(Surface::Gravel), [2, 2 + 2 * rng.chance(0.5) as usize, 4][d])
            } else {
                (biome_road, None, 0)
            }
        }
        SurfaceMode::Gravel => (Surface::Gravel, None, 0),
        SurfaceMode::Tarmac => (Surface::Asphalt, None, 0),
        SurfaceMode::Snow => (Surface::Snow, None, 0),
        SurfaceMode::Mixed => {
            let loose = match biome {
                Biome::Meadow | Biome::Coast => Surface::Gravel,
                Biome::Autumn | Biome::Desert => Surface::Dirt,
                Biome::Snow => Surface::Snow,
            };
            let changes = if len > 2200.0 { 6 } else { 4 };
            (Surface::Asphalt, Some(loose), changes)
        }
    };

    // candidate boundary intervals: on straights, clear of the start and jumps
    let mut intervals: Vec<(f32, f32)> = Vec::new();
    for f in features.iter().filter(|f| f.kind == FeatureKind::Straight) {
        let fl = f.len(len);
        if fl < 45.0 {
            continue;
        }
        intervals.push((f.s0 + 8.0, f.s0 + fl - 30.0));
    }
    let ok_at = |s: f32| {
        let s = s.rem_euclid(len);
        s > 70.0 && s < len - 90.0 && jump_s.iter().all(|&j| arc_dist(j, s + 8.0, len) > 40.0)
    };
    let mut transitions: Vec<SurfaceTransition> = Vec::new();
    if let Some(alt) = alt {
        let mut count = n_changes;
        while count >= 2 && transitions.is_empty() {
            let off = rng.range(0.0, len / count as f32);
            let mut cuts: Vec<f32> = Vec::new();
            for i in 0..count {
                let ideal = off + i as f32 * len / count as f32;
                // nearest admissible point
                let mut best: Option<(f32, f32)> = None;
                for &(a, b) in &intervals {
                    let lo = a;
                    let hi = b.max(a);
                    let steps = (((hi - lo) / 4.0).ceil() as usize).max(1);
                    for k in 0..=steps {
                        let s = (lo + (hi - lo) * k as f32 / steps as f32).rem_euclid(len);
                        if !ok_at(s) {
                            continue;
                        }
                        let dd = arc_dist(s, ideal.rem_euclid(len), len);
                        if best.is_none_or(|b| dd < b.1) {
                            best = Some((s, dd));
                        }
                    }
                }
                if let Some((s, dd)) = best {
                    if dd < len / count as f32 * 0.45 && cuts.iter().all(|&c| arc_dist(c, s, len) > (len * 0.1).max(110.0)) {
                        cuts.push(s);
                    }
                }
            }
            if cuts.len() >= count {
                cuts.sort_by(|a, b| a.partial_cmp(b).unwrap());
                // the start line (s = 0) lies in the section before the first cut: make it `main`
                for (i, &c) in cuts.iter().enumerate() {
                    let (from, to) = if i % 2 == 0 { (main, alt) } else { (alt, main) };
                    let tlen = if is_loose(from) && !is_loose(to) {
                        rng.range(14.0, 20.0) // loose material dragged onto the tarmac
                    } else {
                        rng.range(8.0, 12.0)
                    };
                    transitions.push(SurfaceTransition { s: c, len: tlen, from, to, wobble: rng.range(1.0, 2.2), phase: rng.range(0.0, 6.28) });
                }
            } else {
                count -= 2;
            }
        }
    }
    let section_base = main;
    let in_transition = |s: f32, pad: f32| transitions.iter().any(|t| arc_dist(s, t.s + t.len * 0.5, len) < t.len * 0.5 + pad);

    // ---- patches
    let patch_surface = match biome {
        Biome::Snow => Surface::Ice,
        Biome::Autumn | Biome::Meadow => Surface::Mud,
        Biome::Desert | Biome::Coast => Surface::Sand,
    };
    let full = [match biome {
        Biome::Snow | Biome::Autumn => [1, 2, 3],
        Biome::Desert => [1, 3, 4],
        Biome::Meadow => [0, 1, 2],
        Biome::Coast => [0, 0, 1],
    }][0];
    let mut count = match mode {
        SurfaceMode::BiomeDefault | SurfaceMode::Mixed => full[d],
        // forced single surface: patches only at higher difficulty
        _ => [0, full[1].min(1), full[2]][d],
    };
    if mode == SurfaceMode::Mixed {
        count = count.max(1);
    }
    count += (len > 2200.0 && count > 0) as usize;
    let (lmin, lmax, feather) = match patch_surface {
        Surface::Ice => (7.0, 14.0, 0.22),
        Surface::Sand => (5.0, 11.0, 0.45),
        _ => (7.0, 15.0, 0.38),
    };
    let corner_p = match patch_surface {
        Surface::Ice => [0.0, 0.0, 0.3],
        Surface::Mud => [0.0, 0.3, 0.4],
        _ => [0.2, 0.4, 0.5],
    };
    let straights: Vec<&Feature> = features.iter().filter(|f| f.kind == FeatureKind::Straight && f.len(len) > 60.0).collect();
    let corners: Vec<&Feature> = features.iter().filter(|f| matches!(f.kind, FeatureKind::Medium | FeatureKind::Sweeper | FeatureKind::Opening | FeatureKind::Kink)).collect();
    let mut patches: Vec<SurfacePatch> = Vec::new();
    let mut tries = 0;
    while patches.len() < count && tries < 100 {
        tries += 1;
        let half_len = rng.range(lmin, lmax);
        let l = half_len * 2.0;
        let s0 = if rng.chance(corner_p[d]) && !corners.is_empty() {
            let f = corners[rng.range_i(0, corners.len() as i32) as usize];
            if patch_surface == Surface::Ice {
                f.s0 - l - rng.range(4.0, 12.0) // icy braking zone
            } else {
                f.s0 + rng.range(-6.0, f.len(len) * 0.5)
            }
        } else if !straights.is_empty() {
            let f = straights[rng.range_i(0, straights.len() as i32) as usize];
            f.s0 + rng.range(15.0, (f.len(len) - 15.0 - l).max(16.0))
        } else {
            break;
        };
        let sc = (s0 + half_len).rem_euclid(len);
        let near_start = sc.min(len - sc) < 60.0 + half_len;
        let near_jump = jump_s.iter().any(|&j| arc_dist(j, sc, len) < half_len + 45.0);
        let near_patch = patches.iter().any(|p| arc_dist(p.s, sc, len) < p.half_len + half_len + 30.0);
        if near_start || near_jump || near_patch || in_transition(sc, half_len + 12.0) {
            continue;
        }
        let i = ((sc / spacing) as usize).min(n - 1);
        let w = hw[i];
        // patches either cover the road or lie on one side (sand drifts always cross it)
        let (lat, half_width) = if patch_surface == Surface::Sand || rng.chance(0.55) {
            (rng.range(-0.3, 0.3) * w, w * rng.range(1.25, 1.6))
        } else {
            let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
            (side * w * rng.range(0.35, 0.6), w * rng.range(0.6, 0.85))
        };
        patches.push(SurfacePatch { surface: patch_surface, s: sc, half_len, lat, half_width, feather, phase: [rng.range(0.0, 6.28), rng.range(0.0, 6.28)] });
    }
    patches.sort_by(|a, b| a.s.partial_cmp(&b.s).unwrap());

    // the surface AI/audio assume: the slipperiest section surface (conservative)
    let ai_main = match alt {
        Some(a) if a.props().grip < section_base.props().grip => a,
        _ => section_base,
    };
    SurfacePlan { map: SurfaceMap { base: section_base, transitions, patches, track_length: len }, main: ai_main }
}
