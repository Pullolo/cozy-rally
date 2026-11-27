//! Steering / braking-in-corner test on flat ground (compatible with old and new physics).
//! Holds a target speed with throttle/brake, applies a steering input, and measures the
//! steady lateral G, front tyre slip angle, front skid ("screech") and yaw rate.
//! Then: full brake + full steer from 80 km/h (ABS / steering authority).
//! cargo run --release -p rally_core --example steertest -- [assist]
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, steer_limit_factor, Car};

const DT: f32 = 1.0 / 240.0;

fn fresh(spec: usize, track: &Track, assist: f32, kmh: f32) -> Car {
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 300.0, 0.7, track.terrain.origin.y + 300.0), Quat::IDENTITY);
    c.assist_level = assist;
    for _ in 0..480 {
        c.step(DT, track);
    }
    c.vel = c.forward() * (kmh / 3.6);
    for w in c.wheels.iter_mut() {
        w.spin_speed = kmh / 3.6 / c.spec.wheel_radius;
    }
    c.gear = if kmh > 70.0 { 3 } else { 2 };
    c
}

/// front axle slip angle (rad) from body state and the actual road-wheel angle
fn front_slip(c: &Car) -> f32 {
    let v_f = c.vel + c.ang_vel.cross(c.forward() * c.spec.cg_to_front);
    let theta = v_f.dot(c.right()).atan2(v_f.dot(c.forward()).max(0.5));
    theta - c.steer
}

/// Throttle-in-corner: settle in a steady corner at `kmh` with `steer` input (speed held),
/// then full throttle for 2 s. Driver after the throttle stab:
///   "hold"  : keeps the steering where it was
///   "catch" : counter-steers once the slide passes 6 deg (proportional), keeps the throttle
///   "lift"  : lifts off once the slide passes 12 deg
/// Returns (peak drift deg, yaw gain = peak yaw / steady yaw, spun, recovered: drift < 5 deg
/// within 2.5 s after the stab ends, time from the stab to 10 deg of drift if reached).
fn throttle_in_corner(spec: usize, track: &Track, assist: f32, kmh: f32, steer: f32, driver: &str) -> (f32, f32, bool, bool, Option<f32>) {
    let mut c = fresh(spec, track, assist, kmh);
    let mut steady_yaw = 0.0f32;
    for k in 0..(3.0 / DT) as i32 {
        let err = kmh / 3.6 - c.vel.length();
        c.input = CarInput { throttle: (err * 0.5).clamp(0.0, 1.0), brake: (-err * 0.3).clamp(0.0, 1.0), steer, ..Default::default() };
        c.step(DT, track);
        if k as f32 * DT > 2.5 {
            steady_yaw = steady_yaw.max(c.ang_vel.y.abs());
        }
    }
    let (mut peak_beta, mut peak_yaw, mut spun) = (0.0f32, 0.0f32, false);
    let mut lifted = false;
    let mut t10 = None;
    let mut t = 0.0f32;
    for _ in 0..(2.0 / DT) as i32 {
        let beta = c.telemetry.drift_angle;
        let mut inp = CarInput { throttle: 1.0, steer, ..Default::default() };
        match driver {
            "catch" if beta.abs() > 6f32.to_radians() => {
                // steer toward the direction of travel (velocity is on the side beta points to)
                inp.steer = (beta.signum() * ((beta.abs().to_degrees() - 6.0) / 14.0)).clamp(-1.0, 1.0);
            }
            "lift" if beta.abs() > 12f32.to_radians() || lifted => {
                lifted = true;
                inp.throttle = 0.0;
            }
            _ => {}
        }
        c.input = inp;
        c.step(DT, track);
        if std::env::var("TIC_TRACE").is_ok_and(|v| v == c.spec.name) && driver == "hold" {
            let w = &c.wheels;
            println!("  g{} rpm{:5.0} v{:4.1} beta{:5.1} yaw{:5.2} | rear k{:5.2}/{:5.2} a{:5.2}/{:5.2} tc{:.2}/{:.2} tl{:.2}/{:.2} | front k{:5.2} a{:5.2} tc{:.2}", c.gear, c.rpm, c.vel.length() * 3.6, c.telemetry.drift_angle.to_degrees(), c.ang_vel.y, w[2].slip_ratio, w[3].slip_ratio, w[2].slip_angle, w[3].slip_angle, w[2].tc_cut, w[3].tc_cut, w[2].traction_loss, w[3].traction_loss, w[0].slip_ratio, w[0].slip_angle, w[0].tc_cut);
        }
        t += DT;
        if t10.is_none() && c.telemetry.drift_angle.abs() > 10f32.to_radians() {
            t10 = Some(t);
        }
        peak_beta = peak_beta.max(c.telemetry.drift_angle.abs());
        peak_yaw = peak_yaw.max(c.ang_vel.y.abs());
        spun |= c.telemetry.drift_angle.abs() > 70f32.to_radians();
    }
    // stab over: neutral throttle, centre steering (or keep catching)
    let mut recovered = false;
    for _ in 0..(2.5 / DT) as i32 {
        let beta = c.telemetry.drift_angle;
        let st = if driver == "hold" { steer * 0.5 } else { (beta.signum() * (beta.abs().to_degrees() / 15.0)).clamp(-1.0, 1.0) };
        c.input = CarInput { throttle: 0.3, steer: st, ..Default::default() };
        c.step(DT, track);
        spun |= c.telemetry.drift_angle.abs() > 70f32.to_radians();
        if c.telemetry.drift_angle.abs() < 5f32.to_radians() {
            recovered = true;
        }
    }
    (peak_beta.to_degrees(), peak_yaw / steady_yaw.max(0.05), spun, recovered && !spun, t10)
}

fn main() {
    let assist: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    if std::env::args().nth(2).as_deref() == Some("tic") {
        // TIC_KMH / TIC_STEER: corner speed and steering (e.g. 45 / 1.0 for a slow-corner exit)
        let kmh: f32 = std::env::var("TIC_KMH").ok().and_then(|v| v.parse().ok()).unwrap_or(70.0);
        let st: f32 = std::env::var("TIC_STEER").ok().and_then(|v| v.parse().ok()).unwrap_or(0.9);
        println!("=== THROTTLE IN CORNER (assist {assist}): steady corner at {kmh} km/h near the limit (steer {st}), then full throttle 2 s. peak drift deg / yaw gain / spin / recovered, for driver hold | catch | lift");
        for surf in [Surface::Gravel, Surface::Asphalt] {
            let mut track = Track::flat_for_tests(surf);
            if std::env::var("WET").is_ok() {
                track.conditions = rally_core::weather::StageConditions::uniform(rally_core::weather::WeatherPreset::Storm.weather());
            }
            for spec in 0..car_specs().len() {
                let mut line = format!("{:?} {:8}", surf, car_specs()[spec].name);
                for d in ["hold", "catch", "lift"] {
                    let (b, g, sp, rec, t10) = throttle_in_corner(spec, &track, assist, kmh, st, d);
                    let t10 = t10.map_or("    -".to_string(), |t| format!("{t:4.2}s"));
                    line += &format!(" | {d:5} {b:5.1}deg x{g:3.1} t10 {t10}{}{}", if sp { " SPIN" } else { "" }, if rec { " rec" } else { " NOREC" });
                }
                println!("{line}");
            }
        }
        return;
    }
    println!("=== STEERING (assist {assist}): speed held, steady-state over 1.5-4 s. latG / front slip deg (peak ~8.6) / front skid (screech 0..1) / yaw rad/s");
    for surf in [Surface::Gravel, Surface::Asphalt] {
        let mut track = Track::flat_for_tests(surf);
        if std::env::var("WET").is_ok() {
            track.conditions = rally_core::weather::StageConditions::uniform(rally_core::weather::WeatherPreset::Storm.weather());
        }
        for (kmh, input) in [(60.0, 1.0), (60.0, 0.5), (90.0, 1.0), (90.0, 0.3)] {
            let mut line = format!("{:?} {kmh:3.0} km/h steer {input:.1}:", surf);
            for spec in 0..car_specs().len() {
                let mut c = fresh(spec, &track, assist, kmh);
                let (mut lat, mut slip, mut skid, mut yaw, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
                let mut spun = false;
                for k in 0..(4.0 / DT) as i32 {
                    let err = kmh / 3.6 - c.vel.length();
                    c.input = CarInput { throttle: (err * 0.5).clamp(0.0, 1.0), brake: (-err * 0.3).clamp(0.0, 1.0), steer: input, ..Default::default() };
                    let v0 = c.vel;
                    c.step(DT, &track);
                    if k as f32 * DT > 1.5 {
                        lat += ((c.vel - v0) / DT).dot(c.right()).abs() / 9.81;
                        slip += front_slip(&c).abs().to_degrees();
                        skid = skid.max(c.wheels[0].skid.max(c.wheels[1].skid));
                        yaw += c.ang_vel.y.abs();
                        n += 1.0;
                    }
                    spun |= c.telemetry.drift_angle.abs() > 1.0;
                }
                line += &format!(" | {:7} {:4.2}g {:4.1}deg skid {:3.1} yaw {:4.2}{}", c.spec.name, lat / n, slip / n, skid, yaw / n, if spun { " SPIN" } else { "" });
            }
            println!("{line}");
        }
        // braking in a corner: full brake + full steer from 80 km/h for 1.2 s
        let mut line = format!("{:?} brake+steer 80:", surf);
        for spec in 0..car_specs().len() {
            let mut c = fresh(spec, &track, assist, 80.0);
            let f0 = c.forward();
            let p0 = c.pos;
            let mut lock: f32 = 0.0;
            for _ in 0..(1.2 / DT) as i32 {
                c.input = CarInput { brake: 1.0, steer: 1.0, ..Default::default() };
                c.step(DT, &track);
                for w in &c.wheels {
                    if w.contact && w.spin_speed.abs() < 0.5 && c.vel.length() > 3.0 {
                        lock = lock.max(1.0);
                    }
                }
            }
            let turned = f0.angle_between(c.forward()).to_degrees();
            let lateral = (c.pos - p0).dot(f0.cross(Vec3::Y).normalize() * -1.0).abs();
            line += &format!(" | {:7} turned {turned:4.1}deg lateral {lateral:4.1}m v {:3.0} locked {}", c.spec.name, c.vel.length() * 3.6, lock > 0.0);
        }
        println!("{line}");
        // chicane: full steer one way for 1 s, then full the other way. Right after the switch
        // the front axle still travels toward the first corner, so this is where an unlimited
        // input would scrub the fronts past their peak slip angle.
        // "pushed" = the largest angle the steering input turned the wheels past the limiter's
        // no-slip window (peak slip x limiter factor beyond the front axle's travel direction)
        // while steering normally, i.e. not while the front axle already slides past the peak:
        // ~0 when the limiter holds (Sim has no limiter: n/a). "slip" = largest front slip after
        // the switch, which also counts a slide the released / limited wheels don't correct.
        for kmh in [60.0, 90.0] {
            let mut line = format!("{:?} chicane {kmh:3.0}:", surf);
            for spec in 0..car_specs().len() {
                let mut c = fresh(spec, &track, assist, kmh);
                let span = steer_limit_factor(assist).map(|k| k * c.spec.peak_slip_angle);
                let (mut worst, mut pushed, mut skid) = (0.0f32, 0.0f32, 0.0f32);
                let mut spun = false;
                for k in 0..(2.2 / DT) as i32 {
                    let t = k as f32 * DT;
                    let err = kmh / 3.6 - c.vel.length();
                    let steer: f32 = if t < 1.0 { 1.0 } else { -1.0 };
                    c.input = CarInput { throttle: (err * 0.5).clamp(0.0, 1.0), brake: (-err * 0.3).clamp(0.0, 1.0), steer, ..Default::default() };
                    c.step(DT, &track);
                    if t >= 1.0 {
                        worst = worst.max(front_slip(&c).abs());
                        if let Some(span) = span {
                            let theta = c.front_travel_angle();
                            // front axle already sliding past the peak with straight wheels:
                            // the input may turn them freely there (not normal steering)
                            let sliding = theta * steer.signum() < 0.0 && theta.abs() > span;
                            if !sliding {
                                let past = ((c.steer - theta) * steer.signum() - span).max(0.0);
                                pushed = pushed.max(past.min((c.steer * steer.signum()).max(0.0)));
                            }
                        }
                        skid = skid.max(c.wheels[0].skid.max(c.wheels[1].skid));
                    }
                    spun |= c.telemetry.drift_angle.abs() > 1.0;
                }
                let pushed = if span.is_some() { format!("{:4.1}", pushed.to_degrees()) } else { " n/a".into() };
                line += &format!(" | {:7} pushed {pushed} slip {:4.1}deg skid {:3.1}{}", c.spec.name, worst.to_degrees(), skid, if spun { " SPIN" } else { "" });
            }
            println!("{line}");
        }
    }
}
