//! Racing-line assist (Forza-style): a soft ribbon on the road along the car's racing line,
//! coloured by what to do *from your current speed*: teal = accelerate, butter = lift,
//! coral = brake. Off / Braking only / Full via settings (`racing_line`, key L in race).
//! One dynamic alpha draw per viewport.
use crate::game::RaceSession;
use crate::render::gpu::{DrawParams, Gpu, View};
use crate::render::mesh::{rgba, Vertex};
use macroquad::math::{Mat4, Vec3};
use rally_core::racing_line::{RacingLine, SpeedParams, SpeedProfile};
use rally_core::track::Track;
use rally_core::vehicle::{Car, CarSpec};
use std::cell::RefCell;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineMode {
    Off,
    BrakingOnly,
    Full,
}

impl LineMode {
    pub fn name(self) -> &'static str {
        match self {
            LineMode::Off => "Racing line off",
            LineMode::BrakingOnly => "Racing line: braking only",
            LineMode::Full => "Racing line on",
        }
    }
}

/// Mode for a car: the setting, where "Auto" = Full, or Off on Sim assists
pub fn mode_for(car: &Car) -> LineMode {
    match crate::settings::get().racing_line {
        1 => LineMode::Off,
        2 => LineMode::BrakingOnly,
        3 => LineMode::Full,
        _ => {
            if car.assist_level <= 0.2 {
                LineMode::Off
            } else {
                LineMode::Full
            }
        }
    }
}

/// Cycle Full -> Braking only -> Off -> Full (stored explicitly in the settings)
pub fn cycle(car: &Car) -> LineMode {
    let next = match mode_for(car) {
        LineMode::Full => LineMode::BrakingOnly,
        LineMode::BrakingOnly => LineMode::Off,
        LineMode::Off => LineMode::Full,
    };
    crate::settings::update(|s| {
        s.racing_line = match next {
            LineMode::Off => 1,
            LineMode::BrakingOnly => 2,
            LineMode::Full => 3,
        }
    });
    crate::settings::save();
    next
}

type Key = (usize, usize, u8);
thread_local! {
    static CACHE: RefCell<Vec<(Key, Arc<RacingLine>, Arc<SpeedProfile>)>> = const { RefCell::new(Vec::new()) };
}

/// Racing line + a "good driver" speed profile for this car (cached per track and car)
fn plan(track: &Track, spec: &CarSpec) -> (Arc<RacingLine>, Arc<SpeedProfile>) {
    let key = (track.samples.as_ptr() as usize, track.samples.len(), spec.id);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, l, p)) = c.iter().find(|e| e.0 == key) {
            return (l.clone(), p.clone());
        }
        let line = RacingLine::shared(track);
        // what a quick, tidy driver can do (the AI's top level, braking a touch earlier)
        let prof = Arc::new(SpeedProfile::build(&line, &SpeedParams::for_car(spec, 0.92 * spec.ai_grip, 0.85)));
        c.push((key, line.clone(), prof.clone()));
        if c.len() > 8 {
            c.remove(0);
        }
        (line, prof)
    })
}

const TEAL: [f32; 3] = [0.36, 0.86, 0.72];
const BUTTER: [f32; 3] = [1.0, 0.86, 0.40];
const CORAL: [f32; 3] = [0.98, 0.42, 0.42];
const HALF_W: f32 = 0.55;
const START: f32 = 3.0;
const LENGTH: f32 = 120.0;
const STEP: f32 = 1.5;

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Draw the ribbon for local player `player` into `view`.
pub fn draw(sess: &RaceSession, gpu: &mut Gpu, view: &View, player: usize) {
    let Some(p) = sess.players.get(player) else { return };
    let car = &sess.world.cars[p.car];
    let mode = mode_for(car);
    if mode == LineMode::Off || sess.world.race.racers[p.car].finished {
        return;
    }
    let track = sess.track();
    let (line, prof) = plan(track, &car.spec);
    let q = track.query(car.pos, car.track_hint);
    let smp = &track.samples[q.index];
    let fwd = car.forward();
    let v_along = car.vel.dot(smp.tangent);
    // hidden when lost, reversing or facing the wrong way
    if q.dist > smp.half_width + 4.0 || car.telemetry.forward_speed < -0.5 || (fwd.dot(smp.tangent) < 0.2 && car.vel.length() > 2.0) {
        return;
    }
    let v = v_along.max(0.0);
    let hf = &track.terrain;
    let flat_right = |s: f32| {
        let (_, _, r, _) = track.frame_at(s);
        Vec3::new(r.x, 0.0, r.z).normalize_or_zero()
    };
    let point = |s: f32, lat: f32| {
        let (c, _, _, _) = track.frame_at(s);
        let r = flat_right(s);
        let x = c + r * (line.offset_at(s, track) + lat);
        Vec3::new(x.x, hf.height(x.x, x.z) + 0.07, x.z)
    };
    let n = (LENGTH / STEP) as usize;
    let mut verts: Vec<Vertex> = Vec::with_capacity(n * 2 + 64);
    let mut idx: Vec<u16> = Vec::with_capacity(n * 6 + 96);
    let mut prev_visible = false;
    // speed projection: you'll keep gaining a little speed until you lift
    let accel = 1.6;
    let mut chevrons: Vec<(f32, [f32; 3], f32)> = Vec::new();
    for k in 0..=n {
        let d = START + k as f32 * STEP;
        let s = q.s + d;
        let limit = prof.limit_at(s, track).max(3.0);
        let v_proj = (v * v + 2.0 * accel * d).sqrt();
        let ratio = v_proj / limit;
        // teal -> butter -> coral as your speed approaches / exceeds what the corner allows
        let to_yellow = smooth(0.89, 0.97, ratio);
        let to_red = smooth(1.03, 1.09, ratio);
        let col = mix(mix(TEAL, BUTTER, to_yellow), CORAL, to_red);
        let mut a = 0.72 * smooth(0.0, 7.0, d - START) * (1.0 - smooth(LENGTH - 30.0, LENGTH, d));
        if mode == LineMode::BrakingOnly {
            a *= to_yellow;
        }
        let c = point(s, 0.0);
        let r = flat_right(s);
        let (l, rr) = (c - r * HALF_W, c + r * HALF_W);
        let (lp, rp) = (Vec3::new(l.x, hf.height(l.x, l.z) + 0.07, l.z), Vec3::new(rr.x, hf.height(rr.x, rr.z) + 0.07, rr.z));
        let base = verts.len() as u16;
        verts.push(Vertex { pos: lp.to_array(), normal: [0.0, 1.0, 0.0], color: rgba(col, a) });
        verts.push(Vertex { pos: rp.to_array(), normal: [0.0, 1.0, 0.0], color: rgba(col, a) });
        let visible = a > 0.01;
        if k > 0 && (visible || prev_visible) {
            idx.extend_from_slice(&[base - 2, base - 1, base + 1, base - 2, base + 1, base]);
        }
        prev_visible = visible;
        if k % 4 == 2 && a > 0.05 {
            chevrons.push((s, col, a));
        }
    }
    // subtle chevrons pointing along the line
    for (s, col, a) in chevrons {
        let light = mix(col, [1.0, 1.0, 0.97], 0.45);
        let tip = point(s + 0.7, 0.0);
        let tip_b = point(s + 0.35, 0.0);
        let r = flat_right(s);
        for side in [-1.0f32, 1.0] {
            let outer = point(s - 0.25, 0.0) + r * side * (HALF_W * 0.75);
            let outer_b = point(s - 0.6, 0.0) + r * side * (HALF_W * 0.75);
            let lift = |p: Vec3| Vec3::new(p.x, hf.height(p.x, p.z) + 0.075, p.z);
            let base = verts.len() as u16;
            for p in [lift(tip), lift(outer), lift(outer_b), lift(tip_b)] {
                verts.push(Vertex { pos: p.to_array(), normal: [0.0, 1.0, 0.0], color: rgba(light, a * 0.8) });
            }
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    if idx.is_empty() {
        return;
    }
    let params = DrawParams { unlit: 1.0, depth_bias: 3e-5, cast_shadow: false, receive_shadow: false, ..Default::default() };
    gpu.draw_dynamic(&verts, &idx, view, &sess.scene.env, Mat4::IDENTITY, &params, 1);
}
