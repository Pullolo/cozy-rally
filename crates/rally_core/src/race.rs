//! Race rules: countdown, checkpoints, laps, placings, wrong-way detection, auto-reset.
use crate::track::Track;
use crate::vehicle::Car;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum RacePhase {
    /// Seconds remaining before GO
    Countdown(f32),
    Racing,
    /// Everyone finished (or time-out after the winner); results are final
    Finished,
}

#[derive(Clone, Debug, Default)]
pub struct RacerProgress {
    /// Current lap, starting at 1
    pub lap: u32,
    /// Index into Track::checkpoints of the next checkpoint to hit
    pub next_cp: usize,
    pub s: f32,
    /// Race distance covered (for placings)
    pub total: f32,
    pub finished: bool,
    pub finish_time: Option<f32>,
    pub lap_start: f32,
    pub lap_times: Vec<f32>,
    pub best_lap: Option<f32>,
    pub place: usize,
    pub wrong_way: bool,
    pub wrong_way_timer: f32,
    pub reset_timer: f32,
    /// Seconds spent stuck (slow, off the road)
    pub stuck_timer: f32,
    pub last_safe_s: f32,
    /// Seconds of "ghost" (no car-car collision) after a reset
    pub ghost: f32,
    pub track_index: usize,
    pub lateral: f32,
    pub off_track: bool,
    /// Point-to-point stages: race clock at each split passed (see `Track::splits`)
    pub split_times: Vec<f32>,
    /// Index of the next split to pass (stages)
    pub next_split: usize,
    /// 0..1 progress through the race (stages: start line -> finish line; loops: all laps)
    pub progress: f32,
}

#[derive(Clone, Debug)]
pub struct Race {
    pub laps: u32,
    pub phase: RacePhase,
    /// Seconds since GO
    pub clock: f32,
    pub racers: Vec<RacerProgress>,
    pub finish_order: Vec<usize>,
    /// After the first finisher, remaining racers get this long before the race ends
    pub finish_grace: f32,
    pub grace_timer: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RaceEvent {
    CountdownTick(u32),
    Go,
    Checkpoint { car: usize },
    Lap { car: usize, lap: u32, time: f32 },
    Finish { car: usize, place: usize, time: f32 },
    Reset { car: usize },
    RaceOver,
}

impl Race {
    pub fn new(n_cars: usize, laps: u32, countdown: f32, track: &Track) -> Race {
        // stages are a single run from the start line to the finish line
        let laps = if track.is_loop() { laps } else { 1 };
        let safe = if track.is_loop() { track.length - 10.0 } else { track.start_s };
        let racers = (0..n_cars)
            .map(|_| RacerProgress { lap: 1, next_cp: 1, last_safe_s: safe, ..Default::default() })
            .collect();
        Race { laps, phase: RacePhase::Countdown(countdown), clock: 0.0, racers, finish_order: Vec::new(), finish_grace: 45.0, grace_timer: None }
    }

    pub fn is_racing(&self) -> bool {
        matches!(self.phase, RacePhase::Racing)
    }

    /// Advance timers and progress. `cars` are updated in place for resets / freezing.
    pub fn update(&mut self, dt: f32, track: &Track, cars: &mut [Car], resets: &[bool], events: &mut Vec<RaceEvent>) {
        match self.phase {
            RacePhase::Countdown(t) => {
                let nt = t - dt;
                if nt.ceil() < t.ceil() && nt > 0.0 && t.ceil() <= 3.0 {
                    events.push(RaceEvent::CountdownTick(nt.ceil() as u32));
                }
                if t.ceil() > 3.0 && nt.ceil() <= 3.0 && nt > 0.0 {
                    events.push(RaceEvent::CountdownTick(3));
                }
                if nt <= 0.0 {
                    self.phase = RacePhase::Racing;
                    events.push(RaceEvent::Go);
                    for c in cars.iter_mut() {
                        c.frozen = false;
                    }
                } else {
                    self.phase = RacePhase::Countdown(nt);
                    for c in cars.iter_mut() {
                        c.frozen = true;
                    }
                }
            }
            RacePhase::Racing => {
                self.clock += dt;
                if let Some(g) = self.grace_timer.as_mut() {
                    *g -= dt;
                    if *g <= 0.0 {
                        self.end_race(events);
                    }
                }
            }
            RacePhase::Finished => {}
        }

        let n_cp = track.checkpoints.len();
        let len = track.length;
        let racing = self.is_racing();
        let mut racers = std::mem::take(&mut self.racers);
        for (i, car) in cars.iter_mut().enumerate() {
            let r = &mut racers[i];
            r.ghost = (r.ghost - dt).max(0.0);
            let q = track.query(car.pos, Some(r.track_index).filter(|_| r.track_index != 0 || r.s != 0.0));
            r.track_index = q.index;
            r.s = q.s;
            r.lateral = q.lateral;
            let smp = &track.samples[q.index];
            r.off_track = q.dist > smp.half_width + 1.0;

            // safe position for respawns
            let upright = car.up().y > 0.85;
            if q.dist < smp.half_width - 1.0 && upright && car.telemetry.wheels_on_ground >= 3 && !car.telemetry.in_water {
                r.last_safe_s = q.s;
            }

            if !track.is_loop() {
                // ---- point-to-point stage: splits in order, then the finish line
                if !r.finished && racing {
                    let near_road = q.dist < smp.half_width + 25.0;
                    while r.next_split < track.splits.len() && near_road {
                        let sp = track.splits[r.next_split];
                        if q.s >= sp && q.s - sp < 60.0 {
                            r.split_times.push(self.clock);
                            r.next_split += 1;
                            events.push(RaceEvent::Checkpoint { car: i });
                        } else {
                            break;
                        }
                    }
                    if r.next_split >= track.splits.len() && near_road && q.s >= track.finish_s && q.s - track.finish_s < 80.0 {
                        let t = self.clock;
                        r.lap_times.push(t);
                        r.best_lap = Some(t);
                        r.finished = true;
                        r.finish_time = Some(t);
                        self.finish_order.push(i);
                        let place = self.finish_order.len();
                        events.push(RaceEvent::Finish { car: i, place, time: t });
                        if self.grace_timer.is_none() {
                            self.grace_timer = Some(self.finish_grace);
                        }
                    }
                    let moving_back = car.vel.dot(smp.tangent) < -3.0;
                    if moving_back {
                        r.wrong_way_timer += dt;
                    } else {
                        r.wrong_way_timer = (r.wrong_way_timer - dt * 2.0).max(0.0);
                    }
                    r.wrong_way = r.wrong_way_timer > 1.2;
                }
                r.progress = if r.finished { 1.0 } else { track.stage_progress(q.s) };
                r.total = q.s - track.start_s;
            } else if !r.finished && racing {
                // checkpoints
                let target_cp = if r.next_cp >= n_cp { 0 } else { r.next_cp };
                let cp_s = track.samples[track.checkpoints[target_cp]].s;
                let ahead = (q.s - cp_s).rem_euclid(len);
                if ahead < 35.0 && q.dist < smp.half_width + 25.0 {
                    if target_cp == 0 {
                        let lap_time = self.clock - r.lap_start;
                        r.lap_times.push(lap_time);
                        r.best_lap = Some(r.best_lap.map_or(lap_time, |b: f32| b.min(lap_time)));
                        r.lap_start = self.clock;
                        events.push(RaceEvent::Lap { car: i, lap: r.lap, time: lap_time });
                        if r.lap >= self.laps {
                            r.finished = true;
                            r.finish_time = Some(self.clock);
                            self.finish_order.push(i);
                            let place = self.finish_order.len();
                            events.push(RaceEvent::Finish { car: i, place, time: self.clock });
                            if self.grace_timer.is_none() {
                                self.grace_timer = Some(self.finish_grace);
                            }
                        } else {
                            r.lap += 1;
                        }
                        r.next_cp = 1;
                    } else {
                        r.next_cp += 1;
                        events.push(RaceEvent::Checkpoint { car: i });
                    }
                }
                // wrong way
                let moving_back = car.vel.dot(smp.tangent) < -3.0;
                if moving_back {
                    r.wrong_way_timer += dt;
                } else {
                    r.wrong_way_timer = (r.wrong_way_timer - dt * 2.0).max(0.0);
                }
                r.wrong_way = r.wrong_way_timer > 1.2;
            }

            // progress total
            if track.is_loop() {
                let mut s_rel = q.s;
                if r.next_cp <= 1 && s_rel > len * 0.6 {
                    s_rel -= len;
                }
                if r.next_cp >= n_cp && s_rel < len * 0.4 {
                    s_rel += len;
                }
                r.total = (r.lap as f32 - 1.0) * len + s_rel;
                r.progress = if r.finished { 1.0 } else { (r.total / (len * self.laps.max(1) as f32)).clamp(0.0, 1.0) };
            }

            // auto reset conditions
            let flipped = car.is_upside_down() && car.vel.length() < 4.0;
            let lost = q.dist > smp.half_width + 60.0;
            let wet = car.telemetry.in_water;
            // stuck off the road (e.g. wedged between trees): help them back after a while
            let edge = smp.half_width + crate::track::SHOULDER;
            let stuck = racing && !r.finished && ((q.dist > edge + 2.0 && car.vel.length() < 2.5) || q.dist > edge + 14.0);
            r.stuck_timer = if stuck { r.stuck_timer + dt } else { (r.stuck_timer - dt * 2.0).max(0.0) };
            if flipped || lost || wet || r.stuck_timer > 4.0 {
                r.reset_timer += dt;
            } else {
                r.reset_timer = 0.0;
            }
            let manual = resets.get(i).copied().unwrap_or(false);
            if (r.reset_timer > 2.5 || manual) && !matches!(self.phase, RacePhase::Countdown(_)) {
                let s = r.last_safe_s;
                let (p, _, _, up) = track.frame_at(s);
                let ride = -car.spec.mount_height + car.spec.susp_rest + car.spec.wheel_radius;
                let pos = p + up * (ride + 0.3);
                car.teleport(pos, track.rotation_at(s));
                car.track_hint = None;
                r.reset_timer = 0.0;
                r.stuck_timer = 0.0;
                r.ghost = 2.5;
                events.push(RaceEvent::Reset { car: i });
            }
        }
        self.racers = racers;

        // placings: finished first by order, then by total progress
        let mut order: Vec<usize> = (0..self.racers.len()).collect();
        order.sort_by(|&a, &b| {
            let ra = &self.racers[a];
            let rb = &self.racers[b];
            match (ra.finish_time, rb.finish_time) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap(),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => rb.total.partial_cmp(&ra.total).unwrap_or(std::cmp::Ordering::Equal),
            }
        });
        for (place, &i) in order.iter().enumerate() {
            self.racers[i].place = place + 1;
        }

        if self.is_racing() && self.racers.iter().all(|r| r.finished) {
            self.end_race(events);
        }
    }

    fn end_race(&mut self, events: &mut Vec<RaceEvent>) {
        if self.phase != RacePhase::Finished {
            self.phase = RacePhase::Finished;
            events.push(RaceEvent::RaceOver);
        }
    }

    /// Final standings: car indices in order
    pub fn standings(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.racers.len()).collect();
        order.sort_by_key(|&i| self.racers[i].place);
        order
    }

    /// Mark a racer finished (e.g. remote player reported a finish)
    pub fn force_finish(&mut self, car: usize, time: f32) {
        let r = &mut self.racers[car];
        if !r.finished {
            r.finished = true;
            r.finish_time = Some(time);
            self.finish_order.push(car);
            if self.grace_timer.is_none() {
                self.grace_timer = Some(self.finish_grace);
            }
        }
    }
}

pub fn format_time(t: f32) -> String {
    let m = (t / 60.0).floor() as u32;
    let s = t - m as f32 * 60.0;
    format!("{}:{:06.3}", m, s)
}
