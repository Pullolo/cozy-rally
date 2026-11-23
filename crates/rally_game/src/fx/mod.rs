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
mod weather;

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
    /// fractional emission accumulators per wheel (spin jet, spin debris, slide fan, slide
    /// debris, lock puffs, lock debris, rolling debris, water)
    acc: [[f32; 12]; 4],
    trail_acc: f32,
    mist_acc: f32,
    exhaust_acc: f32,
    wake_acc: f32,
    prev_wheel: [Vec3; 4],
    has_prev: bool,
    prev_shifting: bool,
    prev_gear: i32,
    /// wheel was on a kerb last frame (circuits)
    prev_kerb: [bool; 4],
    scrape_acc: f32,
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
    /// Weather particles (rain / snow / sand / ripples) + smoothed per-player conditions
    wx: weather::WeatherFx,
    track: Option<std::sync::Arc<rally_core::track::Track>>,
    /// Biome default wind (used when the track has no conditions timeline)
    base_wind: Vec3,
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
    /// draw time, vertices, draw calls, puff screen coverage
    stat_draw: RefCell<(f64, usize, usize, f32)>,
    stat_frames: u32,
    stat_timer: f32,
    /// peak wheelspin / slide / lock seen since the last stats line
    stat_trac: [f32; 3],
    /// `COZY_FX_TEST=launch|slide|lock`: force that traction state on player 1's car
    test: Option<TestMode>,
    test_announced: bool,
    /// `COZY_FX_OFF=1`: skip all FX drawing (debug: tell FX apart from scene issues)
    disabled: bool,
    go_time: Option<f32>,
}

/// Deterministic traction-loss showcase (visual testing). Overrides only the FX's view of
/// player 1's wheels; physics is untouched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TestMode {
    /// Driven wheels spinning flat out: from 2 s into the countdown until 2.5 s after GO
    Launch,
    /// Rear-biased slide to the car's right, from GO+1.5 s for 8 s
    Slide,
    /// All four wheels locked, from GO+1.5 s for 8 s
    Lock,
    /// Floor scraping (sparks) from GO+1.5 s for 8 s
    Scrape,
    /// Wheels treated as in a gravel trap, GO+1.5 s for 8 s
    Trap,
    /// A kerb strike on every wheel twice a second, GO+1.5 s for 8 s
    Kerb,
}

impl Fx {
    pub fn new(world: &World) -> Fx {
        // 3D rain / snow below replace the 2D stand-in streaks
        crate::weather_view::set_overlay_2d(false);
        render::NO_GUARD.store(std::env::var("COZY_FX_NOGUARD").is_ok(), std::sync::atomic::Ordering::Relaxed);
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
            wx: weather::WeatherFx::new(track),
            track: Some(world.track.clone()),
            base_wind: wind,
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
            stat_draw: RefCell::new((0.0, 0, 0, 0.0)),
            stat_frames: 0,
            stat_timer: 0.0,
            stat_trac: [0.0; 3],
            test: match std::env::var("COZY_FX_TEST").as_deref() {
                Ok("launch") => Some(TestMode::Launch),
                Ok("slide") => Some(TestMode::Slide),
                Ok("lock") => Some(TestMode::Lock),
                Ok("scrape") => Some(TestMode::Scrape),
                Ok("trap") => Some(TestMode::Trap),
                Ok("kerb") => Some(TestMode::Kerb),
                _ => None,
            },
            test_announced: false,
            disabled: std::env::var("COZY_FX_OFF").is_ok(),
            go_time: None,
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

    /// Conditions at a car's position on the stage (default when the track has none)
    fn weather_for_car(&self, world: &World, ci: usize) -> rally_core::weather::Weather {
        if !self.wx.active {
            return rally_core::weather::Weather { wind: [0.0, 0.0], ..Default::default() };
        }
        world.race.racers.get(ci).map_or_else(Default::default, |r| world.track.conditions.at(r.s))
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
        // weather at each local player's position (smoothed), drives precipitation + wind
        if self.wx.active {
            let targets: Vec<rally_core::weather::Weather> = self.locals.iter().map(|&ci| self.weather_for_car(world, ci)).collect();
            self.wx.update(&targets, dt);
            if let Some(w) = self.wx.smooth.first() {
                self.wind = Vec3::new(w.wind[0], 0.0, w.wind[1]) * 0.8 + self.base_wind * 0.2;
            }
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
            let kill_r = if p.ambient { ambient::kill_radius(p.group) } else { 0.0 };
            let kill = p.dead() || (p.ambient && self.nearest_local(p.pos) > kill_r);
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
                let (dsum, dverts, dcalls, dcover) = *self.stat_draw.borrow();
                println!(
                    "[fx] {} particles, {} skid + {} track segs | update {:.3} ms/frame, draw {:.3} ms/frame, {} verts/frame, {:.1} draw calls/frame, puff coverage {:.2} | peak spin {:.2} slide {:.2} lock {:.2}",
                    self.parts.len(),
                    self.skids.segs.len(),
                    self.tracks.segs.len(),
                    self.stat_update * 1000.0 / self.stat_frames as f64,
                    dsum * 1000.0 / self.stat_frames as f64,
                    dverts / self.stat_frames as usize,
                    dcalls as f32 / self.stat_frames as f32,
                    dcover / self.stat_frames as f32,
                    self.stat_trac[0],
                    self.stat_trac[1],
                    self.stat_trac[2],
                );
                self.stat_trac = [0.0; 3];
                self.stat_update = 0.0;
                *self.stat_draw.borrow_mut() = (0.0, 0, 0, 0.0);
                self.stat_frames = 0;
                self.stat_timer = 0.0;
            }
        }
    }

    /// Called per viewport inside the 3D pass, after opaque geometry and cars.
    pub fn draw(&self, gpu: &mut Gpu, view: &View, env: &Environment) {
        if self.disabled {
            return;
        }
        let t0 = if self.stats { Some(std::time::Instant::now()) } else { None };
        let mut guard = self.scratch.borrow_mut();
        let sc = &mut *guard;
        let bs = Basis::from_view(view);
        // the local player this view follows (nearest local car to the eye)
        let view_player = (0..self.locals.len()).min_by(|&a, &b| self.poses[self.locals[a]].0.distance(bs.eye).total_cmp(&self.poses[self.locals[b]].0.distance(bs.eye)));
        let weather = if self.wx.active { view_player.and_then(|k| self.wx.smooth.get(k).copied()) } else { None };
        let mut cover = 0.0f32;

        // tyre marks first (they hug the ground)
        sc.marks.clear();
        self.tracks.build(&mut sc.marks, self.time, bs.eye, 140.0);
        self.skids.build(&mut sc.marks, self.time, bs.eye, 160.0);
        for c in &self.cars {
            for ch in &c.chains {
                ch.build_head(&mut sc.marks);
            }
        }
        if let (Some(w), Some(track)) = (weather.as_ref(), self.track.as_ref()) {
            self.wx.build_ground(&mut sc.marks, &bs, w, track, self.time);
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
                Shape::Twinkle => 170.0,
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
        // the car this view follows (nearest local car to the eye): clouds between it and
        // the camera are faded so the player's own car always stays readable
        let focus = self
            .locals
            .iter()
            .map(|&ci| self.poses[ci].0)
            .filter(|p| p.distance(bs.eye) < 25.0)
            .min_by(|a, b| a.distance(bs.eye).total_cmp(&b.distance(bs.eye)))
            .map(|p| p + Vec3::Y * 0.3);
        sc.lit.clear();
        for &(dbits, i) in &sc.order {
            let p = &self.parts[i as usize];
            let d = f32::from_bits(dbits);
            match p.shape {
                Shape::Puff => cover += render::puff(&mut sc.lit, &bs, &self.circle, p, d, focus),
                Shape::Butterfly | Shape::Gull => render::critter(&mut sc.lit, p, d),
                Shape::Tumbleweed => render::tumbleweed(&mut sc.lit, &bs, p, d),
                _ => render::bit(&mut sc.lit, &bs, p, d),
            }
        }
        sc.lit.draw(gpu, view, env, &DrawParams { unlit: 0.6, ..Default::default() });
        sc.glow.clear();
        for &i in &sc.glow_list {
            let p = &self.parts[i as usize];
            render::glow(&mut sc.glow, &bs, &self.circle, p, p.pos.distance(bs.eye), self.time);
        }
        if let Some(w) = weather.as_ref() {
            self.wx.build_air(&mut sc.glow, &bs, w, self.biome, self.time);
        }
        sc.glow.draw(gpu, view, env, &DrawParams { unlit: 1.0, ..Default::default() });

        if let Some(t0) = t0 {
            let verts = sc.marks.vertex_count() + sc.lit.vertex_count() + sc.glow.vertex_count();
            let mut s = self.stat_draw.borrow_mut();
            s.0 += t0.elapsed().as_secs_f64();
            s.1 += verts;
            s.2 += sc.marks.calls() + sc.lit.calls() + sc.glow.calls();
            s.3 += cover;
        }
    }
}


impl Fx {
    /// Forced traction state for the showcase test hook (None = use the real one)
    /// The active showcase test mode for car `ci` (player 1 only), if any
    fn test_mode(&mut self, ci: usize) -> Option<TestMode> {
        let mode = self.test?;
        if self.locals.first() != Some(&ci) {
            return None;
        }
        let t = self.time;
        let active = match (mode, self.go_time) {
            (TestMode::Launch, None) => t >= 2.0,
            (TestMode::Launch, Some(g)) => t < g + 2.5,
            (_, Some(g)) => t >= g + 1.5 && t < g + 9.5,
            _ => false,
        };
        if !active {
            return None;
        }
        if !self.test_announced {
            self.test_announced = true;
            println!("[fx] test {:?} active at fx time {:.2}", mode, t);
        }
        Some(mode)
    }

    fn test_traction(&mut self, ci: usize, wi: usize, car: &rally_core::vehicle::Car) -> Option<Traction> {
        let mode = self.test_mode(ci)?;
        if !matches!(mode, TestMode::Launch | TestMode::Slide | TestMode::Lock) {
            return None;
        }
        let fwd = car.rot * Vec3::NEG_Z;
        let right = car.rot * Vec3::X;
        let mut tr = Traction { fwd, lat_dir: right, surf_sign: 1.0, ..Default::default() };
        match mode {
            TestMode::Launch => {
                use rally_core::vehicle::Drivetrain;
                let driven = match car.spec.drivetrain {
                    Drivetrain::Fwd => wi < 2,
                    Drivetrain::Rwd => wi >= 2,
                    Drivetrain::Awd => true,
                };
                tr.spin = if driven { 1.0 } else { 0.0 };
            }
            TestMode::Slide => tr.slide = if wi >= 2 { 1.0 } else { 0.7 },
            TestMode::Lock => tr.lock = 1.0,
            _ => {}
        }
        tr.loss = tr.spin.max(tr.slide).max(tr.lock);
        Some(tr)
    }
}
