//! Drift behaviour test: can a driver initiate, hold and exit a drift, and does the car swing
//! left/right ("pendulum") or straighten itself out?
//!
//! For each car / assist level / surface:
//! * init: 70 km/h, flick into a corner with a short handbrake pull, full throttle, then
//!   HANDS OFF the steering (keyboard released). Reports how long the drift lasts. Human
//!   drivers get no automatic countersteer, so hands off usually ends in a spin (max ~90).
//! * pad: an analogue "skilled driver" holds a 30 deg drift for 6 s (steering toward the front
//!   axle's direction of travel plus a correction, throttle on the drift angle).
//! * key: the same with keyboard-style digital steering and throttle (game ramps).
//! * exit: after holding 2.5 s (pad), lift and centre the steering: does the car recover without
//!   swinging to the other side?
//!
//! Metrics: held = share of time at 12..55 deg on the original side, flips = side changes
//! beyond 6 deg, opp = largest drift angle on the other side, spun = > 95 deg.
//! cargo run --release -p rally_core --example drifttest -- [assist] [surface]
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};

const DT: f32 = 1.0 / 240.0;

fn fresh(spec: usize, track: &Track, assist: f32, kmh: f32) -> Car {
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 300.0, 0.9, track.terrain.origin.y + 300.0), Quat::IDENTITY);
    c.assist_level = assist;
    for _ in 0..480 {
        c.step(DT, track);
    }
    c.vel = c.forward() * (kmh / 3.6);
    for w in c.wheels.iter_mut() {
        w.spin_speed = kmh / 3.6 / c.spec.wheel_radius;
    }
    let target_w = kmh / 3.6 / c.spec.wheel_radius;
    let n = c.spec.gears.len();
    c.gear = (1..=n as i32).find(|g| target_w * c.spec.gears[*g as usize - 1] * c.spec.final_drive * 60.0 / std::f32::consts::TAU < c.spec.redline * 0.8).unwrap_or(n as i32);
    c
}

/// keyboard steer ramp as in rally_game input.rs
fn kb_steer(steer: &mut f32, target: f32) {
    let rate = if target == 0.0 || target.signum() != steer.signum() { 7.5 } else { 1.8 + 3.2 * steer.abs() };
    *steer += (target - *steer).clamp(-rate * DT, rate * DT);
}

fn kb_pedal(v: &mut f32, on: bool, up: f32) {
    let t = if on { 1.0 } else { 0.0 };
    let r = if on { up } else { 10.0 };
    *v += (t - *v).clamp(-r * DT, r * DT);
}

#[derive(Default)]
struct Stats {
    held: f32,
    flips: u32,
    opp: f32,
    spun: bool,
    max: f32,
    /// time the drift lasted (> 12 deg, original side) from the end of the initiation
    lasted: f32,
    end_kmh: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    HandsOff,
    Pad,
    Key,
    Exit,
}

/// Initiation (shared): right-hand flick with handbrake, then the mode's driver.
fn run(spec: usize, track: &Track, assist: f32, mode: Mode) -> Stats {
    let mut c = fresh(spec, track, assist, 70.0);
    let (mut steer, mut thr) = (0.0f32, 0.0f32);
    let target = 30f32.to_radians();
    let mut st = Stats::default();
    let mut side = 0.0f32;
    let mut cur = 0.0f32;
    let mut alive = true;
    let total = 7.0;
    let t_init = 0.9;
    let mut t = 0.0f32;
    let mut n = 0u32;
    while t < total {
        let beta = c.telemetry.drift_angle;
        let mut inp = CarInput::default();
        if t < t_init {
            // flick right, pull the handbrake briefly, floor it
            kb_steer(&mut steer, 1.0);
            kb_pedal(&mut thr, true, 6.0);
            inp.handbrake = if (0.25..0.55).contains(&t) { 1.0 } else { 0.0 };
        } else {
            if side == 0.0 {
                side = if beta.abs() > 0.05 { beta.signum() } else { -1.0 };
            }
            let b = beta * side; // > 0 = on the drift side
            match mode {
                Mode::HandsOff => {
                    kb_steer(&mut steer, 0.0);
                    kb_pedal(&mut thr, true, 6.0);
                }
                _ if mode == Mode::Pad || (mode == Mode::Exit && t <= t_init + 2.5) => {
                    // wheels along the front axle's travel, plus more countersteer when over angle
                    let theta = c.front_travel_angle();
                    let corr = (b - target) * 1.2;
                    let angle = theta + side * corr * 0.6;
                    steer = c.steer_input_for_angle(angle, false);
                    thr = (0.75 + (target - b) * 4.0).clamp(0.0, 1.0);
                }
                Mode::Exit | Mode::Pad => {
                    // lift and centre (pad, smoothly)
                    steer += (0.0 - steer).clamp(-6.0 * DT, 6.0 * DT);
                    thr = 0.3;
                }
                Mode::Key => {
                    // too much angle: countersteer key; too little: steer into the corner; else off
                    let theta = c.front_travel_angle();
                    let counter = theta.signum();
                    let key = if b > target + 8f32.to_radians() { counter } else if b < target - 10f32.to_radians() { -counter } else { 0.0 };
                    kb_steer(&mut steer, key);
                    kb_pedal(&mut thr, b < target + 4f32.to_radians(), 6.0);
                }
            }
        }
        inp.steer = steer;
        inp.throttle = thr;
        c.input = inp;
        c.step(DT, track);
        t += DT;
        if std::env::var("TRACE").is_ok_and(|v| v == format!("{mode:?}")) && (t * 240.0) as u32 % 12 == 0 && t < 4.0 {
            let w = &c.wheels;
            println!(
                "t {t:4.2} st {:5.2} wheel {:5.2} thr {:3.1} hb {} beta {:6.1} yaw {:6.2} v {:5.1} | rear kappa {:5.2} {:5.2} alpha {:5.2} {:5.2} load {:5.0} {:5.0} | front alpha {:5.2} {:5.2} tc {:4.2} {:4.2} rpm {:5.0} g {}",
                inp.steer, c.steer, inp.throttle, inp.handbrake, c.telemetry.drift_angle.to_degrees(), c.ang_vel.dot(c.up()), c.vel.length() * 3.6,
                w[2].slip_ratio, w[3].slip_ratio, w[2].slip_angle, w[3].slip_angle, w[2].load, w[3].load, w[0].slip_angle, w[1].slip_angle, w[2].tc_cut, w[3].tc_cut, c.rpm, c.gear
            );
        }
        if t > t_init && side != 0.0 {
            let d = c.telemetry.drift_angle * side;
            st.max = st.max.max(d);
            n += 1;
            if (12f32.to_radians()..55f32.to_radians()).contains(&d) {
                st.held += 1.0;
            }
            if alive && d > 12f32.to_radians() {
                st.lasted = t - t_init;
            } else if t - t_init > 0.4 {
                alive = false;
            }
            let sd = c.telemetry.drift_angle;
            if sd.abs() > 6f32.to_radians() {
                if cur != 0.0 && sd.signum() != cur {
                    st.flips += 1;
                }
                cur = sd.signum();
            }
            if d < 0.0 {
                st.opp = st.opp.max(-d);
            }
            if c.telemetry.drift_angle.abs() > 95f32.to_radians() || c.up().y < 0.5 {
                st.spun = true;
            }
        }
    }
    st.held /= n.max(1) as f32;
    st.end_kmh = c.vel.length() * 3.6;
    st
}

fn main() {
    let assists: Vec<(String, f32)> = match std::env::args().nth(1).and_then(|s| s.parse::<f32>().ok()) {
        Some(a) => vec![(format!("{a}"), a)],
        None => vec![("Cozy".into(), 1.3), ("Normal".into(), 1.0), ("Sport".into(), 0.55), ("Sim".into(), 0.0)],
    };
    let surf = match std::env::args().nth(2).as_deref() {
        Some("gravel") => Surface::Gravel,
        Some("snow") => Surface::Snow,
        _ => Surface::Asphalt,
    };
    let track = Track::flat_for_tests(surf);
    let cars: Vec<usize> = std::env::var("CARS").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| vec![0, 1, 2, 4, 8, 5, 7]);
    println!("=== DRIFT on {surf:?}: init(hands off) lasted s | pad hold: held% flips opp | key hold: held% flips opp | exit: flips opp  (deg)");
    let mut score = 0.0f32;
    for spec in cars {
        for (an, a) in &assists {
            let h = run(spec, &track, *a, Mode::HandsOff);
            let p = run(spec, &track, *a, Mode::Pad);
            let k = run(spec, &track, *a, Mode::Key);
            let e = run(spec, &track, *a, Mode::Exit);
            let sp = |s: &Stats| if s.spun { "SPUN" } else { "    " };
            println!(
                "{:9} {an:6} | init {:4.1}s max {:4.0} {} | pad {:3.0}% fl {} opp {:4.1} {} | key {:3.0}% fl {} opp {:4.1} {} | exit fl {} opp {:4.1} {} {:3.0}km/h",
                car_specs()[spec].name,
                h.lasted,
                h.max.to_degrees(),
                sp(&h),
                p.held * 100.0,
                p.flips,
                p.opp.to_degrees(),
                sp(&p),
                k.held * 100.0,
                k.flips,
                k.opp.to_degrees(),
                sp(&k),
                e.flips,
                e.opp.to_degrees(),
                sp(&e),
                e.end_kmh
            );
            score += p.held + k.held - 0.1 * (p.flips + k.flips + e.flips) as f32 - e.opp.to_degrees() * 0.01 - if p.spun || k.spun || e.spun { 0.3 } else { 0.0 };
        }
    }
    println!("score {score:.2}");
}
