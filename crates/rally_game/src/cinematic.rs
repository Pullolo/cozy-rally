//! Slow, cosy "TV director" camera for the title attract mode and the results screen.
//! Produces (eye, target, fov) for `RaceSession::cam_override`, cutting between shots
//! (orbit, trackside, drone flyover, high chase) with a gentle dip-to-cream.
use crate::game::{interp, RaceSession};
use macroquad::math::{vec3, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shot {
    /// Slow orbit around a car
    Orbit,
    /// Fixed camera beside the road ahead of the car, panning as it passes
    Trackside { eye: Vec3 },
    /// High establishing shot gliding along the track
    Drone { s: f32 },
    /// Behind and above the car, smoothed
    HighChase,
}

pub struct Cinematic {
    shot: Shot,
    shot_time: f32,
    shot_len: f32,
    car: usize,
    angle: f32,
    eye: Vec3,
    target: Vec3,
    fov: f32,
    k: u32,
    initialized: bool,
    started: bool,
    /// Only orbit (results screen)
    pub orbit_only: bool,
    /// 0..1 strength of the dip-to-cream around cuts (draw as an overlay)
    pub dip: f32,
}

impl Cinematic {
    pub fn new(seed: u64) -> Self {
        Cinematic {
            shot: Shot::Drone { s: 0.0 },
            shot_time: 0.0,
            shot_len: 9.0,
            car: 0,
            angle: (seed % 628) as f32 * 0.01,
            eye: Vec3::ZERO,
            target: Vec3::ZERO,
            fov: 50f32.to_radians(),
            k: seed as u32,
            initialized: false,
            started: false,
            orbit_only: false,
            dip: 0.0,
        }
    }

    /// Orbit a specific car forever (results)
    pub fn orbit(car: usize) -> Self {
        let mut c = Cinematic::new(7);
        c.shot = Shot::Orbit;
        c.car = car;
        c.orbit_only = true;
        c.shot_len = f32::MAX;
        c
    }

    fn pick_next(&mut self, s: &RaceSession) {
        self.k = self.k.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = s.world.cars.len().max(1);
        // follow someone near the front most of the time (that's where the action is)
        let standings = s.world.race.standings();
        let pick = (self.k >> 8) as usize % n.min(4);
        self.car = standings.get(pick).copied().unwrap_or(0);
        let car = &s.world.cars[self.car];
        let r = &s.world.race.racers[self.car];
        let track = s.track();
        let choice = (self.k >> 16) % 4;
        self.shot = match (s.world.race.is_racing(), choice) {
            (false, _) | (_, 0) => Shot::Orbit,
            (_, 1) => {
                let ahead = r.s + 45.0 + car.vel.length() * 1.5;
                let (p, _, right, _) = track.frame_at(ahead);
                let side = if (self.k >> 4) & 1 == 0 { 1.0 } else { -1.0 };
                let hw = track.samples[r.track_index].half_width;
                let mut eye = p + right * side * (hw + 7.0);
                eye.y = track.visual_height(eye.x, eye.z) + 2.2;
                Shot::Trackside { eye }
            }
            (_, 2) => Shot::Drone { s: r.s - 20.0 },
            _ => Shot::HighChase,
        };
        self.shot_len = match self.shot {
            Shot::Orbit => 10.0,
            Shot::Trackside { .. } => 7.0,
            Shot::Drone { .. } => 11.0,
            Shot::HighChase => 8.0,
        };
        self.shot_time = 0.0;
        self.initialized = false;
    }

    /// Advance and write the camera into `s.cam_override`
    pub fn update(&mut self, s: &mut RaceSession, dt: f32) {
        if s.world.cars.is_empty() {
            return;
        }
        self.shot_time += dt;
        if !self.orbit_only && (self.shot_time > self.shot_len || !self.started) {
            self.started = true;
            self.pick_next(s);
        }
        self.car = self.car.min(s.world.cars.len() - 1);
        let track = s.world.track.clone();
        let car = &s.world.cars[self.car];
        let (pos, _) = interp(car, s.alpha);
        let fwd = car.forward();
        let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
        let (eye, target, fov, smooth) = match self.shot {
            Shot::Orbit => {
                self.angle += dt * if self.orbit_only { 0.22 } else { 0.16 };
                let d = if self.orbit_only { 7.5 } else { 9.0 };
                let eye = pos + vec3(self.angle.cos() * d, 2.6, self.angle.sin() * d);
                (eye, pos + Vec3::Y * 0.6, 46f32.to_radians(), 6.0)
            }
            Shot::Trackside { eye } => {
                let dist = eye.distance(pos);
                // zoom gently with distance like a TV camera
                let fov = (28.0 + 900.0 / dist.max(8.0)).clamp(24.0, 60.0);
                if dist > 70.0 && self.shot_time > 2.0 {
                    self.shot_time = self.shot_len + 1.0;
                }
                (eye, pos + Vec3::Y * 0.5, fov.to_radians(), 40.0)
            }
            Shot::Drone { s: ref mut ds } => {
                *ds += dt * 14.0;
                let (p, t, right, _) = track.frame_at(*ds);
                let eye = p - t * 30.0 + right * 22.0 + Vec3::Y * 26.0;
                let (look, _, _, _) = track.frame_at(*ds + 45.0);
                (eye, look, 55f32.to_radians(), 3.0)
            }
            Shot::HighChase => {
                let eye = pos - flat * 13.0 + Vec3::Y * 6.5;
                (eye, pos + flat * 6.0 + Vec3::Y * 0.5, 52f32.to_radians(), 3.0)
            }
        };
        let mut eye = eye;
        let ground = track.visual_height(eye.x, eye.z);
        eye.y = eye.y.max(ground + 1.4);
        if !self.initialized {
            self.eye = eye;
            self.target = target;
            self.fov = fov;
            self.initialized = true;
        }
        let k = 1.0 - (-dt * smooth).exp();
        self.eye = self.eye.lerp(eye, k);
        self.target = self.target.lerp(target, (k * 2.0).min(1.0));
        self.fov += (fov - self.fov) * k;
        let g = track.visual_height(self.eye.x, self.eye.z);
        self.eye.y = self.eye.y.max(g + 1.0);
        // dip to cream around cuts
        let to_end = self.shot_len - self.shot_time;
        self.dip = if self.orbit_only { 0.0 } else { (1.0 - self.shot_time / 0.5).max(1.0 - to_end / 0.5).clamp(0.0, 1.0) };
        s.cam_override = Some((self.eye, self.target, self.fov));
    }
}
