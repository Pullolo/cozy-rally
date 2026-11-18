//! Static world geometry built from a Track: painted terrain, road, start/checkpoint
//! structures, props (with baked ground AO), ground detail (grass tufts, flowers, pebbles),
//! stylised water, sky dome + clouds and layered distant hills. Also the per-biome look
//! (colour grade) and the scene draw passes (shadow casters / opaque / transparent).
use super::gpu::{DrawParams, Environment, Gpu, GpuMesh, View};
use super::mesh::{mix, rgb, rgba, shade, sway, MeshBuilder, MeshChunk, Vertex};
use super::post::Grade;
use crate::models;
use macroquad::math::{Mat4, Quat, Vec3};
use rally_core::biome::{Biome, BiomeStyle};
use rally_core::noise;
use rally_core::rng::{hash2, Rng};
use rally_core::surface::Surface;
use rally_core::track::{PropKind, Track, SHOULDER};
use std::collections::HashMap;

pub struct Scene {
    pub terrain: GpuMesh,
    /// road surface decals (drawn with a depth bias, no shadow casting)
    pub road: GpuMesh,
    /// start arch, balloons, checkpoint flags
    pub structures: GpuMesh,
    pub props: GpuMesh,
    /// grass tufts / flowers / pebbles near the road (distance culled)
    pub detail: GpuMesh,
    pub water: Option<GpuMesh>,
    pub sky: GpuMesh,
    pub clouds: GpuMesh,
    pub hills: GpuMesh,
    /// ground beyond the terrain edge (dry biomes only)
    pub apron: Option<GpuMesh>,
    pub env: Environment,
    pub sky_top: [f32; 3],
    pub sky_horizon: [f32; 3],
    pub water_level: Option<f32>,
    pub water_shallow: [f32; 3],
    pub grade: Grade,
    pub biome: Biome,
    pub time_of_day: TimeOfDay,
}

/// Draw distance for ground detail (metres)
pub const DETAIL_DIST: f32 = 85.0;
const ROAD_BIAS: f32 = 6e-6;

fn n3(v: [f32; 3]) -> Vec3 {
    Vec3::from(v).normalize()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Per-biome cozy colour grade
pub fn grade_for(biome: Biome) -> Grade {
    let d = Grade::default();
    match biome {
        Biome::Meadow => Grade { lift: [0.96, 0.97, 1.04], gain: [1.04, 1.01, 0.95], saturation: 1.08, warmth: 0.6, ..d },
        Biome::Autumn => Grade { lift: [0.98, 0.94, 1.02], gain: [1.05, 0.99, 0.92], saturation: 1.05, warmth: 0.9, bloom: 1.0, softness: 0.25, ..d },
        Biome::Snow => Grade { lift: [0.95, 0.96, 1.06], gain: [1.03, 1.0, 0.97], saturation: 1.04, contrast: 1.06, warmth: 0.35, threshold: 0.94, knee: 0.35, ..d },
        Biome::Desert => Grade { lift: [0.98, 0.95, 1.03], gain: [1.04, 1.0, 0.94], saturation: 1.02, warmth: 0.7, threshold: 0.9, knee: 0.45, ..d },
        Biome::Coast => Grade { lift: [0.95, 0.98, 1.05], gain: [1.04, 1.01, 0.96], saturation: 1.1, warmth: 0.45, ..d },
    }
}

// ---------------------------------------------------------------------------------- mood

/// Time of day for a track. Picked per seed (about one track in three is golden hour);
/// `COZY_TOD=day|golden` overrides for testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeOfDay {
    Day,
    Golden,
}

pub fn time_of_day(track: &Track) -> TimeOfDay {
    match std::env::var("COZY_TOD").ok().as_deref() {
        Some("day") => return TimeOfDay::Day,
        Some("golden") => return TimeOfDay::Golden,
        _ => {}
    }
    // autumn is already a warm late-afternoon palette
    if track.biome != Biome::Autumn && hash2(track.params.seed as i32, 913, 77) < 0.33 { TimeOfDay::Golden } else { TimeOfDay::Day }
}

/// The biome palette adjusted for the time of day (sky, sun, fog, ambient).
pub fn look_for(track: &Track, tod: TimeOfDay) -> BiomeStyle {
    let mut st = track.style.clone();
    if tod == TimeOfDay::Golden {
        let d = Vec3::from(st.sun_dir).normalize();
        let flat = Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
        let low = (flat * 0.93 + Vec3::Y * 0.36).normalize();
        st.sun_dir = low.to_array();
        st.sun_color = [1.18, 0.84, 0.58];
        st.sky_top = mix(st.sky_top, [0.42, 0.50, 0.78], 0.5);
        st.sky_horizon = mix(st.sky_horizon, [1.0, 0.80, 0.62], 0.65);
        st.fog = mix(st.fog, [0.98, 0.80, 0.66], 0.6);
        st.ambient = mix(st.ambient, [0.50, 0.44, 0.52], 0.6);
        st.fog_density *= 1.1;
    }
    st
}

// ---------------------------------------------------------------------------------- terrain

/// Paints the ground: base palette, meadows, dirt patches, shade under tree clusters.
pub struct GroundPainter<'a> {
    track: &'a Track,
    seed: u32,
    /// tree canopy density on a 4 m grid
    canopy: HashMap<(i32, i32), f32>,
}

const CANOPY_CELL: f32 = 4.0;

impl<'a> GroundPainter<'a> {
    pub fn new(track: &'a Track) -> GroundPainter<'a> {
        let mut canopy: HashMap<(i32, i32), f32> = HashMap::new();
        for p in &track.props {
            let (w, r) = match p.kind {
                PropKind::Tree => (1.0, 3.2),
                PropKind::Pine => (0.9, 2.6),
                PropKind::Palm => (0.5, 2.4),
                PropKind::Bush => (0.4, 1.6),
                _ => continue,
            };
            let r = r * p.scale;
            let c0 = ((p.pos.x - r) / CANOPY_CELL).floor() as i32;
            let c1 = ((p.pos.x + r) / CANOPY_CELL).floor() as i32;
            let r0 = ((p.pos.z - r) / CANOPY_CELL).floor() as i32;
            let r1 = ((p.pos.z + r) / CANOPY_CELL).floor() as i32;
            for cz in r0..=r1 {
                for cx in c0..=c1 {
                    let cc = Vec3::new((cx as f32 + 0.5) * CANOPY_CELL, 0.0, (cz as f32 + 0.5) * CANOPY_CELL);
                    let d = Vec3::new(p.pos.x, 0.0, p.pos.z).distance(cc);
                    let v = w * (1.0 - smoothstep(r * 0.4, r + CANOPY_CELL * 0.5, d));
                    if v > 0.0 {
                        *canopy.entry((cx, cz)).or_insert(0.0) += v;
                    }
                }
            }
        }
        GroundPainter { track, seed: track.params.seed as u32, canopy }
    }

    fn canopy_at(&self, x: f32, z: f32) -> f32 {
        let cx = (x / CANOPY_CELL).floor() as i32;
        let cz = (z / CANOPY_CELL).floor() as i32;
        self.canopy.get(&(cx, cz)).copied().unwrap_or(0.0).min(1.5)
    }

    /// Ground colour at a point (y = height, ny = normal.y). `speckle` adds per-facet variety.
    pub fn color(&self, x: f32, y: f32, z: f32, ny: f32, speckle: bool) -> [f32; 3] {
        let st = &self.track.style;
        let seed = self.seed;
        let biome = self.track.biome;
        let v = noise::fbm(x * 0.018, z * 0.018, 3, seed + 3) * 0.5 + 0.5;
        let mut col = mix(st.ground_a, st.ground_b, smoothstep(0.2, 0.8, v));
        // large soft meadow patches (lighter, warmer)
        let meadow = smoothstep(0.15, 0.55, noise::fbm(x * 0.009 + 40.0, z * 0.009, 2, seed + 17));
        let dirt = smoothstep(0.45, 0.7, noise::fbm(x * 0.03 + 7.0, z * 0.03 - 3.0, 3, seed + 29));
        let canopy = self.canopy_at(x, z);
        match biome {
            Biome::Meadow | Biome::Coast => {
                col = mix(col, [0.72, 0.82, 0.42], meadow * 0.35);
                col = mix(col, shade(st.road_edge, 1.05), dirt * 0.45);
                // deep cool green under trees
                col = mix(col, [0.24, 0.45, 0.26], (canopy * 0.55).min(0.6));
            }
            Biome::Autumn => {
                col = mix(col, [0.80, 0.56, 0.26], meadow * 0.35);
                col = mix(col, shade(st.road_edge, 1.1), dirt * 0.4);
                // leaf litter under the trees: rusty red-orange
                col = mix(col, [0.70, 0.32, 0.16], (canopy * 0.5).min(0.55));
            }
            Biome::Snow => {
                // blue shadows in hollows, a hint of rock and pine needles
                let hollow = smoothstep(0.55, 0.25, v);
                col = mix(col, [0.80, 0.86, 0.98], hollow * 0.5);
                col = mix(col, [0.62, 0.70, 0.66], (canopy * 0.25).min(0.3));
            }
            Biome::Desert => {
                // wind-swept dune banding
                let band = ((y * 1.3 + noise::fbm(x * 0.05, z * 0.05, 2, seed + 5) * 2.0).sin() * 0.5 + 0.5) * 0.12;
                col = shade(col, 0.97 + band);
                col = mix(col, [0.86, 0.56, 0.38], dirt * 0.3);
                col = mix(col, [0.62, 0.62, 0.38], (canopy * 0.25).min(0.3));
            }
        }
        if let Some(w) = self.track.water_level {
            let beach = ((w + 1.4 - y) / 1.4).clamp(0.0, 1.0);
            let sand = if biome == Biome::Snow { [0.80, 0.84, 0.90] } else { mix(st.shoulder, [0.93, 0.86, 0.66], 0.6) };
            col = mix(col, sand, smoothstep(0.0, 1.0, beach));
            // wet sand right at the waterline
            col = mix(col, shade(sand, 0.8), smoothstep(0.55, 1.0, beach) * 0.6);
        }
        let steep = ((0.86 - ny) / 0.16).clamp(0.0, 1.0);
        col = mix(col, st.cliff, steep);
        if biome == Biome::Snow && y > 18.0 {
            col = mix(col, [0.97, 0.98, 1.0], ((y - 18.0) / 6.0).clamp(0.0, 1.0) * (1.0 - steep * 0.5));
        }
        if speckle {
            let s = hash2((x * 2.0).floor() as i32, (z * 2.0).floor() as i32, seed);
            col = shade(col, 0.95 + s * 0.08);
            // wildflower confetti in meadow patches
            if matches!(biome, Biome::Meadow | Biome::Coast) && steep < 0.3 && canopy < 0.3 {
                let f = hash2((x * 1.3).floor() as i32, (z * 1.3).floor() as i32, seed + 77);
                if f < 0.07 * meadow {
                    let fc = [[0.98, 0.90, 0.95], [0.98, 0.88, 0.45], [0.88, 0.70, 0.92]][(f * 1000.0) as usize % 3];
                    col = mix(col, fc, 0.3);
                }
            }
        }
        col
    }
}

/// How far to sink each RENDER terrain vertex under the road ribbon (metres, per heightfield
/// vertex). The terrain mesh's 2.5 m triangles chord the crowned/banked/cresting road profile
/// differently from the road ribbon's own vertices, so without this the ground pokes through
/// the road. The physics heightfield is untouched (the ribbon follows it exactly); only the
/// hidden terrain underneath is pushed down: fully under the road, fading out across the
/// shoulder so there is no visible step at the shoulder's outer edge.
pub fn road_sink(track: &Track) -> Vec<f32> {
    const SINK: f32 = 0.24;
    let hf = &track.terrain;
    let n = track.samples.len();
    let mut dist = vec![f32::MAX; hf.nx * hf.nz];
    let mut hw_at = vec![0.0f32; hf.nx * hf.nz];
    for i in 0..n {
        let a = &track.samples[i];
        let b = &track.samples[(i + 1) % n];
        let (a2, b2) = (macroquad::math::Vec2::new(a.pos.x, a.pos.z), macroquad::math::Vec2::new(b.pos.x, b.pos.z));
        let reach = a.half_width.max(b.half_width) + SHOULDER + hf.cell;
        let lo = a2.min(b2) - macroquad::math::Vec2::splat(reach);
        let hi = a2.max(b2) + macroquad::math::Vec2::splat(reach);
        let ix0 = (((lo.x - hf.origin.x) / hf.cell).floor().max(0.0) as usize).min(hf.nx - 1);
        let ix1 = (((hi.x - hf.origin.x) / hf.cell).ceil().max(0.0) as usize).min(hf.nx - 1);
        let iz0 = (((lo.y - hf.origin.y) / hf.cell).floor().max(0.0) as usize).min(hf.nz - 1);
        let iz1 = (((hi.y - hf.origin.y) / hf.cell).ceil().max(0.0) as usize).min(hf.nz - 1);
        let ab = b2 - a2;
        let len2 = ab.length_squared().max(1e-6);
        for iz in iz0..=iz1 {
            for ix in ix0..=ix1 {
                let q = macroquad::math::Vec2::new(hf.origin.x + ix as f32 * hf.cell, hf.origin.y + iz as f32 * hf.cell);
                let t = ((q - a2).dot(ab) / len2).clamp(0.0, 1.0);
                let d = (a2 + ab * t).distance(q);
                let k = iz * hf.nx + ix;
                if d < dist[k] {
                    dist[k] = d;
                    hw_at[k] = a.half_width + (b.half_width - a.half_width) * t;
                }
            }
        }
    }
    dist.iter()
        .zip(hw_at.iter())
        .map(|(&d, &hw)| if d == f32::MAX { 0.0 } else { SINK * (1.0 - smoothstep(hw + 0.4, hw + SHOULDER, d)) })
        .collect()
}

pub fn terrain_mesh(track: &Track, painter: &GroundPainter) -> MeshBuilder {
    let hf = &track.terrain;
    let sink = road_sink(track);
    let vpos = |ix: usize, iz: usize| hf.vertex_pos(ix, iz) - Vec3::Y * sink[iz.min(hf.nz - 1) * hf.nx + ix.min(hf.nx - 1)];
    let mut m = MeshBuilder::new();
    let tile = 24usize;
    for tz in (0..hf.nz - 1).step_by(tile) {
        for tx in (0..hf.nx - 1).step_by(tile) {
            m.begin_chunk();
            for iz in tz..(tz + tile).min(hf.nz - 1) {
                for ix in tx..(tx + tile).min(hf.nx - 1) {
                    let p00 = vpos(ix, iz);
                    let p10 = vpos(ix + 1, iz);
                    let p01 = vpos(ix, iz + 1);
                    let p11 = vpos(ix + 1, iz + 1);
                    for (a, b, c) in [(p00, p11, p10), (p00, p01, p11)] {
                        let n = (b - a).cross(c - a).normalize();
                        let cen = (a + b + c) / 3.0;
                        let col = painter.color(cen.x, cen.y, cen.z, n.y, true);
                        m.tri_n(a, b, c, n, rgb(col));
                    }
                }
            }
        }
    }
    m
}

// ---------------------------------------------------------------------------------- road

/// Ground-hugging triangle with per-vertex colours, wound counter-clockwise seen from above
/// and with its geometric (upward) normal.
fn tri_up(m: &mut MeshBuilder, v: [(Vec3, [u8; 4]); 3]) {
    let mut n = (v[1].0 - v[0].0).cross(v[2].0 - v[0].0).normalize_or_zero();
    let (a, b, c) = if n.y < 0.0 {
        n = -n;
        (v[0], v[2], v[1])
    } else {
        (v[0], v[1], v[2])
    };
    m.tri_smooth([(a.0, n, a.1), (b.0, n, b.1), (c.0, n, c.1)]);
}

/// Surface mix of the road at sample `i`: (surface, next surface, blend 0..1 towards next).
/// The single place to switch over to the per-sample track fields
/// (`samples[i].surface`, `surface_next`, `surface_blend`) once they exist.
pub fn road_mix_at(track: &Track, i: usize) -> (Surface, Surface, f32) {
    let smp = &track.samples[i % track.samples.len()];
    (smp.surface, smp.surface_next, smp.surface_blend)
}

/// Lerpable visual description of a road cross-section.
#[derive(Clone, Copy, Debug)]
pub struct RoadLook {
    pub base: [f32; 3],
    pub rut: [f32; 3],
    pub edge: [f32; 3],
    /// white edge lines (tarmac)
    pub lines: f32,
    /// dashed centre line (tarmac)
    pub dash: f32,
    /// specular sheen (ice, wet mud)
    pub gloss: f32,
    /// glossy dark puddles in the ruts (mud)
    pub puddles: f32,
    /// pale drifts creeping in from the edges (sand, snow)
    pub drifts: f32,
    pub drift_color: [f32; 3],
    /// loose stones scattered on the surface (gravel, dirt)
    pub stones: f32,
}

impl RoadLook {
    pub fn lerp(&self, o: &RoadLook, t: f32) -> RoadLook {
        let f = |a: f32, b: f32| a + (b - a) * t;
        RoadLook {
            base: mix(self.base, o.base, t),
            rut: mix(self.rut, o.rut, t),
            edge: mix(self.edge, o.edge, t),
            lines: f(self.lines, o.lines),
            dash: f(self.dash, o.dash),
            gloss: f(self.gloss, o.gloss),
            puddles: f(self.puddles, o.puddles),
            drifts: f(self.drifts, o.drifts),
            drift_color: mix(self.drift_color, o.drift_color, t),
            stones: f(self.stones, o.stones),
        }
    }
}

/// The look of a road surface; the biome's own road surface keeps its tuned palette.
pub fn road_look(surface: Surface, st: &BiomeStyle) -> RoadLook {
    let l = |base: [f32; 3], rut: [f32; 3], edge: [f32; 3]| RoadLook { base, rut, edge, lines: 0.0, dash: 0.0, gloss: 0.0, puddles: 0.0, drifts: 0.0, drift_color: base, stones: 0.0 };
    let mut look = match surface {
        Surface::Asphalt => RoadLook { lines: 1.0, dash: 1.0, gloss: 0.12, ..l([0.38, 0.38, 0.43], [0.34, 0.34, 0.39], [0.55, 0.53, 0.50]) },
        Surface::Gravel => RoadLook { stones: 1.0, ..l([0.78, 0.69, 0.54], [0.66, 0.57, 0.44], [0.68, 0.59, 0.45]) },
        Surface::Dirt | Surface::Grass => RoadLook { stones: 0.5, ..l([0.60, 0.45, 0.32], [0.47, 0.34, 0.24], [0.52, 0.39, 0.27]) },
        Surface::Snow => RoadLook { drifts: 0.6, drift_color: [0.97, 0.98, 1.0], ..l([0.90, 0.92, 0.96], [0.72, 0.79, 0.92], [0.86, 0.89, 0.95]) },
        Surface::Ice => RoadLook { gloss: 0.9, drifts: 0.4, drift_color: [0.95, 0.97, 1.0], ..l([0.74, 0.86, 0.95], [0.64, 0.79, 0.92], [0.84, 0.90, 0.96]) },
        Surface::Mud => RoadLook { gloss: 0.25, puddles: 1.0, stones: 0.2, ..l([0.45, 0.33, 0.24], [0.33, 0.24, 0.17], [0.42, 0.32, 0.22]) },
        Surface::Sand => RoadLook { drifts: 0.8, drift_color: [0.95, 0.86, 0.66], ..l([0.88, 0.76, 0.56], [0.80, 0.67, 0.48], [0.85, 0.72, 0.52]) },
    };
    if surface == st.road_surface {
        look.base = st.road;
        look.rut = shade(st.road, 0.86);
        look.edge = st.road_edge;
        if surface == Surface::Snow {
            look.rut = mix(st.road, [0.70, 0.78, 0.93], 0.6);
        }
    }
    look
}

/// Blended road look at sample `i` (smooth over the transition)
pub fn road_look_at(track: &Track, i: usize) -> RoadLook {
    let (a, b, t) = road_mix_at(track, i);
    let la = road_look(a, &track.style);
    if t <= 0.0 || a == b {
        return la;
    }
    la.lerp(&road_look(b, &track.style), smoothstep(0.0, 1.0, t))
}

/// Feathered per-vertex look: the transition boundary wanders across the road with noise so
/// patches get irregular outlines instead of a straight seam.
fn road_look_vertex(track: &Track, i: usize, lat: f32, seed: u32) -> RoadLook {
    // exact (organic) patch / transition shape across the road
    let m = track.surface_mix_at(i as f32 * rally_core::track::SAMPLE_SPACING, lat);
    let (a, b, t) = (m.a, m.b, m.t);
    let la = road_look(a, &track.style);
    if a == b || t <= 0.0 {
        return la;
    }
    let s = i as f32 * 2.0;
    let wobble = noise::fbm(s * 0.09, lat * 0.35, 3, seed + 61) * 0.45;
    let tt = smoothstep(0.0, 1.0, ((t - 0.5) * 1.6 + 0.5 + wobble).clamp(0.0, 1.0));
    la.lerp(&road_look(b, &track.style), tt)
}

#[derive(Clone, Copy, PartialEq)]
enum StripKind {
    Edge,
    Line,
    Road,
    Rut,
}

/// Lateral breakpoints across the road (shared by all surfaces so looks can blend)
fn road_strips(hw: f32) -> Vec<(f32, f32, StripKind)> {
    let r1 = -hw * 0.42;
    let r2 = hw * 0.42;
    vec![
        (-hw, -hw + 0.6, StripKind::Edge),
        (-hw + 0.6, -hw + 0.85, StripKind::Line),
        (-hw + 0.85, r1 - 0.45, StripKind::Road),
        (r1 - 0.45, r1 + 0.45, StripKind::Rut),
        (r1 + 0.45, r2 - 0.45, StripKind::Road),
        (r2 - 0.45, r2 + 0.45, StripKind::Rut),
        (r2 + 0.45, hw - 0.85, StripKind::Road),
        (hw - 0.85, hw - 0.6, StripKind::Line),
        (hw - 0.6, hw, StripKind::Edge),
    ]
}

/// Colour (rgb) and gloss for a road vertex
fn road_vertex_color(look: &RoadLook, kind: StripKind, s: f32, lat: f32, hw: f32, seed: u32) -> ([f32; 3], f32) {
    let wear = 0.95 + (noise::fbm(s * 0.05, lat * 0.3, 2, seed + 41) * 0.5 + 0.5) * 0.08;
    let mut gloss = look.gloss;
    let mut c = match kind {
        StripKind::Edge => look.edge,
        StripKind::Line => mix(look.base, [0.95, 0.95, 0.92], look.lines),
        StripKind::Road => look.base,
        StripKind::Rut => look.rut,
    };
    c = shade(c, wear);
    if look.puddles > 0.0 && kind != StripKind::Edge {
        let p = smoothstep(0.52, 0.66, noise::fbm(s * 0.07, lat * 0.25, 3, seed + 83) * 0.5 + 0.5) * look.puddles;
        c = mix(c, shade(look.rut, 0.7), p * 0.8);
        gloss = gloss.max(p * 0.85);
    }
    if look.drifts > 0.0 {
        // drifts creep in from the edges with an organic outline
        let edge_prox = smoothstep(hw * 0.25, hw, lat.abs());
        let n = noise::fbm(s * 0.045, lat * 0.2, 3, seed + 97) * 0.5 + 0.5;
        let d = smoothstep(0.45, 0.7, n * 0.7 + edge_prox * 0.5) * look.drifts;
        c = mix(c, look.drift_color, d * 0.75);
    }
    (c, gloss)
}

pub fn road_mesh(track: &Track, painter: &GroundPainter) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let st = &track.style;
    let n = track.samples.len();
    let hf = &track.terrain;
    let seed = track.params.seed as u32;
    let ground = |p: Vec3, lift: f32| Vec3::new(p.x, hf.height(p.x, p.z) + lift, p.z);
    let flat_right = |i: usize| {
        let s = &track.samples[i % n];
        Vec3::new(s.right.x, 0.0, s.right.z).normalize()
    };
    // vertex colour with gloss in alpha (255 = matte); see DrawParams::vertex_gloss
    let enc = |(c, g): ([f32; 3], f32)| rgba(c, 1.0 - g.clamp(0.0, 1.0) * 0.8);
    for i in 0..n {
        if i % 16 == 0 {
            m.begin_chunk();
        }
        let ib = (i + 1) % n;
        let a = &track.samples[i];
        let b = &track.samples[ib];
        let fa = flat_right(i);
        let fb = flat_right(ib);
        let sb_scale = b.half_width / a.half_width;
        let (sa, sb) = (i as f32 * 2.0, (i + 1) as f32 * 2.0);
        for (from, to, kind) in road_strips(a.half_width) {
            // subdivide wide strips so the road follows the crowned terrain exactly
            let pieces = ((to - from) / 1.1).ceil().max(1.0) as usize;
            for k in 0..pieces {
                let l0 = from + (to - from) * k as f32 / pieces as f32;
                let l1 = from + (to - from) * (k + 1) as f32 / pieces as f32;
                let lift = if kind == StripKind::Line { 0.045 } else { 0.04 };
                let a0 = ground(a.pos + fa * l0, lift);
                let a1 = ground(a.pos + fa * l1, lift);
                let b0 = ground(b.pos + fb * l0 * sb_scale, lift);
                let b1 = ground(b.pos + fb * l1 * sb_scale, lift);
                let col = |idx: usize, s: f32, l: f32, hw: f32| {
                    let look = road_look_vertex(track, idx, l, seed);
                    enc(road_vertex_color(&look, kind, s, l, hw, seed))
                };
                let (ca0, ca1) = (col(i, sa, l0, a.half_width), col(i, sa, l1, a.half_width));
                let (cb0, cb1) = (col(ib, sb, l0, b.half_width), col(ib, sb, l1, b.half_width));
                tri_up(&mut m, [(a0, ca0), (b1, cb1), (a1, ca1)]);
                tri_up(&mut m, [(a0, ca0), (b0, cb0), (b1, cb1)]);
            }
        }
        // shoulders: blend from the road edge colour into the painted ground
        let (la, lb) = (road_look_at(track, i), road_look_at(track, ib));
        for side in [-1.0f32, 1.0] {
            let steps = [0.0, 0.5, 1.2, SHOULDER];
            for k in 0..steps.len() - 1 {
                let la0 = side * (a.half_width + steps[k]);
                let la1 = side * (a.half_width + steps[k + 1]);
                let lb0 = side * (b.half_width + steps[k]);
                let lb1 = side * (b.half_width + steps[k + 1]);
                let lift = 0.035;
                let p = [ground(a.pos + fa * la0, lift), ground(a.pos + fa * la1, lift), ground(b.pos + fb * lb1, lift), ground(b.pos + fb * lb0, lift)];
                let t = |d: f32| smoothstep(0.0, SHOULDER, d);
                let col = |pt: Vec3, d: f32, look: &RoadLook| {
                    let g = painter.color(pt.x, pt.y, pt.z, 1.0, false);
                    let sh = mix(st.shoulder, look.edge, 0.35 * (1.0 - t(d)));
                    rgb(mix(sh, g, t(d)))
                };
                let c = [col(p[0], steps[k], &la), col(p[1], steps[k + 1], &la), col(p[2], steps[k + 1], &lb), col(p[3], steps[k], &lb)];
                tri_up(&mut m, [(p[0], c[0]), (p[1], c[1]), (p[2], c[2])]);
                tri_up(&mut m, [(p[0], c[0]), (p[2], c[2]), (p[3], c[3])]);
            }
        }
        // dashed centre line (tarmac), fading with the look
        let dash = la.dash.min(lb.dash);
        if dash > 0.05 && (i / 3) % 2 == 0 {
            let a0 = ground(a.pos - fa * 0.12, 0.05);
            let a1 = ground(a.pos + fa * 0.12, 0.05);
            let b0 = ground(b.pos - fb * 0.12, 0.05);
            let b1 = ground(b.pos + fb * 0.12, 0.05);
            let ca = rgba(mix(la.base, [0.97, 0.9, 0.5], la.dash), 1.0 - la.gloss * 0.8);
            let cb = rgba(mix(lb.base, [0.97, 0.9, 0.5], lb.dash), 1.0 - lb.gloss * 0.8);
            tri_up(&mut m, [(a0, ca), (a1, ca), (b1, cb)]);
            tri_up(&mut m, [(a0, ca), (b1, cb), (b0, cb)]);
        }
    }
    // chequered start line
    m.begin_chunk();
    let s0 = &track.samples[0];
    let fr = Vec3::new(s0.right.x, 0.0, s0.right.z).normalize();
    let fwd = Vec3::new(s0.tangent.x, 0.0, s0.tangent.z).normalize();
    let cells = 12;
    for row in 0..2 {
        for k in 0..cells {
            let l0 = -s0.half_width + k as f32 * (2.0 * s0.half_width / cells as f32);
            let l1 = l0 + 2.0 * s0.half_width / cells as f32;
            let z0 = row as f32 * 0.8 - 0.8;
            let z1 = z0 + 0.8;
            let c = if (k + row) % 2 == 0 { [0.97, 0.97, 0.97] } else { [0.16, 0.16, 0.2] };
            let p = |l: f32, z: f32| ground(s0.pos + fr * l + fwd * z, 0.06);
            m.quad(p(l0, z0), p(l1, z0), p(l1, z1), p(l0, z1), rgb(c));
        }
    }
    m
}

/// Start arch, balloons, bunting and checkpoint flags (they cast shadows; flags sway).
pub fn structures_mesh(track: &Track) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let hf = &track.terrain;
    let ground = |p: Vec3, lift: f32| Vec3::new(p.x, hf.height(p.x, p.z) + lift, p.z);
    let s0 = &track.samples[0];
    let fr = Vec3::new(s0.right.x, 0.0, s0.right.z).normalize();
    let fwd = Vec3::new(s0.tangent.x, 0.0, s0.tangent.z).normalize();
    let hw = s0.half_width + 1.2;
    let post = rgb([0.95, 0.95, 0.92]);
    for side in [-1.0f32, 1.0] {
        let base = ground(s0.pos + fr * side * hw, 0.0);
        m.cube(Mat4::from_translation(base + Vec3::Y * 2.6), Vec3::new(0.25, 2.6, 0.25), post);
        // balloons (bobbing in the wind)
        for (k, c) in [[0.95, 0.4, 0.4], [0.4, 0.7, 0.95], [0.98, 0.85, 0.35]].iter().enumerate() {
            let off = Vec3::new(side * (0.3 + k as f32 * 0.2), 5.6 + k as f32 * 0.35, (k as f32 - 1.0) * 0.35);
            m.blob(Mat4::from_translation(base + off), Vec3::new(0.32, 0.4, 0.32), 3, 6, sway(rgb(*c), 0.5));
        }
    }
    let rot = Mat4::from_cols(fr.extend(0.0), Vec3::Y.extend(0.0), (-fwd).extend(0.0), Vec3::ZERO.extend(1.0));
    let top = ground(s0.pos, 0.0).y.max(ground(s0.pos + fr * hw, 0.0).y).max(ground(s0.pos - fr * hw, 0.0).y);
    let center = Vec3::new(s0.pos.x, top + 5.0, s0.pos.z);
    m.cube(Mat4::from_translation(center) * rot, Vec3::new(hw + 0.3, 0.55, 0.2), rgb([0.98, 0.55, 0.45]));
    m.cube(Mat4::from_translation(center) * rot * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.21)), Vec3::new(hw * 0.7, 0.32, 0.02), rgb([0.98, 0.95, 0.85]));
    m.cube(Mat4::from_translation(center) * rot * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.21)), Vec3::new(hw * 0.7, 0.32, 0.02), rgb([0.98, 0.95, 0.85]));
    // bunting flags along the arch
    for k in 0..10 {
        let l = -hw + (k as f32 + 0.5) * (2.0 * hw / 10.0);
        let c = [[0.95, 0.4, 0.4], [0.4, 0.7, 0.95], [0.98, 0.85, 0.35], [0.5, 0.8, 0.5]][k % 4];
        let p = center + fr * l - Vec3::Y * 0.6;
        let col = rgb(c);
        m.tri_smooth([(p - fr * 0.3, -fwd, col), (p + fr * 0.3, -fwd, col), (p - Vec3::Y * 0.5, -fwd, sway(col, 0.6))]);
    }
    // checkpoint flags
    for (ci, &idx) in track.checkpoints.iter().enumerate().skip(1) {
        let smp = &track.samples[idx];
        let fr = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
        let c = if ci % 2 == 0 { [0.98, 0.75, 0.3] } else { [0.4, 0.75, 0.95] };
        for side in [-1.0f32, 1.0] {
            let base = ground(smp.pos + fr * side * (smp.half_width + SHOULDER + 0.6), 0.0);
            m.cylinder(Mat4::from_translation(base + Vec3::Y * 1.6), 0.06, 1.6, 5, rgb([0.95, 0.95, 0.95]), rgb([0.95, 0.95, 0.95]));
            let top = base + Vec3::Y * 3.1;
            let dir = -smp.tangent;
            let n = dir.cross(Vec3::Y).normalize_or_zero();
            let col = rgb(c);
            m.tri_smooth([(top, n, col), (top - Vec3::Y * 0.8, n, col), (top + dir * 1.1 - Vec3::Y * 0.4, n, sway(col, 0.9))]);
        }
    }
    m
}

// ---------------------------------------------------------------------------------- props

pub fn props_mesh(track: &Track) -> MeshBuilder {
    // tile props spatially for culling
    let mut tiles: HashMap<(i32, i32), MeshBuilder> = HashMap::new();
    for p in &track.props {
        let key = ((p.pos.x / 64.0).floor() as i32, (p.pos.z / 64.0).floor() as i32);
        models::add_prop(tiles.entry(key).or_default(), p, &track.style);
    }
    let hf = &track.terrain;
    let mut m = MeshBuilder::new();
    let mut keys: Vec<_> = tiles.keys().cloned().collect();
    keys.sort();
    for k in keys {
        let tile = tiles.get_mut(&k).unwrap();
        // baked ambient occlusion: darken & cool everything close to the ground
        for ch in tile.chunks.iter_mut() {
            for v in ch.vertices.iter_mut() {
                let h = v.pos[1] - hf.height(v.pos[0], v.pos[2]);
                let ao = lerp(0.68, 1.0, smoothstep(-0.05, 0.9, h));
                let c = v.color;
                v.color = [(c[0] as f32 * ao) as u8, (c[1] as f32 * ao) as u8, (c[2] as f32 * (ao * 0.9 + 0.1)) as u8, c[3]];
            }
        }
        m.begin_chunk();
        m.append(tile);
    }
    m
}

// ---------------------------------------------------------------------------------- detail

struct Detail<'a> {
    m: &'a mut MeshBuilder,
    rng: Rng,
}

impl Detail<'_> {
    /// A tuft of `blades` thin triangles, base colour `base`, tips `tip`
    fn tuft(&mut self, p: Vec3, size: f32, blades: usize, base: [f32; 3], tip: [f32; 3]) {
        let up = Vec3::Y;
        for _ in 0..blades {
            let a = self.rng.range(0.0, std::f32::consts::TAU);
            let dir = Vec3::new(a.cos(), 0.0, a.sin());
            let side = Vec3::new(-dir.z, 0.0, dir.x);
            let off = dir * self.rng.range(0.0, 0.12) * size;
            let h = size * self.rng.range(0.6, 1.0);
            let w = 0.05 * size + 0.02;
            let lean = dir * h * self.rng.range(0.2, 0.55);
            let b = p + off;
            let t = b + up * h + lean;
            let tc = mix(tip, base, self.rng.range(0.0, 0.3));
            self.m.tri_smooth([(b - side * w, up, rgb(base)), (b + side * w, up, rgb(base)), (t, up, sway(rgb(tc), 1.0))]);
        }
    }

    fn flower(&mut self, p: Vec3, color: [f32; 3], stem: [f32; 3]) {
        let h = self.rng.range(0.22, 0.42);
        let up = Vec3::Y;
        let a = self.rng.range(0.0, std::f32::consts::TAU);
        let side = Vec3::new(a.cos(), 0.0, a.sin()) * 0.018;
        let top = p + up * h;
        self.m.tri_smooth([(p - side, up, rgb(stem)), (p + side, up, rgb(stem)), (top, up, sway(rgb(stem), 0.8))]);
        // little star of petals facing up
        let r = self.rng.range(0.06, 0.09);
        let petals = 5;
        let centre = sway(rgb([1.0, 0.9, 0.45]), 0.8);
        let pc = sway(rgb(color), 0.8);
        for k in 0..petals {
            let a0 = a + k as f32 / petals as f32 * std::f32::consts::TAU;
            let a1 = a0 + 0.9;
            let p0 = top + Vec3::new(a0.cos(), 0.15, a0.sin()) * r;
            let p1 = top + Vec3::new(a1.cos(), 0.15, a1.sin()) * r;
            self.m.tri_smooth([(top + up * 0.01, up, centre), (p1, up, pc), (p0, up, pc)]);
        }
    }

    fn pebble(&mut self, p: Vec3, size: f32, color: [f32; 3]) {
        let rot = Quat::from_rotation_y(self.rng.range(0.0, 6.28)) * Quat::from_rotation_x(self.rng.range(-0.2, 0.2));
        let m = Mat4::from_rotation_translation(rot, p + Vec3::Y * size * 0.2);
        self.m.box_tapered(m, Vec3::new(size, size * 0.45, size * 0.8), 0.55, rgb(color));
    }

    fn leaf(&mut self, p: Vec3, color: [f32; 3]) {
        let a = self.rng.range(0.0, 6.28);
        let d = Vec3::new(a.cos(), 0.0, a.sin()) * 0.11;
        let s = Vec3::new(-d.z, 0.0, d.x) * 0.6;
        let q = p + Vec3::Y * 0.03;
        let c = rgb(color);
        let n = Vec3::Y;
        self.m.tri_smooth([(q - d, n, c), (q + s, n, c), (q + d, n, c)]);
        self.m.tri_smooth([(q - d, n, c), (q + d, n, c), (q - s, n, c)]);
    }
}

/// Grass tufts, flowers, pebbles and leaves scattered along the road, merged per 32 m tile.
pub fn detail_mesh(track: &Track, painter: &GroundPainter) -> MeshBuilder {
    let st = &track.style;
    let hf = &track.terrain;
    let biome = track.biome;
    let n = track.samples.len();
    let mut tiles: HashMap<(i32, i32), MeshBuilder> = HashMap::new();
    let mut rng = Rng::new(track.params.seed ^ 0xD37A11);
    let flowers: [[f32; 3]; 5] = [[0.98, 0.97, 0.95], [0.99, 0.86, 0.35], [0.96, 0.62, 0.72], [0.74, 0.64, 0.95], [0.55, 0.72, 0.98]];
    let mut hint = None;
    for i in 0..n {
        let smp = &track.samples[i];
        // loose stones on gravel / dirt sections (thinning out across surface transitions)
        let look = road_look_at(track, i);
        let stones = (look.stones * 4.0 * rng.f32()).round() as usize;
        for _ in 0..stones {
            let fr = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
            let along = Vec3::new(smp.tangent.x, 0.0, smp.tangent.z).normalize();
            // mostly near the edges and between the ruts, where tyres don't sweep them away
            let u = rng.range(-1.0, 1.0);
            let lat = u.signum() * (u.abs().powf(0.6)) * (smp.half_width + 0.3);
            let p = smp.pos + fr * lat + along * rng.range(-1.0, 1.0);
            let y = hf.height(p.x, p.z) + 0.03;
            let key = ((p.x / 32.0).floor() as i32, (p.z / 32.0).floor() as i32);
            let mut d = Detail { m: tiles.entry(key).or_default(), rng: Rng::new(rng.next_u64()) };
            let c = mix(look.edge, look.rut, rng.f32());
            d.pebble(Vec3::new(p.x, y, p.z), rng.range(0.035, 0.075), shade(c, rng.range(0.85, 1.12)));
        }
        let fr = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
        let along = Vec3::new(smp.tangent.x, 0.0, smp.tangent.z).normalize();
        for side in [-1.0f32, 1.0] {
            let count = match biome {
                Biome::Snow => 5,
                Biome::Desert => 6,
                _ => 12,
            };
            for _ in 0..count {
                // denser close to the road, thinning out with distance
                let u = rng.f32();
                let lat = smp.half_width + 0.7 + u * u * 16.0;
                let p2 = smp.pos + fr * side * lat + along * rng.range(-1.0, 1.0);
                let (y, nrm) = hf.sample(p2.x, p2.z);
                if nrm.y < 0.82 {
                    continue;
                }
                if let Some(w) = track.water_level {
                    if y < w + 0.25 {
                        continue;
                    }
                }
                let q = track.query(Vec3::new(p2.x, y, p2.z), hint);
                hint = Some(q.index);
                let hw = track.samples[q.index].half_width;
                if q.dist < hw + 0.5 {
                    continue;
                }
                let p = Vec3::new(p2.x, y - 0.02, p2.z);
                let key = ((p.x / 32.0).floor() as i32, (p.z / 32.0).floor() as i32);
                let mut d = Detail { m: tiles.entry(key).or_default(), rng: Rng::new(rng.next_u64()) };
                let g = painter.color(p.x, y, p.z, nrm.y, false);
                let r = rng.f32();
                let near_shoulder = q.dist < hw + SHOULDER + 0.8;
                match biome {
                    Biome::Meadow | Biome::Coast => {
                        if r < 0.13 && !near_shoulder {
                            let c = flowers[rng.range_i(0, if biome == Biome::Coast { 3 } else { 5 }) as usize];
                            for _ in 0..rng.range_i(2, 5) {
                                let o = Vec3::new(rng.range(-0.35, 0.35), 0.0, rng.range(-0.35, 0.35));
                                d.flower(p + o, c, [0.32, 0.55, 0.26]);
                            }
                        } else if r < 0.16 {
                            d.pebble(p, rng.range(0.07, 0.16), shade(st.cliff, rng.range(0.95, 1.15)));
                        } else {
                            let base = shade(g, 0.72);
                            let tip = mix(shade(g, 1.15), [0.85, 0.92, 0.5], 0.25);
                            d.tuft(p, rng.range(0.35, 0.7), rng.range_i(4, 7) as usize, base, tip);
                        }
                    }
                    Biome::Autumn => {
                        if r < 0.35 {
                            for _ in 0..rng.range_i(2, 5) {
                                let o = Vec3::new(rng.range(-0.6, 0.6), 0.0, rng.range(-0.6, 0.6));
                                let c = st.leaves[rng.range_i(0, 3) as usize];
                                d.leaf(p + o, shade(c, rng.range(0.85, 1.1)));
                            }
                        } else if r < 0.42 {
                            d.pebble(p, rng.range(0.07, 0.15), shade(st.cliff, rng.range(0.95, 1.15)));
                        } else if r < 0.47 && !near_shoulder {
                            d.flower(p, [0.98, 0.86, 0.35], [0.45, 0.5, 0.25]);
                        } else {
                            let base = shade(g, 0.7);
                            let tip = mix(shade(g, 1.2), [0.95, 0.8, 0.45], 0.35);
                            d.tuft(p, rng.range(0.35, 0.7), rng.range_i(4, 7) as usize, base, tip);
                        }
                    }
                    Biome::Snow => {
                        if r < 0.35 {
                            d.pebble(p, rng.range(0.08, 0.2), shade(st.cliff, rng.range(0.95, 1.2)));
                        } else if r < 0.55 {
                            // little snow lump
                            let s = rng.range(0.15, 0.35);
                            d.m.blob(Mat4::from_translation(p), Vec3::new(s * 1.4, s * 0.55, s), 2, 6, rgb([0.95, 0.97, 1.0]));
                        } else {
                            d.tuft(p, rng.range(0.25, 0.45), rng.range_i(3, 6) as usize, [0.35, 0.42, 0.36], [0.62, 0.70, 0.58]);
                        }
                    }
                    Biome::Desert => {
                        if r < 0.5 {
                            d.pebble(p, rng.range(0.07, 0.22), shade(st.cliff, rng.range(0.9, 1.15)));
                        } else if r < 0.58 && !near_shoulder {
                            d.flower(p, [0.98, 0.55, 0.62], [0.45, 0.6, 0.35]);
                        } else {
                            d.tuft(p, rng.range(0.3, 0.55), rng.range_i(4, 8) as usize, [0.62, 0.58, 0.36], [0.88, 0.80, 0.5]);
                        }
                    }
                }
            }
        }
    }
    let mut m = MeshBuilder::new();
    let mut keys: Vec<_> = tiles.keys().cloned().collect();
    keys.sort();
    for k in keys {
        m.begin_chunk();
        m.append(&tiles[&k]);
    }
    m
}

// ---------------------------------------------------------------------------------- sky

fn sky_dome() -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let rings = 12;
    let segs = 24;
    let r = 1000.0;
    for i in 0..rings {
        let p0 = -0.35 + i as f32 / rings as f32 * (std::f32::consts::FRAC_PI_2 + 0.35);
        let p1 = -0.35 + (i + 1) as f32 / rings as f32 * (std::f32::consts::FRAC_PI_2 + 0.35);
        for j in 0..segs {
            let t0 = j as f32 / segs as f32 * std::f32::consts::TAU;
            let t1 = (j + 1) as f32 / segs as f32 * std::f32::consts::TAU;
            let pt = |p: f32, t: f32| Vec3::new(p.cos() * t.cos(), p.sin(), p.cos() * t.sin()) * r;
            let v = [pt(p0, t0), pt(p0, t1), pt(p1, t1), pt(p1, t0)];
            let c = [255u8; 4];
            m.tri_smooth([(v[0], Vec3::Y, c), (v[1], Vec3::Y, c), (v[2], Vec3::Y, c)]);
            m.tri_smooth([(v[0], Vec3::Y, c), (v[2], Vec3::Y, c), (v[3], Vec3::Y, c)]);
        }
    }
    // bottom cap so nothing shows through below the horizon
    let cap = Vec3::new(0.0, -r, 0.0);
    for j in 0..segs {
        let t0 = j as f32 / segs as f32 * std::f32::consts::TAU;
        let t1 = (j + 1) as f32 / segs as f32 * std::f32::consts::TAU;
        let p = -0.35f32;
        let a = Vec3::new(p.cos() * t0.cos(), p.sin(), p.cos() * t0.sin()) * r;
        let b = Vec3::new(p.cos() * t1.cos(), p.sin(), p.cos() * t1.sin()) * r;
        m.tri_smooth([(a, Vec3::Y, [255; 4]), (cap, Vec3::Y, [255; 4]), (b, Vec3::Y, [255; 4])]);
    }
    m
}

/// Smooth-shaded puff (sphere with radial normals) for fluffy clouds
fn puff(m: &mut MeshBuilder, c: Vec3, s: Vec3, col: impl Fn(Vec3) -> [u8; 4], flat_bottom: f32) {
    let rings = 5;
    let segs = 9;
    let mut pts = Vec::new();
    for r in 0..=rings {
        let phi = r as f32 / rings as f32 * std::f32::consts::PI;
        let mut row = Vec::new();
        for k in 0..segs {
            let th = k as f32 / segs as f32 * std::f32::consts::TAU;
            let d = Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin());
            let mut p = d * s;
            p.y = p.y.max(-s.y * flat_bottom);
            let nn = Vec3::new(d.x / s.x, d.y / s.y, d.z / s.z).normalize();
            row.push((c + p, nn));
        }
        pts.push(row);
    }
    for r in 0..rings {
        for k in 0..segs {
            let t = (k + 1) % segs;
            let v = [pts[r][k], pts[r][t], pts[r + 1][t], pts[r + 1][k]];
            let cv = |i: usize| (v[i].0, v[i].1, col(v[i].0));
            m.tri_smooth([cv(0), cv(2), cv(1)]);
            m.tri_smooth([cv(0), cv(3), cv(2)]);
        }
    }
}

fn clouds_mesh(track: &Track, st: &BiomeStyle) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let mut rng = Rng::new(track.params.seed ^ 0xC10D);
    let r = 800.0;
    let count = match track.biome {
        Biome::Desert => 9,
        Biome::Autumn => 18,
        _ => 15,
    };
    for _ in 0..count {
        let a = rng.range(0.0, 6.28);
        let el: f32 = rng.range(0.07, 0.4);
        let dist = r * rng.range(0.75, 1.0);
        let c = Vec3::new(el.cos() * a.cos(), el.sin(), el.cos() * a.sin()) * dist;
        let size = rng.range(0.6, 1.25) * (0.6 + el);
        let tangent = Vec3::new(-a.sin(), 0.0, a.cos());
        // haze: low clouds melt into the horizon colour
        let haze = smoothstep(0.35, 0.05, el) * 0.45;
        let white = mix([1.0, 0.99, 0.97], st.sky_horizon, haze);
        let belly = mix([0.84, 0.86, 0.95], st.sky_horizon, haze * 0.6);
        let base_y = c.y;
        let puffs = rng.range_i(4, 8);
        for k in 0..puffs {
            let t = (k as f32 / (puffs - 1) as f32 - 0.5) * 2.0;
            let off = tangent * t * 70.0 * size + Vec3::new(rng.range(-12.0, 12.0), 0.0, rng.range(-12.0, 12.0));
            let rad = (1.0 - t.abs() * 0.45) * rng.range(28.0, 42.0) * size;
            let sc = Vec3::new(rad * 1.2, rad * 0.75, rad);
            let center = c + off + Vec3::Y * (sc.y * 0.35);
            let ext = sc.y * 1.1;
            puff(&mut m, center, sc, |p| rgb(mix(belly, white, smoothstep(base_y - ext * 0.3, base_y + ext, p.y))), 0.35);
        }
    }
    m
}

/// Layered distant hills/mountains with baked aerial perspective (farther = paler).
fn hills_mesh(track: &Track, st: &BiomeStyle) -> MeshBuilder {
    let hf = &track.terrain;
    let size = hf.size();
    let center = Vec3::new(hf.origin.x + size.x * 0.5, 0.0, hf.origin.y + size.y * 0.5);
    let r0 = size.x.max(size.y) * 0.5 * 1.02;
    let segs = 120;
    let mut m = MeshBuilder::new();
    let seed = track.params.seed as u32 + 11;
    let haze = mix(st.fog, st.sky_horizon, 0.5);
    let layers: [(f32, f32, f32, f32); 3] = [
        // (distance, height scale, haze amount, noise freq)
        (r0 + 420.0, 5.5, 0.42, 2.5),
        (r0 + 230.0, 3.4, 0.22, 3.4),
        (r0 + 70.0, 1.6, 0.06, 5.0),
    ];
    let far_c = match track.biome {
        Biome::Snow => [0.80, 0.84, 0.95],
        Biome::Desert => mix(st.cliff, [0.9, 0.6, 0.5], 0.3),
        Biome::Autumn => [0.62, 0.48, 0.52],
        _ => [0.42, 0.55, 0.62],
    };
    for (li, &(dist, hs, hz, freq)) in layers.iter().enumerate() {
        m.begin_chunk();
        let lseed = seed + li as u32 * 31;
        let h_at = |a: f32| -> f32 {
            let n = noise::fbm(a.cos() * freq + 10.0, a.sin() * freq + 10.0, 4, lseed) * 0.5 + 0.5;
            let ridged = 1.0 - (noise::fbm(a.cos() * freq * 1.7 + 3.0, a.sin() * freq * 1.7, 3, lseed + 5)).abs();
            let mut h = (n * 0.7 + ridged * 0.3) * st.hill_amp * hs;
            if track.biome == Biome::Desert && li == 1 {
                // mesas: flat tops
                h = (h / 26.0).round() * 26.0 + h * 0.15;
            }
            h + 8.0
        };
        let base = match li {
            0 => far_c,
            1 => mix(far_c, st.ground_a, 0.45),
            _ => mix(st.ground_a, st.leaves[0], 0.35),
        };
        let col_low = mix(shade(base, 0.85), haze, hz + 0.08);
        let col_high = mix(base, haze, hz);
        let snow = track.biome == Biome::Snow || (li == 0 && track.biome != Biome::Desert && track.biome != Biome::Coast);
        for i in 0..segs {
            let a0 = i as f32 / segs as f32 * std::f32::consts::TAU;
            let a1 = (i + 1) as f32 / segs as f32 * std::f32::consts::TAU;
            let d0 = Vec3::new(a0.cos(), 0.0, a0.sin());
            let d1 = Vec3::new(a1.cos(), 0.0, a1.sin());
            let (h0, h1) = (h_at(a0), h_at(a1));
            let top0 = center + d0 * dist + Vec3::Y * h0;
            let top1 = center + d1 * dist + Vec3::Y * h1;
            let mid0 = center + d0 * (dist - 25.0) + Vec3::Y * (h0 * 0.45);
            let mid1 = center + d1 * (dist - 25.0) + Vec3::Y * (h1 * 0.45);
            let bot0 = center + d0 * (dist - 50.0) + Vec3::Y * -30.0;
            let bot1 = center + d1 * (dist - 50.0) + Vec3::Y * -30.0;
            let peak = |h: f32| -> [f32; 3] {
                let t = smoothstep(st.hill_amp * hs * 0.55, st.hill_amp * hs * 0.85, h);
                if snow && li == 0 { mix(col_high, mix([0.97, 0.97, 1.0], haze, hz * 0.6), t) } else { col_high }
            };
            let n = Vec3::Y;
            let (cl, cm0, cm1) = (rgb(col_low), rgb(mix(col_low, col_high, 0.6)), rgb(mix(col_low, col_high, 0.6)));
            m.tri_smooth([(bot0, n, cl), (mid1, n, cm1), (bot1, n, cl)]);
            m.tri_smooth([(bot0, n, cl), (mid0, n, cm0), (mid1, n, cm1)]);
            m.tri_smooth([(mid0, n, cm0), (top1, n, rgb(peak(h1))), (mid1, n, cm1)]);
            m.tri_smooth([(mid0, n, cm0), (top0, n, rgb(peak(h0))), (top1, n, rgb(peak(h1)))]);
        }
    }
    m
}

/// Flat fogged ground beyond the terrain edge (dry biomes have no sea there), so the far
/// hills sit on land instead of floating over the sky dome's haze.
fn apron_mesh(track: &Track) -> MeshBuilder {
    let hf = &track.terrain;
    let size = hf.size();
    let center = Vec3::new(hf.origin.x + size.x * 0.5, 0.0, hf.origin.y + size.y * 0.5);
    let mut edge_min = f32::MAX;
    for ix in 0..hf.nx {
        edge_min = edge_min.min(hf.get(ix, 0)).min(hf.get(ix, hf.nz - 1));
    }
    for iz in 0..hf.nz {
        edge_min = edge_min.min(hf.get(0, iz)).min(hf.get(hf.nx - 1, iz));
    }
    let y = edge_min - 1.5;
    let r_in = size.x.min(size.y) * 0.45;
    let r_out = size.x.max(size.y) * 0.5 + 900.0;
    let st = &track.style;
    let c_in = rgb(mix(st.ground_a, st.ground_b, 0.5));
    let c_out = rgb(mix(mix(st.ground_a, st.ground_b, 0.5), st.fog, 0.5));
    let mut m = MeshBuilder::new();
    let segs = 48;
    for i in 0..segs {
        let a0 = i as f32 / segs as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segs as f32 * std::f32::consts::TAU;
        let d0 = Vec3::new(a0.cos(), 0.0, a0.sin());
        let d1 = Vec3::new(a1.cos(), 0.0, a1.sin());
        let (i0, i1) = (center + d0 * r_in + Vec3::Y * y, center + d1 * r_in + Vec3::Y * y);
        let (o0, o1) = (center + d0 * r_out + Vec3::Y * y, center + d1 * r_out + Vec3::Y * y);
        tri_up(&mut m, [(i0, c_in), (o1, c_out), (o0, c_out)]);
        tri_up(&mut m, [(i0, c_in), (i1, c_in), (o1, c_out)]);
    }
    m
}

// ---------------------------------------------------------------------------------- water

/// Water surface over the terrain grid (5 m cells, only where it can be seen) plus a coarse
/// outer apron. Vertex alpha = depth below the water level (0..4 m) for colour and foam.
fn water_mesh(track: &Track, level: f32) -> MeshBuilder {
    let hf = &track.terrain;
    let st = &track.style;
    let mut m = MeshBuilder::new();
    let step = 2usize;
    let tile = 16usize; // cells (of `step` terrain cells) per tile
    let deep = shade(st.water, 0.85);
    let enc = |depth: f32| -> [u8; 4] { rgba(deep, (depth / 4.0).clamp(0.0, 1.0)) };
    let gx = (hf.nx - 1) / step;
    let gz = (hf.nz - 1) / step;
    let vpos = |ix: usize, iz: usize| -> (Vec3, f32) {
        let x = hf.origin.x + (ix * step) as f32 * hf.cell;
        let z = hf.origin.y + (iz * step) as f32 * hf.cell;
        let h = hf.get(ix * step, iz * step);
        (Vec3::new(x, level, z), level - h)
    };
    for tz in (0..gz).step_by(tile) {
        for tx in (0..gx).step_by(tile) {
            let mut ch = MeshChunk { min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN), ..Default::default() };
            let w = (tx + tile).min(gx) - tx + 1;
            let h = (tz + tile).min(gz) - tz + 1;
            for iz in 0..h {
                for ix in 0..w {
                    let (p, d) = vpos(tx + ix, tz + iz);
                    ch.vertices.push(Vertex { pos: p.to_array(), normal: [0.0, 1.0, 0.0], color: enc(d) });
                }
            }
            for iz in 0..h - 1 {
                for ix in 0..w - 1 {
                    // visible if any terrain vertex in the cell is below the surface
                    let mut wet = false;
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (_, d) = vpos(tx + ix + dx, tz + iz + dz);
                        wet |= d > -0.3;
                    }
                    if !wet {
                        continue;
                    }
                    let i0 = (iz * w + ix) as u16;
                    let i1 = i0 + 1;
                    let i2 = i0 + w as u16;
                    let i3 = i2 + 1;
                    ch.indices.extend_from_slice(&[i0, i2, i3, i0, i3, i1]);
                    for i in [i0, i1, i2, i3] {
                        let p = Vec3::from(ch.vertices[i as usize].pos);
                        ch.min = ch.min.min(p);
                        ch.max = ch.max.max(p);
                    }
                }
            }
            if !ch.indices.is_empty() {
                m.chunks.push(ch);
            }
        }
    }
    // outer apron (deep water beyond the terrain)
    m.begin_chunk();
    let size = hf.size();
    let (x0, z0) = (hf.origin.x, hf.origin.y);
    let (x1, z1) = (x0 + (gx * step) as f32 * hf.cell, z0 + (gz * step) as f32 * hf.cell);
    let e = 900.0;
    let _ = size;
    let col = enc(4.0);
    let quad = |m: &mut MeshBuilder, ax: f32, az: f32, bx: f32, bz: f32| {
        let n = Vec3::Y;
        let (a, b, c, d) = (Vec3::new(ax, level, az), Vec3::new(bx, level, az), Vec3::new(bx, level, bz), Vec3::new(ax, level, bz));
        m.tri_smooth([(a, n, col), (d, n, col), (c, n, col)]);
        m.tri_smooth([(a, n, col), (c, n, col), (b, n, col)]);
    };
    quad(&mut m, x0 - e, z0 - e, x1 + e, z0);
    quad(&mut m, x0 - e, z1, x1 + e, z1 + e);
    quad(&mut m, x0 - e, z0, x0, z1);
    quad(&mut m, x1, z0, x1 + e, z1);
    m
}

// ---------------------------------------------------------------------------------- scene

/// Debug/screenshot helper: a low camera on the road (`spec` = "crest", "bank" or a 0..1
/// fraction of the lap) looking along it, for checking the road/terrain seam.
pub fn road_viewpoint(track: &Track, spec: &str) -> (Vec3, Vec3) {
    let n = track.samples.len();
    let idx = match spec {
        "crest" => (0..n)
            .max_by(|&a, &b| {
                let c = |i: usize| track.samples[(i + n - 6) % n].pos.y + track.samples[(i + 6) % n].pos.y - 2.0 * track.samples[i].pos.y;
                c(b).partial_cmp(&c(a)).unwrap()
            })
            .unwrap_or(0),
        "bank" => (0..n).max_by(|&a, &b| track.samples[a].bank.abs().partial_cmp(&track.samples[b].bank.abs()).unwrap()).unwrap_or(0),
        f => ((f.parse::<f32>().unwrap_or(0.3).fract() * n as f32) as usize).min(n - 1),
    };
    let i0 = (idx + n - 12) % n;
    let s0 = &track.samples[i0];
    let fr = Vec3::new(s0.right.x, 0.0, s0.right.z).normalize();
    let eye = s0.pos + fr * (s0.half_width * 0.55) + Vec3::Y * 0.45;
    let t = &track.samples[(idx + 6) % n];
    (eye, t.pos + Vec3::Y * 0.2)
}

/// Debug/screenshot helper: a viewpoint on the road looking out over nearby water.
pub fn water_viewpoint(track: &Track) -> Option<(Vec3, Vec3)> {
    let level = track.water_level?;
    let hf = &track.terrain;
    let n = track.samples.len();
    for i in (0..n).step_by(5) {
        let smp = &track.samples[i];
        let fr = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
        for side in [-1.0f32, 1.0] {
            for k in 2..14 {
                let p = smp.pos + fr * side * (k as f32 * 5.0);
                if hf.contains(p.x, p.z) && hf.height(p.x, p.z) < level - 0.8 {
                    let eye = smp.pos - fr * side * 4.0 + Vec3::Y * 3.5;
                    return Some((eye, Vec3::new(p.x, level, p.z) + smp.tangent * 6.0));
                }
            }
        }
    }
    None
}

impl Scene {
    pub fn build(track: &Track, gpu: &mut Gpu) -> Scene {
        let tod = time_of_day(track);
        let look = look_for(track, tod);
        let st = &look;
        let painter = GroundPainter::new(track);
        let terrain = gpu.upload(&terrain_mesh(track, &painter));
        let road = gpu.upload(&road_mesh(track, &painter));
        let structures = gpu.upload(&structures_mesh(track));
        let props = gpu.upload(&props_mesh(track));
        let detail = gpu.upload(&detail_mesh(track, &painter));
        let sky = gpu.upload(&sky_dome());
        let clouds = gpu.upload(&clouds_mesh(track, st));
        let hills = gpu.upload(&hills_mesh(track, st));
        let water = track.water_level.map(|w| gpu.upload(&water_mesh(track, w)));
        let apron = if track.water_level.is_none() { Some(gpu.upload(&apron_mesh(track))) } else { None };
        let water_shallow = match track.biome {
            Biome::Snow => [0.62, 0.86, 0.92],
            Biome::Autumn => [0.48, 0.72, 0.68],
            _ => [0.45, 0.85, 0.82],
        };
        Scene {
            terrain,
            road,
            structures,
            props,
            detail,
            water,
            sky,
            clouds,
            hills,
            apron,
            env: Environment { sun_dir: n3(st.sun_dir), sun_color: st.sun_color, ambient: st.ambient, fog_color: st.fog, fog_density: st.fog_density, time: 0.0 },
            sky_top: st.sky_top,
            sky_horizon: st.sky_horizon,
            water_level: track.water_level,
            water_shallow,
            grade: {
                let mut g = grade_for(track.biome);
                if tod == TimeOfDay::Golden {
                    g.warmth += 0.4;
                    g.bloom *= 1.15;
                    g.softness += 0.1;
                    g.lift = [g.lift[0] * 0.99, g.lift[1] * 0.97, g.lift[2] * 1.03];
                }
                g
            },
            time_of_day: tod,
            biome: track.biome,
        }
    }

    /// Opaque shadow casters (call between `gpu.begin_shadow` / `end_shadow` with the light view)
    pub fn draw_shadow_casters(&self, gpu: &mut Gpu, light: &View) {
        let env = self.env;
        let p = DrawParams::default();
        gpu.draw(&self.structures, light, &env, Mat4::IDENTITY, &p, false);
        gpu.draw(&self.props, light, &env, Mat4::IDENTITY, &p, false);
    }

    pub fn draw_opaque(&self, gpu: &mut Gpu, view: &View) {
        let env = self.env;
        gpu.draw_sky(&self.sky, view, &env, self.sky_top, self.sky_horizon);
        let far = DrawParams { fog: false, cast_shadow: false, receive_shadow: false, ..Default::default() };
        let drift = Mat4::from_translation(Vec3::new(view.eye.x, 0.0, view.eye.z)) * Mat4::from_rotation_y(env.time * 0.004);
        gpu.draw(&self.clouds, view, &env, drift, &DrawParams { unlit: 0.55, ..far }, false);
        gpu.draw(&self.hills, view, &env, Mat4::IDENTITY, &DrawParams { unlit: 0.6, fog: true, fog_max: 0.62, ..far }, false);
        let ground = DrawParams { cast_shadow: false, ..Default::default() };
        if let Some(a) = &self.apron {
            gpu.draw(a, view, &env, Mat4::IDENTITY, &DrawParams { receive_shadow: false, ..ground }, false);
        }
        gpu.draw(&self.terrain, view, &env, Mat4::IDENTITY, &ground, false);
        if std::env::var("COZY_NOWATER").is_err() {
            if let Some(w) = &self.water {
                gpu.draw_water(w, view, &env, mix(self.sky_horizon, self.sky_top, 0.35), self.water_shallow);
            }
        }
        gpu.draw(&self.road, view, &env, Mat4::IDENTITY, &DrawParams { depth_bias: ROAD_BIAS, vertex_gloss: true, ..ground }, false);
        gpu.draw(&self.structures, view, &env, Mat4::IDENTITY, &DrawParams { rim: 0.3, ..Default::default() }, false);
        gpu.draw(&self.props, view, &env, Mat4::IDENTITY, &DrawParams { rim: 0.35, ..Default::default() }, false);
        gpu.draw(&self.detail, view, &env, Mat4::IDENTITY, &DrawParams { cull_dist: DETAIL_DIST, ..ground }, false);
    }

    pub fn draw_transparent(&self, _gpu: &mut Gpu, _view: &View) {}

    pub fn free(self, gpu: &mut Gpu) {
        gpu.free(self.terrain);
        gpu.free(self.road);
        gpu.free(self.structures);
        gpu.free(self.props);
        gpu.free(self.detail);
        gpu.free(self.sky);
        gpu.free(self.clouds);
        gpu.free(self.hills);
        if let Some(a) = self.apron {
            gpu.free(a);
        }
        if let Some(w) = self.water {
            gpu.free(w);
        }
    }
}
