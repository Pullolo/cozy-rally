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

fn main() {
    let assist: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
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
