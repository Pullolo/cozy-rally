//! On-screen touch controls for phones (player 1 only).
//!
//! Layout (landscape): steering bottom-left (two arrows, or a floating slider), gas / brake /
//! handbrake bottom-right, small respawn + camera buttons above the gas. Thumbs can slide
//! between gas and brake or between the arrows. The controls fade out once a gamepad is used and
//! come back on the next touch.
//!
//! Active on Android, or on PC with `COZY_TOUCH=1` (mouse acts as one finger) for testing.
use crate::ui::*;
use macroquad::prelude::*;
use rally_core::input::CarInput;
use std::cell::RefCell;

/// Steering style (settings.touch_steer)
pub const STEER_NAMES: [&str; 2] = ["Arrows", "Slider"];

#[derive(Default)]
struct Touch {
    /// 0..1 fade (1 = shown)
    shown: f32,
    /// Seconds since a gamepad was last used (controls hide while a pad drives)
    pad_quiet: f32,
    steer: f32,
    throttle: f32,
    brake: f32,
    /// Slider mode: (touch id, x where the thumb went down)
    slider: Option<(u64, f32)>,
    slider_x: f32,
    camera: bool,
    /// Night races: the beam pill was tapped (main.rs) -> `Input::beam_pressed`
    beam: bool,
    /// What is held this frame (for drawing)
    held: Held,
}

#[derive(Default, Clone, Copy)]
struct Held {
    left: bool,
    right: bool,
    gas: bool,
    brake: bool,
    handbrake: bool,
    reset: bool,
    camera: bool,
}

thread_local! {
    static TOUCH: RefCell<Touch> = RefCell::new(Touch { shown: 1.0, pad_quiet: 99.0, ..Default::default() });
}

/// Touch controls exist on this platform (Android, or COZY_TOUCH=1 on PC)
pub fn enabled() -> bool {
    crate::platform::MOBILE || std::env::var_os("COZY_TOUCH").is_some_and(|v| v != "0")
}

/// Controls currently on screen (HUD moves its bottom widgets out of the way)
pub fn visible() -> bool {
    enabled() && TOUCH.with(|t| t.borrow().shown > 0.05)
}

/// Camera button tapped this frame
pub fn take_camera() -> bool {
    TOUCH.with(|t| std::mem::take(&mut t.borrow_mut().camera))
}

/// The high / low beam pill was tapped (night races)
pub fn request_beam() {
    TOUCH.with(|t| t.borrow_mut().beam = true);
}

pub fn take_beam() -> bool {
    TOUCH.with(|t| std::mem::take(&mut t.borrow_mut().beam))
}

/// A touch started inside `r` this frame
pub fn tapped(r: Rect) -> bool {
    touches().iter().any(|t| t.phase == TouchPhase::Started && r.contains(t.position))
}

struct Layout {
    k: f32,
    left: Vec2,
    right: Vec2,
    arrow_r: f32,
    gas: Vec2,
    gas_r: f32,
    brake: Vec2,
    brake_r: f32,
    hb: Vec2,
    hb_r: f32,
    reset: Vec2,
    cam: Vec2,
    small_r: f32,
}

fn layout() -> Layout {
    let k = scale();
    let (w, h) = (screen_width(), screen_height());
    Layout {
        k,
        left: vec2(92.0 * k, h - 96.0 * k),
        right: vec2(252.0 * k, h - 96.0 * k),
        arrow_r: 64.0 * k,
        gas: vec2(w - 100.0 * k, h - 104.0 * k),
        gas_r: 74.0 * k,
        brake: vec2(w - 262.0 * k, h - 82.0 * k),
        brake_r: 58.0 * k,
        hb: vec2(w - 222.0 * k, h - 222.0 * k),
        hb_r: 46.0 * k,
        reset: vec2(w - 58.0 * k, h - 250.0 * k),
        cam: vec2(w - 58.0 * k, h - 318.0 * k),
        small_r: 28.0 * k,
    }
}

/// Active pointers this frame: (id, position, just started)
fn pointers() -> Vec<(u64, Vec2, bool)> {
    let mut v: Vec<(u64, Vec2, bool)> = touches()
        .into_iter()
        .filter(|t| t.phase != TouchPhase::Ended && t.phase != TouchPhase::Cancelled)
        .map(|t| (t.id, t.position, t.phase == TouchPhase::Started))
        .collect();
    // PC testing: the mouse is one finger (touch events also emulate the mouse on phones)
    if !crate::platform::MOBILE && is_mouse_button_down(MouseButton::Left) {
        v.push((u64::MAX, mouse_position().into(), is_mouse_button_pressed(MouseButton::Left)));
    }
    v
}

/// Sample the touch controls for player 1. `None` when touch is off or hidden.
pub fn sample(dt: f32, steer_mode: usize) -> Option<CarInput> {
    if !enabled() {
        return None;
    }
    TOUCH.with(|t| {
        let mut t = t.borrow_mut();
        let ptrs = pointers();
        // a pad in use hides the controls; any touch brings them back
        let pad_used = crate::input::pad(0).is_some_and(|p| p.buttons != 0 || p.lx.abs() > 0.3 || p.rt > 0.1 || p.lt > 0.1);
        t.pad_quiet = if pad_used { 0.0 } else { t.pad_quiet + dt };
        if !ptrs.is_empty() {
            t.pad_quiet = t.pad_quiet.max(3.0);
        }
        let want = if t.pad_quiet > 2.0 { 1.0 } else { 0.0 };
        t.shown = approach(t.shown, want, 6.0, dt);
        if t.shown < 0.05 {
            t.held = Held::default();
            t.slider = None;
            return None;
        }
        let l = layout();
        let hit = |p: Vec2, c: Vec2, r: f32| p.distance(c) <= r * 1.18;
        let mut held = Held::default();
        let mut slider_seen = false;
        for &(id, p, started) in &ptrs {
            // slider steering: a thumb that went down on the left 40% owns it until lifted
            if steer_mode == 1 {
                if t.slider.is_some_and(|(sid, _)| sid == id) {
                    slider_seen = true;
                    t.slider_x = p.x;
                    continue;
                }
                if started && t.slider.is_none() && p.x < screen_width() * 0.4 && p.y > screen_height() * 0.35 {
                    t.slider = Some((id, p.x));
                    t.slider_x = p.x;
                    slider_seen = true;
                    continue;
                }
            } else {
                held.left |= hit(p, l.left, l.arrow_r) || (p.x < l.left.x && p.y > l.left.y - l.arrow_r * 1.3);
                held.right |= hit(p, l.right, l.arrow_r);
            }
            held.gas |= hit(p, l.gas, l.gas_r) || (p.x > l.gas.x && p.y > l.gas.y - l.gas_r * 1.2);
            held.brake |= hit(p, l.brake, l.brake_r);
            held.handbrake |= hit(p, l.hb, l.hb_r);
            held.reset |= hit(p, l.reset, l.small_r * 1.2);
            if started && hit(p, l.cam, l.small_r * 1.2) {
                t.camera = true;
                held.camera = true;
            }
        }
        if !slider_seen {
            t.slider = None;
        }
        // steering target
        let target = if let Some((_, x0)) = t.slider {
            ((t.slider_x - x0) / (95.0 * l.k)).clamp(-1.0, 1.0)
        } else {
            (held.right as i32 - held.left as i32) as f32
        };
        if t.slider.is_some() {
            // analogue thumb: light smoothing only
            let r = 12.0 * dt;
            t.steer += (target - t.steer).clamp(-r, r);
        } else {
            // same ramp as the keyboard (input.rs): a tap is a small correction
            let rate = if target == 0.0 || target.signum() != t.steer.signum() { 7.5 } else { 1.8 + 3.2 * t.steer.abs() };
            t.steer += (target - t.steer).clamp(-rate * dt, rate * dt);
        }
        let ramp = |v: &mut f32, on: bool, up: f32| {
            let goal = if on { 1.0 } else { 0.0 };
            let r = if on { up } else { 10.0 };
            *v += (goal - *v).clamp(-r * dt, r * dt);
        };
        ramp(&mut t.throttle, held.gas, 6.0);
        ramp(&mut t.brake, held.brake, 8.0);
        t.held = held;
        Some(CarInput { throttle: t.throttle, brake: t.brake, steer: t.steer, handbrake: if held.handbrake { 1.0 } else { 0.0 }, reset: held.reset, ai: false })
    })
}

/// Reset the controls (new race / pause) so nothing stays held
pub fn release() {
    TOUCH.with(|t| {
        let mut t = t.borrow_mut();
        t.steer = 0.0;
        t.throttle = 0.0;
        t.brake = 0.0;
        t.slider = None;
        t.held = Held::default();
    });
}

fn round_button(c: Vec2, r: f32, base: Color, pressed: bool, a: f32) {
    let sink = if pressed { 2.0 } else { 0.0 } * r / 40.0;
    circle(c.x, c.y + r * 0.07, r, with_alpha(SHADOW, a * 0.9));
    circle(c.x, c.y + sink, r, with_alpha(if pressed { shade(base, 0.86) } else { base }, a * if pressed { 0.95 } else { 0.62 }));
    circle(c.x, c.y + sink - r * 0.06, r * 0.84, with_alpha(lighten(base, 0.25), a * if pressed { 0.5 } else { 0.28 }));
}

fn chevron(c: Vec2, size: f32, dir: Vec2, color: Color) {
    // filled triangle pointing along `dir`
    let n = vec2(-dir.y, dir.x);
    let tip = c + dir * size * 0.55;
    let b = c - dir * size * 0.4;
    draw_triangle(tip, b + n * size * 0.55, b - n * size * 0.55, color);
}

/// Draw the controls (call after the HUD, outside the pause menu)
pub fn draw(steer_mode: usize) {
    if !enabled() {
        return;
    }
    TOUCH.with(|t| {
        let t = t.borrow();
        let a = t.shown;
        if a < 0.05 {
            return;
        }
        let l = layout();
        let h = t.held;
        let ink = with_alpha(INK, a * 0.8);
        let k = l.k;
        // steering
        if steer_mode == 1 {
            let y = l.left.y;
            let (x0, x1) = (60.0 * k, 290.0 * k);
            rounded_rect(x0, y - 10.0 * k, x1 - x0, 20.0 * k, 10.0 * k, with_alpha(CREAM, a * 0.45));
            let knob_x = (x0 + x1) * 0.5 + t.steer * (x1 - x0) * 0.42;
            round_button(vec2(knob_x, y), 40.0 * k, SKY, t.slider.is_some(), a);
            chevron(vec2(knob_x - 15.0 * k, y), 16.0 * k, vec2(-1.0, 0.0), ink);
            chevron(vec2(knob_x + 15.0 * k, y), 16.0 * k, vec2(1.0, 0.0), ink);
        } else {
            round_button(l.left, l.arrow_r, SKY, h.left, a);
            round_button(l.right, l.arrow_r, SKY, h.right, a);
            chevron(l.left, 40.0 * k, vec2(-1.0, 0.0), ink);
            chevron(l.right, 40.0 * k, vec2(1.0, 0.0), ink);
        }
        // pedals
        round_button(l.gas, l.gas_r, MINT, h.gas, a);
        chevron(l.gas - vec2(0.0, 8.0 * k), 34.0 * k, vec2(0.0, -1.0), ink);
        chevron(l.gas + vec2(0.0, 14.0 * k), 34.0 * k, vec2(0.0, -1.0), ink);
        round_button(l.brake, l.brake_r, PEACH, h.brake, a);
        chevron(l.brake, 30.0 * k, vec2(0.0, 1.0), ink);
        round_button(l.hb, l.hb_r, BUTTER, h.handbrake, a);
        text_mid("HB", l.hb.x, l.hb.y, 22.0 * k, ink);
        // respawn: circular arrow
        round_button(l.reset, l.small_r, LAVENDER, h.reset, a);
        arc(l.reset.x, l.reset.y, 9.0 * k, 13.0 * k, 0.6, 5.6, ink);
        chevron(l.reset + vec2(11.0 * k, -5.0 * k), 9.0 * k, vec2(0.0, 1.0), ink);
        // camera
        round_button(l.cam, l.small_r, CREAM, h.camera, a);
        rounded_rect(l.cam.x - 13.0 * k, l.cam.y - 8.0 * k, 26.0 * k, 17.0 * k, 4.0 * k, ink);
        circle(l.cam.x, l.cam.y + 0.5 * k, 5.5 * k, with_alpha(CREAM, a));
    });
}
