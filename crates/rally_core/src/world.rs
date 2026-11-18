//! The simulation world: fixed-step physics for all cars, car-car and car-prop collisions,
//! race logic and an event stream for audio/FX/UI.
use crate::input::CarInput;
use crate::race::{Race, RaceEvent};
use crate::track::Track;
use crate::vehicle::{car_specs, Car};
use glam::Vec3;
use std::sync::Arc;

pub const TICK: f32 = 1.0 / 120.0;
pub const SUBSTEPS: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldEvent {
    /// Impact between a car and another car (Some) or scenery (None)
    Collision { car: usize, other: Option<usize>, pos: Vec3, strength: f32 },
    Landing { car: usize, strength: f32 },
    Race(RaceEvent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controller {
    /// Driven by a local human (index into local players)
    Local(u8),
    Ai,
    /// State comes from the network; not simulated locally
    Remote,
}

pub struct World {
    pub track: Arc<Track>,
    pub cars: Vec<Car>,
    pub controllers: Vec<Controller>,
    pub inputs: Vec<CarInput>,
    pub race: Race,
    pub time: f64,
    pub events: Vec<WorldEvent>,
    acc: f32,
    pending_resets: Vec<bool>,
    collision_cooldown: Vec<f32>,
}

pub struct RacerSetup {
    pub car_spec: usize,
    pub controller: Controller,
}

impl World {
    pub fn new(track: Arc<Track>, racers: &[RacerSetup], laps: u32) -> World {
        let specs = car_specs();
        let slots = track.grid_slots(racers.len());
        let mut cars = Vec::new();
        for (r, (pos, rot)) in racers.iter().zip(slots) {
            let spec = specs[r.car_spec.min(specs.len() - 1)].clone();
            let ride = -spec.mount_height + spec.susp_rest * 0.8 + spec.wheel_radius;
            let pos = glam::Vec3::new(pos.x, track.terrain.height(pos.x, pos.z) + ride, pos.z);
            let mut car = Car::new(spec, pos, rot);
            car.frozen = true;
            cars.push(car);
        }
        let n = cars.len();
        let race = Race::new(n, laps, 4.0, &track);
        let mut w = World {
            track,
            cars,
            controllers: racers.iter().map(|r| r.controller).collect(),
            inputs: vec![CarInput::default(); n],
            race,
            time: 0.0,
            events: Vec::new(),
            acc: 0.0,
            pending_resets: vec![false; n],
            collision_cooldown: vec![0.0; n],
        };
        // settle suspensions before the countdown
        for _ in 0..120 {
            w.physics_tick(TICK);
        }
        w
    }

    pub fn set_input(&mut self, car: usize, input: CarInput) {
        if input.reset && !self.inputs[car].reset {
            self.pending_resets[car] = true;
        }
        self.inputs[car] = input;
    }

    /// Advance by a frame delta; runs as many fixed ticks as needed. Returns the
    /// interpolation alpha (0..1) between the previous and current tick.
    pub fn update(&mut self, frame_dt: f32) -> f32 {
        self.acc += frame_dt.min(0.1);
        while self.acc >= TICK {
            self.tick();
            self.acc -= TICK;
        }
        self.acc / TICK
    }

    pub fn tick(&mut self) {
        let dt = TICK;
        for (i, c) in self.cars.iter_mut().enumerate() {
            c.input = self.inputs[i];
            c.prev_pos = c.pos;
            c.prev_rot = c.rot;
        }
        self.physics_tick(dt);
        let mut race_events = Vec::new();
        let resets: Vec<bool> = self
            .pending_resets
            .iter()
            .enumerate()
            .map(|(i, r)| *r && self.controllers[i] != Controller::Remote)
            .collect();
        self.pending_resets.iter_mut().for_each(|r| *r = false);
        // remote cars are positioned by the network layer; don't auto-reset them
        let mut race = std::mem::replace(&mut self.race, Race::new(0, 0, 0.0, &self.track));
        race.update(dt, &self.track, &mut self.cars, &resets, &mut race_events);
        self.race = race;
        for (i, c) in self.controllers.iter().enumerate() {
            if *c == Controller::Remote {
                self.cars[i].frozen = false;
            }
        }
        for e in race_events {
            self.events.push(WorldEvent::Race(e));
        }
        for (i, car) in self.cars.iter_mut().enumerate() {
            if car.telemetry.last_landing > 3.0 {
                self.events.push(WorldEvent::Landing { car: i, strength: car.telemetry.last_landing });
            }
            car.telemetry.last_landing = 0.0;
        }
        self.time += dt as f64;
    }

    fn physics_tick(&mut self, dt: f32) {
        let sub = dt / SUBSTEPS as f32;
        for cd in self.collision_cooldown.iter_mut() {
            *cd = (*cd - dt).max(0.0);
        }
        for _ in 0..SUBSTEPS {
            for (i, car) in self.cars.iter_mut().enumerate() {
                if self.controllers[i] == Controller::Remote {
                    // dead-reckon remote cars between network updates
                    car.pos += car.vel * sub;
                    continue;
                }
                car.step(sub, &self.track);
            }
            self.collide_cars();
            self.collide_props();
        }
    }

    fn collide_cars(&mut self) {
        let n = self.cars.len();
        for a in 0..n {
            for b in (a + 1)..n {
                let ghost = self.race.racers.get(a).map_or(0.0, |r| r.ghost) > 0.0 || self.race.racers.get(b).map_or(0.0, |r| r.ghost) > 0.0;
                if ghost {
                    continue;
                }
                if self.cars[a].pos.distance_squared(self.cars[b].pos) > 36.0 {
                    continue;
                }
                let sa = self.cars[a].collision_spheres();
                let sb = self.cars[b].collision_spheres();
                for (pa, ra) in sa {
                    for (pb, rb) in sb {
                        let d = pb - pa;
                        let dist = d.length();
                        let min = ra + rb;
                        if dist >= min || dist < 1e-4 {
                            continue;
                        }
                        let nrm = d / dist;
                        let pen = min - dist;
                        let contact = pa + nrm * ra;
                        let a_remote = self.controllers[a] == Controller::Remote;
                        let b_remote = self.controllers[b] == Controller::Remote;
                        let (ma, mb) = (self.cars[a].spec.mass, self.cars[b].spec.mass);
                        let inv_a = if a_remote { 0.0 } else { 1.0 / ma };
                        let inv_b = if b_remote { 0.0 } else { 1.0 / mb };
                        let inv_sum = inv_a + inv_b;
                        if inv_sum <= 0.0 {
                            continue;
                        }
                        // positional correction
                        self.cars[a].pos -= nrm * pen * (inv_a / inv_sum);
                        self.cars[b].pos += nrm * pen * (inv_b / inv_sum);
                        let va = self.cars[a].point_velocity(contact);
                        let vb = self.cars[b].point_velocity(contact);
                        let rel = (vb - va).dot(nrm);
                        if rel < 0.0 {
                            let e = 0.25;
                            let j = -(1.0 + e) * rel / inv_sum * 0.8;
                            let imp = nrm * j;
                            if !a_remote {
                                self.cars[a].apply_impulse(-imp, contact);
                            }
                            if !b_remote {
                                self.cars[b].apply_impulse(imp, contact);
                            }
                            let strength = -rel;
                            if strength > 1.5 && self.collision_cooldown[a] <= 0.0 {
                                self.collision_cooldown[a] = 0.25;
                                self.collision_cooldown[b] = 0.25;
                                self.events.push(WorldEvent::Collision { car: a, other: Some(b), pos: contact, strength });
                            }
                        }
                    }
                }
            }
        }
    }

    fn collide_props(&mut self) {
        let track = self.track.clone();
        for i in 0..self.cars.len() {
            if self.controllers[i] == Controller::Remote {
                continue;
            }
            let spheres = self.cars[i].collision_spheres();
            let mut hits: Vec<(glam::Vec3, glam::Vec3, f32)> = Vec::new();
            track.for_each_prop_near(self.cars[i].pos, 8.0, |p| {
                if p.radius <= 0.0 {
                    return;
                }
                let height = p.kind.collision_height(p.scale);
                for (c, r) in spheres {
                    if c.y > p.pos.y + height + r || c.y < p.pos.y - 1.0 {
                        continue;
                    }
                    let d = glam::Vec3::new(c.x - p.pos.x, 0.0, c.z - p.pos.z);
                    let dist = d.length();
                    let min = r + p.radius;
                    if dist < min && dist > 1e-4 {
                        let nrm = d / dist;
                        hits.push((nrm, c - nrm * r, min - dist));
                    }
                }
            });
            for (nrm, contact, pen) in hits {
                let car = &mut self.cars[i];
                car.pos += nrm * pen;
                let v = car.point_velocity(contact);
                let vn = v.dot(nrm);
                if vn < 0.0 {
                    let inv_m = 1.0 / car.spec.mass;
                    let r = contact - car.pos;
                    let ii = car.inv_inertia_world();
                    let k = inv_m + nrm.dot((ii * r.cross(nrm)).cross(r));
                    let j = -(1.0 + 0.3) * vn / k;
                    // tangential friction
                    let vt = v - nrm * vn;
                    let ft = if vt.length() > 0.01 { -vt.normalize() * (j * 0.3).min(vt.length() / k) } else { glam::Vec3::ZERO };
                    car.apply_impulse(nrm * j + ft, contact);
                    if -vn > 2.0 && self.collision_cooldown[i] <= 0.0 {
                        self.collision_cooldown[i] = 0.3;
                        self.events.push(WorldEvent::Collision { car: i, other: None, pos: contact, strength: -vn });
                    }
                }
            }
        }
    }

    pub fn drain_events(&mut self) -> Vec<WorldEvent> {
        std::mem::take(&mut self.events)
    }
}
