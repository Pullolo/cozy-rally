//! Circuit-class characterisation (plus Thistle as the rally reference).
//! cargo run --release -p rally_core --example circuit [-- assist]
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};
use rally_core::weather::{StageConditions, WeatherPreset};

const DT: f32 = 1.0 / 240.0;

fn fresh(spec: usize, track: &Track, assist: f32, kmh: f32) -> Car {
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 60.0, 0.9, track.terrain.origin.y + 60.0), Quat::IDENTITY);
    c.assist_level = assist;
    for _ in 0..480 {
        c.step(DT, track);
    }
    if kmh > 0.0 {
        c.vel = c.forward() * (kmh / 3.6);
        for w in c.wheels.iter_mut() {
            w.spin_speed = kmh / 3.6 / c.spec.wheel_radius;
        }
        // a sensible gear for the speed
        let target_w = kmh / 3.6 / c.spec.wheel_radius;
        let n = c.spec.gears.len();
        c.gear = (1..=n as i32).find(|g| target_w * c.spec.gears[*g as usize - 1] * c.spec.final_drive * 60.0 / std::f32::consts::TAU < c.spec.redline * 0.85).unwrap_or(n as i32);
    }
    c
}

/// steady cornering at `kmh` with full steering (speed held): mean lateral g, front slip deg
fn lateral(spec: usize, track: &Track, assist: f32, kmh: f32) -> (f32, bool) {
    let mut c = fresh(spec, track, assist, kmh);
    let (mut lat, mut n, mut bad) = (0.0, 0.0, false);
    for k in 0..(3.0 / DT) as i32 {
        let err = kmh / 3.6 - c.vel.length();
        c.input = CarInput { throttle: (err * 0.6).clamp(0.0, 1.0), brake: (-err * 0.3).clamp(0.0, 1.0), steer: 1.0, ..Default::default() };
        let v0 = c.vel;
        c.step(DT, track);
        if k as f32 * DT > 1.8 {
            lat += ((c.vel - v0) / DT).dot(c.right()).abs() / 9.81;
            n += 1.0;
        }
        bad |= !c.pos.is_finite() || c.up().y < 0.5 || c.telemetry.drift_angle.abs() > 1.0;
    }
    (lat / n, bad)
}

fn main() {
    let assist: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let tarmac = Track::flat_for_tests(Surface::Asphalt);
    let gravel = Track::flat_for_tests(Surface::Gravel);
    let mut wet = Track::flat_for_tests(Surface::Asphalt);
    wet.conditions = StageConditions::uniform(WeatherPreset::Storm.weather());
    // kerb strip: +-6 cm ripples every terrain cell on a 150 m band
    let mut kerbs = Track::flat_for_tests(Surface::Asphalt);
    {
        let hf = &mut kerbs.terrain;
        for iz in 0..hf.nz {
            for ix in 0..hf.nx {
                let x = ix as f32 * hf.cell;
                if (100.0..250.0).contains(&x) {
                    hf.heights[iz * hf.nx + ix] = if (ix + iz) % 2 == 0 { 0.06 } else { -0.03 };
                }
            }
        }
    }
    println!("=== CIRCUIT (assist {assist}) on flat tarmac unless noted");
    for spec in [2usize, 5, 6, 7] {
        let name = car_specs()[spec].name;
        // acceleration / top speed
        let mut c = fresh(spec, &tarmac, assist, 0.0);
        c.input = CarInput { throttle: 1.0, ..Default::default() };
        let (mut t, mut t100, mut t200, mut vmax, mut rpm_max) = (0.0f32, None, None, 0.0f32, 0.0f32);
        while t < 70.0 {
            c.step(DT, &tarmac);
            t += DT;
            let v = c.vel.length() * 3.6;
            vmax = vmax.max(v);
            rpm_max = rpm_max.max(c.rpm);
            if v >= 100.0 && t100.is_none() {
                t100 = Some(t);
            }
            if v >= 200.0 && t200.is_none() {
                t200 = Some(t);
            }
        }
        // braking from 200
        let mut c = fresh(spec, &tarmac, assist, 200.0);
        c.input = CarInput { brake: 1.0, ..Default::default() };
        let p0 = c.pos;
        let mut tb = 0.0;
        while c.vel.length() > 0.5 && tb < 15.0 {
            c.step(DT, &tarmac);
            tb += DT;
        }
        let d200 = c.pos.distance(p0);
        let f = |x: Option<f32>| x.map(|v| format!("{v:4.1}s")).unwrap_or(" -- ".into());
        print!("{name:9} 0-100 {} 0-200 {} vmax {vmax:3.0} km/h (rpm {rpm_max:5.0}) brake200 {d200:5.1}m | lateral g:", f(t100), f(t200));
        for kmh in [100.0, 200.0, 280.0] {
            let (g, bad) = lateral(spec, &tarmac, assist, kmh);
            print!(" {kmh:3.0}:{g:4.2}{}", if bad { "!" } else { "" });
        }
        let (gg, _) = lateral(spec, &gravel, assist, 100.0);
        let (gw, _) = lateral(spec, &wet, assist, 100.0);
        let (gw2, _) = lateral(spec, &wet, assist, 200.0);
        print!(" | gravel100 {gg:4.2} wet100 {gw:4.2} wet200 {gw2:4.2}");
        // kerbs + floor scrape: flat out across the ripple band
        let mut c = fresh(spec, &kerbs, assist, 250.0f32.min(vmax - 10.0));
        c.pos.x = kerbs.terrain.origin.x + 60.0;
        c.rot = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2); // face +X
        c.vel = c.forward() * c.vel.length();
        let (mut scrape, mut max_pitch, mut bad) = (0.0f32, 0.0f32, false);
        for _ in 0..(3.0 / DT) as i32 {
            c.input = CarInput { throttle: 1.0, ..Default::default() };
            c.step(DT, &kerbs);
            scrape = scrape.max(c.telemetry.floor_scrape);
            max_pitch = max_pitch.max(c.ang_vel.dot(c.right()).abs());
            bad |= !c.pos.is_finite() || c.up().y < 0.7;
        }
        // flat-out floor scrape on smooth tarmac at top speed (downforce squat)
        let mut c = fresh(spec, &tarmac, assist, vmax - 5.0);
        let mut scrape_flat = 0.0f32;
        for _ in 0..(2.0 / DT) as i32 {
            c.input = CarInput { throttle: 1.0, ..Default::default() };
            c.step(DT, &tarmac);
            scrape_flat = scrape_flat.max(c.telemetry.floor_scrape);
        }
        println!(" | kerbs: pitch {max_pitch:4.2} scrape {scrape:3.1}{} flat-out scrape {scrape_flat:3.1}", if bad { " UNSTABLE" } else { "" });
    }
}
