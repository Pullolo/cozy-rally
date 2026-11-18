//! Chase camera with spring lag, speed-based FOV, drift look-into and terrain avoidance.
use macroquad::math::{Mat4, Vec3};
use rally_core::track::Track;
use rally_core::vehicle::Car;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamMode {
    Chase,
    Far,
    Hood,
}

impl CamMode {
    pub fn next(self) -> CamMode {
        match self {
            CamMode::Chase => CamMode::Far,
            CamMode::Far => CamMode::Hood,
            CamMode::Hood => CamMode::Chase,
        }
    }
}

pub struct ChaseCam {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov: f32,
    pub mode: CamMode,
    look_dir: Vec3,
    /// smoothed sideways look-ahead into the next corner
    ahead: Vec3,
    shake: f32,
    time: f32,
    initialized: bool,
}

impl ChaseCam {
    pub fn new() -> Self {
        ChaseCam { eye: Vec3::ZERO, target: Vec3::ZERO, fov: 62f32.to_radians(), mode: CamMode::Chase, look_dir: Vec3::NEG_Z, ahead: Vec3::ZERO, shake: 0.0, time: 0.0, initialized: false }
    }

    pub fn add_shake(&mut self, amount: f32) {
        self.shake = (self.shake + amount).min(1.0);
    }

    pub fn snap(&mut self) {
        self.initialized = false;
    }

    pub fn update(&mut self, car_pos: Vec3, car: &Car, track: &Track, dt: f32) {
        self.time += dt;
        let fwd = car.forward();
        let flat_fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
        let vel = car.vel;
        let speed = vel.length();
        let flat_vel = Vec3::new(vel.x, 0.0, vel.z);
        // look between heading and travel direction (shows the drift)
        let travel = if speed > 3.0 && vel.dot(fwd) > 0.0 { flat_vel.normalize() } else { flat_fwd };
        let desired_dir = (flat_fwd * 0.65 + travel * 0.35).normalize_or_zero();
        let k_dir = 1.0 - (-dt * 5.0).exp();
        if !self.initialized {
            self.look_dir = flat_fwd;
        }
        self.look_dir = self.look_dir.lerp(desired_dir, k_dir).normalize_or_zero();

        let (dist, height, look_h) = match self.mode {
            CamMode::Chase => (6.4, 2.3, 1.1),
            CamMode::Far => (9.5, 3.6, 1.3),
            CamMode::Hood => (0.0, 0.0, 0.0),
        };
        if self.mode == CamMode::Hood {
            let up = car.up();
            self.eye = car_pos + up * (car.spec.half_extents.y + 0.35) + fwd * 0.2;
            self.target = self.eye + fwd * 10.0;
            self.fov = (70.0 + speed * 0.25).min(85.0).to_radians();
            self.initialized = true;
            return;
        }
        let dist = dist + speed * 0.035;
        let mut desired_eye = car_pos - self.look_dir * dist + Vec3::Y * height;
        let ground = track.visual_height(desired_eye.x, desired_eye.z);
        desired_eye.y = desired_eye.y.max(ground + 1.0);
        if !self.initialized {
            self.eye = desired_eye;
            self.initialized = true;
        }
        let k = 1.0 - (-dt * 9.0).exp();
        self.eye = self.eye.lerp(desired_eye, k);
        // keep a minimum height above whatever terrain we drifted over
        let g = track.visual_height(self.eye.x, self.eye.z);
        self.eye.y = self.eye.y.max(g + 0.8);
        // gently look into upcoming corners: aim a little towards the road ~16 m ahead
        let mut want_ahead = Vec3::ZERO;
        if let Some(h) = car.track_hint {
            let n = track.samples.len();
            if n > 0 && speed > 4.0 {
                let p = track.samples[(h + 8) % n].pos;
                let to = Vec3::new(p.x - car_pos.x, 0.0, p.z - car_pos.z);
                let lateral = to - self.look_dir * to.dot(self.look_dir);
                let w = ((speed - 4.0) / 16.0).clamp(0.0, 1.0);
                want_ahead = lateral.clamp_length_max(5.0) * 0.32 * w;
            }
        }
        let k_ahead = 1.0 - (-dt * 2.5).exp();
        if !self.initialized {
            self.ahead = want_ahead;
        }
        self.ahead = self.ahead.lerp(want_ahead, k_ahead);
        self.target = car_pos + Vec3::Y * look_h + self.look_dir * 2.0 + self.ahead;
        let target_fov = (62.0 + (speed * 3.6 - 40.0).max(0.0) * 0.11).min(80.0);
        self.fov += (target_fov.to_radians() - self.fov) * k;
        // shake
        self.shake = (self.shake - dt * 1.8).max(0.0);
        if self.shake > 0.0 {
            let s = self.shake * self.shake * 0.25;
            self.eye += Vec3::new((self.time * 53.0).sin(), (self.time * 41.0).cos(), (self.time * 37.0).sin()) * s;
        }
    }

    pub fn matrices(&self, aspect: f32) -> (Mat4, Mat4) {
        let view = Mat4::look_at_rh(self.eye, self.target, Vec3::Y);
        let proj = Mat4::perspective_rh_gl(self.fov, aspect, 0.25, 2200.0);
        (view, proj)
    }
}
