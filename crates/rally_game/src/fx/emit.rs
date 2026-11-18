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

/// Traction loss for wheel `i`, from the sim's tyre model (`WheelState::traction_split`:
/// zero below the tyre's peak, so a well-driven corner throws no slide FX).
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
    let _ = v_long;
    let (slide, spin, lock) = w.traction_split();
    let loss = spin.max(lock).max(slide).max(w.traction_loss);
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
        _ => mix(d, [1.0, 0.96, 0.9], 0.55),
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

            let mut trail_surf: Option<Surface> = None;
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
                        self.feed_mark(ci, wi, &track, cp, right, car.spec.wheel_width, 0.0, Surface::Asphalt, 0.0);
                    }
                    continue;
                };
                self.cars[ci].air[wi] = 0.0;
                let mut tr = wheel_traction(car, wi);
                if let Some(t) = self.test_traction(ci, wi, car) {
                    tr = t;
                }
                if self.stats {
                    self.stat_trac[0] = self.stat_trac[0].max(tr.spin);
                    self.stat_trac[1] = self.stat_trac[1].max(tr.slide);
                    self.stat_trac[2] = self.stat_trac[2].max(tr.lock);
                }
                let rear = wi >= 2;
                let roll = (speed / 26.0).clamp(0.0, 1.0);
                let in_water = water.is_some_and(|wl| cp.y < wl + 0.12);
                let props = surface.props();
                // grass and mud don't raise dust clouds
                if props.loose && !in_water && !matches!(surface, Surface::Grass | Surface::Mud) && (rear || trail_surf.is_none()) {
                    trail_surf = Some(surface);
                }

                // ---- tyre marks: faint tracks on loose ground, darker when the tyre lets go,
                // darkest for lock-ups and launches
                let (_, malpha) = mark_style(surface);
                let mut strength = ((tr.loss - 0.08) / 0.5).clamp(0.0, 1.0);
                if props.loose && speed > 2.0 && (rear || tr.loss > 0.1) {
                    strength = strength.max(0.16 * (speed / 8.0).min(1.0));
                }
                let dark = ((tr.lock.max(tr.spin) - 0.15) / 0.6).clamp(0.0, 1.0);
                if in_water {
                    strength = 0.0;
                }
                let width = car.spec.wheel_width * if tr.loss > 0.3 { 1.3 } else { 1.0 };
                self.feed_mark(ci, wi, &track, cp, right, width, (strength * malpha * (1.0 + 0.5 * dark)).min(0.85), surface, dark);

                if speed < 0.6 && tr.spin < 0.05 {
                    continue;
                }
                let lose = tr.spin.max(tr.slide);
                if in_water {
                    let roost_dir = (-tr.fwd * tr.surf_sign * tr.spin + tr.lat_dir * tr.slide).normalize_or_zero();
                    let acc = &mut self.cars[ci].acc[wi][7];
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
                let hard = !props.loose;
                // grass throws clippings and a little soil, not big clouds
                let grassy = matches!(surface, Surface::Grass | Surface::Mud);
                let dcol = if grassy { [0.78, 0.72, 0.58] } else { dust_color(surface) };
                let puff_k = if grassy { 0.4 } else { 1.0 };
                let dirt = if grassy { [0.62, 0.55, 0.42] } else { mix(shade(props.dust, 0.8), dcol, 0.4) };
                let smoke_col = if surface == Surface::Ice { [0.92, 0.96, 1.0] } else { [0.93, 0.93, 0.96] };

                // ---- WHEELSPIN: a narrow jet of dark dirt + debris (or smoke) shooting backwards
                if tr.spin > 0.05 {
                    let back = -tr.fwd * tr.surf_sign;
                    let acc = &mut self.cars[ci].acc[wi][0];
                    *acc += if hard { 22.0 } else { 20.0 } * tr.spin * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    for _ in 0..n.min(8) {
                        let u = self.rng.f();
                        let v = car.vel * 0.5 + back * (self.rng.r(5.0, 10.0) * (0.5 + 0.5 * tr.spin)) + Vec3::Y * self.rng.r(0.8, 2.2) + self.rng.ball() * 0.6;
                        let pos = prev.lerp(cp, 1.0 - u) + Vec3::Y * 0.2 + v * (u * dt);
                        if hard {
                            self.puff(pos, v, smoke_col, 0.4, (2.2, 3.0), 0.3 + 0.35 * tr.spin, (1.8, 2.8), 2.2, -0.05, u * dt, Prio::Spray);
                        } else {
                            let v = v + Vec3::Y * 1.2;
                            self.puff(pos, v, dirt, 0.2, (0.6, 1.0), 0.75 * puff_k, (0.7, 1.1), 3.0, 0.15, u * dt, Prio::Spray);
                        }
                    }
                    if !hard {
                        let acc = &mut self.cars[ci].acc[wi][1];
                        *acc += 150.0 * tr.spin * dt * lod;
                        let n = acc.floor() as usize;
                        *acc -= n as f32;
                        for _ in 0..n.min(22) {
                            let u = self.rng.f();
                            // a rooster-tail fountain: backwards and up in a tight arc
                            let v = car.vel * 0.5 + back * self.rng.r(4.5, 9.0) + Vec3::Y * self.rng.r(3.0, 6.5) + self.rng.ball() * 0.7;
                            let sc = self.rng.r(0.9, 1.2);
                            self.spawn_debris_s(surface, prev.lerp(cp, 1.0 - u) + Vec3::Y * 0.15 + v * (u * dt), v, Prio::Spray, sc);
                        }
                    }
                }
                // ---- SLIDE: a sideways fan of spray (or smoke) from the sliding tyres
                if tr.slide > 0.05 {
                    let acc = &mut self.cars[ci].acc[wi][2];
                    *acc += if hard { 22.0 } else { 28.0 } * tr.slide * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    for _ in 0..n.min(8) {
                        let u = self.rng.f();
                        let v = car.vel * 0.45 + tr.lat_dir * (self.rng.r(3.0, 7.0) * (0.5 + 0.5 * tr.slide)) + tr.fwd * self.rng.r(-2.5, 2.5) + Vec3::Y * self.rng.r(0.8, 2.2);
                        let pos = prev.lerp(cp, 1.0 - u) + Vec3::Y * 0.25 + v * (u * dt);
                        if hard {
                            self.puff(pos, v, smoke_col, 0.4, (2.2, 3.0), 0.2 + 0.25 * tr.slide, (1.8, 2.6), 1.8, -0.05, u * dt, Prio::Spray);
                        } else {
                            let szr = (1.4, 2.2);
                            self.puff(pos, v, dcol, 0.35, szr, 0.62 * puff_k, (1.3, 2.0), 2.2, -0.02, u * dt, Prio::Spray);
                        }
                    }
                    if !hard {
                        let acc = &mut self.cars[ci].acc[wi][3];
                        *acc += 45.0 * tr.slide * dt * lod;
                        let n = acc.floor() as usize;
                        *acc -= n as f32;
                        for _ in 0..n.min(10) {
                            let u = self.rng.f();
                            let v = car.vel * 0.45 + tr.lat_dir * self.rng.r(4.0, 9.0) + tr.fwd * self.rng.r(-3.0, 3.0) + Vec3::Y * self.rng.r(1.5, 3.5) + self.rng.ball() * 0.6;
                            self.spawn_debris(surface, prev.lerp(cp, 1.0 - u) + Vec3::Y * 0.15 + v * (u * dt), v, Prio::Spray);
                        }
                    }
                }
                // ---- LOCK-UP: bursts of smoke/dust boiling up around the locked tyre
                if tr.lock > 0.05 {
                    let acc = &mut self.cars[ci].acc[wi][4];
                    *acc += 26.0 * tr.lock * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    for _ in 0..n.min(8) {
                        let u = self.rng.f();
                        let v = car.vel * 0.3 + self.rng.hdir() * self.rng.r(0.3, 1.4) + Vec3::Y * self.rng.r(0.4, 1.3);
                        let pos = prev.lerp(cp, 1.0 - u) + Vec3::Y * 0.25 + v * (u * dt);
                        let c = if hard { smoke_col } else { dcol };
                        self.puff(pos, v, c, 0.4, (1.3, 1.9), (0.4 + 0.15 * tr.lock) * if hard { 1.0 } else { puff_k }, (1.2, 1.9), 2.5, -0.04, u * dt, Prio::Spray);
                    }
                    if !hard {
                        let acc = &mut self.cars[ci].acc[wi][5];
                        *acc += 18.0 * tr.lock * dt * lod;
                        let n = acc.floor() as usize;
                        *acc -= n as f32;
                        for _ in 0..n.min(6) {
                            let v = car.vel * 0.85 + self.rng.hdir() * self.rng.r(0.5, 2.0) + Vec3::Y * self.rng.r(1.0, 2.5);
                            self.spawn_debris(surface, cp + Vec3::Y * 0.15, v, Prio::Spray);
                        }
                    }
                }
                // ---- normal rolling: just a few pebbles / grass bits (the big cloud comes
                // from the per-car trail emitter below)
                if props.loose && roll > 0.1 {
                    let base = match surface {
                        Surface::Gravel => 6.0,
                        Surface::Grass => 5.0,
                        Surface::Snow => 2.0,
                        _ => 1.5,
                    } * roll * if rear { 1.0 } else { 0.5 };
                    let acc = &mut self.cars[ci].acc[wi][6];
                    *acc += base * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    for _ in 0..n.min(4) {
                        let v = car.vel * 0.45 - fwd * self.rng.r(0.0, 2.0) + Vec3::Y * self.rng.r(1.5, 3.0) + self.rng.ball() * 1.0;
                        self.spawn_debris(surface, cp + Vec3::Y * 0.15, v, Prio::Spray);
                    }
                }
                if surface == Surface::Ice && tr.loss > 0.15 && self.rng.f() < tr.loss * 0.5 {
                    let tp = cp + self.rng.ball() * 0.5 + Vec3::Y * 0.2;
                    self.spawn_twinkle(tp, [0.9, 0.97, 1.0], car.vel * 0.5, Prio::Spray);
                }
            }
            self.cars[ci].has_prev = true;

            // ---------------- the rally dust trail: a big soft cloud that billows out behind
            // the car, rises a little, drifts with the wind and lingers for seconds
            if let Some(surf) = trail_surf {
                let k = ((speed - 3.0) / 22.0).clamp(0.0, 1.0);
                if k > 0.0 {
                    let acc = &mut self.cars[ci].trail_acc;
                    *acc += (7.0 + 20.0 * k) * dt * lod;
                    let n = acc.floor() as usize;
                    *acc -= n as f32;
                    let dcol = dust_color(surf);
                    let (alpha_k, size_k) = match surf {
                        Surface::Snow => (1.1, 0.95),
                        Surface::Sand => (1.0, 1.15),
                        Surface::Grass => (0.35, 0.8),
                        Surface::Mud => (0.55, 0.85),
                        _ => (1.0, 1.0),
                    };
                    for _ in 0..n.min(6) {
                        let u = self.rng.f();
                        let side = self.rng.r(-1.0, 1.0);
                        let base = ipos - fwd * (he.z + 0.6) + right * (side * he.x * 1.1) - car.vel * (u * dt);
                        let gh = track.visual_height(base.x, base.z);
                        // billow outwards and upwards so the cloud swells beyond the car's silhouette
                        let v = car.vel * 0.18 + right * (side.signum() * self.rng.r(0.8, 2.6)) + self.rng.hdir() * 0.4 + Vec3::Y * self.rng.r(0.4, 1.1);
                        let sk = self.rng.r(0.93, 1.05);
                        let p = Particle {
                            pos: Vec3::new(base.x, gh + 0.5, base.z) + v * (u * dt),
                            vel: v,
                            age: u * dt,
                            life: self.rng.r(3.5, 5.5),
                            size0: self.rng.r(0.7, 1.0),
                            size1: self.rng.r(4.0, 6.0) * (0.65 + 0.35 * k) * size_k,
                            color: [dcol[0] * sk, dcol[1] * sk, dcol[2] * sk],
                            alpha: (0.3 + 0.2 * k) * alpha_k,
                            shape: Shape::Puff,
                            ground: Ground::Hover,
                            rot: self.rng.r(0.0, 6.28),
                            spin: self.rng.r(-0.25, 0.25),
                            drag: 1.3,
                            grav: -0.02,
                            wind_k: 1.0,
                            // thin right behind the car, billowing up further back
                            fade_in: 0.45,
                            seed: self.rng.f(),
                            ..Default::default()
                        };
                        self.spawn(p, Prio::Spray);
                    }
                }
            }

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

    /// Generic soft cloud puff
    #[allow(clippy::too_many_arguments)]
    fn puff(&mut self, pos: Vec3, vel: Vec3, color: [f32; 3], size0: f32, size1: (f32, f32), alpha: f32, life: (f32, f32), drag: f32, grav: f32, age: f32, prio: Prio) {
        let sk = self.rng.r(0.94, 1.04);
        let size1 = self.rng.r(size1.0, size1.1);
        let life = self.rng.r(life.0, life.1);
        let p = Particle {
            pos,
            vel,
            age,
            life,
            size0,
            size1,
            color: [color[0] * sk, color[1] * sk, color[2] * sk],
            alpha,
            shape: Shape::Puff,
            ground: Ground::Hover,
            rot: self.rng.r(0.0, 6.28),
            spin: self.rng.r(-0.6, 0.6),
            drag,
            grav,
            wind_k: 1.0,
            seed: self.rng.f(),
            ..Default::default()
        };
        self.spawn(p, prio);
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

    fn feed_mark(&mut self, ci: usize, wi: usize, track: &rally_core::track::Track, cp: Vec3, side: Vec3, width: f32, strength: f32, surface: Surface, dark: f32) {
        let (color, _) = mark_style(surface);
        let color = shade(color, 1.0 - 0.4 * dark);
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
        self.spawn_debris_s(surface, pos, vel, prio, 1.0);
    }

    /// Debris with a size multiplier (roost chunks are chunkier so they read on screen)
    fn spawn_debris_s(&mut self, surface: Surface, pos: Vec3, vel: Vec3, prio: Prio, scale: f32) {
        let dust = surface.props().dust;
        let (shape, color, size, grav, drag, ground) = match surface {
            Surface::Grass => {
                let g = self.rng.pick(&[[0.42, 0.66, 0.28], [0.52, 0.74, 0.32], [0.36, 0.58, 0.26], [0.62, 0.78, 0.38]]);
                (Shape::Flake, g, self.rng.r(0.05, 0.09), 0.55, 1.4, Ground::Settle)
            }
            Surface::Snow => (Shape::Chunk, [0.97, 0.98, 1.0], self.rng.r(0.035, 0.06), 0.9, 0.6, Ground::Bounce),
            Surface::Sand => (Shape::Chunk, shade(dust, self.rng.r(0.88, 1.02)), self.rng.r(0.025, 0.035), 1.0, 0.5, Ground::Die),
            Surface::Mud => (Shape::Chunk, shade(dust, self.rng.r(0.85, 1.1)), self.rng.r(0.03, 0.05), 1.0, 0.3, Ground::Bounce),
            _ => {
                // pebbles 5-12 cm: gravel tan/warm grey, dirt brown (surface coloured)
                let base = if surface == Surface::Gravel { mix(dust, [0.72, 0.7, 0.66], self.rng.r(0.0, 0.45)) } else { dust };
                (Shape::Chunk, shade(base, self.rng.r(0.82, 1.02)), self.rng.r(0.028, 0.05), 1.0, 0.25, Ground::Bounce)
            }
        };
        let p = Particle {
            pos,
            vel,
            life: self.rng.r(1.4, 2.4),
            size0: size * scale,
            size1: size * scale,
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
                WorldEvent::Race(RaceEvent::Go) => {
                    self.go_time = Some(self.time);
                    self.go_confetti();
                }
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
