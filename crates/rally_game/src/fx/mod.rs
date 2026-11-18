//! Visual effects: wheel spray & tyre smoke, skid marks, impact bursts, splashes, confetti,
//! exhaust puffs and ambient biome life. Everything is generated in code and drawn as
//! vertex-coloured geometry through the stream buffer in a handful of draw calls per view.
//!
//! Layout:
//! * `particle` — particle struct + simulation (one flat array, swap-remove)
//! * `emit`     — per-car emitters (wheels, exhaust, water) and world-event bursts
//! * `ambient`  — biome life spawned around local players
//! * `marks`    — tyre-mark ribbons (ring buffers)
//! * `render`   — billboard / tumbling-bit geometry builders and batching
mod ambient;
mod emit;
mod marks;
mod particle;
mod render;

use crate::render::gpu::{DrawParams, Environment, Gpu, View};
use macroquad::math::{Quat, Vec3};
use marks::{MarkChain, MarkRing};
use particle::{Particle, Shape, SimCtx};
use rally_core::biome::Biome;
use rally_core::world::{Controller, World, WorldEvent};
use render::{Basis, Batches, Circle};
use std::cell::RefCell;

#[allow(unused_imports)]
pub use emit::{wheel_traction, Traction};

/// Hard cap on live particles (all views share them)
pub const MAX_PARTICLES: usize = 4000;
/// Above this, low-priority (ambient) spawns are skipped
const SOFT_AMBIENT: usize = 3000;
/// Above this, continuous spray spawns are skipped (bursts may still use the rest)
const SOFT_SPRAY: usize = 3600;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prio {
    Ambient,
    Spray,
    Burst,
}

/// Per-car emitter state
#[derive(Clone, Default)]
struct CarFx {
    /// fractional emission accumulators per wheel: dust, debris, smoke, water
    acc: [[f32; 4]; 4],
    exhaust_acc: f32,
    wake_acc: f32,
    prev_wheel: [Vec3; 4],
    has_prev: bool,
    prev_shifting: bool,
    prev_in_water: bool,
    chains: [MarkChain; 4],
    /// time each wheel has been off the ground (marks bridge small hops)
    air: [f32; 4],
}

/// Small fast RNG (xorshift32)
pub struct FxRng(u32);

impl FxRng {
    #[inline]
    pub fn f(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / 16_777_216.0
    }
    #[inline]
    pub fn r(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
    /// Random unit vector in the horizontal plane
    #[inline]
    pub fn hdir(&mut self) -> Vec3 {
        let a = self.f() * std::f32::consts::TAU;
        Vec3::new(a.cos(), 0.0, a.sin())
    }
    /// Random point in the unit ball
    pub fn ball(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.r(-1.0, 1.0), self.r(-1.0, 1.0), self.r(-1.0, 1.0));
            if v.length_squared() <= 1.0 {
                return v;
            }
        }
    }
    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.f() * items.len() as f32) as usize).min(items.len() - 1)]
    }
}

struct DrawScratch {
    lit: Batches,
    glow: Batches,
    marks: Batches,
    order: Vec<(u32, u16)>,
    glow_list: Vec<u16>,
}

/// Start/finish arch geometry (for confetti)
#[derive(Clone, Copy)]
struct Arch {
    centre: Vec3,
    right: Vec3,
    fwd: Vec3,
    half: f32,
    post_l: Vec3,
    post_r: Vec3,
}

pub struct Fx {
    parts: Vec<Particle>,
    cars: Vec<CarFx>,
    skids: MarkRing,
    tracks: MarkRing,
    rng: FxRng,
    time: f32,
    biome: Biome,
    wind: Vec3,
    arch: Arch,
    /// Interpolated (render) poses of all cars for this frame
    poses: Vec<(Vec3, Quat)>,
    /// Indices of locally-controlled cars
    locals: Vec<usize>,
    ambient_counts: Vec<[u16; 16]>,
    circle: Circle,
    scratch: RefCell<DrawScratch>,
    // debug / profiling hooks
    debug_confetti: Option<f32>,
    stats: bool,
    stat_update: f64,
    stat_draw: RefCell<(f64, usize)>,
    stat_frames: u32,
    stat_timer: f32,
    /// peak wheelspin / slide / lock seen since the last stats line
    stat_trac: [f32; 3],
}

impl Fx {
    pub fn new(world: &World) -> Fx {
        let track = &world.track;
        let s0 = &track.samples[0];
        let fr = Vec3::new(s0.right.x, 0.0, s0.right.z).normalize_or(Vec3::X);
        let fwd = Vec3::new(s0.tangent.x, 0.0, s0.tangent.z).normalize_or(Vec3::Z);
        let hw = s0.half_width + 1.2;
        let g = |p: Vec3| Vec3::new(p.x, track.terrain.height(p.x, p.z), p.z);
        let post_l = g(s0.pos - fr * hw);
        let post_r = g(s0.pos + fr * hw);
        let top = g(s0.pos).y.max(post_l.y).max(post_r.y);
        let arch = Arch { centre: Vec3::new(s0.pos.x, top + 5.0, s0.pos.z), right: fr, fwd, half: hw, post_l, post_r };
        let wind = match track.biome {
            Biome::Meadow => Vec3::new(0.9, 0.0, 0.4),
            Biome::Autumn => Vec3::new(1.3, 0.0, 0.6),
            Biome::Snow => Vec3::new(0.6, 0.0, -0.35),
            Biome::Desert => Vec3::new(2.6, 0.0, 1.1),
            Biome::Coast => Vec3::new(2.0, 0.0, -0.9),
        };
        let locals = (0..world.cars.len()).filter(|&i| matches!(world.controllers[i], Controller::Local(_))).collect();
        Fx {
            parts: Vec::with_capacity(MAX_PARTICLES),
            cars: vec![CarFx::default(); world.cars.len()],
            skids: MarkRing::new(2400, 28.0),
            tracks: MarkRing::new(2600, 9.0),
            rng: FxRng(0x9E37_79B9 ^ (track.params.seed as u32).wrapping_mul(2654435761) | 1),
            time: 0.0,
            biome: track.biome,
            wind,
            arch,
            poses: Vec::new(),
            locals,
            ambient_counts: Vec::new(),
            circle: Circle::new(),
            scratch: RefCell::new(DrawScratch { lit: Batches::default(), glow: Batches::default(), marks: Batches::default(), order: Vec::new(), glow_list: Vec::new() }),
            debug_confetti: std::env::var("COZY_FX_CONFETTI").ok().and_then(|v| v.parse().ok()),
            stats: std::env::var("COZY_FX_STATS").is_ok(),
            stat_update: 0.0,
            stat_draw: RefCell::new((0.0, 0)),
            stat_frames: 0,
            stat_timer: 0.0,
            stat_trac: [0.0; 3],
        }
    }

    #[allow(dead_code)]
    pub fn particle_count(&self) -> usize {
        self.parts.len()
    }

    #[inline]
    fn spawn(&mut self, p: Particle, prio: Prio) -> bool {
        let limit = match prio {
            Prio::Ambient => SOFT_AMBIENT,
            Prio::Spray => SOFT_SPRAY,
            Prio::Burst => MAX_PARTICLES,
        };
        if self.parts.len() >= limit {
            return false;
        }
        self.parts.push(p);
        true
    }

    /// Distance from `p` to the nearest local player's car (or 0 if there are none,
    /// e.g. attract mode: then everything counts as near).
    fn nearest_local(&self, p: Vec3) -> f32 {
        let mut best = f32::MAX;
        for &i in &self.locals {
            best = best.min(self.poses[i].0.distance(p));
        }
        if self.locals.is_empty() {
            0.0
        } else {
            best
        }
    }

    /// Called once per frame after the world advanced. `alpha` = render interpolation.
    pub fn update(&mut self, world: &World, events: &[WorldEvent], alpha: f32, dt: f32) {
        let t0 = if self.stats { Some(std::time::Instant::now()) } else { None };
        if dt <= 0.0 {
            return;
        }
        self.time += dt;
        if self.cars.len() != world.cars.len() {
            self.cars.resize(world.cars.len(), CarFx::default());
        }
        self.poses.clear();
        for c in &world.cars {
            self.poses.push(crate::game::interp(c, alpha));
        }

        // simulate
        let cx = SimCtx { track: &world.track, wind: self.wind };
        for p in self.parts.iter_mut() {
            particle::step(p, dt, &cx);
        }
        // despawn: dead, or ambient life that drifted away from every player
        let mut i = 0;
        while i < self.parts.len() {
            let p = &self.parts[i];
            let kill = p.dead() || (p.ambient && self.nearest_local(p.pos) > ambient::KILL_RADIUS);
            if kill {
                self.parts.swap_remove(i);
            } else {
                i += 1;
            }
        }

        self.handle_events(world, events);
        if let Some(t) = self.debug_confetti {
            if self.time >= t {
                self.debug_confetti = None;
                self.finish_confetti();
            }
        }
        self.emit_cars(world, dt);
        self.update_ambient(world, dt);

        if let Some(t0) = t0 {
            self.stat_update += t0.elapsed().as_secs_f64();
            self.stat_frames += 1;
            self.stat_timer += dt;
            if self.stat_timer > 2.0 {
                let (dsum, dverts) = *self.stat_draw.borrow();
                println!(
                    "[fx] {} particles, {} skid + {} track segs | update {:.3} ms/frame, draw {:.3} ms/frame, {} verts/frame | peak spin {:.2} slide {:.2} lock {:.2}",
                    self.parts.len(),
                    self.skids.segs.len(),
                    self.tracks.segs.len(),
                    self.stat_update * 1000.0 / self.stat_frames as f64,
                    dsum * 1000.0 / self.stat_frames as f64,
                    dverts / self.stat_frames as usize,
                    self.stat_trac[0],
                    self.stat_trac[1],
                    self.stat_trac[2],
                );
                self.stat_trac = [0.0; 3];
                self.stat_update = 0.0;
                *self.stat_draw.borrow_mut() = (0.0, 0);
                self.stat_frames = 0;
                self.stat_timer = 0.0;
            }
        }
    }

    /// Called per viewport inside the 3D pass, after opaque geometry and cars.
    pub fn draw(&self, gpu: &mut Gpu, view: &View, env: &Environment) {
        let t0 = if self.stats { Some(std::time::Instant::now()) } else { None };
        let mut guard = self.scratch.borrow_mut();
        let sc = &mut *guard;
        let bs = Basis::from_view(view);

        // tyre marks first (they hug the ground)
        sc.marks.clear();
        self.tracks.build(&mut sc.marks, self.time, bs.eye, 140.0);
        self.skids.build(&mut sc.marks, self.time, bs.eye, 160.0);
        for c in &self.cars {
            for ch in &c.chains {
                ch.build_head(&mut sc.marks);
            }
        }
        sc.marks.draw(gpu, view, env, &DrawParams { unlit: 0.15, ..Default::default() });

        // particles: lit ones sorted back-to-front, glows after
        sc.order.clear();
        sc.glow_list.clear();
        for (i, p) in self.parts.iter().enumerate() {
            let d = p.pos.distance(bs.eye);
            let reach = match p.shape {
                Shape::Puff => 170.0,
                Shape::Gull | Shape::Tumbleweed => 200.0,
                Shape::Glow | Shape::Star | Shape::Spark => 140.0,
                _ => 90.0,
            };
            if d > reach || !view.sphere_visible(p.pos, p.size().max(p.size1) + 0.5) {
                continue;
            }
            if p.shape.glows() {
                sc.glow_list.push(i as u16);
            } else {
                sc.order.push((d.to_bits(), i as u16));
            }
        }
        sc.order.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        sc.lit.clear();
        for &(dbits, i) in &sc.order {
            let p = &self.parts[i as usize];
            let d = f32::from_bits(dbits);
            match p.shape {
                Shape::Puff => render::puff(&mut sc.lit, &bs, &self.circle, p, d),
                Shape::Butterfly | Shape::Gull => render::critter(&mut sc.lit, p, d),
                Shape::Tumbleweed => render::tumbleweed(&mut sc.lit, &bs, p, d),
                _ => render::bit(&mut sc.lit, &bs, p, d),
            }
        }
        sc.lit.draw(gpu, view, env, &DrawParams { unlit: 0.5, ..Default::default() });
        sc.glow.clear();
        for &i in &sc.glow_list {
            let p = &self.parts[i as usize];
            render::glow(&mut sc.glow, &bs, &self.circle, p, p.pos.distance(bs.eye), self.time);
        }
        sc.glow.draw(gpu, view, env, &DrawParams { unlit: 1.0, ..Default::default() });

        if let Some(t0) = t0 {
            let verts = sc.marks.vertex_count() + sc.lit.vertex_count() + sc.glow.vertex_count();
            let mut s = self.stat_draw.borrow_mut();
            s.0 += t0.elapsed().as_secs_f64();
            s.1 += verts;
        }
    }
}
