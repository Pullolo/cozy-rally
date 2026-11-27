//! Night races: headlights (the only real light), high/low beam, lamps knocked out by big
//! hits, glowing tail lights and fireflies drifting over the meadows.
//!
//! Lighting model: the moon gives a faint blue fill (scene.rs `TimeOfDay::Night`), so what you
//! can actually read of the road is what your own lamps reach. Low beam is wide and short
//! (~55 m); high beam reaches ~115 m but is narrow, so it shows the next corner early and
//! leaves the verges dark. Crests are blind: the beams point at the sky until the nose drops.
use crate::game::{interp, RaceSession};
use crate::render::gpu::{DrawParams, Environment, Gpu, SpotLight, View, MAX_LIGHTS};
use crate::render::mesh::{rgba, Vertex};
use macroquad::math::{Mat4, Vec3};
use rally_core::biome::Biome;
use rally_core::world::WorldEvent;

/// Impact strength (m/s closing speed) that can knock out a headlamp
const LAMP_BREAK: f32 = 8.0;
/// A broken lamp sputters back to life over this many seconds
const LAMP_RECOVER: f32 = 18.0;
const FIREFLIES: usize = 70;

#[derive(Clone, Copy, Debug)]
struct Firefly {
    pos: Vec3,
    phase: f32,
    speed: f32,
}

pub struct Night {
    /// Per car: left / right lamp health 0..1
    pub lamps: Vec<[f32; 2]>,
    /// Per car: high beam on
    pub high: Vec<bool>,
    /// Seconds since a local player's lamp broke (for the HUD message)
    pub lamp_msg: Vec<f32>,
    fireflies: Vec<Firefly>,
    firefly_color: [f32; 3],
    seed: u32,
    /// Lamp brightness for the biome (white snow would wash out)
    power: f32,
    /// Circuit floodlight masts (lit zones only)
    posts: Vec<PostLight>,
}

/// A circuit floodlight: head position and where it is aimed (the nearest bit of track)
#[derive(Clone, Copy, Debug)]
struct PostLight {
    head: Vec3,
    dir: Vec3,
    range: f32,
}

/// Car headlamps take at most this many lights; floodlights get the rest
const CAR_LIGHTS: usize = 6;
/// Floodlights further than this are just glowing heads (no light pool)
const POST_LIGHT_DIST: f32 = 170.0;

fn hash(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x & 0xffffff) as f32 / 16_777_216.0
}

impl Night {
    pub fn new(cars: usize, track: &rally_core::track::Track, seed: u64) -> Night {
        let biome = track.biome;
        let posts = track
            .props
            .iter()
            .filter(|p| p.kind == rally_core::track::PropKind::LampPost)
            .map(|p| {
                let head = p.lamp_head();
                // aim at the middle of the nearest track, a little ahead of straight down
                let q = track.query(head, None);
                let aim = track.samples[q.index].pos;
                let to = aim - head;
                PostLight { head, dir: to.normalize_or(Vec3::NEG_Y), range: to.length() + 20.0 }
            })
            .collect();
        let firefly_color = match biome {
            Biome::Snow | Biome::Taiga => [0.0; 3], // no fireflies in the snow
            Biome::Desert => [1.0, 0.75, 0.45],
            Biome::Coast => [0.55, 1.0, 0.85],
            _ => [0.85, 1.0, 0.45],
        };
        Night { lamps: vec![[1.0, 1.0]; cars], high: vec![std::env::var("COZY_HIGHBEAM").is_ok_and(|v| v == "1"); cars], lamp_msg: vec![f32::INFINITY; cars], fireflies: Vec::new(), firefly_color, seed: seed as u32, posts, power: match biome {
            Biome::Snow | Biome::Taiga => 0.62,
            Biome::Desert => 0.82,
            _ => 1.0,
        } }
    }

    pub fn high_beam(&self, car: usize) -> bool {
        self.high.get(car).copied().unwrap_or(false)
    }

    /// Lamp flicker while a broken lamp sputters back (0 dark .. 1 full)
    fn lamp_level(&self, car: usize, side: usize, time: f32) -> f32 {
        let h = self.lamps[car][side];
        if h >= 1.0 {
            return 1.0;
        }
        // sputter: random dropouts that get rarer as the lamp recovers
        let k = (time * 13.0 + side as f32 * 7.3 + car as f32 * 3.1) as u32;
        let flick = if hash(k ^ self.seed) < 0.35 * (1.0 - h) { 0.15 } else { 1.0 };
        (h * h * 0.85).max(0.0) * flick
    }

    pub fn update(&mut self, sess_events: &[WorldEvent], local_cars: &[usize], dt: f32, eye: Option<Vec3>, track: &rally_core::track::Track, time: f32) {
        for e in sess_events {
            if let WorldEvent::Collision { car, other, strength, .. } = *e {
                if strength < LAMP_BREAK {
                    continue;
                }
                // the front takes it: pick a side from the hit, a second hard knock can take both
                for c in std::iter::once(car).chain(other) {
                    let k = hash((time * 1000.0) as u32 ^ (c as u32 * 7919) ^ self.seed);
                    let side = if k < 0.5 { 0 } else { 1 };
                    let lamps = &mut self.lamps[c];
                    if lamps[side] > 0.5 {
                        lamps[side] = 0.0;
                    } else {
                        lamps[1 - side] = 0.0;
                    }
                    self.lamp_msg[c] = 0.0;
                }
            }
        }
        for (c, l) in self.lamps.iter_mut().enumerate() {
            for h in l.iter_mut() {
                *h = (*h + dt / LAMP_RECOVER).min(1.0);
            }
            self.lamp_msg[c] += dt;
        }
        let _ = local_cars;
        self.update_fireflies(eye, track, dt, time);
    }

    fn update_fireflies(&mut self, eye: Option<Vec3>, track: &rally_core::track::Track, dt: f32, time: f32) {
        let Some(eye) = eye else { return };
        if self.firefly_color == [0.0; 3] {
            return;
        }
        // a cloud of fireflies lives around the camera; ones left behind respawn ahead/around
        let mut n = self.seed.wrapping_add((time * 60.0) as u32);
        let spawn = |n: &mut u32| -> Firefly {
            *n = n.wrapping_add(0x9e37_79b9);
            let a = hash(*n) * std::f32::consts::TAU;
            let r = 6.0 + hash(*n ^ 0x55) * 38.0;
            let x = eye.x + a.cos() * r;
            let z = eye.z + a.sin() * r;
            let y = track.terrain.height(x, z) + 0.4 + hash(*n ^ 0xaa) * 2.2;
            Firefly { pos: Vec3::new(x, y, z), phase: hash(*n ^ 0x33) * 50.0, speed: 0.3 + hash(*n ^ 0x77) * 0.5 }
        };
        while self.fireflies.len() < FIREFLIES {
            let f = spawn(&mut n);
            self.fireflies.push(f);
        }
        for f in self.fireflies.iter_mut() {
            let t = time * f.speed + f.phase;
            f.pos += Vec3::new((t * 1.3).sin(), (t * 2.1).sin() * 0.35, (t * 0.9).cos()) * dt * 0.6;
            let ground = track.terrain.height(f.pos.x, f.pos.z);
            f.pos.y = f.pos.y.clamp(ground + 0.25, ground + 3.0);
            if Vec3::new(f.pos.x - eye.x, 0.0, f.pos.z - eye.z).length() > 48.0 {
                *f = spawn(&mut n);
            }
        }
    }

    /// Headlamps for one viewport: the viewer's car gets both lamps, the nearest other cars one
    /// combined lamp each (MAX_LIGHTS in total).
    pub fn lights(&self, sess: &RaceSession, viewer: Option<usize>, eye: Vec3) -> Vec<SpotLight> {
        let mut out = Vec::new();
        let mut order: Vec<usize> = (0..sess.world.cars.len()).collect();
        order.sort_by(|&a, &b| {
            let pa = sess.world.cars[a].pos.distance_squared(eye);
            let pb = sess.world.cars[b].pos.distance_squared(eye);
            pa.total_cmp(&pb)
        });
        if let Some(v) = viewer {
            order.retain(|&c| c != v);
            order.insert(0, v);
        }
        for c in order {
            if out.len() >= CAR_LIGHTS {
                break;
            }
            let car = &sess.world.cars[c];
            let (pos, rot) = interp(car, sess.alpha);
            if pos.distance(eye) > 260.0 {
                continue;
            }
            let he = car.spec.half_extents;
            let high = self.high_beam(c);
            // beams: low = wide & short, dipped; high = narrow & long
            let (range, inner, outer, power0, dip) = if high { (118.0, 9.0f32, 24.0f32, 2.9, 0.012) } else { (58.0, 15.0f32, 34.0f32, 2.3, 0.07) };
            let power = power0 * self.power;
            let dir = (rot * Vec3::new(0.0, -dip, -1.0)).normalize();
            let warm = [1.0, 0.94, 0.80];
            let levels = [self.lamp_level(c, 0, sess.time), self.lamp_level(c, 1, sess.time)];
            let sides = [-1.0, 1.0];
            let both = Some(c) == viewer && out.len() + 2 <= MAX_LIGHTS;
            if both {
                for k in 0..2 {
                    if levels[k] <= 0.01 {
                        continue;
                    }
                    let local = Vec3::new(sides[k] * he.x * 0.62, -he.y * 0.1, -he.z * 0.95);
                    let p = power * 0.55 * levels[k];
                    out.push(SpotLight { pos: pos + rot * local, dir, color: [warm[0] * p, warm[1] * p, warm[2] * p], range, cos_inner: inner.to_radians().cos(), cos_outer: outer.to_radians().cos() });
                }
            } else {
                let total = levels[0] + levels[1];
                if total <= 0.02 {
                    continue;
                }
                // one lamp at the weighted centre of the working ones
                let x = (levels[1] - levels[0]) / total * he.x * 0.62;
                let p = power * 0.55 * total;
                out.push(SpotLight { pos: pos + rot * Vec3::new(x, -he.y * 0.1, -he.z * 0.95), dir, color: [warm[0] * p, warm[1] * p, warm[2] * p], range, cos_inner: inner.to_radians().cos(), cos_outer: outer.to_radians().cos() });
            }
        }
        self.post_lights(&mut out, viewer.map(|v| {
            let (p, r) = interp(&sess.world.cars[v], sess.alpha);
            p + r * Vec3::new(0.0, 0.0, -25.0)
        }).unwrap_or(eye));
        out
    }

    /// The floodlights nearest to `focus` (a point a little ahead of the viewer's car).
    /// The last ones in the budget fade with distance so pools never pop on or off.
    fn post_lights(&self, out: &mut Vec<SpotLight>, focus: Vec3) {
        let budget = MAX_LIGHTS - out.len();
        if budget == 0 || self.posts.is_empty() {
            return;
        }
        let mut near: Vec<(f32, &PostLight)> = self.posts.iter().map(|p| (p.head.distance(focus), p)).filter(|(d, _)| *d < POST_LIGHT_DIST).collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        // the first light that doesn't fit sets the fade-out distance
        let cutoff = near.get(budget).map_or(POST_LIGHT_DIST, |n| n.0);
        let warm = [1.0, 0.8, 0.55];
        for &(d, p) in near.iter().take(budget) {
            let w = ((cutoff - d) / 22.0).clamp(0.0, 1.0) * ((POST_LIGHT_DIST - d) / 40.0).clamp(0.0, 1.0);
            if w <= 0.01 {
                continue;
            }
            let k = 2.8 * w * self.power;
            out.push(SpotLight { pos: p.head, dir: p.dir, color: [warm[0] * k, warm[1] * k, warm[2] * k], range: p.range, cos_inner: 32f32.to_radians().cos(), cos_outer: 66f32.to_radians().cos() });
        }
    }

    /// Glowing floodlight lenses with a soft halo (every mast in view, lit or not)
    pub fn draw_posts(&self, gpu: &mut Gpu, view: &View, env: &Environment) {
        if self.posts.is_empty() {
            return;
        }
        let mut verts: Vec<Vertex> = Vec::new();
        let mut idx: Vec<u16> = Vec::new();
        let to_cam = |p: Vec3| (view.eye - p).normalize_or_zero();
        for p in &self.posts {
            let d = p.head.distance(view.eye);
            if d > 600.0 || !view.sphere_visible(p.head, 2.0) || verts.len() > 60000 {
                continue;
            }
            let tc = to_cam(p.head);
            let right = tc.cross(Vec3::Y).normalize_or_zero();
            let up = right.cross(tc);
            // halo: bright core fading to a transparent ring (bigger far away so it reads)
            let r = 1.1 + d * 0.012;
            let b = verts.len() as u16;
            let core = p.head - Vec3::Y * 0.08 + tc * 0.4;
            verts.push(Vertex { pos: core.to_array(), normal: [0.0, 1.0, 0.0], color: rgba([1.0, 0.86, 0.62], 0.95) });
            let ring = 10u16;
            for j in 0..ring {
                let a = j as f32 / ring as f32 * std::f32::consts::TAU;
                let q = core + (right * a.cos() + up * a.sin() * 0.8) * r;
                verts.push(Vertex { pos: q.to_array(), normal: [0.0, 1.0, 0.0], color: rgba([1.0, 0.7, 0.4], 0.0) });
            }
            for j in 0..ring {
                idx.extend_from_slice(&[b, b + 1 + j, b + 1 + (j + 1) % ring]);
            }
        }
        if !idx.is_empty() {
            let p = DrawParams { unlit: 1.0, tint: [2.0, 1.9, 1.7, 1.0], cast_shadow: false, receive_shadow: false, ..Default::default() };
            gpu.draw_dynamic(&verts, &idx, view, env, Mat4::IDENTITY, &p, 1);
        }
    }

    /// Headlight lens brightness for drawing a car (matches the lamp health)
    pub fn lens_tint(&self, car: usize, time: f32) -> [f32; 4] {
        let l = (self.lamp_level(car, 0, time) + self.lamp_level(car, 1, time)) * 0.5;
        let k = 0.35 + 1.9 * l;
        [1.2 * k, 1.15 * k, 1.0 * k, 1.0]
    }

    /// Fireflies: tiny glowing camera-facing diamonds that pulse slowly
    pub fn draw_fireflies(&self, gpu: &mut Gpu, view: &View, env: &Environment, time: f32) {
        if self.fireflies.is_empty() {
            return;
        }
        let mut verts: Vec<Vertex> = Vec::with_capacity(self.fireflies.len() * 4);
        let mut idx: Vec<u16> = Vec::with_capacity(self.fireflies.len() * 6);
        let c = self.firefly_color;
        for f in &self.fireflies {
            let pulse = ((time * 1.7 + f.phase).sin() * 0.5 + 0.5).powf(3.0);
            if pulse < 0.05 {
                continue;
            }
            let to_cam = (view.eye - f.pos).normalize_or_zero();
            let right = to_cam.cross(Vec3::Y).normalize_or_zero();
            let up = right.cross(to_cam);
            let s = 0.05 + 0.05 * pulse;
            let b = verts.len() as u16;
            let col = rgba([c[0] * (1.0 + pulse), c[1] * (1.0 + pulse), c[2]], 0.35 + 0.65 * pulse);
            for o in [up, right, -up, -right] {
                let p = f.pos + o * s;
                verts.push(Vertex { pos: p.to_array(), normal: [0.0, 1.0, 0.0], color: col });
            }
            idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        if !idx.is_empty() {
            let p = DrawParams { unlit: 1.0, tint: [2.2, 2.2, 2.2, 1.0], cast_shadow: false, receive_shadow: false, ..Default::default() };
            gpu.draw_dynamic(&verts, &idx, view, env, Mat4::IDENTITY, &p, 1);
        }
    }
}
