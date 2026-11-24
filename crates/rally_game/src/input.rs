//! Player input: keyboard (with analogue-style smoothing) + gamepads (gilrs) -> CarInput,
//! plus gamepad menu navigation and light rumble.
//!
//! Gamepads are compiled in on Windows/macOS by default, and on Linux with the
//! `gamepad-linux` cargo feature (needs libudev headers). Without a backend, or when no pad is
//! connected, every pad query simply returns "nothing pressed" — never panics.
//!
//! Pad layout (Xbox names): left stick / d-pad = steer, RT = throttle, LT = brake/reverse,
//! A or B = handbrake, Y = respawn, X or View/Back = camera, Start = pause.
//! Menus: stick / d-pad = move, A = select, B = back, Start = select.
//! Player 1 = first connected pad, Player 2 (split-screen) = second pad (keyboard still works
//! for both: WASD for P1, arrows for P2).
use macroquad::prelude::*;
use rally_core::input::CarInput;
use std::cell::RefCell;

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

// ------------------------------------------------------------------ gamepad state

/// Logical pad buttons (bit flags)
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PadButton {
    A = 1,
    B = 2,
    X = 4,
    Y = 8,
    Start = 16,
    Back = 32,
    Lb = 64,
    Rb = 128,
    Up = 256,
    Down = 512,
    Left = 1024,
    Right = 2048,
}

/// Snapshot of one connected pad for this frame
#[derive(Clone, Copy, Debug, Default)]
pub struct PadState {
    /// Left stick, -1..1 (x right, y up)
    pub lx: f32,
    pub ly: f32,
    /// Triggers 0..1
    pub lt: f32,
    pub rt: f32,
    pub buttons: u32,
    pub prev_buttons: u32,
}

impl PadState {
    pub fn down(&self, b: PadButton) -> bool {
        self.buttons & b as u32 != 0
    }
    pub fn pressed(&self, b: PadButton) -> bool {
        self.buttons & b as u32 != 0 && self.prev_buttons & b as u32 == 0
    }
}

#[derive(Default)]
struct Pads {
    pads: Vec<PadState>,
    /// stick-as-dpad repeat for menus: (direction bits, next repeat time)
    nav_hold: u32,
    nav_next: f64,
    nav_pulse: u32,
    #[cfg(any(windows, target_os = "macos", feature = "gamepad-linux"))]
    backend: Option<backend::Backend>,
    #[cfg(any(windows, target_os = "macos", feature = "gamepad-linux"))]
    tried: bool,
}

thread_local! {
    static PADS: RefCell<Pads> = RefCell::new(Pads::default());
}

#[cfg(any(windows, target_os = "macos", feature = "gamepad-linux"))]
mod backend {
    use super::{PadButton, PadState};
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
    use gilrs::{Axis, Button, Gilrs};

    pub struct Backend {
        gilrs: Gilrs,
        effects: Vec<(gilrs::ff::Effect, f64)>,
    }

    impl Backend {
        pub fn new() -> Option<Backend> {
            match Gilrs::new() {
                Ok(g) => Some(Backend { gilrs: g, effects: Vec::new() }),
                Err(gilrs::Error::NotImplemented(g)) => Some(Backend { gilrs: g, effects: Vec::new() }),
                Err(_) => None,
            }
        }

        /// Drain events and read every connected pad (in connection order)
        pub fn poll(&mut self, now: f64) -> Vec<PadState> {
            while self.gilrs.next_event().is_some() {}
            self.effects.retain(|(_, until)| *until > now);
            let mut out = Vec::new();
            for (_, gp) in self.gilrs.gamepads() {
                if !gp.is_connected() {
                    continue;
                }
                let btn = |b: Button| gp.button_data(b).map(|d| d.value()).unwrap_or(0.0);
                let ax = |a: Axis| gp.axis_data(a).map(|d| d.value()).unwrap_or(0.0);
                let mut s = PadState { lx: ax(Axis::LeftStickX), ly: ax(Axis::LeftStickY), ..Default::default() };
                // triggers are analogue buttons on mapped pads; some drivers expose them as Z axes
                s.lt = btn(Button::LeftTrigger2).max(ax(Axis::LeftZ).max(0.0));
                s.rt = btn(Button::RightTrigger2).max(ax(Axis::RightZ).max(0.0));
                let map = [
                    (Button::South, PadButton::A),
                    (Button::East, PadButton::B),
                    (Button::West, PadButton::X),
                    (Button::North, PadButton::Y),
                    (Button::Start, PadButton::Start),
                    (Button::Select, PadButton::Back),
                    (Button::LeftTrigger, PadButton::Lb),
                    (Button::RightTrigger, PadButton::Rb),
                    (Button::DPadUp, PadButton::Up),
                    (Button::DPadDown, PadButton::Down),
                    (Button::DPadLeft, PadButton::Left),
                    (Button::DPadRight, PadButton::Right),
                ];
                for (b, p) in map {
                    if gp.is_pressed(b) {
                        s.buttons |= p as u32;
                    }
                }
                if !s.lx.is_finite() {
                    s.lx = 0.0;
                }
                if !s.ly.is_finite() {
                    s.ly = 0.0;
                }
                out.push(s);
            }
            out
        }

        /// Short rumble on the `index`-th connected pad (ignored if unsupported)
        pub fn rumble(&mut self, index: usize, strong: f32, weak: f32, ms: u32, now: f64) {
            let Some(id) = self.gilrs.gamepads().filter(|(_, g)| g.is_connected()).nth(index).map(|(id, g)| (id, g.is_ff_supported())).filter(|x| x.1).map(|x| x.0) else {
                return;
            };
            if self.effects.len() > 8 {
                return;
            }
            let sched = Replay { play_for: Ticks::from_ms(ms), ..Default::default() };
            let e = EffectBuilder::new()
                .add_effect(BaseEffect { kind: BaseEffectType::Strong { magnitude: (strong.clamp(0.0, 1.0) * 65535.0) as u16 }, scheduling: sched, ..Default::default() })
                .add_effect(BaseEffect { kind: BaseEffectType::Weak { magnitude: (weak.clamp(0.0, 1.0) * 65535.0) as u16 }, scheduling: sched, ..Default::default() })
                .gamepads(&[id])
                .finish(&mut self.gilrs);
            if let Ok(e) = e {
                if e.play().is_ok() {
                    // keep the handle alive until the effect has played
                    self.effects.push((e, now + ms as f64 / 1000.0 + 0.1));
                }
            }
        }
    }
}

/// Read the gamepads. Call once per frame, before any other input query.
pub fn poll() {
    let now = get_time();
    PADS.with(|p| {
        let mut p = p.borrow_mut();
        let prev: Vec<u32> = p.pads.iter().map(|s| s.buttons).collect();
        #[allow(unused_mut)]
        let mut pads: Vec<PadState> = Vec::new();
        #[cfg(any(windows, target_os = "macos", feature = "gamepad-linux"))]
        {
            if !p.tried {
                p.tried = true;
                p.backend = backend::Backend::new();
            }
            if let Some(b) = p.backend.as_mut() {
                pads = b.poll(now);
            }
        }
        for (i, s) in pads.iter_mut().enumerate() {
            s.prev_buttons = prev.get(i).copied().unwrap_or(s.buttons);
        }
        p.pads = pads;
        // menu navigation from sticks / d-pads of any pad, with key-repeat
        let mut dir = 0u32;
        for s in &p.pads {
            if s.ly > 0.6 || s.down(PadButton::Up) {
                dir |= PadButton::Up as u32;
            }
            if s.ly < -0.6 || s.down(PadButton::Down) {
                dir |= PadButton::Down as u32;
            }
            if s.lx < -0.6 || s.down(PadButton::Left) {
                dir |= PadButton::Left as u32;
            }
            if s.lx > 0.6 || s.down(PadButton::Right) {
                dir |= PadButton::Right as u32;
            }
        }
        p.nav_pulse = 0;
        if dir != p.nav_hold {
            p.nav_pulse = dir & !p.nav_hold;
            p.nav_hold = dir;
            p.nav_next = now + 0.42;
        } else if dir != 0 && now >= p.nav_next {
            p.nav_pulse = dir;
            p.nav_next = now + 0.14;
        }
    });
}

/// Number of connected gamepads
#[allow(dead_code)]
pub fn pad_count() -> usize {
    PADS.with(|p| p.borrow().pads.len())
}

/// State of the `i`-th connected pad this frame
pub fn pad(i: usize) -> Option<PadState> {
    PADS.with(|p| p.borrow().pads.get(i).copied())
}

fn any_pad_pressed(b: PadButton) -> bool {
    PADS.with(|p| p.borrow().pads.iter().any(|s| s.pressed(b)))
}

/// Menu navigation from gamepads (keyboard is handled by ui::Focus itself)
#[derive(Clone, Copy, Debug, Default)]
pub struct MenuNav {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub accept: bool,
    pub back: bool,
}

pub fn menu_nav() -> MenuNav {
    let pulse = PADS.with(|p| p.borrow().nav_pulse);
    MenuNav {
        up: pulse & PadButton::Up as u32 != 0,
        down: pulse & PadButton::Down as u32 != 0,
        left: pulse & PadButton::Left as u32 != 0,
        right: pulse & PadButton::Right as u32 != 0,
        accept: any_pad_pressed(PadButton::A) || any_pad_pressed(PadButton::Start),
        back: any_pad_pressed(PadButton::B),
    }
}

/// Pause toggle: Start on any pad (keyboard Esc / P is handled by the caller)
pub fn pause_pressed() -> bool {
    any_pad_pressed(PadButton::Start)
}

/// "Back" in menus: Escape or B on any pad (ui::Focus::back already includes the pad)
#[allow(dead_code)]
pub fn back_pressed() -> bool {
    is_key_pressed(KeyCode::Escape) || any_pad_pressed(PadButton::B)
}

/// Light rumble on a pad (0..1 strength). No-op without pads / force feedback.
#[allow(unused_variables)]
pub fn rumble(pad_index: usize, strength: f32, ms: u32) {
    #[cfg(any(windows, target_os = "macos", feature = "gamepad-linux"))]
    PADS.with(|p| {
        if let Some(b) = p.borrow_mut().backend.as_mut() {
            b.rumble(pad_index, strength, strength * 0.6, ms, get_time());
        }
    });
}

/// Analogue stick shaping: radial-ish deadzone, rescale, gentle curve (fine control near the
/// centre, full lock at the edge).
pub fn shape_stick(x: f32) -> f32 {
    const DZ: f32 = 0.13;
    let a = x.abs();
    if a <= DZ || !a.is_finite() {
        return 0.0;
    }
    let t = ((a - DZ) / (1.0 - DZ)).min(1.0);
    x.signum() * (0.35 * t + 0.65 * t * t)
}

fn shape_trigger(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    ((v - 0.04) / 0.92).clamp(0.0, 1.0)
}

// ------------------------------------------------------------------ per-player input

pub struct PlayerInput {
    pub scheme: Scheme,
    /// Which connected pad drives this player (index in connection order)
    pub pad: Option<usize>,
    steer: f32,
    pad_steer: f32,
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
        let pad = match scheme {
            Scheme::Both | Scheme::Wasd => Some(0),
            Scheme::Arrows => Some(1),
        };
        PlayerInput { scheme, pad, steer: 0.0, pad_steer: 0.0, throttle: 0.0, brake: 0.0 }
    }

    fn keys(&self) -> Vec<&'static Keys> {
        match self.scheme {
            Scheme::Both => vec![&WASD, &ARROWS],
            Scheme::Wasd => vec![&WASD],
            Scheme::Arrows => vec![&ARROWS],
        }
    }

    fn pad_state(&self) -> Option<PadState> {
        self.pad.and_then(pad)
    }

    pub fn camera_pressed(&self) -> bool {
        let ks = self.keys();
        let p = self.pad_state().is_some_and(|p| p.pressed(PadButton::X) || p.pressed(PadButton::Back));
        p || pressed(&ks.iter().map(|k| k.camera).collect::<Vec<_>>())
    }

    /// High / low beam toggle (night races): H (Right Alt for the arrows player), pad D-pad up
    pub fn beam_pressed(&self) -> bool {
        let p = self.pad_state().is_some_and(|p| p.pressed(PadButton::Up));
        let k = match self.scheme {
            Scheme::Arrows => is_key_pressed(KeyCode::RightAlt),
            _ => is_key_pressed(KeyCode::H),
        };
        p || k
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
        // keyboard steer ramps in (gentle near centre, quicker as you hold), returns faster
        // (the car's slip-angle limiter maps full input to peak front grip, so a tap is a
        // small correction and holding glides to the limit in ~0.3 s)
        let rate = if target == 0.0 || target.signum() != self.steer.signum() {
            7.5
        } else {
            1.8 + 3.2 * self.steer.abs()
        };
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
        // gamepad
        if let Some(p) = self.pad_state() {
            let mut s = shape_stick(p.lx);
            if p.down(PadButton::Left) {
                s = -1.0;
            }
            if p.down(PadButton::Right) {
                s = 1.0;
            }
            // light smoothing only (analogue input is already smooth)
            let r = 14.0 * dt;
            self.pad_steer += (s - self.pad_steer).clamp(-r, r);
            if self.pad_steer.abs() > out.steer.abs() {
                out.steer = self.pad_steer;
            }
            out.throttle = out.throttle.max(shape_trigger(p.rt));
            out.brake = out.brake.max(shape_trigger(p.lt));
            if p.down(PadButton::A) || p.down(PadButton::B) {
                out.handbrake = 1.0;
            }
            out.reset |= p.down(PadButton::Y);
        } else {
            self.pad_steer = 0.0;
        }
        if let Some(e) = extra {
            out.throttle = out.throttle.max(e.throttle);
            out.brake = out.brake.max(e.brake);
            if e.steer.abs() > out.steer.abs() {
                out.steer = e.steer;
            }
            out.handbrake = out.handbrake.max(e.handbrake);
            out.reset |= e.reset;
        }
        out.steer = out.steer.clamp(-1.0, 1.0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stick_shape() {
        assert_eq!(shape_stick(0.1), 0.0);
        assert!((shape_stick(1.0) - 1.0).abs() < 1e-6);
        assert!((shape_stick(-1.0) + 1.0).abs() < 1e-6);
        assert!(shape_stick(0.5) < 0.5 && shape_stick(0.5) > 0.2);
        assert_eq!(shape_stick(f32::NAN), 0.0);
    }
}

/// Feed gamepad menu navigation into a ui::Focus (called at the end of Focus::begin)
pub fn apply_menu_nav(f: &mut crate::ui::Focus) {
    let nav = menu_nav();
    if nav.down {
        f.index = (f.index + 1) % f.count.max(1);
        crate::audio::ui_sound(crate::audio::UiSound::Move);
    }
    if nav.up {
        f.index = (f.index + f.count.max(1) - 1) % f.count.max(1);
        crate::audio::ui_sound(crate::audio::UiSound::Move);
    }
    f.activated |= nav.accept;
    f.back |= nav.back;
    f.left |= nav.left;
    f.right |= nav.right;
}
