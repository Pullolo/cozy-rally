//! Jump landing stability test. For every jump on a few tracks, an autopilot drives the car
//! over the ramp; in the air one of several scenarios is applied; after landing we measure
//! the peak yaw rate and slip angle and whether the car spun (|beta| > 60 deg).
//!   straight : no steering from take-off until 0.8 s after landing (throttle from autopilot)
//!   air-steer: full right steer held from take-off until 0.4 s after landing (keyboard habit)
//!   crossed  : car yawed 15 deg at take-off (crossed-up), steer released until 0.3 s after landing
//!   cross+st : crossed-up 15 deg and steering held the wrong way through the landing
//! cargo run --release -p rally_core --example jumptest -- [assist]
use rally_core::ai::AiDriver;
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::track::{Track, TrackParams, SAMPLE_SPACING};
use rally_core::vehicle::{car_specs, Car};

const DT: f32 = 1.0 / 240.0;

struct Res {
    yaw_peak: f32,
    beta_peak: f32,
    spun: bool,
    air: f32,
    land_v: f32,
}

fn run(track: &Track, jump: usize, spec: usize, assist: f32, scenario: &str) -> Option<Res> {
    let s0 = track.samples[jump].s - 90.0;
    let (p, t, _, up) = track.frame_at(s0);
    let mut car = Car::new(car_specs()[spec].clone(), p + up * 0.9, track.rotation_at(s0));
    car.assist_level = assist;
    car.vel = t * 26.0;
    for w in car.wheels.iter_mut() {
        w.spin_speed = 26.0 / car.spec.wheel_radius;
    }
    car.gear = 3;
    let mut ai = AiDriver::new(0.9, 0.0);
    let mut airborne = false;
    let mut landed_at: Option<f32> = None;
    let mut t_now = 0.0;
    let mut res = Res { yaw_peak: 0.0, beta_peak: 0.0, spun: false, air: 0.0, land_v: 0.0 };
    let mut ai_input = CarInput::default();
    let mut frame = 0;
    while t_now < 12.0 {
        if frame % 2 == 0 {
            ai_input = ai.drive(&car, track, DT * 2.0);
        }
        frame += 1;
        let mut input = ai_input;
        let on = car.telemetry.wheels_on_ground;
        if !airborne && on == 0 && car.telemetry.airtime > 0.05 {
            airborne = true;
            if scenario.starts_with("cross") {
                car.rot = (Quat::from_rotation_y(-15f32.to_radians()) * car.rot).normalize();
            }
        }
        if airborne && landed_at.is_none() && on >= 3 {
            landed_at = Some(t_now);
            res.air = car.telemetry.airtime.max(res.air);
            res.land_v = car.vel.length();
        }
        if airborne {
            let since = landed_at.map(|l| t_now - l).unwrap_or(-1.0);
            // hands off the wheel around the landing so we measure the car, not the driver
            if since < 0.8 {
                input.steer = 0.0;
            }
            match scenario {
                "air-steer" if since < 0.4 => input.steer = 1.0,
                "crossed" if since < 0.3 => input.steer = 0.0,
                "cross+st" if since < 0.5 => input.steer = 1.0,
                _ => {}
            }
            if landed_at.is_none() {
                res.air += DT;
            }
        }
        car.input = input;
        car.step(DT, track);
        if std::env::var("JT_TRACE").is_ok() && airborne && frame % 6 == 0 && landed_at.is_none_or(|l| t_now - l < 1.0) {
            let w: Vec<String> = car.wheels.iter().map(|w| format!("{}{:4.0} a{:5.2}", if w.contact { "c" } else { "-" }, w.load, w.slip_angle)).collect();
            let q = track.query(car.pos, car.track_hint);
            println!("t{:5.2} on{} yaw{:5.2} roll/pitch({:5.2},{:5.2}) beta{:5.1} steer_in{:5.2} wheel{:5.2} v{:4.1} lat{:5.2} | {}", t_now, car.telemetry.wheels_on_ground, car.ang_vel.dot(car.up()), car.ang_vel.dot(car.forward()), car.ang_vel.dot(car.right()), car.telemetry.drift_angle.to_degrees(), car.input.steer, car.steer, car.vel.length(), q.lateral, w.join(" "));
        }
        t_now += DT;
        if let Some(l) = landed_at {
            if t_now - l < 2.0 {
                if t_now - l < 0.8 {
                    // yaw beyond what the road asks for (landing-induced rotation)
                    let q = track.query(car.pos, car.track_hint);
                    let path_yaw = car.vel.length() * track.samples[q.index].curvature;
                    res.yaw_peak = res.yaw_peak.max((car.ang_vel.y - path_yaw).abs());
                }
                res.beta_peak = res.beta_peak.max(car.telemetry.drift_angle.abs());
                if car.telemetry.drift_angle.abs() > 60f32.to_radians() || car.up().y < 0.3 {
                    res.spun = true;
                }
            } else {
                break;
            }
        }
        if !car.pos.is_finite() {
            return None;
        }
    }
    landed_at.map(|_| res)
}

fn main() {
    let assist: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    if let Ok(sc) = std::env::var("JT_TRACE") {
        let track = Track::generate(&TrackParams { seed: 1, biome: None, length_class: 1, ..Default::default() }.with_env_overrides());
        run(&track, track.jumps[0], 2, assist, &sc);
        return;
    }
    println!("=== JUMP LANDINGS, assist {assist}: peak excess yaw rate (rad/s, first 0.8 s) / peak slip angle (deg, 2 s) after landing, spins");
    for scenario in ["straight", "air-steer", "crossed", "cross+st"] {
        let (mut n, mut spins, mut yaw, mut beta, mut yaw_max) = (0, 0, 0.0, 0.0, 0.0f32);
        for seed in 1..=10u64 {
            let track = Track::generate(&TrackParams { seed, biome: None, length_class: 1, ..Default::default() }.with_env_overrides());
            for &j in &track.jumps {
                for spec in 0..4 {
                    if let Some(r) = run(&track, j, spec, assist, scenario) {
                        if std::env::var("JT_LIST").is_ok() && r.yaw_peak > 0.7 {
                            println!("   seed {seed} jump {j} car {spec}: yaw {:.2} slip {:.1} air {:.2}s v {:.1}", r.yaw_peak, r.beta_peak.to_degrees(), r.air, r.land_v);
                        }
                        n += 1;
                        spins += r.spun as u32;
                        yaw += r.yaw_peak;
                        yaw_max = yaw_max.max(r.yaw_peak);
                        beta += r.beta_peak.to_degrees();
                        let _ = (r.air, r.land_v, SAMPLE_SPACING);
                    }
                }
            }
        }
        let nf = n.max(1) as f32;
        println!("{scenario:10} landings {n:3} | avg peak yaw {:4.2} max {:4.2} | avg peak slip {:5.1} deg | spins {spins}", yaw / nf, yaw_max, beta / nf);
    }
    let _ = Vec3::ZERO;
}
