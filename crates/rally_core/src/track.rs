//! Procedural closed-loop rally track generation.
//!
//! Pipeline: random control loop -> Catmull-Rom spline -> arc-length resample -> smoothing
//! until the minimum corner radius is drivable -> self-clearance check -> elevation profile
//! (smoothed terrain + crests + jumps, slope-limited) -> banking -> heightfield that is
//! flattened under the road and blended into natural hills -> props.
use crate::biome::{Biome, BiomeStyle};
use crate::noise;
use crate::rng::Rng;
use crate::surface::{Surface, SurfaceProps};
use crate::surfacemap::{SurfaceMap, SurfaceMix, SurfaceMode};
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
    /// 0 = short (~1.3km), 1 = medium (~1.8km), 2 = long (~2.5km)
    pub length_class: u8,
    /// Track style / technicality: 0 = Relaxed (flowing, wide), 1 = Rally (default),
    /// 2 = Expert (more hairpins, narrower, more surface patches, kinks after jumps)
    #[serde(default = "default_difficulty")]
    pub difficulty: u8,
    /// Road surface layout (biome default, forced single surface, or mixed stage)
    #[serde(default)]
    pub surface: SurfaceMode,
    /// Closed loop (laps) or point-to-point stage (A→B)
    #[serde(default)]
    pub layout: LayoutKind,
    /// Weather / conditions along the track
    #[serde(default)]
    pub conditions: crate::weather::ConditionsSpec,
}

/// Track topology.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum LayoutKind {
    /// Closed loop raced over laps (current behaviour)
    #[default]
    Loop,
    /// Open rally stage from start to finish (campaign time trials), length in metres
    PointToPoint { length_m: f32 },
}

fn default_difficulty() -> u8 {
    1
}

pub const DIFFICULTY_NAMES: [&str; 3] = ["Relaxed", "Rally", "Expert"];

impl TrackParams {
    /// Test hook: `COZY_TRACK_STYLE=0..2` (Relaxed/Rally/Expert) and
    /// `COZY_SURFACE=biome|gravel|tarmac|snow|mixed` (or 0..4) override the params.
    pub fn with_env_overrides(mut self) -> Self {
        if let Some(d) = std::env::var("COZY_TRACK_STYLE").ok().and_then(|v| v.parse::<u8>().ok()) {
            self.difficulty = d.min(2);
        }
        if let Ok(v) = std::env::var("COZY_SURFACE") {
            let v = v.to_lowercase();
            let modes = crate::surfacemap::SURFACE_MODES;
            if let Some(m) = v.parse::<usize>().ok().and_then(|i| modes.get(i).copied()).or_else(|| modes.iter().copied().find(|m| m.name().to_lowercase() == v)) {
                self.surface = m;
            }
        }
        self
    }
}

impl Default for TrackParams {
    fn default() -> Self {
        TrackParams { seed: 1, biome: None, length_class: 1, difficulty: default_difficulty(), surface: SurfaceMode::BiomeDefault, layout: LayoutKind::Loop, conditions: crate::weather::ConditionsSpec::BiomeDefault }
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
    /// Dominant road surface on the centre line here (sections, patches...)
    pub surface: Surface,
    /// The other surface blended in on the centre line (== `surface` when pure)
    pub surface_next: Surface,
    /// Weight of `surface_next` on the centre line (0 ..= 0.5); colour/grip = lerp(surface,
    /// surface_next, surface_blend). Use `Track::surface_mix_at` for exact values off-centre.
    pub surface_blend: f32,
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
    // --- added scenery kinds (models in rally_game::models::props)
    Birch,
    Blossom,
    Mushroom,
    Grass,
    Fence,
    /// Corner warning board; variant 0 = arrows point right, 1 = left (from the approaching driver)
    Chevron,
    /// Wooden direction arrow; variant 0 = right, 1 = left
    ArrowSign,
    Cottage,
    Barn,
    Windmill,
    Picnic,
    Camper,
    Sheep,
    Cow,
    Deer,
    /// Pennant line between two poles along local X; length = 10 m * scale (poles unscaled)
    Bunting,
    FlagPole,
    Lighthouse,
    BeachUmbrella,
    Boat,
    Mesa,
    Tumbleweed,
    Adobe,
    Snowman,
    SkiCabin,
    FrozenPond,
    Pumpkin,
    Scarecrow,
    HayWagon,
    Beehive,
    FlowerField,
}

impl PropKind {
    /// Height of the collision cylinder above the prop base (only used when radius > 0).
    pub fn collision_height(self, scale: f32) -> f32 {
        match self {
            PropKind::Rock => 1.2 * scale,
            PropKind::HayBale => 1.0,
            PropKind::Sheep | PropKind::Pumpkin | PropKind::Beehive => 1.0 * scale,
            PropKind::Cow | PropKind::Deer | PropKind::Snowman => 1.6 * scale,
            PropKind::Camper | PropKind::HayWagon | PropKind::Boat => 2.4 * scale,
            _ => 6.0 * scale.max(1.0),
        }
    }
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

/// Blended ground under a point (see `Track::ground_at`).
#[derive(Clone, Copy, Debug)]
pub struct GroundInfo {
    /// Dominant surface
    pub primary: Surface,
    /// Second surface in the blend (== primary when pure)
    pub secondary: Surface,
    /// Weight of `secondary` (0 ..= 0.5)
    pub blend: f32,
    /// Blended tyre/handling properties (continuous across every surface change)
    pub props: SurfaceProps,
    pub query: TrackQuery,
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
    /// Designed layout pieces (corners/straights) in arc length, for stats and hints
    pub features: Vec<crate::layout::Feature>,
    /// Surface sections / transitions / patches
    pub surface_map: SurfaceMap,
    /// Weather timeline along the track (StageConditions::at(s)); default = clear
    pub conditions: crate::weather::StageConditions,
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
        let mut style = biome.style();
        let noise_seed = (params.seed as u32).wrapping_mul(2654435761);

        // ---- 1-3. designed layout (corner complexes + straights, closed exactly)
        let diff = params.difficulty.min(2);
        let (hw_narrow, hw_wide) = [(3.75, 5.6), (3.5, 5.3), (3.3, 5.0)][diff as usize];
        let half_w_base = hw_wide + 0.6;
        let mut layout = None;
        for _ in 0..120 {
            if let Some(l) = crate::layout::design(&mut rng, diff, params.length_class, SAMPLE_SPACING, half_w_base, SHOULDER) {
                layout = Some(l);
                break;
            }
        }
        let layout = layout?;
        let pts = layout.pts;
        let n = pts.len();
        let raw_curv = signed_curvature_2d(&pts);
        if raw_curv.iter().any(|c| c.abs() > 1.0 / 10.5) {
            return None;
        }
        let length = n as f32 * SAMPLE_SPACING;
        let grid = SpatialGrid::new(&pts, 16.0, 60.0);
        let mut curv = raw_curv;
        smooth_closed_f32(&mut curv, 2, 2);

        // ---- 6. elevation: natural terrain, slope limited, then designed crests / dips / jumps
        let mut elev: Vec<f32> = pts.iter().map(|p| natural_height(&style, biome, noise_seed, p.x, p.y)).collect();
        smooth_closed_f32(&mut elev, 12, 3);
        slope_limit(&mut elev, 0.11 * SAMPLE_SPACING);
        smooth_closed_f32(&mut elev, 3, 2);
        let jumps = crate::layout::shape_elevation(&mut elev, &layout.features, &curv, &mut rng, diff, SAMPLE_SPACING);
        // flatten the start area
        let start_flat = 22;
        let e0 = elev[0];
        for k in -(start_flat as isize)..=(start_flat as isize) {
            let i = wrap(k, n);
            let t = smoothstep(start_flat as f32, start_flat as f32 * 0.5, k.unsigned_abs() as f32);
            elev[i] = elev[i] + (e0 - elev[i]) * t;
        }

        // ---- 7. width, surface, banking, samples
        let mut tech = layout.tech.clone();
        smooth_closed_f32(&mut tech, 6, 2);
        let mut width_noise: Vec<f32> = (0..n).map(|i| noise::value2(i as f32 * 0.03, 3.3, noise_seed) * 0.25).collect();
        smooth_closed_f32(&mut width_noise, 4, 1);
        let hws: Vec<f32> = (0..n)
            .map(|i| {
                let start_wide = smoothstep(40.0, 10.0, (i.min(n - i)) as f32);
                (hw_wide + (hw_narrow - hw_wide) * tech[i].clamp(0.0, 1.0) + width_noise[i]).max(3.25) + start_wide
            })
            .collect();
        let splan = crate::surfacemap::plan(biome, params.surface, diff, &layout.features, &jumps, &hws, SAMPLE_SPACING, &mut rng);
        if splan.main != style.road_surface {
            // keep (road_surface, road, road_edge) a consistent palette for renderers
            style.road = crate::surfacemap::road_color(biome, splan.main);
            style.road_edge = [style.road[0] * 0.85, style.road[1] * 0.85, style.road[2] * 0.85];
            style.road_surface = splan.main;
        }
        let surface_map = splan.map;
        let mut bank: Vec<f32> = (0..n)
            .map(|i| {
                let b = (curv[i] * 24.0).clamp(-0.14, 0.14);
                let cam = layout.camber[i];
                if cam >= 0.0 { b * cam } else { -b * 0.45 }
            })
            .collect();
        smooth_closed_f32(&mut bank, 5, 2);
        let mut samples = Vec::with_capacity(n);
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
            let (sf, sf_next, sf_blend) = surface_map.mix_at(i as f32 * SAMPLE_SPACING, 0.0).dominant();
            samples.push(TrackSample {
                pos: p,
                tangent,
                right,
                up,
                s: i as f32 * SAMPLE_SPACING,
                half_width: hws[i],
                bank: bank[i],
                curvature: curv[i],
                surface: sf,
                surface_next: sf_next,
                surface_blend: sf_blend,
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
                        // same stretch of road only if close along the loop *relative* to the
                        // spatial gap (hairpin legs are separate sections and blend smoothly)
                        let near_existing = sections.iter().any(|&k| {
                            let d = (k as isize - j as isize).unsigned_abs();
                            let ds = d.min(n - d) as f32 * SAMPLE_SPACING;
                            ds < 1.4 * pts[k].distance(pts[j]) + 8.0
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
            conditions: match params.conditions {
                // Dynamic timelines are generated by the stage generator (tracks work);
                // until then they fall back to clear weather.
                crate::weather::ConditionsSpec::Fixed(p) => crate::weather::StageConditions::uniform(p.weather()),
                _ => crate::weather::StageConditions::default(),
            },
            features: layout.features,
            surface_map,
            grid,
            prop_grid: SpatialGrid::new(&[Vec2::ZERO], 32.0, 1.0),
        };
        track.place_props(&mut rng, noise_seed);
        Some(track)
    }

    fn place_props(&mut self, rng: &mut Rng, noise_seed: u32) {
        let props = PropPlacer::run(self, rng, noise_seed);
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

    /// Dominant surface under a point (see `ground_at` for the smooth blend).
    pub fn surface_at(&self, p: Vec3, hint: Option<usize>) -> (Surface, TrackQuery) {
        let g = self.ground_at(p, hint);
        (g.primary, g.query)
    }

    /// Road surface blend at arc length `s` and lateral offset `lat` (+ = right), ignoring the
    /// road edge. Renderers use this to colour the road continuously.
    pub fn surface_mix_at(&self, s: f32, lat: f32) -> SurfaceMix {
        self.surface_map.mix_at(s, lat)
    }

    /// Blended ground (surface pair + continuous tyre properties) under a point: section
    /// transitions and patches are smooth along and across the road, and the road edge blends
    /// into the shoulder and then the off-road surface over ~0.6-0.8 m.
    pub fn ground_at(&self, p: Vec3, hint: Option<usize>) -> GroundInfo {
        use crate::surfacemap::{shoulder_of, smoothstep as ss};
        let q = self.query(p, hint);
        let hw = self.samples[q.index].half_width;
        let off = self.style.offroad_surface;
        let e2 = ss(hw + SHOULDER - 0.4, hw + SHOULDER + 0.4, q.dist);
        if e2 >= 1.0 {
            return GroundInfo { primary: off, secondary: off, blend: 0.0, props: off.props(), query: q };
        }
        let m = self.surface_map.mix_at(q.s, q.lateral.clamp(-(hw + SHOULDER), hw + SHOULDER));
        let e1 = ss(hw + 0.1, hw + 0.7, q.dist);
        let road = SurfaceProps::lerp(m.a.props(), m.b.props(), m.t);
        let (sa, sb) = (shoulder_of(m.a), shoulder_of(m.b));
        let shoulder = SurfaceProps::lerp(sa.props(), sb.props(), m.t);
        let props = SurfaceProps::lerp(SurfaceProps::lerp(road, shoulder, e1), off.props(), e2);
        // dominant two surfaces by weight
        let parts = [
            (m.a, (1.0 - m.t) * (1.0 - e1) * (1.0 - e2)),
            (m.b, m.t * (1.0 - e1) * (1.0 - e2)),
            (sa, (1.0 - m.t) * e1 * (1.0 - e2)),
            (sb, m.t * e1 * (1.0 - e2)),
            (off, e2),
        ];
        let mut acc: [(Surface, f32); 5] = [(off, 0.0); 5];
        let mut k = 0;
        for (sf, w) in parts {
            if let Some(e) = acc[..k].iter_mut().find(|e| e.0 == sf) {
                e.1 += w;
            } else {
                acc[k] = (sf, w);
                k += 1;
            }
        }
        let acc = &mut acc[..k];
        acc.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let (primary, w1) = acc[0];
        let (secondary, w2) = if acc.len() > 1 { acc[1] } else { (primary, 0.0) };
        GroundInfo { primary, secondary, blend: w2 / (w1 + w2).max(1e-6), props, query: q }
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

fn slope_limit(elev: &mut [f32], max_step: f32) {
    let n = elev.len();
    for _ in 0..4 {
        for i in 0..n {
            let prev = elev[wrap(i as isize - 1, n)];
            elev[i] = elev[i].clamp(prev - max_step, prev + max_step);
        }
        for i in (0..n).rev() {
            let next = elev[(i + 1) % n];
            elev[i] = elev[i].clamp(next - max_step, next + max_step);
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
        let mut t = Track::generate(&TrackParams { seed: 7, biome: Some(Biome::Meadow), length_class: 2, ..Default::default() });
        t.terrain.heights.iter_mut().for_each(|h| *h = 0.0);
        t.style.road_surface = surface;
        t.samples.iter_mut().for_each(|s| {
            s.surface = surface;
            s.surface_next = surface;
            s.surface_blend = 0.0;
        });
        t.surface_map = SurfaceMap::uniform(surface, t.length);
        t.style.offroad_surface = surface;
        t.water_level = None;
        t.props.clear();
        t
    }
}

// ======================================================================== prop placement

/// Yaw that turns a prop's local +Z towards direction `d` (XZ)
fn yaw_facing(d: Vec3) -> f32 {
    d.x.atan2(d.z)
}

/// Yaw that lays a prop's local +X along direction `d` (XZ)
fn yaw_along(d: Vec3) -> f32 {
    (-d.z).atan2(d.x)
}

struct PropPlacer<'a> {
    t: &'a Track,
    props: Vec<Prop>,
    /// circles (centre, radius) the vegetation scatter must avoid
    keep_out: Vec<(Vec2, f32)>,
}

impl<'a> PropPlacer<'a> {
    fn run(t: &'a Track, rng: &mut Rng, noise_seed: u32) -> Vec<Prop> {
        let mut pp = PropPlacer { t, props: Vec::new(), keep_out: Vec::new() };
        pp.start_area(rng);
        pp.corners(rng);
        pp.set_pieces(rng);
        pp.fences(noise_seed);
        pp.roadside(rng);
        pp.scatter(rng, noise_seed);
        pp.jump_cones();
        pp.props
    }

    fn n(&self) -> usize {
        self.t.samples.len()
    }

    fn smp(&self, i: isize) -> TrackSample {
        self.t.samples[wrap(i, self.n())]
    }

    fn flat_right(s: &TrackSample) -> Vec3 {
        Vec3::new(s.right.x, 0.0, s.right.z).normalize()
    }

    /// Distance from p to the road edge (shoulder included) of the nearest track section.
    fn clearance(&self, p: Vec3) -> f32 {
        let q = self.t.query(p, None);
        q.dist - self.t.samples[q.index].half_width - SHOULDER
    }

    fn ground(&self, x: f32, z: f32) -> f32 {
        self.t.terrain.height(x, z)
    }

    fn under_water(&self, y: f32, margin: f32) -> bool {
        self.t.water_level.map_or(false, |w| y < w + margin)
    }

    fn blocked(&self, p: Vec3, r: f32) -> bool {
        let q = Vec2::new(p.x, p.z);
        self.keep_out.iter().any(|(c, cr)| c.distance(q) < cr + r)
    }

    fn push(&mut self, kind: PropKind, p: Vec3, rot_y: f32, scale: f32, radius: f32, variant: u8) {
        self.props.push(Prop { kind, pos: p, rot_y, scale, radius, variant });
    }

    /// Ground point at lateral offset `lat` (signed, right positive) from sample i
    fn beside(&self, i: isize, lat: f32, along: f32) -> Vec3 {
        let s = self.smp(i);
        let p = s.pos + Self::flat_right(&s) * lat + Vec3::new(s.tangent.x, 0.0, s.tangent.z).normalize() * along;
        Vec3::new(p.x, self.ground(p.x, p.z), p.z)
    }

    /// Small crowd facing the road around sample i on `side` at lateral distance `lat`.
    fn crowd(&mut self, rng: &mut Rng, i: isize, side: f32, lat: f32, count: i32, spread: f32) {
        let s = self.smp(i);
        for k in 0..count {
            let along = (k as f32 - count as f32 * 0.5) * spread + rng.range(-0.4, 0.4);
            let p = self.beside(i, side * (s.half_width + SHOULDER + lat + rng.range(0.0, 1.6)), along);
            if self.clearance(p) < lat - 0.5 || self.under_water(p.y, 0.3) {
                continue;
            }
            let to_track = (s.pos - p) * Vec3::new(1.0, 0.0, 1.0);
            let rot = yaw_facing(-to_track.normalize_or_zero()) + rng.range(-0.3, 0.3);
            self.push(PropKind::Spectator, p, rot, rng.range(0.92, 1.08), 0.3, rng.range_i(0, 6) as u8);
        }
    }

    fn bunting_line(&mut self, rng: &mut Rng, i0: isize, i1: isize, lat: f32) {
        // poles every ~5 samples (10 m) following the track
        let mut i = i0;
        while i + 5 <= i1 {
            let a = self.beside(i, lat, 0.0);
            let b = self.beside(i + 5, lat, 0.0);
            let d = (b - a) * Vec3::new(1.0, 0.0, 1.0);
            let len = d.length();
            if self.clearance(a) > lat.abs() - self.smp(i).half_width - SHOULDER - 1.0 && self.clearance(b) > 1.0 && (a.y - b.y).abs() < 1.5 {
                self.push(PropKind::Bunting, a, yaw_along(d / len), len / 10.0, 0.0, rng.range_i(0, 6) as u8);
            }
            i += 5;
        }
    }

    fn start_area(&mut self, rng: &mut Rng) {
        let s0 = self.smp(0);
        self.keep_out.push((Vec2::new(s0.pos.x, s0.pos.z), 42.0));
        for side in [-1.0f32, 1.0] {
            let edge = s0.half_width + SHOULDER;
            // hay bale barrier both sides of the grid
            for k in 0..14 {
                let i = -26 + k * 3;
                let p = self.beside(i, side * (self.smp(i).half_width + SHOULDER + 2.2), 0.0);
                let t = self.smp(i).tangent;
                if self.clearance(p) > 1.2 {
                    self.push(PropKind::HayBale, p, yaw_along(Vec3::new(t.x, 0.0, t.z).normalize()), 1.0, 0.8, (k % 2) as u8);
                }
            }
            // crowds along the straight
            for k in 0..6 {
                let i = -22 + k * 7;
                let c = rng.range_i(3, 6);
                self.crowd(rng, i, side, 4.2, c, 1.3);
            }
            self.bunting_line(rng, -25, 20, side * (edge + 7.5));
            // flag poles
            for k in 0..4 {
                let i = -20 + k * 12;
                let p = self.beside(i, side * (edge + 9.5), 0.0);
                if self.clearance(p) > 8.0 {
                    self.push(PropKind::FlagPole, p, yaw_along(Self::flat_right(&self.smp(i)) * side), 1.0, 0.0, rng.range_i(0, 6) as u8);
                }
            }
            // campers, tents and picnics behind the crowd
            for k in 0..3 {
                let i = -18 + k * 14;
                let p = self.beside(i, side * (edge + rng.range(14.0, 18.0)), rng.range(-3.0, 3.0));
                if self.clearance(p) < 12.0 || self.under_water(p.y, 0.5) {
                    continue;
                }
                let kind = [PropKind::Camper, PropKind::Tent, PropKind::Picnic][(k + (side > 0.0) as isize) as usize % 3];
                let face = yaw_facing((s0.pos - p).normalize_or_zero()) + rng.range(-0.4, 0.4);
                let r = match kind {
                    PropKind::Camper => 2.4,
                    PropKind::Tent => 1.6,
                    _ => 0.0,
                };
                self.push(kind, p, face, 1.0, r, rng.range_i(0, 6) as u8);
            }
        }
    }

    /// Corners: chevron boards on the outside of sharp bends, arrow signs before them,
    /// and spectator clusters (with hay barriers, bunting and camps) at the tightest ones.
    fn corners(&mut self, rng: &mut Rng) {
        let n = self.n() as isize;
        // corner apexes = local curvature maxima
        let mut apexes: Vec<(isize, f32)> = Vec::new();
        for i in 0..n {
            let c = self.smp(i).curvature.abs();
            if c < 1.0 / 60.0 {
                continue;
            }
            let is_max = (-6..=6).all(|k| self.smp(i + k).curvature.abs() <= c);
            if is_max && apexes.iter().all(|&(j, _)| (i - j).abs().min(n - (i - j).abs()) > 20) {
                apexes.push((i, c));
            }
        }
        let mut spectator_spots = 0;
        for &(i, c) in &apexes {
            if i < 30 || i > n - 30 {
                continue;
            }
            let s = self.smp(i);
            let outside = if s.curvature > 0.0 { 1.0 } else { -1.0 };
            let variant = if s.curvature > 0.0 { 1 } else { 0 };
            if c > 1.0 / 52.0 {
                // chevrons around the apex, facing the approaching driver
                let span = if c > 1.0 / 25.0 { 3 } else if c > 1.0 / 38.0 { 2 } else { 1 };
                for k in -span..=span {
                    let j = i + k * 3;
                    let sj = self.smp(j);
                    let p = self.beside(j, outside * (sj.half_width + SHOULDER + 1.3), 0.0);
                    if self.clearance(p) < 0.9 || self.under_water(p.y, 0.2) {
                        continue;
                    }
                    let t = Vec3::new(sj.tangent.x, 0.0, sj.tangent.z).normalize();
                    let face = (-t - Self::flat_right(&sj) * outside * 0.35).normalize();
                    // board front is local -Z
                    self.push(PropKind::Chevron, p, yaw_facing(-face), 1.3, 0.0, variant);
                }
                // wooden arrow before the bend
                let j = i - 20;
                let sj = self.smp(j);
                let p = self.beside(j, outside * (sj.half_width + SHOULDER + 1.6), 0.0);
                if self.clearance(p) > 1.0 && !self.under_water(p.y, 0.2) {
                    let t = Vec3::new(sj.tangent.x, 0.0, sj.tangent.z).normalize();
                    self.push(PropKind::ArrowSign, p, yaw_facing(t), 1.0, 0.0, variant);
                }
            }
            if c > 1.0 / 48.0 && spectator_spots < 7 {
                spectator_spots += 1;
                let j = i + 6;
                let sj = self.smp(j);
                let edge = sj.half_width + SHOULDER;
                let cnt = rng.range_i(4, 9);
                self.crowd(rng, j, outside, 5.0, cnt, 1.4);
                for k in 0..3 {
                    let p = self.beside(j, outside * (edge + 2.2), (k as f32 - 1.0) * 2.4);
                    if self.clearance(p) > 1.4 {
                        let t = Vec3::new(sj.tangent.x, 0.0, sj.tangent.z).normalize();
                        self.push(PropKind::HayBale, p, yaw_along(t), 1.0, 0.8, (k % 2) as u8);
                    }
                }
                let a = self.beside(j - 4, outside * (edge + 9.0), 0.0);
                self.keep_out.push((Vec2::new(a.x, a.z), 10.0));
                self.bunting_line(rng, j - 5, j + 5, outside * (edge + 9.0));
                if rng.chance(0.7) {
                    let p = self.beside(j, outside * (edge + rng.range(13.0, 16.0)), rng.range(-4.0, 4.0));
                    if self.clearance(p) > 11.0 && !self.under_water(p.y, 0.5) {
                        let kind = if rng.chance(0.5) { PropKind::Tent } else { PropKind::Camper };
                        let face = yaw_facing((sj.pos - p).normalize_or_zero());
                        self.push(kind, p, face + rng.range(-0.5, 0.5), 1.0, if kind == PropKind::Camper { 2.4 } else { 1.6 }, rng.range_i(0, 6) as u8);
                        let q = p + Vec3::new(rng.range(-4.0, 4.0), 0.0, rng.range(-4.0, 4.0));
                        let q = Vec3::new(q.x, self.ground(q.x, q.z), q.z);
                        if self.clearance(q) > 9.0 {
                            self.push(PropKind::Picnic, q, rng.range(0.0, 6.28), 1.0, 0.0, rng.range_i(0, 6) as u8);
                        }
                    }
                }
            }
        }
    }

    /// Is the ground around p flat enough for a building of half-size r? Returns base height.
    fn footprint(&self, p: Vec3, r: f32, max_drop: f32) -> Option<f32> {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for (dx, dz) in [(0.0, 0.0), (-r, -r), (r, -r), (r, r), (-r, r), (0.0, r), (0.0, -r), (r, 0.0), (-r, 0.0)] {
            let h = self.ground(p.x + dx, p.z + dz);
            lo = lo.min(h);
            hi = hi.max(h);
        }
        if hi - lo > max_drop {
            None
        } else {
            Some(lo + (hi - lo) * 0.25)
        }
    }

    /// Scatter a few things around a centre (offsets random within ring r0..r1)
    fn around(&mut self, rng: &mut Rng, c: Vec3, kind: PropKind, count: i32, r0: f32, r1: f32, radius: f32, scale: (f32, f32), min_clear: f32) {
        for _ in 0..count {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let d = rng.range(r0, r1);
            let x = c.x + a.cos() * d;
            let z = c.z + a.sin() * d;
            let p = Vec3::new(x, self.ground(x, z), z);
            if self.clearance(p) < min_clear || self.under_water(p.y, 0.4) {
                continue;
            }
            self.push(kind, p, rng.range(0.0, 6.28), rng.range(scale.0, scale.1), radius, rng.range_i(0, 6) as u8);
        }
    }

    fn set_pieces(&mut self, rng: &mut Rng) {
        let n = self.n() as i32;
        let biome = self.t.biome;
        let target = (self.t.length / 260.0).round() as usize + 2;
        let mut placed: Vec<Vec3> = Vec::new();
        let start = self.t.samples[0].pos;
        let mut kit = rng.range_i(0, 4) as usize;
        for _attempt in 0..400 {
            if placed.len() >= target {
                break;
            }
            let i = rng.range_i(0, n) as isize;
            let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let s = self.smp(i);
            let mesa = biome == Biome::Desert && kit % 3 == 1;
            let lat = s.half_width + SHOULDER + if mesa { rng.range(60.0, 110.0) } else { rng.range(24.0, 45.0) };
            let p = self.beside(i, side * lat, 0.0);
            let min_clear = if mesa { 45.0 } else { 20.0 };
            if self.clearance(p) < min_clear || placed.iter().any(|q| q.distance(p) < 45.0) || p.distance(start) < 70.0 || self.blocked(p, 6.0) {
                continue;
            }
            let hf = &self.t.terrain;
            let o = hf.origin;
            let size = hf.size();
            if p.x < o.x + 20.0 || p.z < o.y + 20.0 || p.x > o.x + size.x - 20.0 || p.z > o.y + size.y - 20.0 {
                continue;
            }
            let Some(base_y) = self.footprint(p, if mesa { 4.0 } else { 4.5 }, if mesa { 6.0 } else { 2.2 }) else { continue };
            if self.under_water(base_y, 1.2) {
                continue;
            }
            let c = Vec3::new(p.x, base_y, p.z);
            // the building's front (local -Z) looks at the road
            let face = yaw_facing(-((s.pos - c) * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero());
            let face = face + rng.range(-0.25, 0.25);
            let v = rng.range_i(0, 3) as u8;
            use PropKind::*;
            match (biome, kit % 3) {
                (Biome::Meadow, 0) => {
                    self.push(Cottage, c, face, 1.0, 3.6, v);
                    self.around(rng, c, Beehive, 2, 7.0, 9.0, 0.5, (1.0, 1.0), 12.0);
                    self.around(rng, c, Sheep, 5, 9.0, 16.0, 0.0, (0.9, 1.1), 12.0);
                    self.around(rng, c, Flowers, 4, 4.5, 7.0, 0.0, (0.8, 1.2), 10.0);
                }
                (Biome::Meadow, 1) => {
                    self.push(Barn, c, face, 1.0, 4.6, v);
                    self.around(rng, c, Cow, 4, 9.0, 16.0, 0.0, (0.9, 1.1), 12.0);
                    self.around(rng, c, HayBale, 3, 6.0, 8.0, 0.8, (1.0, 1.0), 12.0);
                }
                (Biome::Meadow, _) => {
                    self.push(Windmill, c, face, 1.0, 2.4, v);
                    self.around(rng, c, FlowerField, 2, 9.0, 13.0, 0.0, (1.0, 1.0), 12.0);
                    self.around(rng, c, Beehive, 1, 6.0, 8.0, 0.5, (1.0, 1.0), 12.0);
                }
                (Biome::Autumn, 0) => {
                    self.push(Barn, c, face, 1.0, 4.6, v);
                    self.around(rng, c, Pumpkin, 5, 6.0, 12.0, 0.0, (0.9, 1.3), 10.0);
                    self.around(rng, c, Scarecrow, 1, 8.0, 11.0, 0.0, (1.0, 1.0), 12.0);
                    self.around(rng, c, HayWagon, 1, 7.0, 9.0, 1.6, (1.0, 1.0), 14.0);
                }
                (Biome::Autumn, 1) => {
                    self.push(Cottage, c, face, 1.0, 3.6, v);
                    self.around(rng, c, Pumpkin, 4, 5.0, 9.0, 0.0, (0.8, 1.2), 10.0);
                    self.around(rng, c, Deer, 2, 12.0, 18.0, 0.0, (0.9, 1.1), 12.0);
                    self.around(rng, c, HayBale, 2, 6.0, 8.0, 0.8, (1.0, 1.0), 12.0);
                }
                (Biome::Autumn, _) => {
                    self.push(Windmill, c, face, 1.0, 2.4, v);
                    self.around(rng, c, FlowerField, 2, 9.0, 13.0, 0.0, (1.0, 1.0), 12.0);
                    self.around(rng, c, Scarecrow, 1, 8.0, 11.0, 0.0, (1.0, 1.0), 12.0);
                }
                (Biome::Snow, 0) | (Biome::Snow, 2) => {
                    self.push(SkiCabin, c, face, 1.0, 3.8, v);
                    self.around(rng, c, Snowman, 2, 5.0, 8.0, 0.5, (0.9, 1.1), 10.0);
                    self.around(rng, c, Deer, 2, 10.0, 16.0, 0.0, (0.9, 1.1), 12.0);
                }
                (Biome::Snow, _) => {
                    if let Some(y) = self.footprint(c, 5.0, 1.1) {
                        self.push(FrozenPond, Vec3::new(c.x, y, c.z), rng.range(0.0, 6.28), 1.0, 0.0, v);
                        self.around(rng, c, Snowman, 1, 7.0, 9.0, 0.5, (0.9, 1.1), 10.0);
                    } else {
                        self.around(rng, c, Snowman, 2, 0.0, 4.0, 0.5, (0.9, 1.1), 10.0);
                    }
                    self.around(rng, c, Deer, 3, 8.0, 14.0, 0.0, (0.9, 1.1), 12.0);
                }
                (Biome::Desert, 1) => {
                    let sc = rng.range(1.6, 2.6);
                    self.push(Mesa, Vec3::new(c.x, c.y - 0.5, c.z), rng.range(0.0, 6.28), sc, 4.0 * sc, v);
                }
                (Biome::Desert, _) => {
                    self.push(Adobe, c, face, 1.0, 3.4, v);
                    self.around(rng, c, Cactus, 3, 6.0, 10.0, 0.35, (0.8, 1.2), 12.0);
                    self.around(rng, c, Tumbleweed, 2, 5.0, 10.0, 0.0, (0.8, 1.2), 8.0);
                    self.around(rng, c, Cow, 2, 9.0, 14.0, 0.0, (0.9, 1.1), 12.0);
                }
                (Biome::Coast, 0) => {
                    self.push(Lighthouse, c, face, 1.0, 2.4, 0);
                    self.around(rng, c, Flowers, 3, 4.0, 7.0, 0.0, (0.8, 1.2), 10.0);
                }
                (Biome::Coast, _) => {
                    self.push(Cottage, c, face, 1.0, 3.6, 2);
                    self.around(rng, c, Sheep, 3, 9.0, 14.0, 0.0, (0.9, 1.1), 12.0);
                    self.around(rng, c, Palm, 2, 6.0, 9.0, 0.45, (0.9, 1.2), 12.0);
                }
            }
            self.keep_out.push((Vec2::new(c.x, c.z), if mesa { 14.0 } else { 9.0 }));
            placed.push(c);
            kit += 1;
        }
        // coast: boats bobbing on the water + umbrellas on the beach
        if let Some(w) = self.t.water_level {
            let mut boats = 0;
            let mut umbrellas = 0;
            let hf = &self.t.terrain;
            for _ in 0..1500 {
                if boats >= 6 && umbrellas >= 10 {
                    break;
                }
                let x = hf.origin.x + rng.range(0.1, 0.9) * hf.size().x;
                let z = hf.origin.y + rng.range(0.1, 0.9) * hf.size().y;
                let y = self.ground(x, z);
                let p = Vec3::new(x, y, z);
                if y < w - 1.5 && y > w - 6.0 && boats < 6 && self.clearance(p) > 15.0 {
                    self.push(PropKind::Boat, Vec3::new(x, w, z), rng.range(0.0, 6.28), rng.range(0.9, 1.2), 0.0, rng.range_i(0, 6) as u8);
                    boats += 1;
                } else if biome == Biome::Coast && y > w + 0.3 && y < w + 1.4 && umbrellas < 10 && self.clearance(p) > 8.0 {
                    self.push(PropKind::BeachUmbrella, p, rng.range(0.0, 6.28), 1.0, 0.0, rng.range_i(0, 6) as u8);
                    umbrellas += 1;
                }
            }
        }
    }

    /// Fences along calm stretches, a few metres beyond the shoulder.
    fn fences(&mut self, noise_seed: u32) {
        let biome = self.t.biome;
        if biome == Biome::Desert {
            return;
        }
        let n = self.n() as isize;
        let mut i = 30;
        while i < n - 30 {
            let v = noise::value2(i as f32 * 0.012, 7.7, noise_seed ^ 0x5eed);
            let s = self.smp(i);
            if v > 0.25 && s.curvature.abs() < 1.0 / 70.0 {
                let side = if noise::value2(i as f32 * 0.004, 1.3, noise_seed) > 0.0 { 1.0 } else { -1.0 };
                let a = self.beside(i, side * (s.half_width + SHOULDER + 4.0), 0.0);
                let b = self.beside(i + 2, side * (self.smp(i + 2).half_width + SHOULDER + 4.0), 0.0);
                let mid = (a + b) * 0.5;
                let d = (b - a) * Vec3::new(1.0, 0.0, 1.0);
                if self.clearance(mid) > 3.0 && !self.blocked(mid, 1.0) && !self.under_water(mid.y, 0.3) {
                    let y = self.ground(mid.x, mid.z);
                    self.push(PropKind::Fence, Vec3::new(mid.x, y, mid.z), yaw_along(d.normalize_or_zero()), d.length() / 4.0, 0.0, (i / 40) as u8);
                }
            }
            i += 2;
        }
    }

    /// Little life right beside the road: grass tufts, flowers, mushrooms, tumbleweeds.
    fn roadside(&mut self, rng: &mut Rng) {
        let biome = self.t.biome;
        let n = self.n() as isize;
        let mut i = 0;
        while i < n {
            for side in [-1.0f32, 1.0] {
                if !rng.chance(0.45) {
                    continue;
                }
                let s = self.smp(i);
                let p = self.beside(i, side * (s.half_width + SHOULDER + rng.range(1.2, 7.0)), rng.range(-1.0, 1.0));
                if self.clearance(p) < 1.0 || self.under_water(p.y, 0.3) || self.blocked(p, 0.5) {
                    continue;
                }
                let r = rng.f32();
                let kind = match biome {
                    Biome::Meadow => {
                        if r < 0.5 { PropKind::Grass } else if r < 0.85 { PropKind::Flowers } else { PropKind::Mushroom }
                    }
                    Biome::Autumn => {
                        if r < 0.55 { PropKind::Grass } else if r < 0.75 { PropKind::Mushroom } else if r < 0.88 { PropKind::Pumpkin } else { PropKind::Flowers }
                    }
                    Biome::Snow => {
                        if r < 0.7 { PropKind::Grass } else if r < 0.97 { PropKind::Bush } else { PropKind::Snowman }
                    }
                    Biome::Desert => {
                        if r < 0.6 { PropKind::Grass } else if r < 0.85 { PropKind::Tumbleweed } else { PropKind::Rock }
                    }
                    Biome::Coast => {
                        if r < 0.55 { PropKind::Grass } else { PropKind::Flowers }
                    }
                };
                let (scale, radius) = match kind {
                    PropKind::Rock => (rng.range(0.4, 0.7), 0.0),
                    PropKind::Snowman => (1.0, 0.0),
                    PropKind::Bush => (rng.range(0.5, 0.8), 0.0),
                    _ => (rng.range(0.8, 1.3), 0.0),
                };
                self.push(kind, p, rng.range(0.0, 6.28), scale, radius, rng.range_i(0, 3) as u8);
            }
            i += 3;
        }
    }

    fn scatter(&mut self, rng: &mut Rng, noise_seed: u32) {
        let t = self.t;
        let style = &t.style;
        let biome = t.biome;
        let hf = &t.terrain;
        let size = hf.size();
        let spacing = 6.5;
        let gx = (size.x / spacing) as i32;
        let gz = (size.y / spacing) as i32;
        for iz in 0..gz {
            for ix in 0..gx {
                let x = hf.origin.x + (ix as f32 + rng.range(0.1, 0.9)) * spacing;
                let z = hf.origin.y + (iz as f32 + rng.range(0.1, 0.9)) * spacing;
                let forest = noise::fbm(x * 0.012, z * 0.012, 3, noise_seed + 999) * 0.5 + 0.5;
                let mut density = style.tree_density * smoothstep(0.35, 0.75, forest) + 0.04;
                let q = t.query(Vec3::new(x, 0.0, z), None);
                let hw = t.samples[q.index].half_width;
                // thinner far from the road, where only fog sees it
                density *= 1.0 - 0.55 * smoothstep(70.0, 160.0, q.dist);
                if !rng.chance(density) {
                    continue;
                }
                if q.dist < hw + SHOULDER + 3.5 {
                    continue;
                }
                let p = Vec3::new(x, 0.0, z);
                if self.blocked(p, 1.0) {
                    continue;
                }
                let (y, nrm) = hf.sample(x, z);
                if self.under_water(y, 0.4) {
                    continue;
                }
                let steep = nrm.y < 0.8;
                let r = rng.f32();
                use PropKind::*;
                let kind = if steep {
                    Rock
                } else {
                    match biome {
                        Biome::Meadow => {
                            if r < 0.08 { Rock } else if r < 0.42 { Tree } else if r < 0.52 { Birch } else if r < 0.58 { Blossom } else if r < 0.66 { Pine } else if r < 0.78 { Bush } else if r < 0.9 { Flowers } else { Grass }
                        }
                        Biome::Autumn => {
                            if r < 0.06 { Rock } else if r < 0.5 { Tree } else if r < 0.62 { Birch } else if r < 0.72 { Pine } else if r < 0.84 { Bush } else if r < 0.9 { Mushroom } else { Grass }
                        }
                        Biome::Snow => {
                            if r < 0.15 { Rock } else if r < 0.82 { Pine } else if r < 0.9 { Birch } else if r < 0.995 { Bush } else { Snowman }
                        }
                        Biome::Desert => {
                            if r < 0.33 { Rock } else if r < 0.63 { Cactus } else if r < 0.73 { Bush } else if r < 0.8 { Palm } else if r < 0.9 { Tumbleweed } else { Grass }
                        }
                        Biome::Coast => {
                            if r < 0.12 { Rock } else if r < 0.42 { Palm } else if r < 0.58 { Tree } else if r < 0.7 { Bush } else if r < 0.82 { Flowers } else { Grass }
                        }
                    }
                };
                let tall = matches!(kind, Tree | Pine | Palm | Birch | Blossom);
                if tall && q.dist < hw + SHOULDER + 6.0 {
                    continue;
                }
                let scale = match kind {
                    Rock => rng.range(0.6, 2.2),
                    Flowers | Bush | Grass | Mushroom | Tumbleweed => rng.range(0.7, 1.3),
                    Snowman => 1.0,
                    _ => rng.range(0.8, 1.45),
                };
                let radius = match kind {
                    Tree | Pine | Palm | Blossom => 0.45 * scale,
                    Birch => 0.3 * scale,
                    Cactus => 0.35 * scale,
                    Rock => 0.9 * scale,
                    Snowman => 0.5,
                    _ => 0.0,
                };
                // never anything solid within reach of the road edge
                if radius > 0.0 && q.dist < hw + SHOULDER + radius + 1.5 {
                    continue;
                }
                self.push(kind, Vec3::new(x, y, z), rng.range(0.0, 6.283), scale, radius, rng.range_i(0, 3) as u8);
            }
        }
    }

    fn jump_cones(&mut self) {
        let n = self.n() as isize;
        for &j in &self.t.jumps {
            let smp = self.smp(j as isize - 10);
            for side in [-1.0f32, 1.0] {
                let p = smp.pos + smp.right * side * (smp.half_width + 0.6);
                let y = self.ground(p.x, p.z);
                self.props.push(Prop { kind: PropKind::Cone, pos: Vec3::new(p.x, y, p.z), rot_y: 0.0, scale: 1.0, radius: 0.0, variant: 0 });
            }
        }
        let _ = n;
    }
}
