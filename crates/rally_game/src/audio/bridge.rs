//! Game -> synth translation: picks which cars get an engine voice, computes distance
//! attenuation / pan / doppler relative to the local cameras, turns wheel state into
//! surface mix + traction-loss amounts, and maps world events to one-shots.
//! Depends only on rally_core so the offline renderer can drive it with a real simulation.
use super::synth::{Cmd, EngineParams, SfxKind, MAX_CARS};
use rally_core::glam::Vec3;
use rally_core::race::RaceEvent;
use rally_core::surface::Surface;
use rally_core::vehicle::{Car, WheelState};
use rally_core::world::{World, WorldEvent};

/// Nearest AI cars that get an engine voice
pub const AI_VOICES: usize = 3;
/// AI cars farther than this are silent
const AI_RANGE: f32 = 150.0;
const SPEED_OF_SOUND: f32 = 343.0;

#[derive(Clone, Copy, Debug)]
pub struct Listener {
    pub eye: Vec3,
    pub target: Vec3,
}

/// How far a wheel is beyond its grip, split by kind. All 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelSlip {
    /// Sideways slide (scrub / drift)
    pub lat: f32,
    /// Driven wheel turning faster than the ground (wheelspin)
    pub spin: f32,
    /// Wheel turning slower than the ground (brake / handbrake lock-up)
    pub lock: f32,
}

/// THE single place that derives traction loss from wheel state.
///
/// Currently derived from `slide_lat` (m/s of lateral sliding beyond the slip-angle peak),
/// `slide_long` (|wheel surface speed - ground speed|) and the wheel spin direction. When
/// `WheelState::traction_loss` / `slip_ratio` / `slip_angle` land in rally_core, swap the body
/// of this function for them (e.g. `lat = w.traction_loss * lateral_share`, `spin =
/// slip_ratio.max(0)`, `lock = (-slip_ratio).max(0)`).
pub fn wheel_slip(car: &Car, w: &WheelState) -> WheelSlip {
    if !w.contact {
        return WheelSlip::default();
    }
    let lat = ((w.slide_lat - 0.6) / 5.0).clamp(0.0, 1.0);
    let long = ((w.slide_long - 1.0) / 6.0).clamp(0.0, 1.0);
    let wheel_v = (w.spin_speed * car.spec.wheel_radius).abs();
    let ground_v = car.telemetry.forward_speed.abs();
    if wheel_v >= ground_v {
        WheelSlip { lat, spin: long, lock: 0.0 }
    } else {
        // a stationary car can't lock its wheels
        let lock = if ground_v > 1.5 { long } else { 0.0 };
        WheelSlip { lat, spin: 0.0, lock }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SurfClass {
    Hard,
    Loose,
    Snow,
    Grass,
}

fn class(s: Surface) -> SurfClass {
    match s {
        Surface::Asphalt | Surface::Ice => SurfClass::Hard,
        Surface::Gravel | Surface::Dirt | Surface::Sand | Surface::Mud => SurfClass::Loose,
        Surface::Snow => SurfClass::Snow,
        Surface::Grass => SurfClass::Grass,
    }
}

/// Fill the car-intrinsic part of the engine params (no spatial info)
pub fn car_params(id: usize, car: &Car) -> EngineParams {
    let t = &car.telemetry;
    let mut p = EngineParams {
        id: id as u32,
        kind: car.spec.id,
        rpm: t.rpm.max(car.spec.idle_rpm * 0.5),
        idle_rpm: car.spec.idle_rpm,
        redline: car.spec.redline,
        throttle: t.throttle.clamp(0.0, 1.0),
        load: t.engine_load.clamp(0.0, 1.0),
        shifting: t.shifting,
        speed: t.speed,
        in_water: t.in_water,
        gain: 1.0,
        doppler: 1.0,
        ..Default::default()
    };
    let mut on = 0usize;
    let mut counts = [0usize; 4];
    for w in car.wheels.iter() {
        if !w.contact {
            continue;
        }
        on += 1;
        let c = class(w.surface.unwrap_or(Surface::Asphalt));
        let s = wheel_slip(car, w);
        match c {
            SurfClass::Hard => {
                counts[0] += 1;
                p.slip_lat_hard = p.slip_lat_hard.max(s.lat);
                p.spin_hard = p.spin_hard.max(s.spin);
                p.lock_hard = p.lock_hard.max(s.lock);
            }
            _ => {
                counts[match c {
                    SurfClass::Loose => 1,
                    SurfClass::Snow => 2,
                    _ => 3,
                }] += 1;
                p.slip_lat_loose = p.slip_lat_loose.max(s.lat);
                p.spin_loose = p.spin_loose.max(s.spin);
                p.lock_loose = p.lock_loose.max(s.lock);
            }
        }
    }
    p.ground = on as f32 / 4.0;
    if on > 0 {
        let k = 1.0 / on as f32;
        p.s_hard = counts[0] as f32 * k;
        p.s_loose = counts[1] as f32 * k;
        p.s_snow = counts[2] as f32 * k;
        p.s_grass = counts[3] as f32 * k;
    }
    p
}

/// Spatial cue of a point for the closest listener
#[derive(Clone, Copy, Debug)]
pub struct Spatial {
    pub dist: f32,
    pub gain: f32,
    pub pan: f32,
    pub doppler: f32,
    pub muffle: f32,
}

pub fn attenuation(d: f32) -> f32 {
    if d < 7.0 { 1.0 } else { (7.0 / d).powf(1.15) }
}

#[derive(Default)]
pub struct Bridge {
    prev_eyes: Vec<Vec3>,
    lis_vel: Vec<Vec3>,
    prev_water: Vec<bool>,
}

impl Bridge {
    pub fn new() -> Bridge {
        Bridge::default()
    }

    fn spatial(&self, listeners: &[Listener], pos: Vec3, vel: Vec3) -> Spatial {
        let mut best = (f32::MAX, 0usize);
        for (i, l) in listeners.iter().enumerate() {
            let d = l.eye.distance(pos);
            if d < best.0 {
                best = (d, i);
            }
        }
        let Some(l) = listeners.get(best.1) else {
            return Spatial { dist: 0.0, gain: 1.0, pan: 0.0, doppler: 1.0, muffle: 0.0 };
        };
        let d = best.0.max(0.01);
        let dir = (pos - l.eye) / d;
        let fwd = (l.target - l.eye).normalize_or_zero();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let split = if listeners.len() > 1 { 0.5 } else { 1.0 };
        // close sources are less sharply panned (they surround you)
        let pan = dir.dot(right) * 0.85 * split * ((d - 2.0) / 8.0).clamp(0.2, 1.0);
        let lv = self.lis_vel.get(best.1).copied().unwrap_or(Vec3::ZERO);
        let f = (SPEED_OF_SOUND + lv.dot(dir)) / (SPEED_OF_SOUND + vel.dot(dir));
        let doppler = (1.0 + (f - 1.0) * 0.7).clamp(0.85, 1.18);
        Spatial { dist: d, gain: attenuation(d), pan, doppler, muffle: ((d - 10.0) / 140.0).clamp(0.0, 1.0) }
    }

    /// Per-frame update. `locals` are the car indices of local players, `listeners` their cameras.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(&mut self, world: &World, locals: &[usize], listeners: &[Listener], events: &[WorldEvent], paused: bool, dt: f32, send: &mut dyn FnMut(Cmd)) {
        // listener velocities (for doppler); camera snaps reset them
        self.prev_eyes.resize(listeners.len(), Vec3::ZERO);
        self.lis_vel.resize(listeners.len(), Vec3::ZERO);
        for (i, l) in listeners.iter().enumerate() {
            let delta = l.eye - self.prev_eyes[i];
            let v = if dt > 1e-4 && delta.length() < 40.0 * dt.max(0.016) + 1.0 { delta / dt } else { Vec3::ZERO };
            self.lis_vel[i] = self.lis_vel[i].lerp(v, 0.3);
            self.prev_eyes[i] = l.eye;
        }

        // ---- engines ----
        let mut cars = [EngineParams::default(); MAX_CARS];
        let mut n = 0;
        let local_level = if locals.len() > 1 { 0.85 } else { 1.0 };
        for &c in locals {
            if c < world.cars.len() && n < MAX_CARS {
                let car = &world.cars[c];
                let mut p = car_params(c, car);
                let sp = self.spatial(listeners, car.pos, car.vel);
                p.gain = sp.gain.max(0.6) * local_level;
                p.pan = sp.pan * 0.5;
                p.doppler = sp.doppler;
                p.muffle = sp.muffle * 0.5;
                p.local = true;
                cars[n] = p;
                n += 1;
            }
        }
        let mut others: [(f32, usize); 64] = [(f32::MAX, 0); 64];
        let mut no = 0;
        for (i, car) in world.cars.iter().enumerate() {
            if locals.contains(&i) || no >= others.len() {
                continue;
            }
            let d = listeners.iter().map(|l| l.eye.distance(car.pos)).fold(f32::MAX, f32::min);
            if d < AI_RANGE {
                others[no] = (d, i);
                no += 1;
            }
        }
        others[..no].sort_by(|a, b| a.0.total_cmp(&b.0));
        for &(_, i) in others[..no].iter().take(AI_VOICES) {
            if n >= MAX_CARS {
                break;
            }
            let car = &world.cars[i];
            let mut p = car_params(i, car);
            let sp = self.spatial(listeners, car.pos, car.vel);
            p.gain = sp.gain * 0.9;
            p.pan = sp.pan;
            p.doppler = sp.doppler;
            p.muffle = sp.muffle;
            cars[n] = p;
            n += 1;
        }
        send(Cmd::Cars { n: n as u8, cars, paused });

        // ---- water splashes ----
        self.prev_water.resize(world.cars.len(), false);
        for (i, car) in world.cars.iter().enumerate() {
            let w = car.telemetry.in_water;
            if w != self.prev_water[i] {
                let sp = self.spatial(listeners, car.pos, Vec3::ZERO);
                if sp.dist < 80.0 {
                    let strength = car.telemetry.speed * if w { 1.0 } else { 0.4 };
                    send(Cmd::Sfx { kind: SfxKind::Splash { strength }, gain: sp.gain * if w { 1.0 } else { 0.5 }, pan: sp.pan });
                }
            }
            self.prev_water[i] = w;
        }

        // ---- events ----
        let laps = world.race.laps;
        let is_local = |c: usize| locals.contains(&c);
        for e in events {
            match *e {
                WorldEvent::Collision { car, other, pos, strength } => {
                    let sp = self.spatial(listeners, pos, Vec3::ZERO);
                    let involved = is_local(car) || other.is_some_and(is_local);
                    let gain = if involved { sp.gain.max(0.7) } else { sp.gain };
                    if gain > 0.02 {
                        send(Cmd::Sfx { kind: SfxKind::Collision { strength, scenery: other.is_none() }, gain, pan: sp.pan });
                    }
                }
                WorldEvent::Landing { car, strength } => {
                    if let Some(c) = world.cars.get(car) {
                        let sp = self.spatial(listeners, c.pos, Vec3::ZERO);
                        let gain = if is_local(car) { sp.gain.max(0.7) } else { sp.gain };
                        if gain > 0.03 {
                            send(Cmd::Sfx { kind: SfxKind::Landing { strength }, gain, pan: sp.pan * 0.6 });
                        }
                    }
                }
                WorldEvent::Race(r) => match r {
                    RaceEvent::CountdownTick(k) => send(Cmd::Sfx { kind: SfxKind::CountdownTick(k), gain: 1.0, pan: 0.0 }),
                    RaceEvent::Go => send(Cmd::Sfx { kind: SfxKind::Go, gain: 1.0, pan: 0.0 }),
                    RaceEvent::Checkpoint { car } if is_local(car) => send(Cmd::Sfx { kind: SfxKind::Checkpoint, gain: 1.0, pan: 0.0 }),
                    RaceEvent::Lap { car, lap, .. } if is_local(car) => {
                        if lap + 1 == laps {
                            send(Cmd::Sfx { kind: SfxKind::FinalLap, gain: 1.0, pan: 0.0 });
                        } else if lap < laps {
                            send(Cmd::Sfx { kind: SfxKind::Lap, gain: 1.0, pan: 0.0 });
                        }
                        // lap == laps: the Finish event follows with the fanfare
                    }
                    RaceEvent::Finish { car, place, .. } if is_local(car) => {
                        send(Cmd::Sfx { kind: SfxKind::Finish { place: place as u32 }, gain: 1.0, pan: 0.0 })
                    }
                    RaceEvent::Reset { car } => {
                        if is_local(car) {
                            send(Cmd::Sfx { kind: SfxKind::Respawn, gain: 0.9, pan: 0.0 });
                        } else if let Some(c) = world.cars.get(car) {
                            let sp = self.spatial(listeners, c.pos, Vec3::ZERO);
                            if sp.gain > 0.05 {
                                send(Cmd::Sfx { kind: SfxKind::Respawn, gain: sp.gain * 0.6, pan: sp.pan });
                            }
                        }
                    }
                    _ => {}
                },
            }
        }
    }
}
