//! AI drivers.
//!
//! * Follows a precomputed racing line (`racing_line.rs`) with pure-pursuit steering.
//! * Speed from a per-car speed profile (curvature + braking zones), scaled by skill.
//! * Awareness of other cars: follows instead of ramming, picks a side to overtake,
//!   gives room when side by side.
//! * Personality: skill, aggression, consistency (+ occasional small mistakes).
//! * Cozy rubber-banding against the human players, scaled by difficulty.
//! * Stuck recovery (reverse out, three-point turn) and respawn as a last resort.
use crate::input::CarInput;
use crate::racing_line::{RacingLine, SpeedParams, SpeedProfile};
use crate::rng::Rng;
use crate::track::Track;
use crate::vehicle::Car;
use crate::world::{Controller, World};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Personality {
    /// 0..1: how much of the car's grip the driver dares to use, braking precision
    pub skill: f32,
    /// 0..1: following distance, willingness to overtake / squeeze through
    pub aggression: f32,
    /// 0..1: 1 = metronome, lower = more speed wobble and more mistakes
    pub consistency: f32,
}

/// Cozy catch-up: how strongly an AI eases off when far ahead of the humans, and pushes
/// when far behind (0 = off).
#[derive(Clone, Copy, Debug, Default)]
pub struct RubberBand {
    pub ease_ahead: f32,
    pub push_behind: f32,
}

impl RubberBand {
    /// Catch-up tuned for a difficulty (0..1 base skill: Chill 0.35, Normal 0.7, Spicy 1.0)
    pub fn for_difficulty(skill: f32) -> RubberBand {
        RubberBand { ease_ahead: (1.25 - skill * 0.75).clamp(0.3, 1.0), push_behind: (0.35 + skill * 0.5).clamp(0.3, 0.85) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mistake {
    None,
    /// Brakes too late and runs a bit wide
    LateBrake,
    /// Drifts wide on exit
    Wide,
    /// Lifts / hesitates
    Lift,
}

/// Counters for headless playtests
#[derive(Clone, Copy, Debug, Default)]
pub struct AiStats {
    pub off_track_time: f32,
    pub mistakes: u32,
    pub recoveries: u32,
    pub resets: u32,
    pub overtakes_tried: u32,
}

/// What the AI is currently thinking (for debugging overlays / tests)
#[derive(Clone, Copy, Debug, Default)]
pub struct AiDebug {
    pub target_speed: f32,
    pub line_offset: f32,
    pub avoid: f32,
    pub band: f32,
    pub following: bool,
}

/// Another car as seen from this driver, in track coordinates
#[derive(Clone, Copy, Debug)]
struct Other {
    /// Distance ahead along the track (m, negative = behind)
    ds: f32,
    lat: f32,
    v_along: f32,
}

#[derive(Default)]
struct Context {
    others: Vec<Other>,
    /// my race distance - leading human's race distance (m), None = no humans to pace
    human_gap: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct AiDriver {
    /// Kept for compatibility: same as personality.skill
    pub skill: f32,
    /// Small preferred lateral offset from the racing line (m)
    pub lane_bias: f32,
    pub personality: Personality,
    pub band: RubberBand,
    /// Engine torque multiplier the AI wants (rubber-banding); `drive_all` applies it
    pub power_scale: f32,
    /// Baseline engine torque multiplier (easier opponents are a little gentler)
    pub base_power: f32,
    pub stats: AiStats,
    pub debug: AiDebug,
    line: Option<Arc<RacingLine>>,
    line_track_len: f32,
    profile: Option<(u8, SpeedProfile)>,
    rng: Rng,
    time: f32,
    phase: f32,
    avoid: f32,
    // progress watchdog
    last_s: Option<f32>,
    progress: f32,
    progress_mark: f32,
    no_progress: f32,
    off_track: f32,
    recover_timer: f32,
    recover_steer: f32,
    attempts: u32,
    reset_cooldown: f32,
    // mistakes
    mistake: Mistake,
    mistake_timer: f32,
    mistake_cooldown: f32,
    was_braking: bool,
    /// Lateral position at the start and progress since (hold your lane off the grid)
    start_lat: Option<f32>,
    start_progress: f32,
    prev_v_along: f32,
    accel_along: f32,
    /// Tyre friction the speed plan assumes (x surface grip per sample)
    mu_plan: f32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn wrap_half(d: f32, len: f32) -> f32 {
    let mut d = d.rem_euclid(len);
    if d > len * 0.5 {
        d -= len;
    }
    d
}

impl AiDriver {
    /// Simple driver with a fixed skill (used for autopilots); no catch-up.
    pub fn new(skill: f32, lane_bias: f32) -> Self {
        let p = Personality { skill, aggression: 0.5, consistency: 1.0 };
        let mut d = AiDriver::with_personality(p, RubberBand::default(), (lane_bias * 1000.0) as u64 ^ 0x5eed);
        d.lane_bias = lane_bias;
        d
    }

    pub fn with_personality(p: Personality, band: RubberBand, seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0xA15EED);
        let phase = rng.range(0.0, 100.0);
        AiDriver {
            skill: p.skill,
            lane_bias: 0.0,
            personality: p,
            band,
            power_scale: 1.0,
            base_power: 1.0,
            stats: AiStats::default(),
            debug: AiDebug::default(),
            line: None,
            line_track_len: 0.0,
            profile: None,
            rng,
            time: 0.0,
            phase,
            avoid: 0.0,
            last_s: None,
            progress: 0.0,
            progress_mark: 0.0,
            no_progress: 0.0,
            off_track: 0.0,
            recover_timer: 0.0,
            recover_steer: 0.0,
            attempts: 0,
            reset_cooldown: 0.0,
            mistake: Mistake::None,
            mistake_timer: 0.0,
            mistake_cooldown: 3.0,
            was_braking: false,
            start_lat: None,
            start_progress: 0.0,
            prev_v_along: 0.0,
            accel_along: 0.0,
            mu_plan: 1.0,
        }
    }

    /// A race opponent: personality rolled around the difficulty's base skill.
    /// `index` makes each opponent different, `seed` keeps it deterministic per race.
    pub fn racer(index: usize, base_skill: f32, seed: u64) -> Self {
        let mut rng = Rng::new(seed.wrapping_mul(31).wrapping_add(index as u64 * 7919));
        let skill = (base_skill + rng.range(-0.07, 0.07)).clamp(0.05, 1.0);
        let aggression = rng.range(0.3, 1.0);
        let consistency = (0.55 + base_skill * 0.35 + rng.range(-0.12, 0.12)).clamp(0.3, 1.0);
        let p = Personality { skill, aggression, consistency };
        let mut d = AiDriver::with_personality(p, RubberBand::for_difficulty(base_skill), seed ^ index as u64);
        d.base_power = (0.85 + 0.15 * base_skill).min(1.0);
        d
    }

    fn grip_use(&self) -> f32 {
        // calibrated with `skilltest SKILL_SWEEP` on the designed rally stages: the lap-time
        // optimum is ~0.95 (Chill 0.73, Normal 0.84, Spicy 0.93 = just under it)
        0.62 + 0.31 * self.personality.skill
    }

    fn ensure_plan(&mut self, car: &Car, track: &Track) {
        if self.line.is_none() || self.line_track_len != track.length || self.line.as_ref().map(|l| l.offset.len()) != Some(track.samples.len()) {
            self.line = Some(RacingLine::shared(track));
            self.line_track_len = track.length;
            self.profile = None;
        }
        if self.profile.as_ref().map(|p| p.0) != Some(car.spec.id) {
            let sp = SpeedParams::for_car(&car.spec, self.grip_use() * car.spec.ai_grip, (0.6 + 0.35 * self.personality.skill).min(0.97));
            self.mu_plan = sp.mu;
            let prof = SpeedProfile::build(self.line.as_ref().unwrap(), &sp);
            self.profile = Some((car.spec.id, prof));
        }
    }

    /// Predicted flying-lap time for `car` on `track` with this driver's planning (s)
    pub fn predicted_lap(&mut self, car: &Car, track: &Track) -> f32 {
        self.ensure_plan(car, track);
        self.profile.as_ref().unwrap().1.lap_time
    }

    /// Drive alone: racing line + speed profile + stuck recovery, no traffic awareness.
    pub fn drive(&mut self, car: &Car, track: &Track, dt: f32) -> CarInput {
        self.drive_inner(car, track, &Context::default(), dt)
    }

    /// Drive car `me` in `world`, aware of the other cars and of the human players.
    pub fn drive_world(&mut self, me: usize, world: &World, dt: f32) -> CarInput {
        let track = &world.track;
        let car = &world.cars[me];
        let race = &world.race;
        let mut ctx = Context::default();
        let my = race.racers.get(me);
        let my_ghost = my.map_or(0.0, |r| r.ghost) > 0.0;
        let q = track.query(car.pos, car.track_hint);
        if !my_ghost {
            for (j, other) in world.cars.iter().enumerate() {
                if j == me || race.racers.get(j).map_or(0.0, |r| r.ghost) > 0.0 {
                    continue;
                }
                if other.pos.distance_squared(car.pos) > 60.0 * 60.0 {
                    continue;
                }
                let oq = track.query(other.pos, other.track_hint);
                let ds = wrap_half(oq.s - q.s, track.length);
                let t = track.samples[oq.index].tangent;
                ctx.others.push(Other { ds, lat: oq.lateral, v_along: other.vel.dot(t) });
            }
        }
        // rubber band against the leading unfinished human
        if let Some(my) = my {
            let mut best: Option<f32> = None;
            let mut any_human_finished = false;
            for (j, c) in world.controllers.iter().enumerate() {
                if matches!(c, Controller::Local(_)) && j != me {
                    let r = &race.racers[j];
                    if r.finished {
                        any_human_finished = true;
                    } else {
                        best = Some(best.map_or(r.total, |b: f32| b.max(r.total)));
                    }
                }
            }
            if !any_human_finished && !my.finished && race.is_racing() {
                ctx.human_gap = best.map(|b| my.total - b);
            }
        }
        self.drive_inner(car, track, &ctx, dt)
    }

    fn drive_inner(&mut self, car: &Car, track: &Track, ctx: &Context, dt: f32) -> CarInput {
        self.ensure_plan(car, track);
        let line = self.line.clone().unwrap();
        let p = self.personality;
        self.time += dt;
        self.reset_cooldown = (self.reset_cooldown - dt).max(0.0);
        self.mistake_cooldown = (self.mistake_cooldown - dt).max(0.0);

        // track position, kept continuous: if the nearest-point query suddenly jumps to another
        // part of the stage (we left the road where the course loops back nearby), ignore it
        // and head back to where we left, so no checkpoint gets skipped
        let n_samples = track.samples.len();
        let hint = self.last_s.map(|ls| ((ls / crate::track::SAMPLE_SPACING) as usize) % n_samples).or(car.track_hint);
        let mut q = track.query(car.pos, hint);
        let mut rejoining = false;
        if let Some(ls) = self.last_s {
            if wrap_half(q.s - ls, track.length).abs() > 40.0 {
                rejoining = true;
                let i = ((ls / crate::track::SAMPLE_SPACING) as usize) % n_samples;
                let (p0, _, r0, _) = track.frame_at(ls);
                let d = car.pos - p0;
                q = crate::track::TrackQuery { index: i, s: ls, lateral: d.dot(r0), dist: (d - r0 * d.dot(r0)).length().max(d.dot(r0).abs()) };
            }
        }
        let smp = &track.samples[q.index];
        let speed = car.vel.length();
        let fwd = car.forward();
        let v_along = car.vel.dot(smp.tangent);
        let heading_dot = fwd.dot(smp.tangent);

        // ---------------- progress watchdog
        if let Some(ls) = self.last_s {
            self.progress += wrap_half(q.s - ls, track.length);
        }
        if !rejoining {
            self.last_s = Some(q.s);
        } else if speed > 3.0 {
            // driving back to the road is progress too
            self.no_progress = 0.0;
        }
        let off = q.dist > smp.half_width + 1.0;
        if off && !car.frozen {
            self.stats.off_track_time += dt;
            self.off_track += dt;
        } else {
            self.off_track = (self.off_track - dt * 2.0).max(0.0);
        }
        if dt > 0.0 {
            let a = (v_along - self.prev_v_along) / dt;
            self.accel_along += (a.clamp(-30.0, 30.0) - self.accel_along) * (1.0 - (-dt * 3.0).exp());
        }
        self.prev_v_along = v_along;
        // slowly but surely pulling away (e.g. uphill on snow) is not "stuck"
        let gaining = self.accel_along > 0.2 && v_along > 0.0 && heading_dot > 0.3;
        if car.frozen {
            self.no_progress = 0.0;
            self.progress_mark = self.progress;
            self.attempts = 0;
        } else if self.progress > self.progress_mark + 4.0 {
            self.progress_mark = self.progress;
            self.no_progress = 0.0;
            self.attempts = 0;
        } else if gaining && self.no_progress < 6.0 {
            self.no_progress += dt * 0.15;
        } else {
            self.no_progress += dt;
            // never let the mark run away when going backwards
            self.progress_mark = self.progress_mark.min(self.progress + 4.0);
        }

        // ---------------- steering: pure pursuit on the racing line (+ avoidance offset)
        let wheelbase = car.spec.cg_to_front + car.spec.cg_to_rear;
        let ld = (5.5 + speed * 0.42).clamp(6.0, 28.0);
        let s_t = q.s + ld;
        // lane bias shrinks on narrow roads so it never pushes a car onto the shoulder
        let width_k = ((line.limit_at(s_t, track) - 0.5) / 3.5).clamp(0.0, 1.0);
        let mut line_off_t = line.offset_at(s_t, track) + self.lane_bias * 0.3 * width_k;
        // off the grid: keep our lane for the first stretch, then merge onto the line
        if car.frozen {
            self.start_lat = Some(q.lateral);
            self.start_progress = self.progress;
        }
        if let Some(sl) = self.start_lat {
            let t = ((self.progress - self.start_progress) / 160.0).clamp(0.0, 1.0);
            if t >= 1.0 {
                self.start_lat = None;
            } else {
                let k = t * t * (3.0 - 2.0 * t);
                line_off_t = sl + (line_off_t - sl) * k;
            }
        }
        let lim_t = line.limit_at(s_t, track) + 0.4;

        // speed this driver would like here without traffic (profile, look-ahead)
        let free_speed = {
            let (_, prof) = self.profile.as_ref().unwrap();
            let look = (speed * 0.45).max(4.0);
            let mut v = f32::INFINITY;
            let mut k = 0.0;
            while k <= look {
                v = v.min(prof.limit_at(q.s + k, track));
                k += 2.0;
            }
            v
        };
        let mut passing = false;
        let mut alongside = false;

        // ---- traffic: decide where to be and how fast we may go
        let mut want_lat: Option<f32> = None;
        let mut follow_cap = f32::INFINITY;
        let mut emergency = false;
        let my_lat = q.lateral;
        let mut following = false;
        for o in &ctx.others {
            let closing = v_along - o.v_along;
            // side by side: give room
            if o.ds.abs() < 5.8 && (o.lat - my_lat).abs() < 3.0 {
                let side = if my_lat <= o.lat { -1.0 } else { 1.0 };
                let w = (o.lat + side * 3.1).clamp(-lim_t, lim_t);
                want_lat = Some(w);
                alongside = true;
                // squeezed (no room left on my side) and not ahead: yield instead of rubbing
                if (w - o.lat).abs() < 2.4 && o.ds > -1.0 {
                    follow_cap = follow_cap.min(o.v_along - 0.8);
                    following = true;
                }
                continue;
            }
            if o.ds > 0.0 && o.ds < 38.0 {
                let planned = line.offset_at(q.s + o.ds, track) + self.avoid;
                let overlap = (o.lat - planned).abs() < 2.3;
                if !overlap || closing < -0.5 {
                    continue;
                }
                let ttc = (o.ds - 4.6).max(0.1) / closing.max(0.1);
                // want to pass if closing fast, or if stuck behind a slower car we could beat
                let faster = free_speed > o.v_along + 0.4 && o.ds < 16.0 + p.aggression * 14.0;
                // no new moves into tight corners (hairpins / squares): wait for the exit
                let tight_ahead = line.max_curv_ahead(q.index, 35.0).abs() > 1.0 / 30.0;
                if (ttc < 2.8 + p.aggression * 1.5 || faster) && (!tight_ahead || self.avoid.abs() > 1.0) {
                    // try a pass: pick a feasible side, preferring the racing line ahead
                    let lim_o = line.limit_at(q.s + o.ds, track) + 0.4;
                    let ahead_line = line.offset_at(q.s + o.ds + 15.0, track);
                    let mut best: Option<f32> = None;
                    for cand in [o.lat - 2.7, o.lat + 2.7] {
                        if cand.abs() <= lim_o {
                            let better = best.map_or(true, |b| (cand - ahead_line).abs() < (b - ahead_line).abs());
                            if better {
                                best = Some(cand);
                            }
                        }
                    }
                    if let Some(b) = best.filter(|_| p.aggression > 0.2 || o.ds > 10.0) {
                        if want_lat.is_none() {
                            self.stats.overtakes_tried += (self.debug.avoid.abs() < 0.3) as u32;
                            want_lat = Some(b);
                            passing = true;
                        }
                    }
                }
                // never ram: keep a gap to the car ahead in our path (tighter while pulling out)
                let gap = if passing { 4.6 + (1.0 - p.aggression) * 1.5 } else { 5.0 + (1.0 - p.aggression) * 4.0 + speed * 0.12 };
                let cap = o.v_along + (o.ds - gap) * 0.9;
                if cap < follow_cap {
                    follow_cap = cap.max(0.0);
                    following = true;
                }
                if o.ds < 8.0 && closing > 1.5 && (o.lat - my_lat).abs() < 2.0 {
                    emergency = true;
                }
            }
        }
        // smoothed lateral avoidance relative to the racing line
        let avoid_target = match want_lat {
            Some(w) => (w - line_off_t).clamp(-6.0, 6.0),
            None => 0.0,
        };
        let rate = if alongside { 5.0 } else if want_lat.is_some() { 3.2 } else { 1.4 };
        self.avoid += (avoid_target - self.avoid).clamp(-rate * dt, rate * dt);
        // mistakes: running wide
        let mut wide = 0.0;
        if self.mistake == Mistake::Wide {
            let c = line.max_curv_ahead(q.index, 20.0);
            wide = -c.signum() * 1.2;
        }
        let lat_t = (line_off_t + self.avoid + wide).clamp(-lim_t, lim_t);
        let (pt, _, rt, _) = track.frame_at(s_t);
        let target = pt + rt * lat_t;
        let to = target - car.pos;
        let x = to.dot(car.right());
        let z = to.dot(fwd);
        let mut steer;
        let sf = (1.0 / (1.0 + (speed / 22.0).powi(2) * 0.9)).max(0.22);
        if z < 1.0 {
            // target behind us (facing the wrong way): full lock toward it
            steer = if x >= 0.0 { 1.0 } else { -1.0 };
        } else {
            let kappa = 2.0 * x / (x * x + z * z);
            let mut delta = (kappa * wheelbase).atan();
            // yaw-rate feedback: steer more if the car isn't rotating as asked (understeer)
            let yaw_right = -car.ang_vel.dot(car.up());
            let want_yaw = speed * kappa;
            delta += (want_yaw - yaw_right) * 0.12;
            // the car no longer counter-steers by itself: the AI does it, by keeping the
            // front tyres inside their peak-slip window around the front axle's direction
            // of travel (in a slide that means steering into it)
            if speed > 4.0 {
                let theta = car.front_travel_angle();
                let half = car.spec.peak_slip_angle * 0.95;
                delta = delta.clamp(theta - half, theta + half);
            }
            // invert the car's steering system (assists / slip-angle limiter included)
            steer = car.steer_input_for_angle(delta, true);
        }
        if car.telemetry.wheels_on_ground == 0 && car.telemetry.airtime > 0.05 {
            // keep the wheels straight in the air: land pointing where we're going
            steer = 0.0;
        }
        steer = steer.clamp(-1.0, 1.0);
        let _ = sf;

        // ---------------- speed
        let mut v_target = free_speed;
        // weather the speed plan doesn't know about: time-based weather (the plan was made
        // for the conditions at the start, so follow the grip there is *now*), standing water
        // (aquaplaning at the speed we'd like to carry) and slicks losing extra grip in the
        // wet / on ice. v ~ sqrt(mu) keeps the braking points right. Rally stages with rally
        // tyres are unaffected (factor 1).
        if !track.conditions.keys.is_empty() {
            let w = track.conditions.at(q.s);
            let slick = car.spec.tyre == crate::vehicle::TyreType::Slick;
            let mut ratio = 1.0f32;
            if track.conditions.is_timed() {
                let planned = line.grip[q.index % line.grip.len()].max(0.05);
                ratio = (track.road_grip_at(q.s, q.lateral) / planned).clamp(0.2, 1.05);
            }
            if w.water > 0.0 {
                ratio *= crate::weather::aquaplane_grip(w.water * 0.85, v_target.min(90.0), slick);
            }
            if slick {
                let g = track.ground_at(car.pos, Some(q.index));
                ratio *= crate::vehicle::tyre_surface_factor(car.spec.tyre, &g.props, w.wetness);
            }
            // and a touch of caution when they can barely see
            let murk = 1.0 - 0.05 * ((w.fog - 0.8) / 0.2).clamp(0.0, 1.0);
            v_target *= ratio.clamp(0.05, 1.05).sqrt() * murk;
        }
        // personality: slow wobble
        let wobble = (1.0 - p.consistency) * 0.035 * ((self.time * 0.31 + self.phase).sin() * 0.7 + (self.time * 0.83 + self.phase * 1.7).sin() * 0.3);
        let mut scale = 1.0 + wobble;
        // rubber band
        self.power_scale = self.base_power;
        self.debug.band = 0.0;
        if let Some(gap) = ctx.human_gap {
            if gap > 0.0 {
                let ease = smoothstep(6.0, 90.0, gap) * self.band.ease_ahead;
                scale *= 1.0 - 0.09 * ease;
                self.power_scale = self.base_power * (1.0 - 0.3 * ease);
                self.debug.band = -ease;
            } else {
                // a little push right behind too, so a close battle can turn into a pass
                let close = if -gap < 40.0 { 0.4 } else { 0.0 };
                let push = smoothstep(10.0, 120.0, -gap).max(close) * self.band.push_behind;
                scale *= 1.0 + 0.035 * push;
                self.power_scale = self.base_power * (1.0 + 0.15 * push);
                self.debug.band = push;
            }
        }
        // mistakes are rolled at the start of braking zones
        let braking_zone = v_target < speed - 2.5;
        if braking_zone && !self.was_braking && self.mistake == Mistake::None && self.mistake_cooldown <= 0.0 && !car.frozen {
            let chance = (1.0 - p.consistency) * 0.22 + (1.0 - p.skill) * 0.05;
            if self.rng.chance(chance) {
                self.mistake = match self.rng.range_i(0, 3) {
                    0 => Mistake::LateBrake,
                    1 => Mistake::Wide,
                    _ => Mistake::Lift,
                };
                self.mistake_timer = match self.mistake {
                    Mistake::LateBrake => 1.6,
                    Mistake::Wide => 2.4,
                    _ => 1.0,
                };
                self.stats.mistakes += 1;
            }
            self.mistake_cooldown = 4.0;
        }
        self.was_braking = braking_zone;
        if self.mistake != Mistake::None {
            self.mistake_timer -= dt;
            if self.mistake_timer <= 0.0 {
                self.mistake = Mistake::None;
            }
        }
        // running out of road on the outside: ease off and forget the mistake
        let edge = smp.half_width - 0.9;
        let lat_vel = car.vel.dot(smp.right);
        if q.lateral.abs() > edge && lat_vel * q.lateral.signum() > 0.4 && !off {
            scale *= 0.9;
            self.mistake = Mistake::None;
        }
        if self.mistake == Mistake::LateBrake {
            scale *= 1.04;
        }
        if passing {
            // commit to the move
            scale *= 1.0 + 0.03 * p.aggression;
        }
        // off the racing line the corners are tighter than planned: slow down for them
        if self.avoid.abs() > 0.8 {
            let c = line.max_curv_ahead(q.index, 25.0).abs();
            let tight = ((c - 1.0 / 80.0) / (1.0 / 20.0 - 1.0 / 80.0)).clamp(0.0, 1.0);
            scale *= 1.0 - 0.12 * tight * (self.avoid.abs() / 3.0).min(1.0);
        }
        v_target *= scale;
        if off {
            // gentle on grass/sand: get back onto the road first
            v_target = v_target.min(14.0 + 5.0 * p.skill);
        }
        v_target = v_target.min(follow_cap);

        let mut throttle;
        let mut brake = 0.0;
        let err = v_target - speed;
        if err > 0.8 {
            throttle = 1.0;
        } else if err > -0.6 {
            throttle = (0.35 + err * 0.5).clamp(0.08, 1.0);
        } else {
            throttle = 0.0;
            brake = (-err * 0.22).clamp(0.12, 1.0);
            if self.mistake == Mistake::LateBrake {
                brake *= 0.6;
            }
        }
        if emergency {
            throttle = 0.0;
            brake = brake.max(0.7);
        }
        if self.mistake == Mistake::Lift {
            throttle = throttle.min(0.45);
        }
        // corner exits: don't feed in full power while the tyres are still busy turning
        // (stops running wide out of hairpins)
        {
            let i = q.index;
            let n = line.curvature.len();
            let c = line.curvature[i].abs().max(line.curvature[(i + 2) % n].abs());
            let mu = self.mu_plan * line.grip[i];
            let lat_use = speed * speed * c / (mu * crate::vehicle::GRAVITY).max(0.5);
            if lat_use > 0.8 && throttle > 0.0 {
                throttle *= (1.0 - (lat_use - 0.8) * 2.5).clamp(0.35, 1.0);
            }
        }
        // own feel for traction on top of the car's TC: ease off in big slides
        let drift = car.telemetry.drift_angle.abs();
        if drift > 0.3 {
            throttle *= (1.0 - (drift - 0.3) * 1.5).clamp(0.35, 1.0);
        }
        if car.telemetry.wheelspin > 0.6 {
            throttle *= 0.8;
        }
        self.debug.target_speed = v_target;
        self.debug.line_offset = line_off_t;
        self.debug.avoid = self.avoid;
        self.debug.following = following;

        // ---------------- stuck recovery
        let mut reset = false;
        if !car.frozen {
            let wrong_way = heading_dot < -0.35 && speed < 7.0;
            let wait = if wrong_way { 0.8 } else { 2.2 };
            if self.recover_timer <= 0.0 && self.no_progress > wait {
                self.attempts += 1;
                self.stats.recoveries += 1;
                self.recover_timer = if wrong_way { 1.8 } else { 1.3 };
                // reversing turns the nose the other way round
                self.recover_steer = if steer >= 0.0 { -1.0 } else { 1.0 };
                self.no_progress = 0.0;
            }
            let lost = self.off_track > 8.0 && q.dist > smp.half_width + 10.0;
            if (self.attempts >= 4 || lost) && self.reset_cooldown <= 0.0 {
                reset = true;
                self.attempts = 0;
                self.off_track = 0.0;
                self.reset_cooldown = 4.0;
                self.recover_timer = 0.0;
                self.stats.resets += 1;
            }
        }
        if reset {
            self.last_s = None;
            self.avoid = 0.0;
            return CarInput { throttle: 0.0, brake: 0.0, steer: 0.0, handbrake: 0.0, reset: true, ai: true };
        }
        if self.recover_timer > 0.0 {
            self.recover_timer -= dt;
            if self.recover_timer <= 0.0 {
                // give it a chance to drive off before judging again
                self.no_progress = 0.0;
            }
            return CarInput { throttle: 0.0, brake: 1.0, steer: self.recover_steer, handbrake: 0.0, reset: false, ai: true };
        }
        if car.gear < 0 {
            // got into reverse somehow: drive forward
            throttle = throttle.max(0.6);
            brake = 0.0;
        }
        CarInput { throttle, brake, steer, handbrake: 0.0, reset: false, ai: true }
    }
}

/// Slipstream: a car tucked in 4..22 m behind another (laterally within ~1.6 m) gets up to
/// 35% less aero drag — for everyone, humans included. Makes straights tactical and passes
/// happen. Called from `drive_all` each frame.
pub fn apply_slipstream(world: &mut World) {
    let n = world.cars.len();
    for i in 0..n {
        let (pos, vel) = (world.cars[i].pos, world.cars[i].vel);
        let speed = vel.length();
        let mut best = 0.0f32;
        if speed > 15.0 {
            let dir = vel / speed;
            for j in 0..n {
                if j == i {
                    continue;
                }
                let d = world.cars[j].pos - pos;
                let along = d.dot(dir);
                if !(4.0..22.0).contains(&along) {
                    continue;
                }
                let lateral = (d - dir * along).length();
                if lateral < 1.6 {
                    let k = (1.0 - (along - 4.0) / 18.0) * (1.0 - lateral / 1.6).max(0.0).sqrt();
                    best = best.max(k);
                }
            }
        }
        let target = 1.0 - 0.35 * best;
        let c = &mut world.cars[i];
        c.drag_scale += (target - c.drag_scale) * 0.2;
    }
}

/// Run every AI in `ais` (index = car index) for this frame: inputs + catch-up power, and
/// the slipstream for all cars.
pub fn drive_all(ais: &mut [Option<AiDriver>], world: &mut World, dt: f32) {
    apply_slipstream(world);
    let n = ais.len().min(world.cars.len());
    let mut inputs = Vec::with_capacity(n);
    for (i, ai) in ais.iter_mut().enumerate().take(n) {
        inputs.push(ai.as_mut().map(|a| (a.drive_world(i, world, dt), a.power_scale)));
    }
    for (i, inp) in inputs.into_iter().enumerate() {
        if let Some((input, power)) = inp {
            world.set_input(i, input);
            world.cars[i].power_scale = power;
            // AI opponents drive with a touch more stability assist than Normal (mild TC)
            if world.controllers[i] == Controller::Ai {
                world.cars[i].assist_level = 1.15;
            }
        }
    }
}
