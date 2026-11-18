//! Does driving well pay off? Lap times for four drivers on the same tracks (one car, no AI).
//! Driver "dare" is the AI planner's grip usage (calibrated optimum ~1.2 of the tyre mu).
//!   clean-pad : near-optimal: good line, brakes before corners, uses all the grip (analogue)
//!   clean-kbd : the same decisions through a keyboard (digital keys + the game's steer ramp)
//!   cautious  : safe but timid (uses ~90% of grip)
//!   sloppy    : keyboard, over-drives every corner (+25% grip demand), brakes late, panics
//!               with full brake mid-corner now and then, saws at the wheel
//! Reports best lap, total time, on-road spins (>60 deg slip above 8 m/s), off-track seconds.
//! cargo run --release -p rally_core --example skilltest -- [seeds] [assist] [car]
use rally_core::ai::{AiDriver, Personality, RubberBand};
use rally_core::race::{format_time, RacePhase};
use rally_core::rng::Rng;
use rally_core::track::{Track, TrackParams};
use rally_core::world::{Controller, RacerSetup, World, TICK};
use std::sync::Arc;

const NAMES: [&str; 4] = ["clean-pad", "clean-kbd", "cautious", "sloppy"];

/// The game's keyboard steering ramp (input.rs)
fn key_ramp(cur: &mut f32, target: f32, dt: f32) {
    let rate = if target == 0.0 || target.signum() != cur.signum() { 7.5 } else { 1.8 + 3.2 * cur.abs() };
    *cur += (target - *cur).clamp(-rate * dt, rate * dt);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let seeds: u64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(6);
    let assist: f32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let car: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
    if let Some(sk) = std::env::var("SKILL_SWEEP").ok() {
        // analogue autopilot at several "dare" levels: where is the optimum?
        for skill in sk.split(',').filter_map(|v| v.parse::<f32>().ok()) {
            let (mut best_sum, mut off_sum, mut spins) = (0.0, 0.0, 0);
            for seed in 1..=seeds {
                let track = Arc::new(Track::generate(&TrackParams { seed, biome: None, length_class: 1 }));
                let mut world = World::new(track.clone(), &[RacerSetup { car_spec: car, controller: Controller::Local(0) }], 3);
                world.cars[0].assist_level = assist;
                let mut ai = AiDriver::with_personality(Personality { skill, aggression: 1.0, consistency: 1.0 }, RubberBand::default(), seed);
                let mut spinning = false;
                let mut frames = 0;
                while world.race.phase != RacePhase::Finished && frames < 60 * 400 {
                    frames += 1;
                    let inp = ai.drive(&world.cars[0], &world.track, TICK * 2.0);
                    world.set_input(0, inp);
                    world.update(TICK * 2.0);
                    let c = &world.cars[0];
                    let sp = c.telemetry.drift_angle.abs() > 1.05 && c.vel.length() > 3.0;
                    spins += (sp && !spinning) as u32;
                    spinning = sp;
                    if world.race.racers[0].off_track && world.race.is_racing() {
                        off_sum += TICK * 2.0;
                    }
                }
                best_sum += world.race.racers[0].best_lap.unwrap_or(99.0);
            }
            println!("skill {skill:4.2}: avg best lap {:6.2}s off-track {:4.1}s/race spins {spins}", best_sum / seeds as f32, off_sum / seeds as f32);
        }
        return;
    }
    println!("=== SKILL TEST: assist {assist}, car {car}, 3 laps per track");
    let mut totals = [(0.0f32, 0.0f32, 0u32, 0.0f32, 0u32); 4];
    for seed in 1..=seeds {
        let track = Arc::new(Track::generate(&TrackParams { seed, biome: None, length_class: 1 }));
        let mut line = format!("seed {seed:2} {:9}", format!("{:?}", track.style.road_surface));
        for (d, name) in NAMES.iter().enumerate() {
            let mut world = World::new(track.clone(), &[RacerSetup { car_spec: car, controller: Controller::Local(0) }], 3);
            world.cars[0].assist_level = assist;
            // grip use = 0.72 + 0.48 * skill: clean 1.0 -> 1.20, cautious 0.35 -> 0.89, sloppy 1.6 -> 1.49
            let skill = [1.0, 1.0, 0.35, 1.6][d];
            let mut ai = AiDriver::with_personality(Personality { skill, aggression: 1.0, consistency: 1.0 }, RubberBand::default(), seed);
            let mut rng = Rng::new(seed * 77 + d as u64);
            let (mut steer, mut panic_t, mut spins, mut spinning, mut off) = (0.0f32, 0.0f32, 0u32, false, 0.0f32);
            let dt = TICK * 2.0; // 60 fps
            let mut frames = 0;
            while world.race.phase != RacePhase::Finished && frames < 60 * 400 {
                frames += 1;
                let mut inp = ai.drive(&world.cars[0], &world.track, dt);
                if d == 1 || d == 3 {
                    // keyboard: digital keys through the game's ramp
                    let key = if inp.steer > 0.2 { 1.0 } else if inp.steer < -0.2 { -1.0 } else { 0.0 };
                    key_ramp(&mut steer, key, dt);
                    inp.steer = steer;
                    inp.throttle = if inp.throttle > 0.3 { 1.0 } else { 0.0 };
                    inp.brake = if inp.brake > 0.08 { 1.0 } else { 0.0 };
                }
                if d == 3 {
                    // panic brake mid-corner now and then
                    let c = &world.cars[0];
                    if panic_t <= 0.0 && steer.abs() > 0.6 && c.vel.length() > 15.0 && rng.chance(0.004) {
                        panic_t = 0.35;
                    }
                    if panic_t > 0.0 {
                        panic_t -= dt;
                        inp.brake = 1.0;
                        inp.throttle = 0.0;
                    }
                    // sawing at the wheel
                    if rng.chance(0.01) {
                        steer = -steer * 0.5;
                    }
                }
                world.set_input(0, inp);
                world.update(dt);
                let c = &world.cars[0];
                // a spin on the road (not manoeuvring on the grass to rejoin)
                let s = c.telemetry.drift_angle.abs() > 60f32.to_radians() && c.vel.length() > 8.0 && !world.race.racers[0].off_track;
                if s && !spinning {
                    spins += 1;
                    if std::env::var("SKILL_DEBUG").is_ok() {
                        let r = &world.race.racers[0];
                        println!("      spin {name} seed {seed} t {:.1} s {:.0} lat {:.1} off {} v {:.1} panic {:.2} steer {:.2}", world.race.clock, r.s, r.lateral, r.off_track, c.vel.length(), panic_t, steer);
                    }
                }
                spinning = s;
                if world.race.racers[0].off_track && world.race.is_racing() {
                    off += dt;
                }
            }
            let r = &world.race.racers[0];
            let best = r.best_lap.unwrap_or(f32::NAN);
            let total = r.finish_time.unwrap_or(f32::NAN);
            line += &format!(" | {name:9} best {} total {} spins {spins} off {off:4.1}s", format_time(best), format_time(total));
            let t = &mut totals[d];
            t.0 += best;
            t.1 += total;
            t.2 += spins;
            t.3 += off;
            t.4 += r.finish_time.is_none() as u32;
        }
        println!("{line}");
    }
    let n = seeds as f32;
    let base = totals[0].0;
    for (d, name) in NAMES.iter().enumerate() {
        let t = totals[d];
        println!("{name:9}: avg best lap {:6.2}s ({:+.1}% vs clean-pad) | avg total {:6.1}s | spins {} | off-track {:.1}s/race | DNF {}", t.0 / n, (t.0 / base - 1.0) * 100.0, t.1 / n, t.2, t.3 / n, t.4);
    }
}
