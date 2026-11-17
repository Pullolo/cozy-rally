//! Per-car emitters (wheel spray, tyre smoke, debris, marks, exhaust, water) and bursts for
//! world events (landings, collisions, splashes, confetti).
use super::marks::MarkSeg;
use super::particle::{Ground, Particle, Shape};
use super::{Fx, Prio};
use crate::render::mesh::{mix, shade};
use macroquad::math::{Quat, Vec3};
use rally_core::race::RaceEvent;
use rally_core::surface::Surface;
use rally_core::track::PropKind;
use rally_core::vehicle::Car;
use rally_core::world::{Controller, World, WorldEvent};

/// How far beyond grip a tyre is, split by cause. All 0..1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Traction {
    /// Wheelspin: tyre surface faster than the ground
    pub spin: f32,
    /// Lock-up: tyre surface slower than the ground (braking / handbrake)
    pub lock: f32,
    /// Lateral sliding
    pub slide: f32,
    /// Overall traction loss (max of the above)
    pub loss: f32,
    /// Wheel heading projected on the ground
    pub fwd: Vec3,
    /// Direction the contact patch slides sideways (unit, on ground)
    pub lat_dir: Vec3,
    /// +1 if the tyre surface moves forward relative to the car (driving forward)
    pub surf_sign: f32,
}

/// Traction loss for wheel `i`. Derived locally from the existing wheel state; when
/// rally_core grows `WheelState::traction_loss` / `slip_ratio` / `slip_angle`, swap the body
/// of this function for those fields.
pub fn wheel_traction(car: &Car, i: usize) -> Traction {
    let w = &car.wheels[i];
    if !w.contact {
        return Traction::default();
    }
    let n = w.normal.normalize_or(Vec3::Y);
    let heading = car.rot * (Quat::from_rotation_y(-w.steer_angle) * Vec3::NEG_Z);
    let f = (heading - n * heading.dot(n)).normalize_or_zero();
    let r = f.cross(n).normalize_or_zero();
    let vc = car.point_velocity(w.contact_point);
    let v_long = vc.dot(f);
    let v_lat = vc.dot(r);
    let surf = w.spin_speed * car.spec.wheel_radius;
    // wheelspin: tyre surface outruns the ground (or spins against it)
    let spin_ex = if surf * v_long >= 0.0 { (surf.abs() - v_long.abs()).max(0.0) } else { (surf - v_long).abs() };
    let spin = ((spin_ex - 1.2) / 7.0).clamp(0.0, 1.0);
    // lock-up: tyre surface much slower than the ground
    let lock_ex = if surf * v_long >= 0.0 { (v_long.abs() - surf.abs()).max(0.0) } else { 0.0 };
    let lock = ((lock_ex / v_long.abs().max(3.0) - 0.2) / 0.55).clamp(0.0, 1.0) * ((v_long.abs() - 1.5) / 4.0).clamp(0.0, 1.0);
    let slide = ((w.slide_lat.max(v_lat.abs() * w.skid) - 0.9) / 5.0).clamp(0.0, 1.0);
    let loss = spin.max(lock).max(slide).max(w.skid * 0.8);
    Traction { spin, lock, slide, loss, fwd: f, lat_dir: r * v_lat.signum(), surf_sign: if surf >= 0.0 { 1.0 } else { -1.0 } }
}

const PASTEL: [[f32; 3]; 8] = [
    [1.0, 0.56, 0.62],
    [1.0, 0.83, 0.42],
    [0.55, 0.80, 1.0],
    [0.62, 0.90, 0.60],
    [0.80, 0.66, 1.0],
    [1.0, 0.70, 0.46],
    [1.0, 0.98, 0.95],
    [0.50, 0.90, 0.88],
];

fn dust_color(s: Surface) -> [f32; 3] {
    // pastel-ified surface dust
    let d = s.props().dust;
    match s {
        Surface::Snow | Surface::Ice => [0.97, 0.98, 1.0],
        Surface::Grass => mix(d, [0.92, 0.95, 0.80], 0.45),
        Surface::Mud => mix(d, [0.85, 0.75, 0.62], 0.35),
        _ => mix(d, [1.0, 0.97, 0.92], 0.45),
    }
}

fn mark_style(s: Surface) -> ([f32; 3], f32) {
    match s {
        Surface::Asphalt => ([0.17, 0.17, 0.2], 0.55),
        Surface::Ice => ([0.72, 0.84, 0.95], 0.5),
        Surface::Snow => ([0.66, 0.73, 0.86], 0.6),
        Surface::Grass => ([0.30, 0.44, 0.20], 0.45),
        Surface::Sand => (shade(s.props().dust, 0.72), 0.5),
        _ => (shade(s.props().dust, 0.72), 0.45),
    }
}

/// Base dust emission per second at full intensity
fn dust_rate(s: Surface) -> f32 {
    match s {
        Surface::Gravel => 16.0,
        Surface::Dirt => 18.0,
        Surface::Sand => 22.0,
        Surface::Snow => 11.0,
        Surface::Grass => 5.0,
        Surface::Mud => 8.0,
        _ => 0.0,
    }
}

impl Fx {
    pub(super) fn emit_cars(&mut self, world: &World, dt: f32) {
        let track = world.track.clone();
        let water = track.water_level;
        for ci in 0..world.cars.len() {
            let car = &world.cars[ci];
            let (ipos, irot) = self.poses[ci];
            let near = self.nearest_local(ipos);
            if near > 130.0 {
                self.cars[ci].has_prev = false;
                for ch in self.cars[ci].chains.iter_mut() {
                    ch.active = false;
                }
                continue;
            }
            let lod = if near < 45.0 { 1.0 } else if near < 80.0 { 0.55 } else { 0.3 };
            let inv = car.rot.inverse();
            let to_render = |p: Vec3| ipos + irot * (inv * (p - car.pos));
            let speed = car.telemetry.speed;
            let up = irot * Vec3::Y;
            let fwd = irot * Vec3::NEG_Z;
            let right = irot * Vec3::X;
            let he = car.spec.half_extents;

            // ---------------- wheels
            for wi in 0..4 {
                let w = car.wheels[wi];
                let local_contact = w.local_mount - Vec3::Y * (w.susp_length + car.spec.wheel_radius);
                let cp = if w.contact { to_render(w.contact_point) } else { ipos + irot * local_contact };
                let prev = if self.cars[ci].has_prev && self.cars[ci].prev_wheel[wi].distance(cp) < 4.0 { self.cars[ci].prev_wheel[wi] } else { cp };
                self.cars[ci].prev_wheel[wi] = cp;
                let Some(surface) = w.surface.filter(|_| w.contact) else {
                    // bridge little bumps; end the ribbon on a real jump
                    self.cars[ci].air[wi] += dt;
                    if self.cars[ci].air[wi] > 0.15 {
                        self.feed_mark(ci, wi, &track, cp, right, car.spec.wheel_width, 0.0, Surface::Asphalt);
                    }
                    continue;
                };
                self.cars[ci].air[wi] = 0.0;
                let tr = wheel_traction(car, wi);
                if self.stats {
                    self.stat_trac[0] = self.stat_trac[0].max(tr.spin);
                    self.stat_trac[1] = self.stat_trac[1].max(tr.slide);
                    self.stat_trac[2] = self.stat_trac[2].max(tr.lock);
                }
                let rear = wi >= 2;
                let roll = (speed / 26.0).clamp(0.0, 1.0);
                let in_water = water.is_some_and(|wl| cp.y < wl + 0.12);
                let props = surface.props();

                // ---- tyre marks
                let (mcol, malpha) = mark_style(surface);
                let mut strength = ((tr.loss - 0.08) / 0.5).clamp(0.0, 1.0);
                if props.loose && speed > 2.0 && (rear || tr.loss > 0.1) {
                    // faint tracks always on loose ground
                    strength = strength.max(0.16 * (speed / 8.0).min(1.0));
                }
                if in_water {
                    strength = 0.0;
                }
                self.feed_mark(ci, wi, &track, cp, right, car.spec.wheel_width * if tr.loss > 0.3 { 1.25 } else { 1.0 }, strength * malpha, surface);
                let _ = mcol;

                if speed < 0.6 && tr.spin < 0.05 {
                    continue;
                }
                let sgn = tr.surf_sign;
                // where debris goes: backwards off spinning tyres, sideways off sliding ones
                let roost_dir = (-tr.fwd * sgn * tr.spin * 1.4 + tr.lat_dir * tr.slide + tr.fwd * tr.lock * 0.4).normalize_or_zero();
                let lose = tr.spin.max(tr.slide);

                if in_water {
                    let acc = &mut self.cars[ci].acc[wi][3];
                    *acc += (speed * 2.2 + lose * 30.0) * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    for _ in 0..n.min(8) {
                        let pos = prev.lerp(cp, self.rng.f());
                        let v = car.vel * 0.3 + roost_dir * self.rng.r(1.0, 3.0) + Vec3::Y * self.rng.r(1.5, 3.5 + speed * 0.1) + self.rng.ball() * 1.0;
                        self.spawn_drop(pos, v);
                    }
                    continue;
                }

                if props.loose {
                    // ---- dust clouds
                    let wheel_k = if rear { 1.0 } else { 0.55 };
                    let intensity = (roll * 0.4 * wheel_k + tr.spin * 2.2 + tr.slide * 1.8 + tr.lock * 0.9).min(1.6);
                    let acc = &mut self.cars[ci].acc[wi][0];
                    *acc += dust_rate(surface) * intensity * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    let dcol = dust_color(surface);
                    let big = if surface == Surface::Sand { 1.15 } else { 1.0 };
                    // powder is bright: keep it airy so cars stay readable
                    let airy = if surface == Surface::Snow || surface == Surface::Sand { 0.6 } else { 1.0 };
                    for _ in 0..n.min(10) {
                        let u = self.rng.f();
                        let jitter = Vec3::new(self.rng.r(-0.2, 0.2), self.rng.r(0.1, 0.35), self.rng.r(-0.2, 0.2));
                        let kick = roost_dir * (lose * self.rng.r(2.0, 6.5)) + Vec3::Y * (self.rng.r(0.5, 1.6) + lose * self.rng.r(0.5, 2.5));
                        let v = car.vel * 0.28 + kick + self.rng.hdir() * self.rng.r(0.2, 1.0);
                        let shade_k = self.rng.r(0.92, 1.06);
                        let alpha = (0.5 + lose * 0.2 + roll * 0.1) * airy * if surface == Surface::Grass { 0.45 } else { 1.0 };
                        let s1 = self.rng.r(1.0, 1.7) * big * (1.0 + lose * 0.7);
                        let p = Particle {
                            pos: prev.lerp(cp, 1.0 - u) + jitter + v * (u * dt),
                            vel: v,
                            age: u * dt,
                            life: self.rng.r(1.1, 1.9) * (1.0 + lose * 0.5),
                            size0: self.rng.r(0.25, 0.4),
                            size1: s1,
                            color: [dcol[0] * shade_k, dcol[1] * shade_k, dcol[2] * shade_k],
                            alpha,
                            shape: Shape::Puff,
                            ground: super::particle::Ground::Hover,
                            rot: self.rng.r(0.0, 6.28),
                            spin: self.rng.r(-0.6, 0.6),
                            drag: 2.4,
                            grav: -0.04,
                            wind_k: 1.0,
                            seed: self.rng.f(),
                            ..Default::default()
                        };
                        self.spawn(p, Prio::Spray);
                    }
                    // ---- debris: pebbles / snow clumps / grass bits / sand grains (the roost)
                    let acc = &mut self.cars[ci].acc[wi][1];
                    let base = match surface {
                        Surface::Gravel => 6.0 * roll,
                        Surface::Grass => 5.0 * roll,
                        Surface::Snow => 2.0 * roll,
                        _ => 1.0 * roll,
                    } * if rear { 1.0 } else { 0.5 };
                    *acc += (base + lose * 34.0 + tr.lock * 6.0) * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    for _ in 0..n.min(10) {
                        let u = self.rng.f();
                        let v = car.vel * 0.45 + roost_dir * self.rng.r(3.0, 8.0) * lose.max(0.25) + Vec3::Y * self.rng.r(1.8, 4.5) + self.rng.ball() * 1.2;
                        let pos = prev.lerp(cp, 1.0 - u) + Vec3::Y * 0.15 + v * (u * dt);
                        self.spawn_debris(surface, pos, v, Prio::Spray);
                    }
                } else if tr.loss > 0.12 {
                    // ---- tyre smoke on hard ground (asphalt / ice): spin, slide and lock-ups
                    let k = ((tr.loss - 0.12) / 0.6).clamp(0.0, 1.0);
                    let acc = &mut self.cars[ci].acc[wi][2];
                    *acc += 30.0 * k * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    let ice = surface == Surface::Ice;
                    for _ in 0..n.min(8) {
                        let u = self.rng.f();
                        let v = car.vel * 0.2 + roost_dir * self.rng.r(0.5, 2.5) + Vec3::Y * self.rng.r(0.4, 1.2) + self.rng.hdir() * 0.5;
                        let g = self.rng.r(0.9, 0.97);
                        let p = Particle {
                            pos: prev.lerp(cp, 1.0 - u) + Vec3::Y * 0.25 + v * (u * dt),
                            vel: v,
                            age: u * dt,
                            life: self.rng.r(2.0, 3.2),
                            size0: self.rng.r(0.35, 0.55),
                            size1: self.rng.r(2.4, 3.6),
                            color: if ice { [0.92, 0.96, 1.0] } else { [g, g, g + 0.03] },
                            alpha: 0.22 + 0.3 * k,
                            shape: Shape::Puff,
                            ground: Ground::Hover,
                            rot: self.rng.r(0.0, 6.28),
                            spin: self.rng.r(-0.5, 0.5),
                            drag: 1.6,
                            grav: -0.06,
                            wind_k: 1.0,
                            seed: self.rng.f(),
                            ..Default::default()
                        };
                        self.spawn(p, Prio::Spray);
                        if ice && self.rng.f() < 0.5 {
                            let tp = cp + self.rng.ball() * 0.5 + Vec3::Y * 0.2;
                            self.spawn_twinkle(tp, [0.9, 0.97, 1.0], v * 0.5, Prio::Spray);
                        }
                    }
                }
            }
            self.cars[ci].has_prev = true;

            // ---------------- water: entry splash + wake
            let in_water = car.telemetry.in_water;
            if in_water && !self.cars[ci].prev_in_water && speed > 2.0 {
                let wl = water.unwrap_or(ipos.y);
                self.splash(Vec3::new(ipos.x, wl, ipos.z), speed, car.vel);
            }
            if in_water && speed > 1.2 {
                let wl = water.unwrap_or(ipos.y);
                let acc = &mut self.cars[ci].wake_acc;
                *acc += (4.0 + speed * 0.7) * dt * lod;
                let n = acc.floor() as usize;
                *acc -= n as f32;
                for _ in 0..n.min(6) {
                    let side = if self.rng.f() < 0.5 { -1.0 } else { 1.0 };
                    let pos = ipos + right * (side * he.x * 1.1) + fwd * self.rng.r(-he.z, he.z * 0.5);
                    let p = Particle {
                        pos: Vec3::new(pos.x, wl + 0.1, pos.z),
                        vel: right * side * self.rng.r(0.8, 2.0) + Vec3::Y * 0.3,
                        life: self.rng.r(1.2, 2.0),
                        size0: 0.4,
                        size1: self.rng.r(1.4, 2.2),
                        color: [0.96, 0.99, 1.0],
                        alpha: 0.55,
                        shape: Shape::Puff,
                        rot: self.rng.r(0.0, 6.28),
                        drag: 2.5,
                        seed: self.rng.f(),
                        ..Default::default()
                    };
                    self.spawn(p, Prio::Spray);
                }
            }
            self.cars[ci].prev_in_water = in_water;

            // ---------------- exhaust
            let shifting = car.telemetry.shifting;
            let exhaust = ipos + irot * Vec3::new(he.x * 0.45, -0.27, he.z + 0.12);
            let back = -fwd;
            if !in_water {
                let throttle = car.telemetry.throttle;
                let rate = if speed < 3.0 { 5.0 + throttle * 6.0 } else if speed < 12.0 { 2.5 + throttle * 3.0 } else { 0.8 };
                let acc = &mut self.cars[ci].exhaust_acc;
                *acc += rate * dt * lod;
                let n = acc.floor() as usize;
                *acc -= n as f32;
                for _ in 0..n.min(4) {
                    let v = car.vel * 0.85 + back * self.rng.r(0.8, 1.6) + up * 0.35 + self.rng.ball() * 0.25;
                    self.spawn_exhaust(exhaust, v, 1.0);
                }
                if shifting && !self.cars[ci].prev_shifting && car.gear > 1 && speed > 5.0 {
                    // cute little "pop": a warm flash and a ring of puffs
                    for _ in 0..4 {
                        let v = car.vel * 0.9 + back * self.rng.r(1.5, 3.0) + self.rng.ball() * 0.8;
                        self.spawn_exhaust(exhaust, v, 1.5);
                    }
                    let p = Particle {
                        pos: exhaust + back * 0.15,
                        vel: car.vel,
                        life: 0.14,
                        size0: 0.32,
                        size1: 0.18,
                        color: [1.0, 0.72, 0.45],
                        alpha: 0.9,
                        shape: Shape::Glow,
                        drag: 0.0,
                        ..Default::default()
                    };
                    self.spawn(p, Prio::Burst);
                }
            }
            self.cars[ci].prev_shifting = shifting;
        }
    }

    fn spawn_exhaust(&mut self, pos: Vec3, vel: Vec3, scale: f32) {
        let g = self.rng.r(0.88, 0.96);
        let p = Particle {
            pos,
            vel,
            life: self.rng.r(0.9, 1.5),
            size0: 0.1 * scale,
            size1: self.rng.r(0.45, 0.7) * scale,
            color: [g, g, g + 0.02],
            alpha: 0.42,
            shape: Shape::Puff,
            rot: self.rng.r(0.0, 6.28),
            spin: self.rng.r(-1.0, 1.0),
            drag: 2.2,
            grav: -0.12,
            wind_k: 1.0,
            seed: self.rng.f(),
            ..Default::default()
        };
        self.spawn(p, Prio::Spray);
    }

    fn feed_mark(&mut self, ci: usize, wi: usize, track: &rally_core::track::Track, cp: Vec3, side: Vec3, width: f32, strength: f32, surface: Surface) {
        let (color, _) = mark_style(surface);
        let flat_side = Vec3::new(side.x, 0.0, side.z).normalize_or(Vec3::X);
        let seg_len = if strength > 0.2 { 0.55 } else { 1.1 };
        let ch = &mut self.cars[ci].chains[wi];
        if let Some(mut seg) = ch.feed(track, cp, flat_side, width, strength, color, false, seg_len) {
            seg.born = self.time;
            self.push_mark(seg);
        }
    }

    fn push_mark(&mut self, seg: MarkSeg) {
        if seg.alpha[0].max(seg.alpha[1]) > 0.2 {
            self.skids.push(seg);
        } else {
            self.tracks.push(seg);
        }
    }

    fn spawn_drop(&mut self, pos: Vec3, vel: Vec3) {
        let p = Particle {
            pos,
            vel,
            life: self.rng.r(0.7, 1.3),
            size0: self.rng.r(0.05, 0.09),
            size1: 0.05,
            color: [0.82, 0.93, 1.0],
            alpha: 0.9,
            shape: Shape::Drop,
            ground: Ground::Die,
            drag: 0.4,
            grav: 1.0,
            ..Default::default()
        };
        self.spawn(p, Prio::Spray);
    }

    fn spawn_twinkle(&mut self, pos: Vec3, color: [f32; 3], vel: Vec3, prio: super::Prio) {
        let p = Particle {
            pos,
            vel,
            life: self.rng.r(0.4, 0.9),
            size0: self.rng.r(0.08, 0.14),
            size1: 0.05,
            color,
            alpha: 0.9,
            shape: Shape::Twinkle,
            rot: self.rng.r(0.0, 1.0),
            drag: 2.0,
            seed: self.rng.f(),
            ..Default::default()
        };
        self.spawn(p, prio);
    }

    /// One piece of surface debris
    fn spawn_debris(&mut self, surface: Surface, pos: Vec3, vel: Vec3, prio: Prio) {
        let dust = surface.props().dust;
        let (shape, color, size, grav, drag, ground) = match surface {
            Surface::Grass => {
                let g = self.rng.pick(&[[0.42, 0.66, 0.28], [0.52, 0.74, 0.32], [0.36, 0.58, 0.26], [0.62, 0.78, 0.38]]);
                (Shape::Flake, g, self.rng.r(0.05, 0.09), 0.55, 1.4, Ground::Settle)
            }
            Surface::Snow => (Shape::Chunk, [0.97, 0.98, 1.0], self.rng.r(0.06, 0.12), 0.9, 0.6, Ground::Bounce),
            Surface::Sand => (Shape::Chunk, shade(dust, self.rng.r(0.85, 1.0)), self.rng.r(0.03, 0.05), 1.0, 0.5, Ground::Die),
            Surface::Mud => (Shape::Chunk, shade(dust, self.rng.r(0.8, 1.1)), self.rng.r(0.05, 0.09), 1.0, 0.3, Ground::Bounce),
            _ => {
                // pebbles: a few pastel greys/browns
                let base = mix(dust, [0.6, 0.58, 0.56], self.rng.r(0.0, 0.6));
                (Shape::Chunk, shade(base, self.rng.r(0.7, 0.95)), self.rng.r(0.04, 0.08), 1.0, 0.25, Ground::Bounce)
            }
        };
        let p = Particle {
            pos,
            vel,
            life: self.rng.r(1.4, 2.4),
            size0: size,
            size1: size,
            color,
            alpha: 1.0,
            shape,
            ground,
            rot: self.rng.r(0.0, 6.28),
            spin: self.rng.r(-14.0, 14.0),
            yaw: self.rng.r(0.0, 6.28),
            drag,
            grav,
            aspect: 0.45,
            seed: self.rng.f(),
            ..Default::default()
        };
        self.spawn(p, prio);
    }

    fn cloud(&mut self, pos: Vec3, vel: Vec3, color: [f32; 3], size: f32, alpha: f32, life: f32) {
        let p = Particle {
            pos,
            vel,
            life: life * self.rng.r(0.8, 1.2),
            size0: size * 0.3,
            size1: size * self.rng.r(0.8, 1.2),
            color,
            alpha,
            shape: Shape::Puff,
            ground: Ground::Hover,
            rot: self.rng.r(0.0, 6.28),
            spin: self.rng.r(-0.6, 0.6),
            drag: 2.2,
            grav: -0.03,
            wind_k: 1.0,
            seed: self.rng.f(),
            ..Default::default()
        };
        self.spawn(p, Prio::Burst);
    }

    pub(super) fn handle_events(&mut self, world: &World, events: &[WorldEvent]) {
        for e in events {
            match *e {
                WorldEvent::Landing { car, strength } => self.landing(world, car, strength),
                WorldEvent::Collision { car, other, pos, strength } => self.collision(world, car, other, pos, strength),
                WorldEvent::Race(RaceEvent::Go) => self.go_confetti(),
                WorldEvent::Race(RaceEvent::Finish { car, .. }) => {
                    if matches!(world.controllers.get(car), Some(Controller::Local(_))) {
                        self.finish_confetti();
                    }
                }
                _ => {}
            }
        }
    }

    fn landing(&mut self, world: &World, ci: usize, strength: f32) {
        let Some(car) = world.cars.get(ci) else { return };
        let (ipos, _) = self.poses[ci];
        if self.nearest_local(ipos) > 120.0 {
            return;
        }
        let track = &world.track;
        let g = track.visual_height(ipos.x, ipos.z);
        if track.water_level.is_some_and(|wl| g <= wl + 0.05) {
            self.splash(Vec3::new(ipos.x, g, ipos.z), strength * 1.5, car.vel);
            return;
        }
        let surface = car.wheels.iter().find_map(|w| w.surface).unwrap_or(track.style.offroad_surface);
        let k = ((strength - 3.0) / 6.0).clamp(0.15, 1.0);
        let n = (8.0 + 22.0 * k) as usize;
        let color = if surface.props().loose { dust_color(surface) } else { [0.9, 0.9, 0.9] };
        let alpha = if surface.props().loose { 0.5 } else { 0.25 };
        for j in 0..n {
            let a = j as f32 / n as f32 * std::f32::consts::TAU + self.rng.r(-0.2, 0.2);
            let d = Vec3::new(a.cos(), 0.0, a.sin());
            let pos = Vec3::new(ipos.x, g + 0.3, ipos.z) + d * self.rng.r(1.0, 2.0);
            let v = car.vel * 0.3 + d * self.rng.r(2.5, 5.0) * (0.6 + k) + Vec3::Y * self.rng.r(0.3, 1.2);
            let sz = self.rng.r(1.6, 2.6) * (0.8 + k * 0.5);
            self.cloud(pos, v, color, sz, alpha, 1.8);
        }
        if surface.props().loose {
            for _ in 0..(6.0 + 14.0 * k) as usize {
                let d = self.rng.hdir();
                let v = car.vel * 0.4 + d * self.rng.r(2.0, 4.5) + Vec3::Y * self.rng.r(2.5, 5.0) * (0.5 + k);
                self.spawn_debris(surface, Vec3::new(ipos.x, g + 0.2, ipos.z) + d * 1.2, v, Prio::Burst);
            }
        }
    }

    fn splash(&mut self, pos: Vec3, strength: f32, vel: Vec3) {
        let k = (strength / 12.0).clamp(0.3, 1.0);
        for _ in 0..(20.0 + 40.0 * k) as usize {
            let d = self.rng.hdir();
            let v = vel * 0.25 + d * self.rng.r(1.0, 4.0) * k + Vec3::Y * self.rng.r(2.5, 7.0) * k;
            let p = Particle {
                pos: pos + d * self.rng.r(0.5, 1.6) + Vec3::Y * 0.2,
                vel: v,
                life: self.rng.r(1.0, 1.6),
                size0: self.rng.r(0.07, 0.13),
                size1: 0.06,
                color: [0.82, 0.93, 1.0],
                alpha: 0.95,
                shape: Shape::Drop,
                ground: Ground::Die,
                drag: 0.3,
                grav: 1.0,
                ..Default::default()
            };
            self.spawn(p, Prio::Burst);
        }
        for j in 0..12 {
            let a = j as f32 / 12.0 * std::f32::consts::TAU;
            let d = Vec3::new(a.cos(), 0.0, a.sin());
            self.cloud(pos + d * 1.4 + Vec3::Y * 0.3, d * 2.0 * k + Vec3::Y * 0.6, [0.96, 0.99, 1.0], 1.8 + k, 0.6, 1.4);
        }
        for _ in 0..8 {
            let tp = pos + self.rng.ball() * 1.5 + Vec3::Y * 0.8;
            self.spawn_twinkle(tp, [1.0, 1.0, 1.0], Vec3::Y * 1.0, Prio::Burst);
        }
    }

    fn collision(&mut self, world: &World, ci: usize, other: Option<usize>, pos: Vec3, strength: f32) {
        if self.nearest_local(pos) > 120.0 {
            return;
        }
        let k = (strength / 10.0).clamp(0.2, 1.0);
        let base_vel = world.cars.get(ci).map_or(Vec3::ZERO, |c| c.vel * 0.3);
        if other.is_some() {
            self.bonk(pos, base_vel, k);
            self.cloud(pos, base_vel + Vec3::Y * 0.5, [0.95, 0.94, 0.92], 1.4, 0.35, 1.2);
            return;
        }
        // scenery: what did we hit?
        let mut best: Option<(f32, PropKind, Vec3, f32)> = None;
        world.track.for_each_prop_near(pos, 6.0, |p| {
            let d = Vec3::new(p.pos.x - pos.x, 0.0, p.pos.z - pos.z).length() - p.radius;
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, p.kind, p.pos, p.scale));
            }
        });
        let style = &world.track.style;
        match best.map(|b| (b.1, b.2, b.3)) {
            Some((PropKind::Tree | PropKind::Pine | PropKind::Palm | PropKind::Bush, ppos, scale)) => {
                let bush = matches!(best.unwrap().1, PropKind::Bush);
                let n = (14.0 + 26.0 * k) as usize;
                for _ in 0..n {
                    let c = self.rng.pick(&style.leaves);
                    let h = if bush { self.rng.r(0.4, 1.4) } else { self.rng.r(2.0, 4.5) * scale.max(0.6) };
                    let start = Vec3::new(ppos.x, ppos.y + h, ppos.z) + self.rng.ball() * if bush { 0.8 } else { 1.6 };
                    let v = (start - pos).normalize_or_zero() * self.rng.r(0.5, 2.5) + Vec3::Y * self.rng.r(0.5, 2.5) + base_vel * 0.3;
                    let lc = shade(c, self.rng.r(0.9, 1.1));
                    let sz = self.rng.r(0.12, 0.2);
                    self.leaf(start, v, lc, sz, Prio::Burst);
                }
                self.cloud(pos + Vec3::Y * 0.4, base_vel, dust_color(style.offroad_surface), 1.6, 0.3, 1.2);
            }
            Some((PropKind::Flowers, ppos, _)) => {
                for _ in 0..(16.0 + 20.0 * k) as usize {
                    let c = self.rng.pick(&PASTEL);
                    let start = ppos + Vec3::Y * 0.4 + self.rng.ball() * 0.6;
                    let v = Vec3::Y * self.rng.r(1.5, 3.5) + self.rng.hdir() * self.rng.r(0.5, 2.0) + base_vel * 0.4;
                    let sz = self.rng.r(0.06, 0.1);
                    self.leaf(start, v, c, sz, Prio::Burst);
                }
            }
            Some((PropKind::Rock, _, _)) => {
                let rock = mix(style.cliff, [1.0, 0.98, 0.95], 0.3);
                for _ in 0..(5.0 + 6.0 * k) as usize {
                    let v = base_vel + self.rng.hdir() * self.rng.r(1.0, 3.0) + Vec3::Y * self.rng.r(0.4, 1.4);
                    let cp = pos + self.rng.ball() * 0.5;
                    let sz = self.rng.r(1.2, 2.0);
                    self.cloud(cp, v, rock, sz, 0.45, 1.6);
                }
                for _ in 0..(8.0 + 10.0 * k) as usize {
                    let v = base_vel + self.rng.hdir() * self.rng.r(1.5, 4.0) + Vec3::Y * self.rng.r(2.0, 4.5);
                    self.spawn_debris(Surface::Gravel, pos + Vec3::Y * 0.3, v, Prio::Burst);
                }
            }
            Some((PropKind::HayBale, ppos, _)) => {
                for _ in 0..(20.0 + 20.0 * k) as usize {
                    let c = self.rng.pick(&[[0.98, 0.88, 0.5], [0.92, 0.78, 0.4], [1.0, 0.94, 0.65]]);
                    let v = Vec3::Y * self.rng.r(1.5, 4.0) + self.rng.hdir() * self.rng.r(0.5, 2.5) + base_vel * 0.3;
                    let p = Particle {
                        pos: ppos + Vec3::Y * 0.8 + self.rng.ball() * 0.6,
                        vel: v,
                        life: self.rng.r(2.5, 4.0),
                        size0: self.rng.r(0.1, 0.16),
                        size1: 0.1,
                        color: c,
                        shape: Shape::Flake,
                        ground: Ground::Settle,
                        spin: self.rng.r(-8.0, 8.0),
                        yaw: self.rng.r(0.0, 6.28),
                        drag: 1.6,
                        grav: 0.25,
                        wind_k: 1.0,
                        flutter: 0.6,
                        aspect: 0.15,
                        seed: self.rng.f(),
                        ..Default::default()
                    };
                    self.spawn(p, Prio::Burst);
                }
                self.cloud(pos + Vec3::Y * 0.5, base_vel, [0.98, 0.92, 0.72], 1.6, 0.4, 1.5);
            }
            _ => {
                self.bonk(pos, base_vel, k);
            }
        }
    }

    /// Cartoon bonk: stars + a few sparks
    fn bonk(&mut self, pos: Vec3, base_vel: Vec3, k: f32) {
        for _ in 0..(4.0 + 6.0 * k) as usize {
            let v = base_vel + self.rng.ball() * 3.0 + Vec3::Y * self.rng.r(1.5, 3.5);
            let p = Particle {
                pos: pos + Vec3::Y * 0.4,
                vel: v,
                life: self.rng.r(0.6, 1.0),
                size0: self.rng.r(0.16, 0.26),
                size1: 0.08,
                color: self.rng.pick(&[[1.0, 0.86, 0.35], [1.0, 0.95, 0.6], [1.0, 0.65, 0.7], [0.7, 0.9, 1.0]]),
                alpha: 1.0,
                shape: Shape::Star,
                rot: self.rng.r(0.0, 6.28),
                spin: self.rng.r(-6.0, 6.0),
                drag: 2.0,
                grav: 0.35,
                ..Default::default()
            };
            self.spawn(p, Prio::Burst);
        }
        for _ in 0..(6.0 + 10.0 * k) as usize {
            let v = base_vel + self.rng.ball() * 7.0 + Vec3::Y * 2.0;
            let p = Particle {
                pos,
                vel: v,
                life: self.rng.r(0.25, 0.5),
                size0: 0.035,
                size1: 0.02,
                color: [1.0, 0.85, 0.5],
                alpha: 1.0,
                shape: Shape::Spark,
                drag: 1.0,
                grav: 0.6,
                ..Default::default()
            };
            self.spawn(p, Prio::Burst);
        }
    }

    pub(super) fn leaf(&mut self, pos: Vec3, vel: Vec3, color: [f32; 3], size: f32, prio: Prio) -> bool {
        let p = Particle {
            pos,
            vel,
            life: self.rng.r(4.0, 6.5),
            size0: size,
            size1: size,
            color,
            alpha: 1.0,
            shape: Shape::Leaf,
            ground: Ground::Settle,
            rot: self.rng.r(0.0, 6.28),
            spin: self.rng.r(-5.0, 5.0),
            yaw: self.rng.r(0.0, 6.28),
            drag: 1.8,
            grav: 0.12,
            wind_k: 1.0,
            flutter: 1.0,
            aspect: 0.6,
            seed: self.rng.f(),
            ..Default::default()
        };
        self.spawn(p, prio)
    }

    fn confetti(&mut self, pos: Vec3, vel: Vec3) {
        let c = self.rng.pick(&PASTEL);
        let p = Particle {
            pos,
            vel,
            life: self.rng.r(5.5, 8.5),
            size0: self.rng.r(0.08, 0.12),
            size1: 0.1,
            color: c,
            alpha: 1.0,
            shape: Shape::Flake,
            ground: Ground::Settle,
            rot: self.rng.r(0.0, 6.28),
            spin: self.rng.r(-12.0, 12.0),
            yaw: self.rng.r(0.0, 6.28),
            drag: 1.7,
            grav: 0.22,
            wind_k: 0.8,
            flutter: 0.9,
            aspect: 0.6,
            seed: self.rng.f(),
            ..Default::default()
        };
        self.spawn(p, Prio::Burst);
    }

    fn go_confetti(&mut self) {
        let a = self.arch;
        for _ in 0..140 {
            let along = self.rng.r(-a.half, a.half);
            let pos = a.centre + a.right * along + Vec3::Y * 0.6;
            let v = Vec3::Y * self.rng.r(2.0, 6.0) + a.fwd * self.rng.r(-3.0, 3.0) + a.right * self.rng.r(-1.5, 1.5);
            self.confetti(pos, v);
        }
        for _ in 0..6 {
            let pos = a.centre + a.right * self.rng.r(-a.half, a.half);
            self.spawn_twinkle(pos, [1.0, 1.0, 0.9], Vec3::Y * 1.5, Prio::Burst);
        }
    }

    pub(super) fn finish_confetti(&mut self) {
        let a = self.arch;
        // big shower from the arch beam
        for _ in 0..300 {
            let along = self.rng.r(-a.half, a.half);
            let pos = a.centre + a.right * along + Vec3::Y * self.rng.r(0.0, 0.8);
            let v = Vec3::Y * self.rng.r(3.0, 9.0) + a.fwd * self.rng.r(-5.0, 5.0) + a.right * self.rng.r(-2.0, 2.0);
            self.confetti(pos, v);
        }
        // two cannons from the posts, angled up and inwards
        for (post, inward) in [(a.post_l, a.right), (a.post_r, -a.right)] {
            for _ in 0..130 {
                let pos = post + Vec3::Y * self.rng.r(0.5, 2.0);
                let dir = (inward * 0.55 + Vec3::Y + a.fwd * self.rng.r(-0.4, 0.4) + self.rng.ball() * 0.25).normalize();
                let sp = self.rng.r(8.0, 15.0);
                self.confetti(pos, dir * sp);
            }
        }
        for _ in 0..24 {
            let pos = a.centre + a.right * self.rng.r(-a.half, a.half) + self.rng.ball() * 2.0;
            let c = self.rng.pick(&PASTEL);
            self.spawn_twinkle(pos, c, Vec3::Y * 1.0, Prio::Burst);
        }
    }
}
