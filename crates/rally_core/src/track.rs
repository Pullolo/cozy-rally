//! Procedural closed-loop rally track generation.
//!
//! Pipeline: random control loop -> Catmull-Rom spline -> arc-length resample -> smoothing
//! until the minimum corner radius is drivable -> self-clearance check -> elevation profile
//! (smoothed terrain + crests + jumps, slope-limited) -> banking -> heightfield that is
//! flattened under the road and blended into natural hills -> props.
use crate::biome::{Biome, BiomeStyle};
use crate::noise;
use crate::rng::Rng;
use crate::surface::Surface;
use crate::terrain::Heightfield;
use glam::{Mat3, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub const SAMPLE_SPACING: f32 = 2.0;
pub const SHOULDER: f32 = 2.5;
pub const BLEND: f32 = 26.0;
pub const TERRAIN_CELL: f32 = 2.5;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TrackParams {
    pub seed: u64,
    pub biome: Option<Biome>,
    /// 0 = short (~1.4km), 1 = medium (~2km), 2 = long (~2.8km)
    pub length_class: u8,
}

impl Default for TrackParams {
    fn default() -> Self {
        TrackParams { seed: 1, biome: None, length_class: 1 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TrackSample {
    pub pos: Vec3,
    /// Unit 3D direction of travel
    pub tangent: Vec3,
    /// Unit vector pointing to the right of travel, tilted by banking
    pub right: Vec3,
    /// Road surface normal
    pub up: Vec3,
    pub s: f32,
    pub half_width: f32,
    /// Bank angle (rad), positive = right side higher (left turn)
    pub bank: f32,
    /// Signed curvature (1/m), positive = left turn
    pub curvature: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropKind {
    Tree,
    Pine,
    Bush,
    Rock,
    Cactus,
    Palm,
    Flowers,
    Spectator,
    HayBale,
    Tent,
    Cone,
}

#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub kind: PropKind,
    pub pos: Vec3,
    pub rot_y: f32,
    pub scale: f32,
    /// Collision cylinder radius in XZ (0 = no collision)
    pub radius: f32,
    /// Variation index for colours etc.
    pub variant: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TrackQuery {
    /// Index of the sample at the start of the closest segment
    pub index: usize,
    /// Arc length position along the loop [0, length)
    pub s: f32,
    /// Signed lateral offset (positive = right)
    pub lateral: f32,
    /// Distance in XZ to the centre line
    pub dist: f32,
}

struct SpatialGrid {
    origin: Vec2,
    cell: f32,
    nx: i32,
    nz: i32,
    cells: Vec<Vec<u32>>,
}

impl SpatialGrid {
    fn new(points: &[Vec2], cell: f32, margin: f32) -> Self {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for p in points {
            min = min.min(*p);
            max = max.max(*p);
        }
        min -= Vec2::splat(margin);
        max += Vec2::splat(margin);
        let nx = ((max.x - min.x) / cell).ceil() as i32 + 1;
        let nz = ((max.y - min.y) / cell).ceil() as i32 + 1;
        let mut cells = vec![Vec::new(); (nx * nz) as usize];
        for (i, p) in points.iter().enumerate() {
            let cx = ((p.x - min.x) / cell) as i32;
            let cz = ((p.y - min.y) / cell) as i32;
            cells[(cz * nx + cx) as usize].push(i as u32);
        }
        SpatialGrid { origin: min, cell, nx, nz, cells }
    }

    fn for_each_near(&self, p: Vec2, radius: f32, mut f: impl FnMut(u32)) {
        let r = (radius / self.cell).ceil() as i32;
        let cx = ((p.x - self.origin.x) / self.cell).floor() as i32;
        let cz = ((p.y - self.origin.y) / self.cell).floor() as i32;
        for z in (cz - r).max(0)..=(cz + r).min(self.nz - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.nx - 1) {
                for &i in &self.cells[(z * self.nx + x) as usize] {
                    f(i);
                }
            }
        }
    }
}

pub struct Track {
    pub params: TrackParams,
    pub biome: Biome,
    pub style: BiomeStyle,
    pub samples: Vec<TrackSample>,
    pub length: f32,
    pub terrain: Heightfield,
    pub props: Vec<Prop>,
    /// Sample indices of checkpoints; checkpoint 0 is the start/finish line (index 0)
    pub checkpoints: Vec<usize>,
    pub water_level: Option<f32>,
    /// Sample indices where jumps take off (for UI/FX hints)
    pub jumps: Vec<usize>,
    grid: SpatialGrid,
    prop_grid: SpatialGrid,
}

fn wrap(i: isize, n: usize) -> usize {
    i.rem_euclid(n as isize) as usize
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn catmull_rom(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// Resample a closed polyline to (approximately) uniform spacing.
fn resample_closed(pts: &[Vec2], spacing: f32) -> Vec<Vec2> {
    let n = pts.len();
    let mut total = 0.0;
    for i in 0..n {
        total += pts[i].distance(pts[(i + 1) % n]);
    }
    let count = (total / spacing).round().max(8.0) as usize;
    let step = total / count as f32;
    let mut out = Vec::with_capacity(count);
    let mut seg = 0;
    let mut seg_start = 0.0;
    let mut seg_len = pts[0].distance(pts[1 % n]);
    for k in 0..count {
        let target = k as f32 * step;
        while seg_start + seg_len < target && seg < n * 2 {
            seg_start += seg_len;
            seg += 1;
            seg_len = pts[seg % n].distance(pts[(seg + 1) % n]);
        }
        let t = if seg_len > 1e-6 { (target - seg_start) / seg_len } else { 0.0 };
        out.push(pts[seg % n].lerp(pts[(seg + 1) % n], t.clamp(0.0, 1.0)));
    }
    out
}

fn signed_curvature_2d(pts: &[Vec2]) -> Vec<f32> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let a = pts[wrap(i as isize - 1, n)];
            let b = pts[i];
            let c = pts[(i + 1) % n];
            let t0 = (b - a).normalize_or_zero();
            let t1 = (c - b).normalize_or_zero();
            // In XZ with Y up: cross(t0,t1).y = t0.z*t1.x - t0.x*t1.z ; positive = left turn
            let cross_y = t0.y * t1.x - t0.x * t1.y;
            let ang = cross_y.atan2(t0.dot(t1));
            let ds = 0.5 * (a.distance(b) + b.distance(c));
            ang / ds.max(1e-3)
        })
        .collect()
}

fn smooth_closed_f32(v: &mut [f32], radius: usize, passes: usize) {
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

fn natural_height(style: &BiomeStyle, biome: Biome, seed: u32, x: f32, z: f32) -> f32 {
    let f = style.hill_freq;
    let base = noise::fbm(x * f, z * f, 4, seed) * style.hill_amp;
    let detail = noise::fbm(x * f * 6.0, z * f * 6.0, 2, seed + 77) * style.hill_amp * 0.06;
    match biome {
        Biome::Snow => {
            let m = noise::ridged(x * f * 0.7, z * f * 0.7, 4, seed + 5) * style.hill_amp * 1.4;
            base * 0.5 + m + detail
        }
        Biome::Desert => {
            // mesas: terrace the hills
            let h = base + style.hill_amp * 0.3;
            let step = 9.0;
            let terr = (h / step).floor() * step + smoothstep(0.75, 1.0, (h / step).fract()) * step;
            terr * 0.8 + h * 0.2 + detail
        }
        Biome::Coast => {
            // land slopes down toward the sea on one side
            base + (x * 0.6 + z * 0.4) * 0.05 + detail
        }
        _ => base + detail,
    }
}

impl Track {
    pub fn generate(params: &TrackParams) -> Track {
        let mut attempt = 0u64;
        loop {
            if let Some(t) = Self::try_generate(params, attempt) {
                return t;
            }
            attempt += 1;
            assert!(attempt < 200, "track generation failed");
        }
    }

    fn try_generate(params: &TrackParams, attempt: u64) -> Option<Track> {
        let mut rng = Rng::new(params.seed.wrapping_mul(31).wrapping_add(attempt * 7919));
        let biome = params.biome.unwrap_or_else(|| Biome::from_index((Rng::new(params.seed).next_u64() % 5) as usize));
        let style = biome.style();
        let noise_seed = (params.seed as u32).wrapping_mul(2654435761);

        // ---- 1. control loop
        let base_r = match params.length_class {
            0 => 150.0,
            1 => 215.0,
            _ => 290.0,
        } * rng.range(0.9, 1.1);
        let n_ctrl = rng.range_i(9, 15) as usize;
        let mut ctrl: Vec<Vec2> = Vec::with_capacity(n_ctrl);
        for i in 0..n_ctrl {
            let ang = std::f32::consts::TAU * (i as f32 + rng.range(-0.3, 0.3)) / n_ctrl as f32;
            let mut r = base_r * rng.range(0.6, 1.25);
            if rng.chance(0.18) {
                r *= 0.55; // pinch -> hairpin-ish
            }
            ctrl.push(Vec2::new(ang.cos(), ang.sin()) * r);
        }
        // ---- 2. spline
        let mut dense = Vec::new();
        for i in 0..n_ctrl {
            let p0 = ctrl[wrap(i as isize - 1, n_ctrl)];
            let p1 = ctrl[i];
            let p2 = ctrl[(i + 1) % n_ctrl];
            let p3 = ctrl[(i + 2) % n_ctrl];
            for k in 0..24 {
                dense.push(catmull_rom(p0, p1, p2, p3, k as f32 / 24.0));
            }
        }
        let mut pts = resample_closed(&dense, SAMPLE_SPACING);
        // ---- 3. smooth until min radius is ok
        let min_radius = 16.0;
        for _ in 0..60 {
            let curv = signed_curvature_2d(&pts);
            let worst = curv.iter().fold(0.0f32, |m, c| m.max(c.abs()));
            if worst < 1.0 / min_radius {
                break;
            }
            let n = pts.len();
            let mut next = pts.clone();
            for i in 0..n {
                if curv[i].abs() > 0.7 / min_radius {
                    for k in -3isize..=3 {
                        let j = wrap(i as isize + k, n);
                        let avg = (pts[wrap(j as isize - 1, n)] + pts[(j + 1) % n]) * 0.5;
                        next[j] = pts[j].lerp(avg, 0.5);
                    }
                }
            }
            pts = resample_closed(&next, SAMPLE_SPACING);
        }
        // general light smoothing for flow
        for _ in 0..4 {
            let n = pts.len();
            let prev = pts.clone();
            for i in 0..n {
                let avg = (prev[wrap(i as isize - 1, n)] + prev[(i + 1) % n]) * 0.5;
                pts[i] = prev[i].lerp(avg, 0.5);
            }
        }
        pts = resample_closed(&pts, SAMPLE_SPACING);
        let n = pts.len();
        let curv = signed_curvature_2d(&pts);
        if curv.iter().any(|c| c.abs() > 1.0 / 13.0) {
            return None;
        }
        let length = n as f32 * SAMPLE_SPACING;
        if length < 700.0 {
            return None;
        }

        // ---- 4. clearance check (track must not come near itself)
        let half_w_base = 5.6;
        let clearance = 2.0 * (half_w_base + SHOULDER) + 12.0;
        let grid = SpatialGrid::new(&pts, 16.0, 60.0);
        for i in (0..n).step_by(2) {
            let mut bad = false;
            grid.for_each_near(pts[i], clearance, |j| {
                let j = j as usize;
                let di = (i as isize - j as isize).unsigned_abs();
                let di = di.min(n - di);
                if di as f32 * SAMPLE_SPACING > clearance * 2.2 && pts[i].distance(pts[j]) < clearance {
                    bad = true;
                }
            });
            if bad {
                return None;
            }
        }

        // ---- 5. choose start: middle of the straightest stretch
        let mut best = (f32::MAX, 0usize);
        let win = 30isize;
        for i in 0..n {
            let mut sum = 0.0;
            for k in -win..=win {
                sum += curv[wrap(i as isize + k, n)].abs();
            }
            if sum < best.0 {
                best = (sum, i);
            }
        }
        pts.rotate_left(best.1);
        let grid = SpatialGrid::new(&pts, 16.0, 60.0);
        let mut curv = signed_curvature_2d(&pts);
        smooth_closed_f32(&mut curv, 3, 2);

        // ---- 6. elevation
        let mut elev: Vec<f32> = pts.iter().map(|p| natural_height(&style, biome, noise_seed, p.x, p.y)).collect();
        smooth_closed_f32(&mut elev, 12, 3);
        // slope limit
        let max_slope = 0.11 * SAMPLE_SPACING;
        for _ in 0..4 {
            for i in 0..n {
                let prev = elev[wrap(i as isize - 1, n)];
                elev[i] = elev[i].clamp(prev - max_slope, prev + max_slope);
            }
            for i in (0..n).rev() {
                let next = elev[(i + 1) % n];
                elev[i] = elev[i].clamp(next - max_slope, next + max_slope);
            }
        }
        smooth_closed_f32(&mut elev, 3, 2);
        // flatten the start area
        let start_flat = 20;
        let e0 = elev[0];
        for k in -(start_flat as isize)..=(start_flat as isize) {
            let i = wrap(k, n);
            let t = smoothstep(start_flat as f32, start_flat as f32 * 0.5, k.unsigned_abs() as f32);
            elev[i] = elev[i] + (e0 - elev[i]) * t;
        }
        // jumps on straight-ish sections away from start
        let mut jumps = Vec::new();
        let jump_count = rng.range_i(1, 3);
        let mut tries = 0;
        while (jumps.len() as i32) < jump_count && tries < 200 {
            tries += 1;
            let i = rng.range_i(60, n as i32 - 60) as usize;
            let straight = (-25isize..=25).all(|k| curv[wrap(i as isize + k, n)].abs() < 1.0 / 90.0);
            let far = jumps.iter().all(|&j: &usize| (j as isize - i as isize).unsigned_abs() > 80);
            if straight && far {
                jumps.push(i);
            }
        }
        for &j in &jumps {
            // ramp up over 16m then drop over ~5m
            let up = 8isize;
            let down = 3isize;
            let h = rng.range(1.3, 1.9);
            for k in -up..=down {
                let i = wrap(j as isize + k, n);
                let f = if k <= 0 {
                    let t = (k + up) as f32 / up as f32;
                    t * t * (3.0 - 2.0 * t)
                } else {
                    1.0 - (k as f32 / down as f32)
                };
                elev[i] += h * f;
            }
        }

        // ---- 7. samples (tangent / banking)
        let mut bank: Vec<f32> = curv.iter().map(|c| (c * 22.0).clamp(-0.13, 0.13)).collect();
        smooth_closed_f32(&mut bank, 6, 2);
        let mut samples = Vec::with_capacity(n);
        let mut width_noise: Vec<f32> = (0..n).map(|i| noise::value2(i as f32 * 0.02, 3.3, noise_seed) * 0.6).collect();
        smooth_closed_f32(&mut width_noise, 4, 1);
        for i in 0..n {
            let p = Vec3::new(pts[i].x, elev[i], pts[i].y);
            let pn = Vec3::new(pts[(i + 1) % n].x, elev[(i + 1) % n], pts[(i + 1) % n].y);
            let pp = {
                let j = wrap(i as isize - 1, n);
                Vec3::new(pts[j].x, elev[j], pts[j].y)
            };
            let tangent = (pn - pp).normalize();
            let flat_right = tangent.cross(Vec3::Y).normalize();
            // tilt the right vector: positive bank lifts the right side
            let right = (flat_right * bank[i].cos() + Vec3::Y * bank[i].sin()).normalize();
            let up = right.cross(tangent).normalize();
            let start_wide = smoothstep(40.0, 10.0, (i.min(n - i)) as f32) * 1.2;
            samples.push(TrackSample {
                pos: p,
                tangent,
                right,
                up,
                s: i as f32 * SAMPLE_SPACING,
                half_width: half_w_base + width_noise[i] + start_wide,
                bank: bank[i],
                curvature: curv[i],
            });
        }

        // ---- 8. terrain heightfield
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for p in &pts {
            min = min.min(*p);
            max = max.max(*p);
        }
        let margin = 170.0;
        min -= Vec2::splat(margin);
        max += Vec2::splat(margin);
        let nx = ((max.x - min.x) / TERRAIN_CELL).ceil() as usize + 1;
        let nz = ((max.y - min.y) / TERRAIN_CELL).ceil() as usize + 1;
        let mut hf = Heightfield::new(min, TERRAIN_CELL, nx, nz);
        let road_zone = BLEND + half_w_base + 1.5 + SHOULDER;
        let mut candidates: Vec<(usize, f32)> = Vec::with_capacity(64);
        for iz in 0..nz {
            for ix in 0..nx {
                let x = min.x + ix as f32 * TERRAIN_CELL;
                let z = min.y + iz as f32 * TERRAIN_CELL;
                let nat = natural_height(&style, biome, noise_seed, x, z);
                let q = Vec2::new(x, z);
                candidates.clear();
                grid.for_each_near(q, road_zone, |j| {
                    let d = pts[j as usize].distance_squared(q);
                    if d < road_zone * road_zone {
                        candidates.push((j as usize, d));
                    }
                });
                let mut h = nat;
                if !candidates.is_empty() {
                    // keep the closest sample per track section
                    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                    let mut sections: Vec<usize> = Vec::new();
                    for &(j, _) in candidates.iter() {
                        let near_existing = sections.iter().any(|&k| {
                            let d = (k as isize - j as isize).unsigned_abs();
                            d.min(n - d) < 40
                        });
                        if !near_existing {
                            sections.push(j);
                        }
                    }
                    let mut wsum = 1.0;
                    let mut hsum = nat;
                    let mut exact = None;
                    for &j in &sections {
                        let (lat, along_h, hw) = road_profile_at(&samples, j, q);
                        let core = hw + SHOULDER;
                        let d = lat.abs();
                        let t = smoothstep(core, core + BLEND, d);
                        if t <= 1e-4 {
                            exact = Some(along_h);
                            break;
                        }
                        let w = ((1.0 - t) / t).powi(2);
                        wsum += w;
                        hsum += w * along_h;
                    }
                    h = exact.unwrap_or(hsum / wsum);
                }
                let id = hf.idx(ix, iz);
                hf.heights[id] = h;
            }
        }

        // water
        let water_level = if style.has_water {
            let min_road = elev.iter().cloned().fold(f32::MAX, f32::min);
            Some(min_road - 2.2)
        } else {
            None
        };

        let mut checkpoints = Vec::new();
        let n_cp = 8;
        for k in 0..n_cp {
            checkpoints.push(k * n / n_cp);
        }

        let mut track = Track {
            params: params.clone(),
            biome,
            style,
            samples,
            length,
            terrain: hf,
            props: Vec::new(),
            checkpoints,
            water_level,
            jumps,
            grid,
            prop_grid: SpatialGrid::new(&[Vec2::ZERO], 32.0, 1.0),
        };
        track.place_props(&mut rng, noise_seed);
        Some(track)
    }

    fn place_props(&mut self, rng: &mut Rng, noise_seed: u32) {
        let style = &self.style;
        let biome = self.biome;
        let mut props = Vec::new();
        let hf = &self.terrain;
        let size = hf.size();
        let spacing = 6.5;
        let gx = (size.x / spacing) as i32;
        let gz = (size.y / spacing) as i32;
        for iz in 0..gz {
            for ix in 0..gx {
                let x = hf.origin.x + (ix as f32 + rng.range(0.1, 0.9)) * spacing;
                let z = hf.origin.y + (iz as f32 + rng.range(0.1, 0.9)) * spacing;
                let forest = noise::fbm(x * 0.012, z * 0.012, 3, noise_seed + 999) * 0.5 + 0.5;
                let density = style.tree_density * smoothstep(0.35, 0.75, forest) + 0.04;
                if !rng.chance(density) {
                    continue;
                }
                let q = self.query(Vec3::new(x, 0.0, z), None);
                let hw = self.samples[q.index].half_width;
                if q.dist < hw + SHOULDER + 3.5 {
                    continue;
                }
                let (y, nrm) = hf.sample(x, z);
                if let Some(w) = self.water_level {
                    if y < w + 0.4 {
                        continue;
                    }
                }
                let steep = nrm.y < 0.8;
                let r = rng.f32();
                let kind = match biome {
                    Biome::Meadow => {
                        if steep || r < 0.12 { PropKind::Rock } else if r < 0.55 { PropKind::Tree } else if r < 0.7 { PropKind::Pine } else if r < 0.85 { PropKind::Bush } else { PropKind::Flowers }
                    }
                    Biome::Autumn => {
                        if steep || r < 0.08 { PropKind::Rock } else if r < 0.7 { PropKind::Tree } else if r < 0.85 { PropKind::Pine } else { PropKind::Bush }
                    }
                    Biome::Snow => {
                        if steep || r < 0.18 { PropKind::Rock } else if r < 0.92 { PropKind::Pine } else { PropKind::Bush }
                    }
                    Biome::Desert => {
                        if steep || r < 0.4 { PropKind::Rock } else if r < 0.8 { PropKind::Cactus } else if r < 0.9 { PropKind::Bush } else { PropKind::Palm }
                    }
                    Biome::Coast => {
                        if steep || r < 0.15 { PropKind::Rock } else if r < 0.5 { PropKind::Palm } else if r < 0.7 { PropKind::Tree } else if r < 0.85 { PropKind::Bush } else { PropKind::Flowers }
                    }
                };
                // keep tall things a bit further from the road
                if matches!(kind, PropKind::Tree | PropKind::Pine | PropKind::Palm) && q.dist < hw + SHOULDER + 6.0 {
                    continue;
                }
                let scale = match kind {
                    PropKind::Rock => rng.range(0.6, 2.2),
                    PropKind::Flowers | PropKind::Bush => rng.range(0.7, 1.3),
                    _ => rng.range(0.8, 1.45),
                };
                let radius = match kind {
                    PropKind::Tree | PropKind::Pine | PropKind::Palm => 0.45 * scale,
                    PropKind::Cactus => 0.35 * scale,
                    PropKind::Rock => 0.9 * scale,
                    _ => 0.0,
                };
                props.push(Prop { kind, pos: Vec3::new(x, y, z), rot_y: rng.range(0.0, 6.283), scale, radius, variant: rng.range_i(0, 3) as u8 });
            }
        }
        // spectators on the outside of the sharpest corners
        let n = self.samples.len();
        let mut corner_spots: Vec<usize> = Vec::new();
        for i in (0..n).step_by(5) {
            let c = self.samples[i].curvature.abs();
            if c > 1.0 / 45.0 && corner_spots.iter().all(|&j| (j as isize - i as isize).unsigned_abs().min(n - (j as isize - i as isize).unsigned_abs()) > 50) {
                corner_spots.push(i);
            }
        }
        // and near the start line
        corner_spots.push(8);
        for &i in &corner_spots {
            let smp = self.samples[i];
            let outside = if smp.curvature > 0.0 { 1.0 } else { -1.0 }; // left turn -> outside is right
            let count = rng.range_i(3, 8);
            for k in 0..count {
                let along = (k as f32 - count as f32 * 0.5) * 1.6 + rng.range(-0.4, 0.4);
                let lat = outside * (smp.half_width + SHOULDER + rng.range(4.5, 7.5));
                let flat_right = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
                let p = smp.pos + flat_right * lat + smp.tangent * along;
                let y = self.terrain.height(p.x, p.z);
                let to_track = (smp.pos - p).normalize();
                let rot = to_track.x.atan2(to_track.z);
                props.push(Prop { kind: PropKind::Spectator, pos: Vec3::new(p.x, y, p.z), rot_y: rot, scale: rng.range(0.9, 1.1), radius: 0.3, variant: rng.range_i(0, 6) as u8 });
            }
            // hay bale barrier in front of spectators
            for k in 0..3 {
                let along = (k as f32 - 1.0) * 2.4;
                let lat = outside * (smp.half_width + SHOULDER + 2.0);
                let flat_right = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
                let p = smp.pos + flat_right * lat + smp.tangent * along;
                let y = self.terrain.height(p.x, p.z);
                let rot = smp.tangent.x.atan2(smp.tangent.z);
                props.push(Prop { kind: PropKind::HayBale, pos: Vec3::new(p.x, y, p.z), rot_y: rot, scale: 1.0, radius: 0.8, variant: 0 });
            }
            if rng.chance(0.5) {
                let lat = outside * (smp.half_width + SHOULDER + 11.0);
                let flat_right = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
                let p = smp.pos + flat_right * lat;
                let y = self.terrain.height(p.x, p.z);
                props.push(Prop { kind: PropKind::Tent, pos: Vec3::new(p.x, y, p.z), rot_y: rng.range(0.0, 6.28), scale: 1.0, radius: 1.6, variant: rng.range_i(0, 4) as u8 });
            }
        }
        // cones marking the jump take-offs
        for &j in &self.jumps.clone() {
            let smp = self.samples[wrap(j as isize - 10, n)];
            for side in [-1.0f32, 1.0] {
                let p = smp.pos + smp.right * side * (smp.half_width + 0.6);
                let y = self.terrain.height(p.x, p.z);
                props.push(Prop { kind: PropKind::Cone, pos: Vec3::new(p.x, y, p.z), rot_y: 0.0, scale: 1.0, radius: 0.0, variant: 0 });
            }
        }
        let pts: Vec<Vec2> = props.iter().map(|p| Vec2::new(p.pos.x, p.pos.z)).collect();
        if !pts.is_empty() {
            self.prop_grid = SpatialGrid::new(&pts, 12.0, 10.0);
        }
        self.props = props;
    }

    /// Closest point on the centre line. `hint` = previous index for fast local search.
    pub fn query(&self, p: Vec3, hint: Option<usize>) -> TrackQuery {
        let n = self.samples.len();
        let q = Vec2::new(p.x, p.z);
        let mut best = (f32::MAX, 0usize);
        if let Some(h) = hint {
            for k in -30isize..=30 {
                let i = wrap(h as isize + k, n);
                let s = self.samples[i].pos;
                let d = Vec2::new(s.x, s.z).distance_squared(q);
                if d < best.0 {
                    best = (d, i);
                }
            }
            if best.0 > 25.0 * 25.0 {
                best = (f32::MAX, 0);
            }
        }
        if best.0 == f32::MAX {
            let mut radius = 24.0;
            while best.0 == f32::MAX && radius < 200.0 {
                self.grid.for_each_near(q, radius, |i| {
                    let s = self.samples[i as usize].pos;
                    let d = Vec2::new(s.x, s.z).distance_squared(q);
                    if d < best.0 {
                        best = (d, i as usize);
                    }
                });
                radius *= 2.0;
            }
            if best.0 == f32::MAX {
                for (i, s) in self.samples.iter().enumerate() {
                    let d = Vec2::new(s.pos.x, s.pos.z).distance_squared(q);
                    if d < best.0 {
                        best = (d, i);
                    }
                }
            }
        }
        // refine on the two adjacent segments
        let i = best.1;
        let mut res = TrackQuery { index: i, s: 0.0, lateral: 0.0, dist: f32::MAX };
        for (a, b) in [(wrap(i as isize - 1, n), i), (i, (i + 1) % n)] {
            let pa = self.samples[a].pos;
            let pb = self.samples[b].pos;
            let a2 = Vec2::new(pa.x, pa.z);
            let b2 = Vec2::new(pb.x, pb.z);
            let ab = b2 - a2;
            let t = ((q - a2).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let c = a2 + ab * t;
            let d = c.distance(q);
            if d < res.dist {
                let r = self.samples[a].right;
                let r2 = Vec2::new(r.x, r.z).normalize_or_zero();
                let s = (self.samples[a].s + t * SAMPLE_SPACING).rem_euclid(self.length);
                res = TrackQuery { index: a, s, lateral: (q - c).dot(r2), dist: d };
            }
        }
        res
    }

    pub fn surface_at(&self, p: Vec3, hint: Option<usize>) -> (Surface, TrackQuery) {
        let q = self.query(p, hint);
        let hw = self.samples[q.index].half_width;
        let s = if q.dist <= hw + 0.4 {
            self.style.road_surface
        } else if q.dist <= hw + SHOULDER {
            match self.style.road_surface {
                Surface::Asphalt => Surface::Gravel,
                other => other,
            }
        } else {
            self.style.offroad_surface
        };
        (s, q)
    }

    /// Interpolated centre-line frame at arc length s: (position, tangent, right, up)
    pub fn frame_at(&self, s: f32) -> (Vec3, Vec3, Vec3, Vec3) {
        let n = self.samples.len();
        let s = s.rem_euclid(self.length);
        let f = s / SAMPLE_SPACING;
        let i = (f.floor() as usize) % n;
        let j = (i + 1) % n;
        let t = f.fract();
        let a = &self.samples[i];
        let b = &self.samples[j];
        (
            a.pos.lerp(b.pos, t),
            a.tangent.lerp(b.tangent, t).normalize(),
            a.right.lerp(b.right, t).normalize(),
            a.up.lerp(b.up, t).normalize(),
        )
    }

    /// Orientation for a car facing along the track at s (car forward = -Z local).
    pub fn rotation_at(&self, s: f32) -> Quat {
        let (_, t, r, u) = self.frame_at(s);
        Quat::from_mat3(&Mat3::from_cols(r, u, -t)).normalize()
    }

    /// Starting grid slots behind the start line. Returns (position, rotation).
    pub fn grid_slots(&self, count: usize) -> Vec<(Vec3, Quat)> {
        (0..count)
            .map(|k| {
                let row = k / 2;
                let side = if k % 2 == 0 { -1.0 } else { 1.0 };
                let s = -8.0 - row as f32 * 8.0 - (k % 2) as f32 * 3.0;
                let (p, _, r, _) = self.frame_at(s);
                let mut pos = p + r * side * 2.6;
                pos.y = self.terrain.height(pos.x, pos.z) + 0.6;
                (pos, self.rotation_at(s))
            })
            .collect()
    }

    pub fn for_each_prop_near(&self, p: Vec3, radius: f32, mut f: impl FnMut(&Prop)) {
        if self.props.is_empty() {
            return;
        }
        self.prop_grid.for_each_near(Vec2::new(p.x, p.z), radius, |i| f(&self.props[i as usize]));
    }

    /// Ground height including water surface (for cameras / particles)
    pub fn visual_height(&self, x: f32, z: f32) -> f32 {
        let h = self.terrain.height(x, z);
        match self.water_level {
            Some(w) => h.max(w),
            None => h,
        }
    }
}

/// Road height at q using the cross-section at sample j; returns (lateral, height, half_width)
fn road_profile_at(samples: &[TrackSample], j: usize, q: Vec2) -> (f32, f32, f32) {
    let n = samples.len();
    // refine onto the nearer adjacent segment
    let mut best = (f32::MAX, 0.0f32, 0.0f32, 0.0f32);
    for (a, b) in [(wrap(j as isize - 1, n), j), (j, (j + 1) % n)] {
        let sa = &samples[a];
        let sb = &samples[b];
        let a2 = Vec2::new(sa.pos.x, sa.pos.z);
        let b2 = Vec2::new(sb.pos.x, sb.pos.z);
        let ab = b2 - a2;
        let t = ((q - a2).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let c = a2 + ab * t;
        let d = c.distance(q);
        if d < best.0 {
            let r = sa.right.lerp(sb.right, t);
            let r2 = Vec2::new(r.x, r.z).normalize_or_zero();
            let lat = (q - c).dot(r2);
            let hw = sa.half_width + (sb.half_width - sa.half_width) * t;
            let lat_c = lat.clamp(-(hw + SHOULDER), hw + SHOULDER);
            let rise = if r.y.abs() > 1e-5 { r.y / Vec2::new(r.x, r.z).length().max(1e-3) } else { 0.0 };
            let base = sa.pos.y + (sb.pos.y - sa.pos.y) * t;
            // slight crown so water would run off — and it looks nicer
            let crown = -0.012 * lat_c.abs();
            best = (d, lat, base + lat_c * rise + crown, hw);
        }
    }
    (best.1, best.2, best.3)
}

impl Track {
    /// A generated track whose terrain is perfectly flat and whose every surface is `surface`.
    /// Used by physics tests.
    pub fn flat_for_tests(surface: Surface) -> Track {
        let mut t = Track::generate(&TrackParams { seed: 7, biome: Some(Biome::Meadow), length_class: 2 });
        t.terrain.heights.iter_mut().for_each(|h| *h = 0.0);
        t.style.road_surface = surface;
        t.style.offroad_surface = surface;
        t.water_level = None;
        t.props.clear();
        t
    }
}
