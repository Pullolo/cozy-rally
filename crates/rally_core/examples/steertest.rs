//! Steering / braking-in-corner test on flat ground (compatible with old and new physics).
//! Holds a target speed with throttle/brake, applies a steering input, and measures the
//! steady lateral G, front tyre slip angle, front skid ("screech") and yaw rate.
//! Then: full brake + full steer from 80 km/h (ABS / steering authority).
//! cargo run --release -p rally_core --example steertest -- [assist]
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};

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
/// within 2.5 s after the stab ends).
fn throttle_in_corner(spec: usize, track: &Track, assist: f32, kmh: f32, steer: f32, driver: &str) -> (f32, f32, bool, bool) {
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
    (peak_beta.to_degrees(), peak_yaw / steady_yaw.max(0.05), spun, recovered && !spun)
}

fn main() {
    let assist: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    if std::env::args().nth(2).as_deref() == Some("tic") {
        println!("=== THROTTLE IN CORNER (assist {assist}): steady corner at 70 km/h near the limit (steer 0.9), then full throttle 2 s. peak drift deg / yaw gain / spin / recovered, for driver hold | catch | lift");
        for surf in [Surface::Gravel, Surface::Asphalt] {
            let track = Track::flat_for_tests(surf);
            for spec in 0..4 {
                let mut line = format!("{:?} {:8}", surf, car_specs()[spec].name);
                for d in ["hold", "catch", "lift"] {
                    let st: f32 = std::env::var("TIC_STEER").ok().and_then(|v| v.parse().ok()).unwrap_or(0.9);
                    let (b, g, sp, rec) = throttle_in_corner(spec, &track, assist, 70.0, st, d);
                    line += &format!(" | {d:5} {b:5.1}deg x{g:3.1}{}{}", if sp { " SPIN" } else { "" }, if rec { " rec" } else { " NOREC" });
                }
                println!("{line}");
            }
        }
        return;
    }
    println!("=== STEERING (assist {assist}): speed held, steady-state over 1.5-4 s. latG / front slip deg (peak ~8.6) / front skid (screech 0..1) / yaw rad/s");
    for surf in [Surface::Gravel, Surface::Asphalt] {
        let track = Track::flat_for_tests(surf);
        for (kmh, input) in [(60.0, 1.0), (60.0, 0.5), (90.0, 1.0), (90.0, 0.3)] {
            let mut line = format!("{:?} {kmh:3.0} km/h steer {input:.1}:", surf);
            for spec in 0..4 {
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
        for spec in 0..4 {
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
    }
}
