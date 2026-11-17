//! Physics characterisation on flat ground. cargo run -p rally_core --release --example dyno
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};
use rally_core::glam::{Quat, Vec3};

const DT: f32 = 1.0 / 240.0;

fn fresh(spec: usize, track: &Track) -> Car {
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 300.0, 0.7, track.terrain.origin.y + 300.0), Quat::IDENTITY);
    c.input = CarInput::default();
    for _ in 0..480 { c.step(DT, track); }
    c
}

fn main() {
    for surf in [Surface::Gravel, Surface::Asphalt] {
        let track = Track::flat_for_tests(surf);
        println!("=== {:?}", surf);
        for spec in 0..4 {
            let name = car_specs()[spec].name;
            // acceleration
            let mut c = fresh(spec, &track);
            c.input = CarInput { throttle: 1.0, ..Default::default() };
            let mut t = 0.0; let mut t100 = None; let mut vmax: f32 = 0.0; let mut t60=None;
            while t < 40.0 { c.step(DT, &track); t += DT; let v = c.vel.length()*3.6; vmax = vmax.max(v); if v >= 60.0 && t60.is_none() { t60 = Some(t);} if v >= 100.0 && t100.is_none() { t100 = Some(t); } }
            // braking from 100
            let mut c = fresh(spec, &track);
            c.vel = c.forward() * (100.0/3.6);
            for w in c.wheels.iter_mut() { w.spin_speed = 100.0/3.6/c.spec.wheel_radius; }
            c.input = CarInput { brake: 1.0, ..Default::default() };
            let p0 = c.pos; let mut tb = 0.0;
            while c.vel.length() > 0.3 && tb < 10.0 { c.step(DT, &track); tb += DT; }
            let dist = c.pos.distance(p0);
            // step steer at 70 km/h with partial throttle, 3 s
            let mut res = String::new();
            for (thr, label) in [(0.3, "cruise"), (1.0, "fullthr")] {
                let mut c = fresh(spec, &track);
                c.vel = c.forward() * (70.0/3.6);
                for w in c.wheels.iter_mut() { w.spin_speed = 70.0/3.6/c.spec.wheel_radius; }
                c.gear = 2;
                c.input = CarInput { throttle: thr, steer: 1.0, ..Default::default() };
                let mut max_beta: f32 = 0.0; let mut yaw = 0.0; let mut lat_acc = 0.0;
                for k in 0..(3.0/DT) as i32 { let v0 = c.vel; c.step(DT, &track); max_beta = max_beta.max(c.telemetry.drift_angle.abs()); if k > (1.5/DT) as i32 { yaw = c.ang_vel.y; lat_acc = ((c.vel - v0)/DT).dot(c.right()); } }
                res += &format!(" {label}: beta {:4.1}deg yaw {:4.2} latG {:4.2} v {:3.0} |", max_beta.to_degrees(), yaw, lat_acc.abs()/9.81, c.vel.length()*3.6);
            }
            // handbrake turn at 60
            let mut c = fresh(spec, &track);
            c.vel = c.forward() * (60.0/3.6); c.gear = 2;
            for w in c.wheels.iter_mut() { w.spin_speed = 60.0/3.6/c.spec.wheel_radius; }
            c.input = CarInput { steer: 1.0, handbrake: 1.0, ..Default::default() };
            let f0 = c.forward();
            for _ in 0..(1.2/DT) as i32 { c.step(DT, &track); }
            let rot = f0.angle_between(c.forward()).to_degrees();
            println!("{name:8} 0-60 {:4.1}s 0-100 {:>5} vmax {:3.0} brake100 {:4.1}m |{res} hb-turn {rot:3.0}deg up {:.2}",
                t60.unwrap_or(99.0), t100.map(|t| format!("{t:.1}s")).unwrap_or("-".into()), vmax, dist, c.up().y);
        }
    }
}
