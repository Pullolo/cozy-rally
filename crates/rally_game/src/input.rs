//! Keyboard (and touch, later) to CarInput with analogue-style smoothing.
use macroquad::prelude::*;
use rally_core::input::CarInput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// WASD + arrows both drive (single player)
    Both,
    /// Player 1 in split-screen
    Wasd,
    /// Player 2 in split-screen
    Arrows,
}

pub struct Keys {
    pub up: KeyCode,
    pub down: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub handbrake: KeyCode,
    pub reset: KeyCode,
    pub camera: KeyCode,
}

pub const WASD: Keys = Keys { up: KeyCode::W, down: KeyCode::S, left: KeyCode::A, right: KeyCode::D, handbrake: KeyCode::Space, reset: KeyCode::R, camera: KeyCode::C };
pub const ARROWS: Keys = Keys { up: KeyCode::Up, down: KeyCode::Down, left: KeyCode::Left, right: KeyCode::Right, handbrake: KeyCode::RightShift, reset: KeyCode::Backspace, camera: KeyCode::RightControl };

pub struct PlayerInput {
    pub scheme: Scheme,
    steer: f32,
    throttle: f32,
    brake: f32,
}

fn down(k: &[KeyCode]) -> bool {
    k.iter().any(|k| is_key_down(*k))
}
fn pressed(k: &[KeyCode]) -> bool {
    k.iter().any(|k| is_key_pressed(*k))
}

impl PlayerInput {
    pub fn new(scheme: Scheme) -> Self {
        PlayerInput { scheme, steer: 0.0, throttle: 0.0, brake: 0.0 }
    }

    fn keys(&self) -> Vec<&'static Keys> {
        match self.scheme {
            Scheme::Both => vec![&WASD, &ARROWS],
            Scheme::Wasd => vec![&WASD],
            Scheme::Arrows => vec![&ARROWS],
        }
    }

    pub fn camera_pressed(&self) -> bool {
        let ks = self.keys();
        pressed(&ks.iter().map(|k| k.camera).collect::<Vec<_>>())
    }

    pub fn sample(&mut self, dt: f32, extra: Option<CarInput>) -> CarInput {
        let ks = self.keys();
        let up = down(&ks.iter().map(|k| k.up).collect::<Vec<_>>());
        let dn = down(&ks.iter().map(|k| k.down).collect::<Vec<_>>());
        let l = down(&ks.iter().map(|k| k.left).collect::<Vec<_>>());
        let r = down(&ks.iter().map(|k| k.right).collect::<Vec<_>>());
        let hb = down(&ks.iter().map(|k| k.handbrake).collect::<Vec<_>>());
        let reset = down(&ks.iter().map(|k| k.reset).collect::<Vec<_>>());
        let target = (r as i32 - l as i32) as f32;
        // steer ramps in, and returns to centre faster
        let rate = if target == 0.0 || target.signum() != self.steer.signum() { 7.0 } else { 3.6 };
        let d = (target - self.steer).clamp(-rate * dt, rate * dt);
        self.steer += d;
        let ramp = |v: &mut f32, on: bool, up_rate: f32| {
            let t = if on { 1.0 } else { 0.0 };
            let r = if on { up_rate } else { 10.0 };
            *v += (t - *v).clamp(-r * dt, r * dt);
        };
        ramp(&mut self.throttle, up, 6.0);
        ramp(&mut self.brake, dn, 8.0);
        let mut out = CarInput { throttle: self.throttle, brake: self.brake, steer: self.steer, handbrake: if hb { 1.0 } else { 0.0 }, reset };
        if let Some(e) = extra {
            out.throttle = out.throttle.max(e.throttle);
            out.brake = out.brake.max(e.brake);
            if e.steer.abs() > out.steer.abs() {
                out.steer = e.steer;
            }
            out.handbrake = out.handbrake.max(e.handbrake);
            out.reset |= e.reset;
        }
        out
    }
}
