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
    /// Rally stage/loop (default) or a tarmac racing circuit for the circuit car class
    #[serde(default)]
    pub discipline: Discipline,
    /// Night race: moonlight + headlights (visual; the same road and conditions as by day)
    #[serde(default)]
    pub night: bool,
}

/// Track discipline (shared interface).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Discipline {
    /// Rally roads (gravel/dirt/snow/mixed), current behaviour
    #[default]
    Rally,
    /// Tarmac racing circuit: wide, fast, kerbs, run-off, grandstands (closed loop, laps)
    Circuit,
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
        // COZY_STAGE=<campaign stage id> plays that point-to-point stage instead
        if let Some(st) = std::env::var("COZY_STAGE").ok().and_then(|id| crate::campaign::find_stage(&id)) {
            return st.params;
        }
        // COZY_CIRCUIT=<circuit id> plays a named circuit; COZY_DISCIPLINE=circuit a random one
        if let Some(c) = std::env::var("COZY_CIRCUIT").ok().and_then(|id| crate::circuits::find_circuit(&id)) {
            return c.params;
        }
        if std::env::var("COZY_DISCIPLINE").is_ok_and(|v| v.eq_ignore_ascii_case("circuit")) {
            self.discipline = Discipline::Circuit;
            self.surface = SurfaceMode::Tarmac;
        }
        // COZY_NIGHT=1: night race (visual testing)
        if std::env::var("COZY_NIGHT").is_ok_and(|v| v == "1") {
            self.night = true;
        }
        // COZY_WEATHER=<preset>|dynamic|stormy: fixed weather for loop races (visual testing)
        if let Ok(v) = std::env::var("COZY_WEATHER") {
            use crate::weather::{ConditionsSpec as C, WeatherPreset as P};
            self.conditions = match v.to_lowercase().as_str() {
                "overcast" => C::Fixed(P::Overcast),
                "drizzle" => C::Fixed(P::Drizzle),
                "rain" => C::Fixed(P::Rain),
                "storm" => C::Fixed(P::Storm),
                "fog" => C::Fixed(P::Fog),
                "snow" | "snowfall" => C::Fixed(P::Snowfall),
                "dynamic" => C::Dynamic,
                "stormy" => C::Storm,
                _ => self.conditions,
            };
        }
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
        TrackParams { seed: 1, biome: None, length_class: 1, difficulty: default_difficulty(), surface: SurfaceMode::BiomeDefault, layout: LayoutKind::Loop, conditions: crate::weather::ConditionsSpec::BiomeDefault, discipline: Discipline::Rally, night: false }
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
    // --- open-stage (point-to-point) furniture
    /// Gantry spanning the road (legs at local x = ±5 m * scale); variant 0 = START (countdown
    /// lights), 1 = FINISH (chequered)
    Gantry,
    /// Roadside board; variant & 7: 0 = flying FINISH, 1 = STOP control, 2 = SPLIT (number in
    /// variant >> 3), 3 = START/time control
    StageBoard,
    /// Timing hut with timekeepers
    TimingHut,
    /// Marshal with flag, radio sign and chair
    MarshalPost,
    /// Red/white safety tape on posts along local X; length = 10 m * scale
    SafetyTape,
    /// Triangular warning sign; variant 0 hairpin left, 1 hairpin right, 2 jump, 3 ford, 4 caution
    WarningSign,
    /// Bridge parapet segment along local X (4 m * scale)
    BridgeRail,
    // --- circuit furniture (Discipline::Circuit). Segments run along local X, fronts (-Z) face the track.
    /// Pit garage block, 12 m along X, with a tarmac apron in front (-Z)
    PitBuilding,
    /// Pit wall + debris fence, 4 m along X
    PitWall,
    /// Grandstand with crowd, 12 m along X
    Grandstand,
    /// Armco barrier, 4 m along X
    Armco,
    /// Tyre wall, 4 m along X
    TyreWall,
    /// Brake marker board; variant = hundreds of metres (1..3)
    BrakeBoard,
    /// Sponsor banner (made-up cozy brands), 6 m along X; variant picks the brand
    AdBanner,
    /// Footbridge over the track; legs at local x = ±5 m * scale
    TrackBridge,
    /// Striped gravel-trap marker post
    MarkerPost,
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
    /// Which part of the road cross-section this is (kerbs / run-off exist on circuits)
    pub zone: GroundZone,
    /// Circuits: outside the white line (track limits); always false on rally tracks
    pub beyond_limits: bool,
}

/// Cross-section zone under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroundZone {
    Road,
    Kerb,
    /// Shoulder / paved verge
    Shoulder,
    /// Prepared run-off (tarmac or gravel trap)
    Runoff,
    Offroad,
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
    /// Weather events (start s, end s, kind) behind the timeline, for UI / stats
    pub weather_events: Vec<(f32, f32, crate::conditions::WeatherEvent)>,
    /// false for point-to-point stages (samples do not wrap around)
    pub closed: bool,
    /// Start / finish line arc lengths (loops: 0 and `length`)
    pub start_s: f32,
    pub finish_s: f32,
    /// Split points (arc lengths) between start and finish; empty for loops
    pub splits: Vec<f32>,
    /// Coarse distance-to-road field (corridor terrain, prop budgets, render LOD / culling)
    pub road_dist: crate::corridor::RoadDistanceField,
    /// Circuits: kerbs and run-off per sample (empty on rally tracks; use `edge(i)`)
    pub circuit_edges: Vec<CircuitEdge>,
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
            let t = match params.layout {
                LayoutKind::Loop if params.discipline == Discipline::Circuit => Self::try_generate_circuit(params, attempt),
                LayoutKind::Loop => Self::try_generate(params, attempt),
                LayoutKind::PointToPoint { length_m } => Self::try_generate_stage(params, attempt, length_m),
            };
            if let Some(t) = t {
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
            conditions: crate::weather::StageConditions::default(),
            weather_events: Vec::new(),
            closed: true,
            start_s: 0.0,
            finish_s: length,
            splits: Vec::new(),
            road_dist: crate::corridor::RoadDistanceField::build(&pts, min, max, 10.0, 400.0),
            circuit_edges: Vec::new(),
            features: layout.features,
            surface_map,
            grid,
            prop_grid: SpatialGrid::new(&[Vec2::ZERO], 32.0, 1.0),
        };
        let heights: Vec<f32> = track.samples.iter().map(|s| s.pos.y).collect();
        (track.conditions, track.weather_events) = build_conditions(params, biome, &heights, 0.0, length, true);
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
                let i = self.sample_index(h as isize + k);
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
            if !self.closed && (b == 0 || a + 1 != b) {
                continue; // open road: no segment from the last sample back to the first
            }
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
                let s = self.samples[a].s + t * SAMPLE_SPACING;
                let s = if self.closed { s.rem_euclid(self.length) } else { s };
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
        if self.is_circuit() {
            return self.circuit_ground(q);
        }
        let hw = self.samples[q.index].half_width;
        let off = self.style.offroad_surface;
        let e2 = ss(hw + SHOULDER - 0.4, hw + SHOULDER + 0.4, q.dist);
        if e2 >= 1.0 {
            return GroundInfo { primary: off, secondary: off, blend: 0.0, props: off.props(), query: q, zone: GroundZone::Offroad, beyond_limits: false };
        }
        let m = self.surface_map.mix_at(q.s, q.lateral.clamp(-(hw + SHOULDER), hw + SHOULDER));
        let e1 = ss(hw + 0.1, hw + 0.7, q.dist);
        let road = SurfaceProps::lerp(m.a.props(), m.b.props(), m.t);
        let (sa, sb) = (shoulder_of(m.a), shoulder_of(m.b));
        let shoulder = SurfaceProps::lerp(sa.props(), sb.props(), m.t);
        let mut props = SurfaceProps::lerp(SurfaceProps::lerp(road, shoulder, e1), off.props(), e2);
        if !self.conditions.keys.is_empty() {
            // weather: wet roads lose grip (per surface), puddles in the ruts a bit more
            let w = self.conditions.at(q.s);
            if w.wetness > 0.0 {
                let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
                let f_road = l(w.grip_factor(m.a), w.grip_factor(m.b), m.t);
                let f_sh = l(w.grip_factor(sa), w.grip_factor(sb), m.t);
                let mut f = l(l(f_road, f_sh, e1), w.grip_factor(off), e2);
                let puddle = self.puddle_at(q.s, q.lateral) * (1.0 - e1);
                f *= 1.0 - 0.16 * puddle;
                props.grip *= f;
                props.slide_grip *= 1.0 - 0.08 * puddle;
            }
        }
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
        let zone = if q.dist <= hw + 0.4 { GroundZone::Road } else { GroundZone::Shoulder };
        GroundInfo { primary, secondary, blend: w2 / (w1 + w2).max(1e-6), props, query: q, zone, beyond_limits: false }
    }

    /// Interpolated centre-line frame at arc length s: (position, tangent, right, up)
    pub fn frame_at(&self, s: f32) -> (Vec3, Vec3, Vec3, Vec3) {
        let n = self.samples.len();
        let (i, j, t) = if self.closed {
            let f = s.rem_euclid(self.length) / SAMPLE_SPACING;
            let i = (f.floor() as usize) % n;
            (i, (i + 1) % n, f.fract())
        } else {
            let f = (s / SAMPLE_SPACING).clamp(0.0, (n - 1) as f32);
            let i = (f.floor() as usize).min(n - 2);
            (i, i + 1, (f - i as f32).clamp(0.0, 1.0))
        };
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
                // a lone stage car starts centred right at the line
                let side = if count == 1 && !self.closed { 0.0 } else if k % 2 == 0 { -1.0 } else { 1.0 };
                let circuit = self.is_circuit();
                // circuits: staggered grid boxes, one car every 7 m alternating sides
                let back = if count == 1 && !self.closed { 3.0 } else if circuit { 6.0 + k as f32 * 7.0 } else { 8.0 + row as f32 * 8.0 + (k % 2) as f32 * 3.0 };
                let s = self.start_s - back;
                let (p, _, r, _) = self.frame_at(s);
                let lat = if circuit { self.samples[self.sample_index((s / SAMPLE_SPACING) as isize)].half_width * 0.42 } else { 2.6 };
                let mut pos = p + r * side * lat;
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

    /// Closed loop (laps) or open point-to-point stage
    pub fn is_loop(&self) -> bool {
        self.closed
    }

    /// Distance between start and finish line (loops: one lap)
    pub fn stage_length(&self) -> f32 {
        self.finish_s - self.start_s
    }

    /// Sample index of the start line (0 on loops)
    pub fn start_index(&self) -> usize {
        ((self.start_s / SAMPLE_SPACING).round() as usize).min(self.samples.len() - 1)
    }

    /// Sample index of the finish line (0 on loops = the start line)
    pub fn finish_index(&self) -> usize {
        if self.closed {
            0
        } else {
            ((self.finish_s / SAMPLE_SPACING).round() as usize).min(self.samples.len() - 1)
        }
    }

    /// Sample index for a possibly out-of-range index: wraps on loops, clamps on stages.
    /// Use this instead of `% n` for neighbour lookups.
    pub fn sample_index(&self, i: isize) -> usize {
        let n = self.samples.len() as isize;
        if self.closed { i.rem_euclid(n) as usize } else { i.clamp(0, n - 1) as usize }
    }

    /// Arc length moved forward by `d` (wraps on loops, clamps to the road on stages)
    pub fn s_ahead(&self, s: f32, d: f32) -> f32 {
        if self.closed { (s + d).rem_euclid(self.length) } else { (s + d).clamp(0.0, self.length) }
    }

    /// Fraction of the stage covered at arc length `s` (0 at the start line, 1 at the finish)
    pub fn stage_progress(&self, s: f32) -> f32 {
        ((s - self.start_s) / self.stage_length().max(1.0)).clamp(0.0, 1.0)
    }

    /// Approximate XZ distance to the road centre line (coarse field, cheap)
    pub fn road_distance(&self, x: f32, z: f32) -> f32 {
        self.road_dist.at(x, z)
    }

    /// Weather at arc length s
    pub fn weather_at(&self, s: f32) -> crate::weather::Weather {
        self.conditions.at(s)
    }

    /// Standing water 0..1 at (s, lateral): puddles collect in the wheel ruts when the road is
    /// wet (0 when dry). Physics and rendering share it.
    pub fn puddle_at(&self, s: f32, lat: f32) -> f32 {
        if self.conditions.keys.is_empty() {
            return 0.0;
        }
        let wet = self.conditions.at(s).wetness;
        if wet < 0.25 {
            return 0.0;
        }
        let i = ((s / SAMPLE_SPACING) as isize).clamp(0, self.samples.len() as isize - 1) as usize;
        let hw = self.samples[i].half_width;
        let rut = (lat.abs() - hw * 0.42).abs();
        let in_rut = 1.0 - smoothstep(0.5, 1.4, rut);
        let n = noise::value2(s * 0.09, lat * 0.6, self.params.seed as u32 ^ 0x9D11) * 0.5 + 0.5;
        let blob = smoothstep(0.55, 0.7, n);
        (blob * in_rut * smoothstep(0.25, 0.7, wet)).clamp(0.0, 1.0)
    }

    /// Grip multiplier of the road surface at (s, lateral) including weather and puddles
    /// (what the racing line / AI should plan with).
    pub fn road_grip_at(&self, s: f32, lat: f32) -> f32 {
        let m = self.surface_map.mix_at(s, lat);
        let g = m.a.props().grip + (m.b.props().grip - m.a.props().grip) * m.t;
        if self.conditions.keys.is_empty() {
            return g;
        }
        let w = self.conditions.at(s);
        let f = w.grip_factor(m.a) + (w.grip_factor(m.b) - w.grip_factor(m.a)) * m.t;
        g * f * (1.0 - 0.16 * self.puddle_at(s, lat))
    }
}

/// Conditions timeline for a track from its params.
fn build_conditions(params: &TrackParams, biome: Biome, heights: &[f32], start_s: f32, finish_s: f32, closed: bool) -> (crate::weather::StageConditions, Vec<(f32, f32, crate::conditions::WeatherEvent)>) {
    use crate::weather::{ConditionsSpec, StageConditions};
    match params.conditions {
        ConditionsSpec::BiomeDefault => (StageConditions::default(), Vec::new()),
        ConditionsSpec::Fixed(p) => (StageConditions::uniform(p.weather()), Vec::new()),
        ConditionsSpec::Dynamic | ConditionsSpec::Storm => crate::conditions::generate(&crate::conditions::ConditionsInput {
            seed: params.seed,
            biome,
            difficulty: params.difficulty,
            storm: params.conditions == ConditionsSpec::Storm,
            heights,
            spacing: SAMPLE_SPACING,
            start_s,
            finish_s,
            closed,
        }),
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
// =============================================================================================
// Tarmac racing circuits (Discipline::Circuit)
// =============================================================================================

/// What lies beyond the paved verge on one side of a circuit sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunoffKind {
    /// Grass verge (no prepared run-off)
    #[default]
    Grass,
    /// Paved run-off area
    Tarmac,
    /// Gravel trap: deep loose gravel that slows cars down hard (terrible for slicks)
    Gravel,
}

/// Run-off area on one side: kind and width (m) beyond the paved verge (`half_width + SHOULDER`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Runoff {
    pub kind: RunoffKind,
    pub width: f32,
}

/// Extra circuit data per sample (all zero / grass on rally tracks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CircuitEdge {
    /// Kerb width (m) just outside the white line on the left / right (0 = no kerb).
    /// Kerbs span `half_width .. half_width + kerb` laterally.
    pub kerb_left: f32,
    pub kerb_right: f32,
    pub runoff_left: Runoff,
    pub runoff_right: Runoff,
}

/// Surface properties of zones that have no own `Surface` variant.
pub fn kerb_props() -> SurfaceProps {
    SurfaceProps { bumpiness: 0.022, rolling: 0.014, ..Surface::Asphalt.props() }
}

pub fn gravel_trap_props() -> SurfaceProps {
    SurfaceProps { grip: 0.5, slide_grip: 0.72, rolling: 0.17, bumpiness: 0.035, ..Surface::Gravel.props() }
}

/// Reference circuit car for grading / budgets: slick grip and downforce per mass
/// (v^2 = mu g R / (1 - mu c R), c = k/m with downforce F = k v^2).
pub const CIRCUIT_REF_MU: f32 = 1.5;
pub const CIRCUIT_REF_AERO: f32 = 0.0009;

/// Max cornering speed (m/s) of the reference circuit car on radius `r` (m), friction `mu`.
pub fn circuit_corner_speed(r: f32, mu: f32) -> f32 {
    let denom = 1.0 - mu * CIRCUIT_REF_AERO * r;
    if denom <= 0.08 {
        return 95.0;
    }
    (mu * 9.81 * r / denom).sqrt().min(95.0)
}

impl Track {
    /// Tarmac racing circuit (kerbs, run-off, pit straight, track limits)
    pub fn is_circuit(&self) -> bool {
        self.params.discipline == Discipline::Circuit
    }

    /// Circuit data at a sample (kerbs / run-off); default (none) on rally tracks
    pub fn edge(&self, i: usize) -> CircuitEdge {
        self.circuit_edges.get(i).copied().unwrap_or_default()
    }

    fn try_generate_circuit(params: &TrackParams, attempt: u64) -> Option<Track> {
        let mut rng = Rng::new(params.seed.wrapping_mul(31).wrapping_add(attempt * 7919) ^ 0xC1C0_17);
        let biome = params.biome.unwrap_or_else(|| Biome::from_index((Rng::new(params.seed).next_u64() % 5) as usize));
        let mut style = biome.style();
        let style0 = biome.style();
        let noise_seed = (params.seed as u32).wrapping_mul(2654435761);
        let diff = params.difficulty.min(2);
        let size = params.length_class.min(2);
        let ring = size == 2;
        // 12-16 m wide (the Ring is narrower and rougher), pit straight wider still
        let (hw_lo, hw_hi) = [(6.2, 7.2), (6.5, 7.8), (5.0, 6.3)][size as usize];
        let half_w_base = hw_hi + 1.6;
        let nat = |x: f32, z: f32| natural_height(&style0, biome, noise_seed, x, z);

        let mut lay = None;
        for _ in 0..200 {
            if let Some(l) = crate::layout::design_circuit(&mut rng, size, diff, SAMPLE_SPACING, hw_hi, SHOULDER) {
                lay = Some(l);
                break;
            }
        }
        let lay = lay?;
        let pts = lay.pts;
        let n = pts.len();
        let length = n as f32 * SAMPLE_SPACING;
        let raw = signed_curvature_2d(&pts);
        if raw.iter().any(|c| c.abs() > 1.0 / 12.0) {
            return None;
        }
        let mut curv = raw;
        smooth_closed_f32(&mut curv, 2, 2);
        let grid = SpatialGrid::new(&pts, 16.0, 60.0);

        // ---- elevation: gentle parkland circuits, a hilly "Ring"
        let amp = if ring { 2.2 } else { 0.4 };
        let mut elev: Vec<f32> = pts.iter().map(|p| nat(p.x, p.y) * amp).collect();
        smooth_closed_f32(&mut elev, if ring { 10 } else { 22 }, 3);
        slope_limit(&mut elev, if ring { 0.11 } else { 0.05 } * SAMPLE_SPACING);
        smooth_closed_f32(&mut elev, 4, 2);
        // the Ring: wide crests (some go light at speed), compressions, blind brows before bends
        let mut jumps = Vec::new();
        if ring {
            let feats = &lay.features;
            let mut bumps: Vec<(f32, f32, f32)> = Vec::new();
            let count = (length / 650.0) as usize;
            let mut tries = 0;
            while bumps.len() < count && tries < 400 {
                tries += 1;
                let f = &feats[rng.range_i(0, feats.len() as i32) as usize];
                let crest = rng.chance(0.62);
                let c = if f.kind.is_corner() && crest { f.s0 - rng.range(10.0, 40.0) } else { f.s0 + rng.range(0.0, f.len(length).max(1.0)) }.rem_euclid(length);
                let w = rng.range(45.0, 85.0);
                if c.min(length - c) < 350.0 || bumps.iter().any(|b| { let d = (b.0 - c).rem_euclid(length); d.min(length - d) < b.2 + w + 30.0 }) {
                    continue;
                }
                // speed there: reference car on the local curvature (straights ~ 75 m/s)
                let i = (c / SAMPLE_SPACING) as usize % n;
                let k = (-15..=15).map(|o: isize| curv[(i as isize + o).rem_euclid(n as isize) as usize].abs()).fold(1e-4, f32::max);
                let v = circuit_corner_speed(1.0 / k, CIRCUIT_REF_MU).min(78.0);
                let g = if crest && rng.chance(0.35) { 1.15 } else { 0.8 }; // a few "flying" brows
                let a_max = g * 9.81 * 2.0 * w * w / (std::f32::consts::PI.powi(2) * v * v);
                let a = rng.range(1.2, 3.2).min(a_max);
                if a < 0.4 {
                    continue;
                }
                bumps.push((c, if crest { a } else { -a.min(2.2) }, w));
            }
            for &(c, a, w) in &bumps {
                let r = (w / SAMPLE_SPACING).ceil() as isize;
                let ci = (c / SAMPLE_SPACING).round() as isize;
                for k in -r..=r {
                    let x = (k as f32 * SAMPLE_SPACING / w).clamp(-1.0, 1.0);
                    elev[(ci + k).rem_euclid(n as isize) as usize] += a * 0.5 * (1.0 + (x * std::f32::consts::PI).cos());
                }
                if a > 1.5 {
                    jumps.push(((c / SAMPLE_SPACING) as usize) % n);
                }
            }
            slope_limit(&mut elev, 0.14 * SAMPLE_SPACING);
        }
        // level pit straight around the start/finish line
        let e0 = elev[0];
        for k in -170isize..=170 {
            let i = k.rem_euclid(n as isize) as usize;
            let t = smoothstep(170.0, 110.0, k.unsigned_abs() as f32);
            elev[i] += (e0 - elev[i]) * t;
        }

        // ---- width, banking, samples (all tarmac)
        let mut wn: Vec<f32> = (0..n).map(|i| noise::value2(i as f32 * 0.012, 7.7, noise_seed) * 0.5 + 0.5).collect();
        smooth_closed_f32(&mut wn, 10, 2);
        let hws: Vec<f32> = (0..n)
            .map(|i| {
                let pit = smoothstep(320.0, 220.0, (i.min(n - i)) as f32 * SAMPLE_SPACING);
                hw_lo + (hw_hi - hw_lo) * wn[i] + pit * 1.5
            })
            .collect();
        let mut bank: Vec<f32> = (0..n)
            .map(|i| {
                let cam = lay.camber[i];
                if cam > 1.5 { (curv[i] * 30.0).clamp(-0.26, 0.26) } else { (curv[i] * 18.0).clamp(-0.05, 0.05) }
            })
            .collect();
        smooth_closed_f32(&mut bank, 6, 2);
        style.road_surface = Surface::Asphalt;
        style.road = crate::surfacemap::road_color(biome, Surface::Asphalt);
        style.road_edge = [0.95, 0.95, 0.92];
        style.shoulder = [0.46, 0.46, 0.5];
        let surface_map = crate::surfacemap::SurfaceMap::uniform(Surface::Asphalt, length);
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let p = Vec3::new(pts[i].x, elev[i], pts[i].y);
            let j1 = (i + 1) % n;
            let j0 = (i + n - 1) % n;
            let tangent = (Vec3::new(pts[j1].x, elev[j1], pts[j1].y) - Vec3::new(pts[j0].x, elev[j0], pts[j0].y)).normalize();
            let flat_right = tangent.cross(Vec3::Y).normalize();
            let right = (flat_right * bank[i].cos() + Vec3::Y * bank[i].sin()).normalize();
            let up = right.cross(tangent).normalize();
            samples.push(TrackSample {
                pos: p,
                tangent,
                right,
                up,
                s: i as f32 * SAMPLE_SPACING,
                half_width: hws[i],
                bank: bank[i],
                curvature: curv[i],
                surface: Surface::Asphalt,
                surface_next: Surface::Asphalt,
                surface_blend: 0.0,
            });
        }

        // ---- kerbs and run-off from the corners
        let edges = circuit_edges(&samples, &pts, &grid, &mut rng);

        // ---- terrain: corridor detail (run-off kept flat), coarse far away
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for p in &pts {
            min = min.min(*p);
            max = max.max(*p);
        }
        let margin = 170.0;
        min -= Vec2::splat(margin);
        max += Vec2::splat(margin);
        let road_dist = crate::corridor::RoadDistanceField::build(&pts, min, max, 10.0, 600.0);
        let nx = ((max.x - min.x) / TERRAIN_CELL).ceil() as usize + 1;
        let nz = ((max.y - min.y) / TERRAIN_CELL).ceil() as usize + 1;
        let mut hf = Heightfield::new(min, TERRAIN_CELL, nx, nz);
        let cs = 4usize;
        let (cnx, cnz) = (nx / cs + 2, nz / cs + 2);
        let coarse: Vec<f32> = (0..cnx * cnz).map(|k| nat(min.x + ((k % cnx) * cs) as f32 * TERRAIN_CELL, min.y + ((k / cnx) * cs) as f32 * TERRAIN_CELL) * amp).collect();
        let max_runoff = 34.0;
        let road_zone = BLEND + half_w_base + SHOULDER + max_runoff;
        let mut candidates: Vec<(usize, f32)> = Vec::with_capacity(96);
        for iz in 0..nz {
            for ix in 0..nx {
                let x = min.x + ix as f32 * TERRAIN_CELL;
                let z = min.y + iz as f32 * TERRAIN_CELL;
                let dist = road_dist.at(x, z);
                let id = hf.idx(ix, iz);
                if dist > STAGE_TERRAIN_CORRIDOR {
                    let (cx, cz) = (ix / cs, iz / cs);
                    let (fx, fz) = ((ix % cs) as f32 / cs as f32, (iz % cs) as f32 / cs as f32);
                    let c = |a: usize, b: usize| coarse[b.min(cnz - 1) * cnx + a.min(cnx - 1)];
                    let h0 = c(cx, cz) + (c(cx + 1, cz) - c(cx, cz)) * fx;
                    let h1 = c(cx, cz + 1) + (c(cx + 1, cz + 1) - c(cx, cz + 1)) * fx;
                    hf.heights[id] = h0 + (h1 - h0) * fz;
                    continue;
                }
                let nat_h = nat(x, z) * amp;
                let mut h = nat_h;
                if dist < road_zone + 12.0 {
                    let q = Vec2::new(x, z);
                    candidates.clear();
                    grid.for_each_near(q, road_zone, |j| {
                        let d = pts[j as usize].distance_squared(q);
                        if d < road_zone * road_zone {
                            candidates.push((j as usize, d));
                        }
                    });
                    if !candidates.is_empty() {
                        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                        let mut sections: Vec<usize> = Vec::new();
                        for &(j, _) in candidates.iter() {
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
                        let mut hsum = nat_h;
                        let mut exact = None;
                        for &j in &sections {
                            let (lat, along_h, hw) = road_profile_at(&samples, j, q);
                            let e = edges[j];
                            let ro = if lat < 0.0 { e.runoff_left.width } else { e.runoff_right.width };
                            let core = hw + SHOULDER + ro;
                            let t = smoothstep(core, core + BLEND, lat.abs());
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
                }
                hf.heights[id] = h;
            }
        }
        let water_level = if style.has_water {
            let min_road = elev.iter().cloned().fold(f32::MAX, f32::min);
            Some(min_road - 3.5)
        } else {
            None
        };
        let n_cp = if ring { 16 } else { 8 };
        let checkpoints = (0..n_cp).map(|k| k * n / n_cp).collect();
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
            conditions: crate::weather::StageConditions::default(),
            weather_events: Vec::new(),
            closed: true,
            start_s: 0.0,
            finish_s: length,
            splits: Vec::new(),
            road_dist,
            circuit_edges: edges,
            features: lay.features,
            surface_map,
            grid,
            prop_grid: SpatialGrid::new(&[Vec2::ZERO], 32.0, 1.0),
        };
        let heights: Vec<f32> = track.samples.iter().map(|s| s.pos.y).collect();
        (track.conditions, track.weather_events) = build_conditions(params, biome, &heights, 0.0, length, true);
        track.place_props(&mut rng, noise_seed);
        // circuit hygiene: no rally furniture, nothing solid on the track side of the run-off
        let mut props = std::mem::take(&mut track.props);
        let landmark = |k: PropKind| matches!(k, PropKind::Windmill | PropKind::Lighthouse | PropKind::Mesa | PropKind::Barn | PropKind::Cottage | PropKind::SkiCabin);
        props.retain(|p| {
            if matches!(p.kind, PropKind::Chevron | PropKind::ArrowSign | PropKind::HayBale | PropKind::Cone) {
                return false;
            }
            let q = track.query(p.pos, None);
            let e = track.edge(q.index);
            let ro = if q.lateral < 0.0 { e.runoff_left.width } else { e.runoff_right.width };
            let clear = track.samples[q.index].half_width + SHOULDER + ro + if p.radius > 0.0 || p.kind == PropKind::Spectator { 4.0 } else { 1.0 };
            q.dist > clear && (q.dist < STAGE_PROP_CORRIDOR || (landmark(p.kind) && q.dist < 420.0))
        });
        let pp: Vec<Vec2> = props.iter().map(|p| Vec2::new(p.pos.x, p.pos.z)).collect();
        if !pp.is_empty() {
            track.prop_grid = SpatialGrid::new(&pp, 12.0, 10.0);
        }
        track.props = props;
        Some(track)
    }

    /// Circuit ground zones: track (inside the white line), kerb, paved verge, run-off
    /// (tarmac or gravel trap), grass. Smooth 0.3-0.6 m blends between zones.
    fn circuit_ground(&self, q: TrackQuery) -> GroundInfo {
        use crate::surfacemap::smoothstep as ss;
        let smp = &self.samples[q.index];
        let hw = smp.half_width;
        let e = self.edge(q.index);
        let left = q.lateral < 0.0;
        let kerb = if left { e.kerb_left } else { e.kerb_right };
        let ro = if left { e.runoff_left } else { e.runoff_right };
        let d = q.dist;
        let road = Surface::Asphalt.props();
        let verge = SurfaceProps { grip: road.grip * 0.97, bumpiness: 0.006, ..road };
        let ro_props = match ro.kind {
            RunoffKind::Tarmac => road,
            RunoffKind::Gravel => gravel_trap_props(),
            RunoffKind::Grass => Surface::Grass.props(),
        };
        let grass = self.style.offroad_surface;
        let mut props = road;
        let mut zone = GroundZone::Road;
        let mut primary = Surface::Asphalt;
        let mut secondary = Surface::Asphalt;
        let mut blend = 0.0;
        if kerb > 0.0 {
            let t = ss(hw - 0.05, hw + 0.25, d) * (1.0 - ss(hw + kerb - 0.1, hw + kerb + 0.3, d));
            props = SurfaceProps::lerp(props, kerb_props(), t);
            if t > 0.5 {
                zone = GroundZone::Kerb;
            }
        }
        let t_v = ss(hw + kerb.max(0.2) - 0.1, hw + kerb.max(0.2) + 0.3, d);
        props = SurfaceProps::lerp(props, verge, t_v);
        if t_v > 0.5 {
            zone = GroundZone::Shoulder;
        }
        let edge = hw + SHOULDER;
        let t_r = ss(edge - 0.3, edge + 0.3, d);
        props = SurfaceProps::lerp(props, ro_props, t_r);
        if t_r > 0.0 && ro.kind != RunoffKind::Tarmac {
            secondary = if ro.kind == RunoffKind::Gravel { Surface::Gravel } else { grass };
            blend = t_r.min(0.5);
        }
        if t_r > 0.5 {
            zone = if ro.width > 0.5 { GroundZone::Runoff } else { GroundZone::Offroad };
            if ro.kind != RunoffKind::Tarmac {
                std::mem::swap(&mut primary, &mut secondary);
                blend = 1.0 - t_r.max(0.5);
            }
        }
        let t_g = ss(edge + ro.width - 0.3, edge + ro.width + 0.3, d);
        if ro.width > 0.5 && t_g > 0.0 {
            props = SurfaceProps::lerp(props, grass.props(), t_g);
            if t_g > 0.5 {
                zone = GroundZone::Offroad;
                primary = grass;
                secondary = grass;
                blend = 0.0;
            }
        }
        if !self.conditions.keys.is_empty() {
            let w = self.conditions.at(q.s);
            if w.wetness > 0.0 {
                props.grip *= w.grip_factor(primary);
            }
        }
        GroundInfo { primary, secondary, blend, props, query: q, zone, beyond_limits: d > hw + 0.25 }
    }
}

/// Kerbs at turn-in / apex / exit and run-off on the outside of corners. Run-off width is
/// limited so it never reaches another part of the circuit.
fn circuit_edges(samples: &[TrackSample], pts: &[Vec2], grid: &SpatialGrid, rng: &mut Rng) -> Vec<CircuitEdge> {
    let n = samples.len();
    let mut edges = vec![CircuitEdge::default(); n];
    let k_on = 1.0 / 260.0;
    let idx = |i: isize| i.rem_euclid(n as isize) as usize;
    // corner runs
    let start = (0..n).find(|&i| samples[i].curvature.abs() < k_on).unwrap_or(0);
    let mut runs: Vec<(usize, usize, f32)> = Vec::new();
    let mut cur: Option<(usize, usize, f32)> = None;
    for o in 0..=n {
        let i = start + o;
        let k = samples[i % n].curvature;
        let on = k.abs() >= k_on && o < n;
        match cur.as_mut() {
            Some(r) if on && k.signum() == r.2 => r.1 = i,
            _ => {
                if let Some(r) = cur.take() {
                    runs.push(r);
                }
                if on {
                    cur = Some((i, i, k.signum()));
                }
            }
        }
    }
    let mut want_ro = vec![(Runoff::default(), Runoff::default()); n];
    for &(a, b, sign) in &runs {
        let len = (b - a + 1) as f32;
        let peak = (a..=b).map(|i| samples[i % n].curvature.abs()).fold(0.0, f32::max);
        let r = 1.0 / peak.max(1e-4);
        if r > 420.0 || len < 4.0 {
            continue; // flat kink: no kerbs
        }
        let apex = (a..=b).max_by(|&x, &y| samples[x % n].curvature.abs().partial_cmp(&samples[y % n].curvature.abs()).unwrap()).unwrap();
        // inside = left for left turns (curvature > 0)
        let inside_left = sign > 0.0;
        let set = |e: &mut CircuitEdge, left: bool, w: f32| {
            if left {
                e.kerb_left = e.kerb_left.max(w);
            } else {
                e.kerb_right = e.kerb_right.max(w);
            }
        };
        let kw = if r < 30.0 { 1.2 } else { 1.6 };
        let span = ((len * 0.3) as isize).max(5);
        for k in -span..=span {
            set(&mut edges[idx(apex as isize + k)], inside_left, kw);
        }
        // turn-in (outside) and exit (outside, longer)
        for k in -4..=((len * 0.15) as isize).max(3) {
            set(&mut edges[idx(a as isize + k)], !inside_left, 1.2);
        }
        let exit_len = ((len * 0.3) as isize + 10).min(30);
        for k in -((len * 0.25) as isize)..=exit_len {
            set(&mut edges[idx(b as isize + k)], !inside_left, kw + 0.2);
        }
        // run-off outside, sized by the reference speed
        let v = circuit_corner_speed(r, CIRCUIT_REF_MU) * 3.6;
        let kind = if v > 150.0 { if rng.chance(0.6) { RunoffKind::Gravel } else { RunoffKind::Tarmac } } else if rng.chance(0.7) { RunoffKind::Tarmac } else { RunoffKind::Gravel };
        let width = (v * 0.13).clamp(12.0, 32.0);
        for k in -12..=((len as isize) + 25) {
            let i = idx(a as isize + k);
            let ro = Runoff { kind, width };
            if inside_left {
                if want_ro[i].1.width < width {
                    want_ro[i].1 = ro;
                }
            } else if want_ro[i].0.width < width {
                want_ro[i].0 = ro;
            }
        }
        // a little paved inside run-off at slow corners
        if v < 110.0 {
            for k in 0..=(len as isize) {
                let i = idx(a as isize + k);
                let ro = Runoff { kind: RunoffKind::Tarmac, width: 6.0 };
                if inside_left {
                    if want_ro[i].0.width < 6.0 {
                        want_ro[i].0 = ro;
                    }
                } else if want_ro[i].1.width < 6.0 {
                    want_ro[i].1 = ro;
                }
            }
        }
    }
    // limit by the space to other parts of the circuit; feather the wanted widths along s
    let mut lim = vec![[60.0f32; 2]; n];
    for i in 0..n {
        let smp = &samples[i];
        let mut space = [60.0f32, 60.0f32]; // left, right
        grid.for_each_near(pts[i], 80.0, |j| {
            let j = j as usize;
            let d = (i as isize - j as isize).unsigned_abs();
            if d.min(n - d) < 40 {
                return;
            }
            let off = pts[j] - pts[i];
            let r2 = Vec2::new(smp.right.x, smp.right.z).normalize_or_zero();
            let side = if off.dot(r2) < 0.0 { 0 } else { 1 };
            space[side] = space[side].min(off.length() * 0.5);
        });
        for s in 0..2 {
            lim[i][s] = (space[s] - smp.half_width - SHOULDER - 3.0).max(0.0);
        }
    }
    for side in 0..2 {
        let mut w: Vec<f32> = want_ro.iter().map(|r| if side == 0 { r.0.width } else { r.1.width }).collect();
        smooth_closed_f32(&mut w, 5, 2);
        for i in 0..n {
            let mut ro = if side == 0 { want_ro[i].0 } else { want_ro[i].1 };
            if ro.width == 0.0 && w[i] > 0.5 {
                // feathered edge of a neighbouring run-off: take its kind
                let k = (1..=12).find_map(|o| {
                    let a = if side == 0 { want_ro[idx(i as isize - o)].0 } else { want_ro[idx(i as isize - o)].1 };
                    let b = if side == 0 { want_ro[idx(i as isize + o)].0 } else { want_ro[idx(i as isize + o)].1 };
                    if a.width > 0.0 { Some(a.kind) } else if b.width > 0.0 { Some(b.kind) } else { None }
                });
                ro.kind = k.unwrap_or_default();
            }
            ro.width = w[i].min(lim[i][side]);
            if side == 0 {
                edges[i].runoff_left = ro;
            } else {
                edges[i].runoff_right = ro;
            }
        }
    }
    edges
}

// =============================================================================================
// Point-to-point stages
// =============================================================================================

fn smooth_open_f32(v: &mut [f32], radius: usize, passes: usize) {
    let n = v.len() as isize;
    let mut tmp = v.to_vec();
    for _ in 0..passes {
        for i in 0..n {
            let mut s = 0.0;
            for k in -(radius as isize)..=(radius as isize) {
                s += v[(i + k).clamp(0, n - 1) as usize];
            }
            tmp[i as usize] = s / (2 * radius + 1) as f32;
        }
        v.copy_from_slice(&tmp);
    }
}

fn slope_limit_open(elev: &mut [f32], max_step: f32) {
    let n = elev.len();
    for _ in 0..4 {
        for i in 1..n {
            let prev = elev[i - 1];
            elev[i] = elev[i].clamp(prev - max_step, prev + max_step);
        }
        for i in (0..n - 1).rev() {
            let next = elev[i + 1];
            elev[i] = elev[i].clamp(next - max_step, next + max_step);
        }
    }
}

/// Props further than this from the road are dropped on stages (fog hides them anyway)
pub const STAGE_PROP_CORRIDOR: f32 = 170.0;
/// Terrain further than this from the road is built from a coarse 10 m height grid
pub const STAGE_TERRAIN_CORRIDOR: f32 = 170.0;

impl Track {
    fn try_generate_stage(params: &TrackParams, attempt: u64, stage_len: f32) -> Option<Track> {
        let mut rng = Rng::new(params.seed.wrapping_mul(31).wrapping_add(attempt * 7919) ^ 0x5A6E_0001);
        let biome = params.biome.unwrap_or_else(|| Biome::from_index((Rng::new(params.seed).next_u64() % 5) as usize));
        let mut style = biome.style();
        let style0 = biome.style();
        let noise_seed = (params.seed as u32).wrapping_mul(2654435761);
        let diff = params.difficulty.min(2);
        let (hw_narrow, hw_wide) = [(3.75, 5.6), (3.5, 5.3), (3.3, 5.0)][diff as usize];
        let half_w_base = hw_wide + 0.6;
        let stage_len = stage_len.clamp(800.0, 12000.0);
        let nat = |x: f32, z: f32| natural_height(&style0, biome, noise_seed, x, z);

        // ---- layout
        let mut lay = None;
        for _ in 0..30 {
            if let Some(l) = crate::layout::design_stage(&mut rng, diff, stage_len, SAMPLE_SPACING, half_w_base, SHOULDER, &nat) {
                lay = Some(l);
                break;
            }
        }
        let lay = lay?;
        let pts = lay.pts;
        let n = pts.len();
        let length = (n - 1) as f32 * SAMPLE_SPACING;
        let (start_s, finish_s) = (lay.start_s, lay.finish_s);
        let mut curv = signed_curvature_2d(&pts);
        curv[0] = curv[1];
        curv[n - 1] = curv[n - 2];
        if curv.iter().any(|c| c.abs() > 1.0 / 10.5) {
            return None;
        }
        smooth_open_f32(&mut curv, 2, 2);
        let grid = SpatialGrid::new(&pts, 16.0, 60.0);

        // ---- elevation: real terrain, designed climbs (switchbacks), crests, dips, jumps
        let mut elev: Vec<f32> = pts.iter().map(|p| nat(p.x, p.y)).collect();
        smooth_open_f32(&mut elev, 12, 3);
        slope_limit_open(&mut elev, 0.11 * SAMPLE_SPACING);
        smooth_open_f32(&mut elev, 3, 2);
        for &(a, b, rise) in &lay.climbs {
            // ramp up through the hairpins, then ease back to the natural ground (5% grade)
            let relax = rise.abs() / 0.05;
            for (i, e) in elev.iter_mut().enumerate() {
                let s = i as f32 * SAMPLE_SPACING;
                *e += rise * smoothstep(a, b, s) * (1.0 - smoothstep(b + 60.0, b + 60.0 + relax, s));
            }
        }
        let jumps = crate::layout::shape_elevation_ex(&mut elev, &lay.features, &curv, &mut rng, diff, SAMPLE_SPACING, Some((start_s, finish_s)));
        // flat start area and a level finish line
        for (zone_c, half) in [(start_s, 45.0f32), (finish_s, 30.0)] {
            let i0 = ((zone_c / SAMPLE_SPACING) as usize).min(n - 1);
            let e0 = elev[i0];
            for (i, e) in elev.iter_mut().enumerate() {
                let d = (i as f32 * SAMPLE_SPACING - zone_c).abs();
                let t = smoothstep(half * 1.6, half, d);
                *e += (e0 - *e) * t;
            }
        }
        {
            // the start area before the line is level with the line
            let i0 = ((start_s / SAMPLE_SPACING) as usize).min(n - 1);
            let e0 = elev[i0];
            for e in elev.iter_mut().take(i0) {
                *e = e0;
            }
            // the run-off rolls out gently
            let fi = ((finish_s / SAMPLE_SPACING) as usize).min(n - 1);
            for i in fi + 1..n {
                elev[i] = elev[i].clamp(elev[i - 1] - 0.06 * SAMPLE_SPACING, elev[i - 1] + 0.06 * SAMPLE_SPACING);
            }
        }

        // ---- width, surfaces, banking, samples
        let mut tech = lay.tech.clone();
        smooth_open_f32(&mut tech, 6, 2);
        let mut width_noise: Vec<f32> = (0..n).map(|i| noise::value2(i as f32 * 0.03, 3.3, noise_seed) * 0.25).collect();
        smooth_open_f32(&mut width_noise, 4, 1);
        let hws: Vec<f32> = (0..n)
            .map(|i| {
                let s = i as f32 * SAMPLE_SPACING;
                let area = smoothstep(45.0, 15.0, (s - start_s).abs()).max(smoothstep(finish_s - 20.0, finish_s + 10.0, s));
                (hw_wide + (hw_narrow - hw_wide) * tech[i].clamp(0.0, 1.0) + width_noise[i]).max(3.25) + area
            })
            .collect();
        let splan = crate::surfacemap::plan_ex(biome, params.surface, diff, &lay.features, &jumps, &hws, SAMPLE_SPACING, &mut rng, Some((start_s, finish_s)));
        if splan.main != style.road_surface {
            style.road = crate::surfacemap::road_color(biome, splan.main);
            style.road_edge = [style.road[0] * 0.85, style.road[1] * 0.85, style.road[2] * 0.85];
            style.road_surface = splan.main;
        }
        let surface_map = splan.map;
        let mut bank: Vec<f32> = (0..n)
            .map(|i| {
                let b = (curv[i] * 24.0).clamp(-0.14, 0.14);
                let cam = lay.camber[i];
                if cam >= 0.0 { b * cam } else { -b * 0.45 }
            })
            .collect();
        smooth_open_f32(&mut bank, 5, 2);
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let p = Vec3::new(pts[i].x, elev[i], pts[i].y);
            let j1 = (i + 1).min(n - 1);
            let j0 = i.saturating_sub(1);
            let pn = Vec3::new(pts[j1].x, elev[j1], pts[j1].y);
            let pp = Vec3::new(pts[j0].x, elev[j0], pts[j0].y);
            let tangent = (pn - pp).normalize();
            let flat_right = tangent.cross(Vec3::Y).normalize();
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

        // ---- terrain: full detail in a corridor around the road, coarse further out
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for p in &pts {
            min = min.min(*p);
            max = max.max(*p);
        }
        let margin = 170.0;
        min -= Vec2::splat(margin);
        max += Vec2::splat(margin);
        let road_dist = crate::corridor::RoadDistanceField::build(&pts, min, max, 10.0, 600.0);
        let nx = ((max.x - min.x) / TERRAIN_CELL).ceil() as usize + 1;
        let nz = ((max.y - min.y) / TERRAIN_CELL).ceil() as usize + 1;
        let mut hf = Heightfield::new(min, TERRAIN_CELL, nx, nz);
        // coarse natural heights every 4 cells (10 m)
        let cs = 4usize;
        let cnx = nx / cs + 2;
        let cnz = nz / cs + 2;
        let coarse: Vec<f32> = (0..cnx * cnz)
            .map(|k| {
                let (cx, cz) = (k % cnx, k / cnx);
                nat(min.x + (cx * cs) as f32 * TERRAIN_CELL, min.y + (cz * cs) as f32 * TERRAIN_CELL)
            })
            .collect();
        let road_zone = BLEND + half_w_base + 1.5 + SHOULDER;
        let mut candidates: Vec<(usize, f32)> = Vec::with_capacity(64);
        for iz in 0..nz {
            for ix in 0..nx {
                let x = min.x + ix as f32 * TERRAIN_CELL;
                let z = min.y + iz as f32 * TERRAIN_CELL;
                let dist = road_dist.at(x, z);
                let id = hf.idx(ix, iz);
                if dist > STAGE_TERRAIN_CORRIDOR {
                    let (cx, cz) = (ix / cs, iz / cs);
                    let (fx, fz) = ((ix % cs) as f32 / cs as f32, (iz % cs) as f32 / cs as f32);
                    let c = |a: usize, b: usize| coarse[b.min(cnz - 1) * cnx + a.min(cnx - 1)];
                    let h0 = c(cx, cz) + (c(cx + 1, cz) - c(cx, cz)) * fx;
                    let h1 = c(cx, cz + 1) + (c(cx + 1, cz + 1) - c(cx, cz + 1)) * fx;
                    hf.heights[id] = h0 + (h1 - h0) * fz;
                    continue;
                }
                let nat_h = nat(x, z);
                let mut h = nat_h;
                if dist < road_zone + 12.0 {
                    let q = Vec2::new(x, z);
                    candidates.clear();
                    grid.for_each_near(q, road_zone, |j| {
                        let d = pts[j as usize].distance_squared(q);
                        if d < road_zone * road_zone {
                            candidates.push((j as usize, d));
                        }
                    });
                    if !candidates.is_empty() {
                        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                        let mut sections: Vec<usize> = Vec::new();
                        for &(j, _) in candidates.iter() {
                            let near_existing = sections.iter().any(|&k| {
                                let ds = (k as isize - j as isize).unsigned_abs() as f32 * SAMPLE_SPACING;
                                ds < 1.4 * pts[k].distance(pts[j]) + 8.0
                            });
                            if !near_existing {
                                sections.push(j);
                            }
                        }
                        let mut wsum = 1.0;
                        let mut hsum = nat_h;
                        let mut exact = None;
                        for &j in &sections {
                            let (lat, along_h, hw) = road_profile_at_open(&samples, j, q);
                            let core = hw + SHOULDER;
                            let t = smoothstep(core, core + BLEND, lat.abs());
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
                }
                hf.heights[id] = h;
            }
        }
        let water_level = if style.has_water {
            let min_road = elev.iter().cloned().fold(f32::MAX, f32::min);
            Some(min_road - 2.2)
        } else {
            None
        };

        // ---- splits at 20 / 40 / 60 / 80 % and the checkpoint indices (start, splits, finish)
        let splits: Vec<f32> = (1..=4).map(|k| start_s + (finish_s - start_s) * k as f32 / 5.0).collect();
        let mut checkpoints = vec![((start_s / SAMPLE_SPACING).round() as usize).min(n - 1)];
        checkpoints.extend(splits.iter().map(|s| ((s / SAMPLE_SPACING).round() as usize).min(n - 1)));
        checkpoints.push(((finish_s / SAMPLE_SPACING).round() as usize).min(n - 1));

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
            conditions: crate::weather::StageConditions::default(),
            weather_events: Vec::new(),
            closed: false,
            start_s,
            finish_s,
            splits,
            road_dist,
            circuit_edges: Vec::new(),
            features: lay.features,
            surface_map,
            grid,
            prop_grid: SpatialGrid::new(&[Vec2::ZERO], 32.0, 1.0),
        };
        (track.conditions, track.weather_events) = build_conditions(params, biome, &elev, start_s, finish_s, false);
        track.place_props(&mut rng, noise_seed);
        // budget: only what can be seen from the road
        let landmark = |k: PropKind| matches!(k, PropKind::Windmill | PropKind::Lighthouse | PropKind::Mesa | PropKind::Barn | PropKind::Cottage | PropKind::SkiCabin);
        let rd = &track.road_dist;
        let mut props = std::mem::take(&mut track.props);
        props.retain(|p| {
            let d = rd.at(p.pos.x, p.pos.z);
            d < STAGE_PROP_CORRIDOR || (landmark(p.kind) && d < 420.0)
        });
        let pp: Vec<Vec2> = props.iter().map(|p| Vec2::new(p.pos.x, p.pos.z)).collect();
        if !pp.is_empty() {
            track.prop_grid = SpatialGrid::new(&pp, 12.0, 10.0);
        }
        track.props = props;
        Some(track)
    }
}

/// `road_profile_at` without wrap-around (open stages)
fn road_profile_at_open(samples: &[TrackSample], j: usize, q: Vec2) -> (f32, f32, f32) {
    let n = samples.len();
    let mut best = (f32::MAX, 0.0f32, 0.0f32, 0.0f32);
    for (a, b) in [(j.saturating_sub(1), j), (j, (j + 1).min(n - 1))] {
        if a == b {
            continue;
        }
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
            let crown = -0.012 * lat_c.abs();
            best = (d, lat, base + lat_c * rise + crown, hw);
        }
    }
    (best.1, best.2, best.3)
}

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
        if pp.circuit() {
            pp.circuit_furniture(rng);
            pp.set_pieces(rng);
            pp.scatter(rng, noise_seed);
            return pp.props;
        }
        if pp.open() {
            pp.stage_start(rng);
            pp.stage_finish(rng);
            pp.stage_splits(rng);
            pp.marshal_posts(rng);
            pp.villages(rng);
        } else {
            pp.start_area(rng);
        }
        pp.corners(rng);
        pp.warnings();
        pp.water_crossings();
        pp.set_pieces(rng);
        pp.fences(noise_seed);
        pp.roadside(rng);
        pp.scatter(rng, noise_seed);
        if pp.open() {
            pp.jump_crowds(rng);
        }
        pp.jump_cones();
        pp.props
    }

    fn n(&self) -> usize {
        self.t.samples.len()
    }

    /// Sample i; wraps on loops, clamps on point-to-point stages.
    fn smp(&self, i: isize) -> TrackSample {
        self.t.samples[self.t.sample_index(i)]
    }

    fn open(&self) -> bool {
        matches!(self.t.params.layout, LayoutKind::PointToPoint { .. })
    }

    fn circuit(&self) -> bool {
        self.t.params.discipline == Discipline::Circuit
    }

    /// Run-off width (road edge -> barrier) on `side` at sample i. Wider on the outside of
    /// fast/tight corners. Hook for the circuit generator's real run-off data.
    fn runoff(&self, i: isize, side: f32) -> f32 {
        if !self.t.circuit_edges.is_empty() {
            // real run-off from the circuit generator (starts at the shoulder edge); barrier
            // sits just beyond it
            let n = self.t.circuit_edges.len() as isize;
            let e = self.t.circuit_edges[i.rem_euclid(n) as usize];
            let ro = if side < 0.0 { e.runoff_left.width } else { e.runoff_right.width };
            return ro + 1.0;
        }
        let c = self.smp(i).curvature;
        let outside = (side > 0.0 && c > 0.0) || (side < 0.0 && c < 0.0);
        if outside {
            6.0 + 10.0 * smoothstep(1.0 / 90.0, 1.0 / 30.0, c.abs())
        } else {
            6.0
        }
    }

    /// Segment prop along the road at sample i (local X along travel), scaled to the chord.
    fn along(&mut self, kind: PropKind, i: isize, lat: f32, variant: u8, seg: f32) -> bool {
        let a = self.beside(i, lat, 0.0);
        let b = self.beside(i + (seg / SAMPLE_SPACING) as isize, lat, 0.0);
        let d = (b - a) * Vec3::new(1.0, 0.0, 1.0);
        let len = d.length();
        if len < 0.5 || self.under_water(a.y, 0.2) {
            return false;
        }
        let mid = (a + b) * 0.5;
        let mid = Vec3::new(mid.x, self.ground(mid.x, mid.z), mid.z);
        if self.clearance(mid) < lat.abs() - self.edge(i) - 1.0 {
            return false;
        }
        // fronts (-Z) must face the road: local X runs with travel on the left, against it on the right
        let dir = if lat < 0.0 { -d / len } else { d / len };
        self.push(kind, mid, yaw_along(dir), len / seg, 0.0, variant);
        true
    }

    fn circuit_furniture(&mut self, rng: &mut Rng) {
        let n = self.n() as isize;
        let calm = |pp: &Self, i: isize| pp.smp(i).curvature.abs() < 1.0 / 200.0;
        let mut back = 0;
        while back < 120 && calm(self, -back) {
            back += 1;
        }
        let mut fwd = 0;
        while fwd < 120 && calm(self, fwd) {
            fwd += 1;
        }
        let (a, b) = (-back.max(20), fwd.max(20));
        let pit = -1.0f32;
        // start/finish gantry with start lights over the grid
        {
            let s = self.smp(0);
            let half = self.edge(0) + 1.5;
            let p = Vec3::new(s.pos.x, self.ground(s.pos.x, s.pos.z), s.pos.z);
            self.push(PropKind::Gantry, p, yaw_along(-Self::flat_right(&s)), half / 5.0, 0.0, 2);
        }
        // pit wall + garages on the pit side, grandstands opposite
        let mut i = a + 2;
        while i < b - 2 {
            if i.abs() > 2 {
                self.along(PropKind::PitWall, i, pit * (self.edge(i) + 1.8), ((i / 2).rem_euclid(3)) as u8, 4.0);
            }
            i += 2;
        }
        let mut i = a + 6;
        let mut k = 0u8;
        while i < b - 6 {
            if self.along(PropKind::PitBuilding, i, pit * (self.edge(i) + 16.0), k, 12.0) {
                let p = self.beside(i + 3, pit * (self.edge(i) + 16.0), 0.0);
                self.keep_out.push((Vec2::new(p.x, p.z), 10.0));
            }
            k = k.wrapping_add(1);
            i += 6;
        }
        let mut i = a / 2;
        let mut k = 0u8;
        while i < b / 2 {
            if self.along(PropKind::Grandstand, i, -pit * (self.edge(i) + 8.0), k % 4, 12.0) {
                let p = self.beside(i + 3, -pit * (self.edge(i) + 10.0), 0.0);
                self.keep_out.push((Vec2::new(p.x, p.z), 9.0));
            }
            k += 1;
            i += 6;
        }
        let on_main = |i: isize| i >= a && i <= b;
        // barriers all the way round: tyre walls on the outside of tight corners, armco elsewhere
        let mut i = 0;
        while i < n {
            for side in [-1.0f32, 1.0] {
                if on_main(i) || on_main(i - n) {
                    continue;
                }
                let c = self.smp(i).curvature;
                let outside = (side > 0.0 && c > 0.0) || (side < 0.0 && c < 0.0);
                let ro = self.runoff(i, side);
                let kind = if outside && c.abs() > 1.0 / 45.0 { PropKind::TyreWall } else { PropKind::Armco };
                self.along(kind, i, side * (self.edge(i) + ro), (i / 2 % 4) as u8, 4.0);
                if outside && c.abs() > 1.0 / 60.0 && i % 6 == 0 {
                    let p = self.beside(i, side * (self.edge(i) + 1.5), 0.0);
                    if self.clearance(p) > 1.0 {
                        self.push(PropKind::MarkerPost, p, 0.0, 1.0, 0.0, 0);
                    }
                }
                // sponsor banners just behind the barrier on calm stretches
                if c.abs() < 1.0 / 150.0 && i % 14 == if side > 0.0 { 0 } else { 7 } {
                    let brand = (rng.range_i(0, 8)) as u8;
                    self.along(PropKind::AdBanner, i, side * (self.edge(i) + ro + 1.0), brand, 6.0);
                }
            }
            i += 2;
        }
        // brake marker boards before corners that follow a long straight
        let mut run = 0;
        for i in 0..n {
            let c = self.smp(i).curvature.abs();
            if c < 1.0 / 110.0 {
                run += 1;
                continue;
            }
            if c > 1.0 / 80.0 && run >= 35 {
                let side = if self.smp(i + 10).curvature > 0.0 { 1.0 } else { -1.0 };
                // boards at 300/200/100 m when the straight allows, else compressed
                let scale = ((run as f32 - 5.0) / 150.0).min(1.0);
                for (k, back) in [(3u8, 150.0f32), (2, 100.0), (1, 50.0)] {
                    let back = (back * scale) as isize;
                    if back > 8 {
                        self.board(PropKind::BrakeBoard, i - back, side, 2.2, k);
                    }
                }
            }
            run = 0;
        }
        // marshal posts every ~250 m behind the barriers
        let mut i = 40isize;
        let mut side = 1.0f32;
        while i < n - 20 {
            let ro = self.runoff(i, side);
            self.beside_facing(PropKind::MarshalPost, i, side * (self.edge(i) + ro + 2.0), 0.0, 0.0, rng.range_i(0, 3) as u8);
            side = -side;
            i += 125;
        }
        // grandstands + crowds at hairpins, behind the run-off
        for i in 30..n - 30 {
            let c = self.smp(i).curvature;
            if c.abs() > 1.0 / 30.0 && (-10..=10).all(|k| self.smp(i + k).curvature.abs() <= c.abs()) {
                let outside = if c > 0.0 { 1.0 } else { -1.0 };
                let ro = self.runoff(i, outside);
                for j in [i - 8, i + 2] {
                    self.along(PropKind::Grandstand, j, outside * (self.edge(j) + ro + 4.0), (j % 4) as u8, 12.0);
                }
                let cnt = rng.range_i(4, 8);
                self.crowd(rng, i + 12, outside, ro + 3.0, cnt, 1.4);
            }
        }
        // a footbridge over a long straight on bigger circuits
        if self.t.length > 2000.0 {
            let mut best = (0isize, 0isize);
            let mut run = 0isize;
            for i in 0..n {
                if self.smp(i).curvature.abs() < 1.0 / 200.0 {
                    run += 1;
                    if run > best.1 && (i - run / 2) > b + 60 && (i - run / 2) < n + a - 60 {
                        best = (i - run / 2, run);
                    }
                } else {
                    run = 0;
                }
            }
            if best.1 > 40 {
                let i = best.0;
                let s = self.smp(i);
                let half = self.edge(i) + 7.0;
                let p = Vec3::new(s.pos.x, self.ground(s.pos.x, s.pos.z), s.pos.z);
                self.push(PropKind::TrackBridge, p, yaw_along(-Self::flat_right(&s)), half / 5.0, 0.0, rng.range_i(0, 4) as u8);
            }
        }
        let s0 = self.smp(0).pos;
        self.keep_out.push((Vec2::new(s0.x, s0.z), 30.0));
    }

    /// Finish line sample on a stage: at `length_m` when the road runs on past it (the run-out
    /// to the stop control), otherwise just before the end of the road.
    fn finish_idx(&self) -> isize {
        let n = self.n() as isize;
        let s = match self.t.params.layout {
            LayoutKind::PointToPoint { length_m } if length_m > 100.0 && length_m < self.t.length - 60.0 => length_m,
            _ => self.t.length - 24.0,
        };
        ((s / SAMPLE_SPACING) as isize).clamp(10, n - 2)
    }

    fn edge(&self, i: isize) -> f32 {
        self.smp(i).half_width + SHOULDER
    }

    fn fwd(&self, i: isize) -> Vec3 {
        let t = self.smp(i).tangent;
        Vec3::new(t.x, 0.0, t.z).normalize()
    }

    /// Board on the `side` of the road at sample i, front facing the approaching driver.
    fn board(&mut self, kind: PropKind, i: isize, side: f32, extra: f32, variant: u8) {
        let p = self.beside(i, side * (self.edge(i) + extra), 0.0);
        if self.clearance(p) < extra - 0.6 || self.under_water(p.y, 0.2) {
            return;
        }
        let s = self.smp(i);
        let face = (-self.fwd(i) - Self::flat_right(&s) * side * 0.3).normalize();
        // board fronts are local -Z
        self.push(kind, p, yaw_facing(-face), 1.0, 0.0, variant);
    }

    /// A small hut/tent-like solid placed beside the road facing it
    fn beside_facing(&mut self, kind: PropKind, i: isize, lat: f32, along: f32, radius: f32, variant: u8) -> bool {
        let p = self.beside(i, lat, along);
        if self.clearance(p) < radius + 1.5 || self.under_water(p.y, 0.4) || self.blocked(p, radius) {
            return false;
        }
        let s = self.smp(i);
        let to_road = ((s.pos - p) * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero();
        // fronts (local -Z) look at the road
        self.push(kind, p, yaw_facing(-to_road), 1.0, radius, variant);
        self.keep_out.push((Vec2::new(p.x, p.z), radius + 2.0));
        true
    }

    fn gantry(&mut self, i: isize, variant: u8) {
        let s = self.smp(i);
        let half = self.edge(i) + 1.2;
        let p = Vec3::new(s.pos.x, self.ground(s.pos.x, s.pos.z), s.pos.z);
        // local X runs to the driver's left so the gantry's -Z face greets them
        self.push(PropKind::Gantry, p, yaw_along(-Self::flat_right(&s)), half / 5.0, 0.0, variant);
        self.keep_out.push((Vec2::new(p.x, p.z), half + 4.0));
    }

    fn tape_line(&mut self, i0: isize, i1: isize, lat: f32) {
        let mut i = i0;
        while i + 5 <= i1 {
            let a = self.beside(i, lat, 0.0);
            let b = self.beside(i + 5, lat, 0.0);
            let d = (b - a) * Vec3::new(1.0, 0.0, 1.0);
            let len = d.length();
            if len > 1.0 && self.clearance(a) > lat.abs() - self.edge(i) - 0.8 && self.clearance(b) > 0.8 && (a.y - b.y).abs() < 1.5 {
                self.push(PropKind::SafetyTape, a, yaw_along(d / len), len / 10.0, 0.0, 0);
            }
            i += 5;
        }
    }

    fn stage_start(&mut self, rng: &mut Rng) {
        let s0 = self.smp(0);
        self.beside_facing(PropKind::TimingHut, 2, -(self.edge(2) + 4.5), 0.0, 1.8, 0);
        self.keep_out.push((Vec2::new(s0.pos.x, s0.pos.z), 36.0));
        self.gantry(3, 0);
        self.board(PropKind::StageBoard, 0, 1.0, 1.4, 3);
        for (i, side) in [(6isize, 1.0f32), (12, -1.0)] {
            self.beside_facing(PropKind::MarshalPost, i, side * (self.edge(i) + 3.0), 0.0, 0.0, rng.range_i(0, 3) as u8);
        }
        for side in [-1.0f32, 1.0] {
            let e = self.edge(10);
            self.tape_line(6, 36, side * (e + 2.2));
            for k in 0..4 {
                let c = rng.range_i(3, 6);
                self.crowd(rng, 9 + k * 7, side, 4.0, c, 1.3);
            }
            self.bunting_line(rng, 4, 34, side * (e + 7.5));
            let p = self.beside(20, side * (e + rng.range(13.0, 17.0)), 0.0);
            if self.clearance(p) > 11.0 && !self.under_water(p.y, 0.5) {
                let kind = if side > 0.0 { PropKind::Camper } else { PropKind::Tent };
                self.push(kind, p, rng.range(0.0, 6.28), 1.0, if kind == PropKind::Camper { 2.4 } else { 1.6 }, rng.range_i(0, 6) as u8);
            }
        }
    }

    fn stage_finish(&mut self, rng: &mut Rng) {
        let fi = self.finish_idx();
        let n = self.n() as isize;
        // flying-finish warning board ~100 m before, the finish gantry, timing hut + crowd
        self.board(PropKind::StageBoard, fi - 50, 1.0, 1.4, 0);
        self.board(PropKind::StageBoard, fi - 50, -1.0, 1.4, 0);
        self.beside_facing(PropKind::TimingHut, fi + 1, -(self.edge(fi) + 4.5), 0.0, 1.8, 1);
        self.gantry(fi, 1);
        self.beside_facing(PropKind::MarshalPost, fi - 4, self.edge(fi) + 3.0, 0.0, 0.0, 1);
        for side in [-1.0f32, 1.0] {
            let e = self.edge(fi);
            self.tape_line(fi - 30, fi - 3, side * (e + 2.2));
            for k in 0..3 {
                let c = rng.range_i(3, 6);
                self.crowd(rng, fi - 24 + k * 8, side, 4.0, c, 1.3);
            }
            self.bunting_line(rng, fi - 25, fi + 5, side * (e + 7.0));
        }
        // stop control ~150 m after the flying finish (or at the end of the road)
        let si = (fi + 75).min(n - 3);
        if si > fi + 15 {
            self.board(PropKind::StageBoard, si, 1.0, 1.4, 1);
            self.beside_facing(PropKind::TimingHut, si + 2, self.edge(si) + 4.5, 0.0, 1.8, 0);
            self.beside_facing(PropKind::MarshalPost, si - 3, -(self.edge(si) + 3.0), 0.0, 0.0, 2);
            self.board(PropKind::StageBoard, si - 25, -1.0, 1.4, 1);
        }
        let f = self.smp(fi).pos;
        self.keep_out.push((Vec2::new(f.x, f.z), 30.0));
    }

    fn stage_splits(&mut self, rng: &mut Rng) {
        let fi = self.finish_idx();
        let mut k = 0u8;
        let cps = self.t.checkpoints.clone();
        for &c in cps.iter().skip(1) {
            let c = c as isize;
            if c < 100 || c > fi - 100 {
                continue;
            }
            k += 1;
            self.board(PropKind::StageBoard, c, 1.0, 1.4, 2 | (k.min(31) << 3));
            self.beside_facing(PropKind::MarshalPost, c + 2, -(self.edge(c) + 3.0), 0.0, 0.0, rng.range_i(0, 3) as u8);
            let p = self.smp(c).pos;
            self.keep_out.push((Vec2::new(p.x, p.z), 8.0));
        }
    }

    /// Marshal posts every ~450 m along long stages (alternating sides).
    fn marshal_posts(&mut self, rng: &mut Rng) {
        let fi = self.finish_idx();
        let step = 225isize;
        let mut i = 160isize;
        let mut side = 1.0f32;
        while i < fi - 120 {
            let j = i + rng.range_i(-20, 20) as isize;
            if self.smp(j).curvature.abs() < 1.0 / 60.0 {
                let e = self.edge(j);
                self.beside_facing(PropKind::MarshalPost, j, side * (e + 3.2), 0.0, 0.0, rng.range_i(0, 3) as u8);
                if rng.chance(0.5) {
                    let p = self.beside(j, side * (e + rng.range(7.0, 10.0)), rng.range(-3.0, 3.0));
                    if self.clearance(p) > 6.0 && !self.under_water(p.y, 0.4) {
                        self.push(PropKind::Picnic, p, rng.range(0.0, 6.28), 1.0, 0.0, rng.range_i(0, 6) as u8);
                    }
                }
            }
            side = -side;
            i += step;
        }
    }

    /// Small villages the stage runs through, on calm straights away from start/finish.
    fn villages(&mut self, rng: &mut Rng) {
        let fi = self.finish_idx();
        let want = (self.t.length / 2500.0) as usize + 1;
        let mut spots: Vec<isize> = Vec::new();
        for f in &self.t.features {
            if f.kind.is_corner() || f.s1 < f.s0 {
                continue;
            }
            let (a, b) = ((f.s0 / SAMPLE_SPACING) as isize, (f.s1 / SAMPLE_SPACING) as isize);
            if b - a > 70 {
                let c = (a + b) / 2;
                if c > 150 && c < fi - 150 && spots.iter().all(|&x| (x - c).abs() > 400) {
                    spots.push(c);
                }
            }
        }
        if spots.is_empty() {
            // no layout features: use the calmest stretches
            let mut i = 300;
            while i < fi - 300 {
                if (-30..=30).all(|k| self.smp(i + k).curvature.abs() < 1.0 / 90.0) {
                    spots.push(i);
                    i += 500;
                }
                i += 20;
            }
        }
        for &c in spots.iter().take(want) {
            for side in [-1.0f32, 1.0] {
                for k in -3..=3 {
                    if !rng.chance(0.75) {
                        continue;
                    }
                    let j = c + k * 10;
                    let lat = side * (self.edge(j) + rng.range(11.0, 15.0));
                    let kind = if rng.chance(0.85) { PropKind::Cottage } else { PropKind::Barn };
                    if self.beside_facing(kind, j, lat, rng.range(-2.0, 2.0), if kind == PropKind::Cottage { 3.6 } else { 4.6 }, rng.range_i(0, 3) as u8) {
                        // garden fence + flowers in front
                        let fp = self.beside(j, side * (self.edge(j) + 5.5), 0.0);
                        if self.clearance(fp) > 4.0 {
                            self.push(PropKind::Fence, fp, yaw_along(self.fwd(j)), 1.0, 0.0, 1);
                        }
                        let q = self.beside(j, side * (self.edge(j) + 7.0), rng.range(-3.0, 3.0));
                        self.push(PropKind::Flowers, q, rng.range(0.0, 6.28), 1.0, 0.0, 0);
                    }
                }
                let cnt = rng.range_i(2, 5);
                let at = c + rng.range_i(-15, 15) as isize;
                self.crowd(rng, at, side, 3.5, cnt, 1.5);
            }
            // bunting strung across the road at both ends of the village
            for j in [c - 35, c + 35] {
                let e = self.edge(j) + 1.5;
                let a = self.beside(j, -e, 0.0);
                let b = self.beside(j, e, 0.0);
                let d = (b - a) * Vec3::new(1.0, 0.0, 1.0);
                if (a.y - b.y).abs() < 1.0 && self.clearance(a) > 1.0 && self.clearance(b) > 1.0 {
                    self.push(PropKind::Bunting, a, yaw_along(d.normalize()), d.length() / 10.0, 0.0, rng.range_i(0, 6) as u8);
                }
            }
            let p = self.smp(c).pos;
            self.keep_out.push((Vec2::new(p.x, p.z), 45.0));
        }
    }

    /// Warning signs ~70 m before hairpins, nasty corners, jumps (both layouts).
    fn warnings(&mut self) {
        let n = self.n() as isize;
        let mut hazards: Vec<(isize, u8)> = Vec::new();
        for f in &self.t.features {
            let a = (f.s0 / SAMPLE_SPACING) as isize;
            let b = if f.s1 >= f.s0 { (f.s1 / SAMPLE_SPACING) as isize } else { a + 20 };
            let mid = (a + b) / 2;
            let left = self.smp(mid).curvature > 0.0;
            match f.kind {
                crate::layout::FeatureKind::Hairpin => hazards.push((a, if left { 0 } else { 1 })),
                crate::layout::FeatureKind::Tightening | crate::layout::FeatureKind::Chicane | crate::layout::FeatureKind::Square => hazards.push((a, 4)),
                _ => {}
            }
        }
        if self.t.features.is_empty() {
            for i in 0..n {
                let c = self.smp(i).curvature;
                if c.abs() > 1.0 / 22.0 && (-8..=8).all(|k| self.smp(i + k).curvature.abs() <= c.abs()) {
                    hazards.push((i - 10, if c > 0.0 { 0 } else { 1 }));
                }
            }
        }
        for &j in &self.t.jumps.clone() {
            hazards.push((j as isize, 2));
        }
        hazards.sort();
        let mut last = -1000isize;
        for (a, v) in hazards {
            let i = a - 35;
            if (!self.open() && (i < 40 || i > n - 40)) || (self.open() && (i < 25 || i > self.finish_idx() - 10)) || i - last < 30 {
                continue;
            }
            self.board(PropKind::WarningSign, i, 1.0, 1.6, v);
            last = i;
        }
    }

    /// Bridge parapets where the road crosses water; ford warnings where it runs through it.
    fn water_crossings(&mut self) {
        let Some(w) = self.t.water_level else { return };
        let n = self.n() as isize;
        let (lo, hi) = if self.open() { (0, n - 1) } else { (0, n) };
        let mut in_ford = false;
        let mut i = lo;
        while i < hi {
            let s = self.smp(i);
            let e = self.edge(i);
            let road_y = s.pos.y;
            let wet_l = self.beside(i, -(e + 8.0), 0.0).y < w - 0.3;
            let wet_r = self.beside(i, e + 8.0, 0.0).y < w - 0.3;
            if road_y < w + 0.6 {
                if !in_ford {
                    self.board(PropKind::WarningSign, i - 30, 1.0, 1.6, 3);
                }
                in_ford = true;
            } else {
                in_ford = false;
                if wet_l && wet_r && i % 2 == 0 {
                    for side in [-1.0f32, 1.0] {
                        let p = self.beside(i, side * (e + 0.4), 0.0);
                        self.push(PropKind::BridgeRail, Vec3::new(p.x, road_y, p.z), yaw_along(self.fwd(i)), 1.0, 0.0, 0);
                    }
                }
            }
            i += 1;
        }
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
        // stages: the start line is not at sample 0
        let o = self.t.start_index() as isize;
        let s0 = self.smp(o);
        self.keep_out.push((Vec2::new(s0.pos.x, s0.pos.z), 42.0));
        for side in [-1.0f32, 1.0] {
            let edge = s0.half_width + SHOULDER;
            // hay bale barrier both sides of the grid
            for k in 0..14 {
                let i = o - 26 + k * 3;
                let p = self.beside(i, side * (self.smp(i).half_width + SHOULDER + 2.2), 0.0);
                let t = self.smp(i).tangent;
                if self.clearance(p) > 1.2 {
                    self.push(PropKind::HayBale, p, yaw_along(Vec3::new(t.x, 0.0, t.z).normalize()), 1.0, 0.8, (k % 2) as u8);
                }
            }
            // crowds along the straight
            for k in 0..6 {
                let i = o - 22 + k * 7;
                let c = rng.range_i(3, 6);
                self.crowd(rng, i, side, 4.2, c, 1.3);
            }
            self.bunting_line(rng, o - 25, o + 20, side * (edge + 7.5));
            // flag poles
            for k in 0..4 {
                let i = o - 20 + k * 12;
                let p = self.beside(i, side * (edge + 9.5), 0.0);
                if self.clearance(p) > 8.0 {
                    self.push(PropKind::FlagPole, p, yaw_along(Self::flat_right(&self.smp(i)) * side), 1.0, 0.0, rng.range_i(0, 6) as u8);
                }
            }
            // campers, tents and picnics behind the crowd
            for k in 0..3 {
                let i = o - 18 + k * 14;
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
            if self.open() && c > 1.0 / 30.0 {
                // stages: safety tape behind the chevrons on the outside of tight bends
                let e = self.edge(i);
                self.tape_line(i - 12, i + 12, outside * (e + 3.2));
            }
            let max_spots = if self.open() { (self.t.length / 450.0) as usize + 3 } else { 7 };
            if c > 1.0 / 48.0 && spectator_spots < max_spots {
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
        let target = ((self.t.length / 260.0).round() as usize + 2).min(if self.open() { 18 } else { 99 });
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
                // stages: nothing far from the road corridor (cheap distance field check)
                if (!t.closed || t.is_circuit()) && t.road_distance(x, z) > STAGE_PROP_CORRIDOR {
                    continue;
                }
                let forest = noise::fbm(x * 0.012, z * 0.012, 3, noise_seed + 999) * 0.5 + 0.5;
                let mut density = style.tree_density * smoothstep(0.35, 0.75, forest) + 0.04;
                let q = t.query(Vec3::new(x, 0.0, z), None);
                let hw = t.samples[q.index].half_width;
                // thinner far from the road, where only fog sees it; long open stages keep a band
                if self.circuit() {
                    density *= 0.55;
                }
                if self.open() {
                    if q.dist > 60.0 {
                        continue;
                    }
                    density *= 0.6 - 0.4 * smoothstep(18.0, 60.0, q.dist);
                } else {
                    density *= 1.0 - 0.72 * smoothstep(50.0, 140.0, q.dist);
                }
                if !rng.chance(density) {
                    continue;
                }
                if q.dist < hw + SHOULDER + if self.circuit() { 20.0 } else { 3.5 } {
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

    fn jump_crowds(&mut self, rng: &mut Rng) {
        for &j in &self.t.jumps.clone() {
            let j = j as isize + 8;
            for side in [-1.0f32, 1.0] {
                let e = self.edge(j);
                let cnt = rng.range_i(3, 6);
                self.crowd(rng, j, side, 5.0, cnt, 1.4);
                self.tape_line(j - 6, j + 6, side * (e + 2.6));
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
