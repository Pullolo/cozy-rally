//! High-speed steering stability test ("tank-slapper" check) for keyboard-style input.
//! Drives each car at speed on flat tarmac (speed held), applies a keyboard steer tap/hold
//! with the same ramp the game uses, releases, and measures whether the yaw motion dies out
//! or keeps swinging left/right. Also a lane-change (left tap then right tap).
//! cargo run --release -p rally_core --example weave -- [assist] [kmh]
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};

const DT: f32 = 1.0 / 240.0;

fn fresh(spec: usize, track: &Track, assist: f32, kmh: f32) -> Car {
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 60.0, 0.9, track.terrain.origin.y + 60.0), Quat::IDENTITY);
    c.assist_level = assist;
    let e = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
    if spec == 7 || std::env::var("TWEAK_ALL").is_ok() {
        if let Some(v) = e("AF") { c.spec.aero_front = v; }
        if let Some(v) = e("RGB") { c.spec.rear_grip_bias = v; }
        if let Some(v) = e("SA") { c.spec.assist = v; }
        if let Some(v) = e("LSD") { c.spec.lsd = v; }
    }
    for _ in 0..480 {
        c.step(DT, track);
    }
    c.vel = c.forward() * (kmh / 3.6);
    for w in c.wheels.iter_mut() {
        w.spin_speed = kmh / 3.6 / c.spec.wheel_radius;
    }
    let target_w = kmh / 3.6 / c.spec.wheel_radius;
    let n = c.spec.gears.len();
    c.gear = (1..=n as i32).find(|g| target_w * c.spec.gears[*g as usize - 1] * c.spec.final_drive * 60.0 / std::f32::consts::TAU < c.spec.redline * 0.85).unwrap_or(n as i32);
    c
}

/// keyboard ramp as in rally_game input.rs
fn kb(steer: &mut f32, target: f32) {
    let rate = if target == 0.0 || target.signum() != steer.signum() { 7.5 } else { 1.8 + 3.2 * steer.abs() };
    *steer += (target - *steer).clamp(-rate * DT, rate * DT);
}

/// pattern(t) -> keyboard target (-1, 0, 1). Returns (max yaw-rate after release (deg/s) in
/// the last 2 s, yaw sign changes after release, max drift deg, spun)
fn run(spec: usize, track: &Track, assist: f32, kmh: f32, pattern: &dyn Fn(f32) -> f32, release_t: f32) -> (f32, u32, f32, bool) {
    let mut c = fresh(spec, track, assist, kmh);
    let mut steer = 0.0f32;
    let (mut tail_yaw, mut flips, mut drift, mut spun) = (0.0f32, 0u32, 0.0f32, false);
    let mut last_sign = 0.0f32;
    let total = release_t + 5.0;
    let mut t = 0.0;
    while t < total {
        kb(&mut steer, pattern(t));
        let v = c.vel.length() * 3.6;
        let thr = if v < kmh { ((kmh - v) * 0.3).clamp(0.0, 1.0) } else { 0.0 };
        let brk = if v > kmh + 3.0 { 0.3 } else { 0.0 };
        c.input = CarInput { throttle: thr, brake: brk, steer, ..Default::default() };
        c.step(DT, track);
        t += DT;
        let yaw = c.ang_vel.dot(c.up()).to_degrees();
        drift = drift.max(c.telemetry.drift_angle.abs().to_degrees());
        if c.telemetry.drift_angle.abs() > 1.2 || c.up().y < 0.5 {
            spun = true;
        }
        if t > release_t + 0.3 {
            if yaw.abs() > 2.0 {
                let s = yaw.signum();
                if last_sign != 0.0 && s != last_sign {
                    flips += 1;
                }
                last_sign = s;
            }
            if t > total - 2.0 {
                tail_yaw = tail_yaw.max(yaw.abs());
            }
        }
    }
    (tail_yaw, flips, drift, spun)
}

fn main() {
    let assist: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let kmh_arg: Option<f32> = std::env::args().nth(2).and_then(|s| s.parse().ok());
    let track = Track::flat_for_tests(Surface::Asphalt);
    println!("=== WEAVE assist {assist}: tail yaw = max |yaw rate| in the last 2 s after release (deg/s); flips = left/right swings after release");
    for spec in [2usize, 4, 5, 6, 7] {
        let name = car_specs()[spec].name;
        let speeds: Vec<f32> = match kmh_arg {
            Some(k) => vec![k],
            None => if spec >= 5 { vec![150.0, 220.0, 280.0] } else { vec![100.0, 140.0] },
        };
        for kmh in speeds {
            let tap = run(spec, &track, assist, kmh, &|t| if t < 0.25 { 1.0 } else { 0.0 }, 0.25);
            let hold = run(spec, &track, assist, kmh, &|t| if t < 1.2 { 1.0 } else { 0.0 }, 1.2);
            let lane = run(spec, &track, assist, kmh, &|t| if t < 0.3 { -1.0 } else if t < 0.6 { 1.0 } else { 0.0 }, 0.6);
            println!(
                "{name:9} {kmh:3.0} km/h | tap: tail {:5.1} flips {:2} drift {:4.1} {} | hold 1.2s: tail {:5.1} flips {:2} drift {:4.1} {} | lane: tail {:5.1} flips {:2} drift {:4.1} {}",
                tap.0, tap.1, tap.2, if tap.3 { "SPUN" } else { "" },
                hold.0, hold.1, hold.2, if hold.3 { "SPUN" } else { "" },
                lane.0, lane.1, lane.2, if lane.3 { "SPUN" } else { "" },
            );
        }
    }
}
