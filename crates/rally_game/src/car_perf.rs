//! Honest car stats for the car picker: measured with the real vehicle sim on a flat test
//! track (the same procedure as rally_core's `examples/circuit.rs`), once per car, on a
//! background thread at startup. The UI shows estimates until the numbers are ready.
use rally_core::glam::{Quat, Vec3};
use rally_core::input::CarInput;
use rally_core::surface::Surface;
use rally_core::track::Track;
use rally_core::vehicle::{car_specs, Car};
use std::sync::Mutex;

const DT: f32 = 1.0 / 240.0;

#[derive(Clone, Copy, Debug)]
pub struct Perf {
    /// Flat-out top speed on tarmac (km/h)
    pub vmax_kmh: f32,
    /// Steady-state cornering at 150 km/h on tarmac (g)
    pub lat_g: f32,
}

static PERF: Mutex<Option<Vec<Perf>>> = Mutex::new(None);

/// Kick off the measurements (call once at startup)
pub fn start() {
    std::thread::spawn(|| {
        let t0 = std::time::Instant::now();
        let v = measure_all();
        if std::env::var("COZY_PERF_LOG").is_ok() {
            for (spec, p) in car_specs().iter().zip(&v) {
                println!("perf {:10} vmax {:5.1} km/h  lat150 {:4.2} g", spec.name, p.vmax_kmh, p.lat_g);
            }
            println!("perf measured in {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0);
        }
        if let Ok(mut g) = PERF.lock() {
            *g = Some(v);
        }
    });
}

/// Measured numbers for a car (None while still measuring)
pub fn get(car: usize) -> Option<Perf> {
    PERF.lock().ok().and_then(|g| g.as_ref().and_then(|v| v.get(car).copied()))
}

fn fresh(spec: usize, track: &Track, kmh: f32) -> Car {
    let mut c = Car::new(car_specs()[spec].clone(), Vec3::new(track.terrain.origin.x + 60.0, 0.9, track.terrain.origin.y + 60.0), Quat::IDENTITY);
    for _ in 0..240 {
        c.step(DT, track);
    }
    if kmh > 0.0 {
        c.vel = c.forward() * (kmh / 3.6);
        let w = kmh / 3.6 / c.spec.wheel_radius;
        for wh in c.wheels.iter_mut() {
            wh.spin_speed = w;
        }
        let n = c.spec.gears.len();
        c.gear = (1..=n as i32).find(|g| w * c.spec.gears[*g as usize - 1] * c.spec.final_drive * 60.0 / std::f32::consts::TAU < c.spec.redline * 0.85).unwrap_or(n as i32);
    }
    c
}

/// Flat-out run: start rolling at 60 km/h (saves time), stop once speed has settled
fn top_speed(spec: usize, track: &Track) -> f32 {
    let mut c = fresh(spec, track, 60.0);
    c.input = CarInput { throttle: 1.0, ..Default::default() };
    let (mut t, mut vmax, mut last_check, mut v_at_check) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    while t < 90.0 {
        c.step(DT, track);
        t += DT;
        let v = c.vel.length() * 3.6;
        vmax = vmax.max(v);
        if t - last_check >= 3.0 {
            if v - v_at_check < 0.3 {
                break;
            }
            last_check = t;
            v_at_check = v;
        }
    }
    vmax
}

/// Steady cornering at `kmh` with full steering, speed held: mean lateral g
fn lateral(spec: usize, track: &Track, kmh: f32) -> f32 {
    let mut c = fresh(spec, track, kmh);
    let (mut lat, mut n) = (0.0f32, 0.0f32);
    for k in 0..(3.0 / DT) as i32 {
        let err = kmh / 3.6 - c.vel.length();
        c.input = CarInput { throttle: (err * 0.6).clamp(0.0, 1.0), brake: (-err * 0.3).clamp(0.0, 1.0), steer: 1.0, ..Default::default() };
        let v0 = c.vel;
        c.step(DT, track);
        if k as f32 * DT > 1.8 {
            lat += ((c.vel - v0) / DT).dot(c.right()).abs() / 9.81;
            n += 1.0;
        }
    }
    lat / n.max(1.0)
}

fn measure_all() -> Vec<Perf> {
    let tarmac = Track::flat_for_tests(Surface::Asphalt);
    (0..car_specs().len()).map(|i| Perf { vmax_kmh: top_speed(i, &tarmac), lat_g: lateral(i, &tarmac, 150.0) }).collect()
}
