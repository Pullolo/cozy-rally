//! Physics characterisation on flat ground. cargo run -p rally_core --release --example dyno
//! Optional arg: "launch" to print only the launch table, "handling" for only handling.
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};

const DT: f32 = 1.0 / 240.0;

fn fresh(spec: usize, track: &Track, assist: f32) -> Car {
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 300.0, 0.7, track.terrain.origin.y + 300.0), Quat::IDENTITY);
    c.assist_level = assist;
    c.input = CarInput::default();
    for _ in 0..480 {
        c.step(DT, track);
    }
    c
}

fn rolling(spec: usize, track: &Track, assist: f32, kmh: f32, gear: i32) -> Car {
    let mut c = fresh(spec, track, assist);
    c.vel = c.forward() * (kmh / 3.6);
    for w in c.wheels.iter_mut() {
        w.spin_speed = kmh / 3.6 / c.spec.wheel_radius;
    }
    c.gear = gear;
    c
}

const ASSISTS: [(&str, f32); 4] = [("Cozy", 1.3), ("Normal", 1.0), ("Sport", 0.55), ("Sim", 0.0)];

fn launch_table() {
    println!("=== LAUNCH (full throttle from standstill): 0-60 / 0-100 km/h, wheelspin seconds (>0.3), peak slip ratio, tl = peak traction_loss (FX)");
    for surf in [Surface::Asphalt, Surface::Gravel, Surface::Snow, Surface::Ice] {
        let track = Track::flat_for_tests(surf);
        println!("--- {:?}", surf);
        for spec in 0..4 {
            let mut line = format!("{:8}", car_specs()[spec].name);
            for (aname, a) in [ASSISTS[3], ASSISTS[2], ASSISTS[1], ASSISTS[0]] {
                let mut c = fresh(spec, &track, a);
                c.input = CarInput { throttle: 1.0, ..Default::default() };
                let (mut t, mut t60, mut t100, mut spin, mut peak, mut rpm1, mut tl) = (0.0f32, None, None, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
                let _ = rpm1;
                let mut nan = false;
                while t < 25.0 && t100.is_none() {
                    c.step(DT, &track);
                    t += DT;
                    let v = c.vel.length() * 3.6;
                    if v >= 60.0 && t60.is_none() {
                        t60 = Some(t);
                    }
                    if v >= 100.0 && t100.is_none() {
                        t100 = Some(t);
                    }
                    if c.telemetry.wheelspin > 0.3 {
                        spin += DT;
                    }
                    for w in &c.wheels {
                        peak = peak.max(w.slip_ratio);
                    }
                    if c.gear == 1 {
                        rpm1 = rpm1.max(c.rpm);
                    }
                    for w in &c.wheels {
                        tl = tl.max(w.traction_loss);
                    }
                    nan |= !c.pos.is_finite();
                }
                let f = |x: Option<f32>| x.map(|v| format!("{v:4.1}")).unwrap_or(" -- ".into());
                line += &format!(" | {aname:6} {}/{} spin {:3.1}s slip {:4.2} tl {:3.1}{}", f(t60), f(t100), spin, peak, tl, if nan { " NaN!" } else { "" });
            }
            println!("{line}");
        }
    }
}

fn handling_table() {
    for surf in [Surface::Gravel, Surface::Asphalt, Surface::Snow] {
        let track = Track::flat_for_tests(surf);
        for (aname, assist) in [ASSISTS[1], ASSISTS[3]] {
            println!("=== {:?} / {aname} assists", surf);
            for spec in 0..4 {
                let name = car_specs()[spec].name;
                let mut c = fresh(spec, &track, assist);
                c.input = CarInput { throttle: 1.0, ..Default::default() };
                let mut t = 0.0;
                let mut vmax: f32 = 0.0;
                while t < 40.0 {
                    c.step(DT, &track);
                    t += DT;
                    vmax = vmax.max(c.vel.length() * 3.6);
                }
                // braking from 100
                let mut c = rolling(spec, &track, assist, 100.0, 4);
                c.input = CarInput { brake: 1.0, ..Default::default() };
                let p0 = c.pos;
                let mut tb = 0.0;
                let mut lock: f32 = 0.0;
                while c.vel.length() > 0.3 && tb < 10.0 {
                    c.step(DT, &track);
                    tb += DT;
                    lock = lock.max(c.telemetry.lockup);
                }
                let dist = c.pos.distance(p0);
                // step steer at 70 km/h, 3 s
                let mut res = String::new();
                for (thr, label) in [(0.3, "cruise"), (1.0, "fullthr")] {
                    let mut c = rolling(spec, &track, assist, 70.0, 2);
                    c.input = CarInput { throttle: thr, steer: 1.0, ..Default::default() };
                    let mut max_beta: f32 = 0.0;
                    let mut yaw = 0.0;
                    let mut lat_acc = 0.0;
                    for k in 0..(3.0 / DT) as i32 {
                        let v0 = c.vel;
                        c.step(DT, &track);
                        max_beta = max_beta.max(c.telemetry.drift_angle.abs());
                        if k > (1.5 / DT) as i32 {
                            yaw = c.ang_vel.y;
                            lat_acc = ((c.vel - v0) / DT).dot(c.right());
                        }
                    }
                    res += &format!(" {label}: beta {:4.1}deg yaw {:4.2} latG {:4.2} v {:3.0} |", max_beta.to_degrees(), yaw, lat_acc.abs() / 9.81, c.vel.length() * 3.6);
                }
                // power-on exit from a slow corner: 30 km/h in 1st, half lock, floor it for 2 s
                let mut c = rolling(spec, &track, assist, 30.0, 1);
                c.input = CarInput { throttle: 1.0, steer: 0.5, ..Default::default() };
                let mut exit_beta: f32 = 0.0;
                let mut exit_spin: f32 = 0.0;
                for _ in 0..(2.0 / DT) as i32 {
                    c.step(DT, &track);
                    exit_beta = exit_beta.max(c.telemetry.drift_angle.abs());
                    exit_spin = exit_spin.max(c.telemetry.wheelspin);
                }
                // handbrake turn at 60
                let mut c = rolling(spec, &track, assist, 60.0, 2);
                c.input = CarInput { steer: 1.0, handbrake: 1.0, ..Default::default() };
                let f0 = c.forward();
                for _ in 0..(1.2 / DT) as i32 {
                    c.step(DT, &track);
                }
                let rot = f0.angle_between(c.forward()).to_degrees();
                println!(
                    "{name:8} vmax {vmax:3.0} brake100 {dist:4.1}m lock {lock:3.1} |{res} exit: beta {:4.1} spin {:3.1} | hb-turn {rot:3.0}deg up {:.2}",
                    exit_beta.to_degrees(),
                    exit_spin,
                    c.up().y
                );
            }
        }
    }
}

/// Standstill behaviour: idle and brake-held on flat ground and on the steepest road slope.
/// Reports drift distance over 6 s and the max |angular velocity| of a wheel (jitter).
fn rest_table() {
    use rally_core::track::TrackParams;
    println!("=== REST (6 s, idle in gear / handbrake held): drift m / max wheel omega rad/s / max body speed m/s");
    let flat = Track::flat_for_tests(Surface::Gravel);
    let hill = Track::generate(&TrackParams { seed: 6, biome: None, length_class: 1, ..Default::default() }.with_env_overrides());
    // steepest sample
    let n = hill.samples.len();
    let (mut best, mut grade) = (0, 0.0f32);
    for i in 0..n {
        let g = (hill.samples[(i + 1) % n].pos.y - hill.samples[i].pos.y).abs() / 2.0;
        let near_jump = hill.jumps.iter().any(|&j| (j as isize - i as isize).abs() < 25);
        if g > grade && !near_jump {
            grade = g;
            best = i;
        }
    }
    for spec in 0..4 {
        let mut line = format!("{:8}", car_specs()[spec].name);
        for (label, on_hill, brake) in [("flat idle", false, 0.0), ("flat hb", false, 1.0), ("slope idle", true, 0.0), ("slope hb", true, 1.0)] {
            let track = if on_hill { &hill } else { &flat };
            let mut c = if on_hill {
                let smp = hill.samples[best];
                let rot = hill.rotation_at(smp.s);
                let p = smp.pos + smp.up * 0.9;
                Car::new(car_specs()[spec].clone(), p, rot)
            } else {
                fresh(spec, track, 1.0)
            };
            c.input = CarInput { handbrake: brake, ..Default::default() };
            for _ in 0..240 {
                c.step(DT, track);
            }
            let p0 = c.pos;
            let (mut wmax, mut vmax) = (0.0f32, 0.0f32);
            for _ in 0..(6.0 / DT) as i32 {
                c.step(DT, track);
                for w in &c.wheels {
                    wmax = wmax.max(w.spin_speed.abs());
                }
                vmax = vmax.max(c.vel.length());
            }
            line += &format!(" | {label} {:5.2}m w{:4.1} v{:4.2}{}", c.pos.distance(p0), wmax, vmax, if c.pos.is_finite() { "" } else { " NaN!" });
        }
        println!("{line}  (slope grade {:.0}%)", grade * 100.0);
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("rest") {
        rest_table();
        return;
    }
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg != "handling" {
        launch_table();
    }
    if arg != "launch" {
        handling_table();
        rest_table();
    }
}
