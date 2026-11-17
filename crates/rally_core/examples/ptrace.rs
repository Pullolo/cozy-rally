//! Physics trace of one manoeuvre: cargo run --release -p rally_core --example ptrace -- car surface assist throttle steer kmh gear
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let g = |i: usize, d: f32| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let spec = g(1, 1.0) as usize;
    let surf = match a.get(2).map(|s| s.as_str()).unwrap_or("gravel") { "asphalt" => Surface::Asphalt, "snow" => Surface::Snow, "ice" => Surface::Ice, _ => Surface::Gravel };
    let (assist, thr, steer, kmh, gear) = (g(3, 1.0), g(4, 1.0), g(5, 1.0), g(6, 70.0), g(7, 2.0) as i32);
    let track = Track::flat_for_tests(surf);
    let dt = 1.0 / 240.0;
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 300.0, 0.7, track.terrain.origin.y + 300.0), Quat::IDENTITY);
    c.assist_level = assist;
    for _ in 0..480 { c.step(dt, &track); }
    c.vel = c.forward() * kmh / 3.6;
    for w in c.wheels.iter_mut() { w.spin_speed = kmh / 3.6 / c.spec.wheel_radius; }
    c.gear = gear;
    c.input = CarInput { throttle: thr, steer, ..Default::default() };
    for k in 0..(3.0 / dt) as i32 {
        c.step(dt, &track);
        if k % 24 == 0 {
            let w: Vec<String> = c.wheels.iter().map(|w| format!("k{:5.2} a{:5.2} tc{:.2} L{:4.0}", w.slip_ratio, w.slip_angle, w.tc_cut, w.load)).collect();
            println!("t{:4.2} v{:5.1} beta{:6.1} yaw{:5.2} g{} rpm{:5.0} steer{:5.2} spin{:.2} | {}", k as f32 * dt, c.vel.length() * 3.6, c.telemetry.drift_angle.to_degrees(), c.ang_vel.y, c.gear, c.rpm, c.steer, c.telemetry.wheelspin, w.join(" | "));
        }
    }
}
