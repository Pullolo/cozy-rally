//! Headless playtest: generate tracks, race AI cars, report stats.
//! cargo run -p rally_core --release --example sim -- [seeds] [laps] [ai_count] [difficulty] [reference]
//!   difficulty: AI base skill (Chill 0.35, Normal 0.7, Spicy 1.0), default 0.7
//!   reference:  1 (default) = add a "human" stand-in driven by a plain autopilot (skill 0.85,
//!               traffic-aware, no catch-up) as Controller::Local, so rubber-banding is active and
//!               AI pace can be compared to it; 0 = AI only.
use rally_core::ai::{drive_all, AiDriver};
use rally_core::race::{format_time, RaceEvent, RacePhase};
use rally_core::rng::Rng;
use rally_core::track::{Track, TrackParams};
use rally_core::vehicle::car_specs;
use rally_core::world::{Controller, RacerSetup, World, WorldEvent, TICK};
use std::sync::Arc;
use std::time::Instant;

#[derive(Default)]
struct Totals {
    races: u32,
    dnf: u32,
    nan: u32,
    car_hits: u32,
    scenery_hits: u32,
    resets: u32,
    spread: f32,
    ref_place: f32,
    pace_ratio: f32,
    pace_n: u32,
    lead_changes: u32,
    off_track: f32,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seeds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let laps: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let n_ai: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5);
    let difficulty: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.7);
    let with_ref = args.get(5).map(|s| s != "0").unwrap_or(true);
    println!("{} AI at difficulty {difficulty}{}, {laps} laps", n_ai, if with_ref { " + reference autopilot (\"human\")" } else { "" });
    let mut tot = Totals::default();
    let watch: Option<usize> = std::env::var("SIM_WATCH").ok().and_then(|v| v.parse().ok());
    let only: Option<u64> = std::env::var("SIM_SEED").ok().and_then(|v| v.parse().ok());
    for seed in 1..=seeds {
        if only.is_some_and(|o| o != seed) {
            continue;
        }
        let t0 = Instant::now();
        let mut params = TrackParams { seed, biome: None, length_class: 1, ..Default::default() }.with_env_overrides();
        // SIM_WEATHER=clear|drizzle|rain|storm|fog|snowfall|dynamic
        if let Ok(w) = std::env::var("SIM_WEATHER") {
            use rally_core::weather::{ConditionsSpec, WeatherPreset as P};
            params.conditions = match w.as_str() {
                "drizzle" => ConditionsSpec::Fixed(P::Drizzle),
                "rain" => ConditionsSpec::Fixed(P::Rain),
                "storm" => ConditionsSpec::Fixed(P::Storm),
                "fog" => ConditionsSpec::Fixed(P::Fog),
                "snowfall" => ConditionsSpec::Fixed(P::Snowfall),
                "dynamic" => ConditionsSpec::Dynamic,
                _ => ConditionsSpec::Fixed(P::Clear),
            };
        }
        let track = Arc::new(Track::generate(&params));
        let gen_ms = t0.elapsed().as_millis();
        println!(
            "seed {seed}: {} ({:?}) len {:.0}m jumps {} gen {gen_ms}ms",
            track.style.name,
            track.style.road_surface,
            track.length,
            track.jumps.len()
        );
        let mut rng = Rng::new(seed ^ 0xA1);
        // SIM_CARS=circuit: circuit-class cars only (else rally class)
        let circuit = std::env::var("SIM_CARS").is_ok_and(|v| v == "circuit");
        let pool: Vec<usize> = car_specs().iter().enumerate().filter(|(_, s)| (s.class == rally_core::vehicle::CarClass::Circuit) == circuit).map(|(i, _)| i).collect();
        let mut racers: Vec<RacerSetup> = (0..n_ai).map(|_| RacerSetup { car_spec: pool[rng.range_i(0, pool.len() as i32) as usize], controller: Controller::Ai }).collect();
        let ref_idx = if with_ref {
            racers.push(RacerSetup { car_spec: pool[(seed as usize) % pool.len()], controller: Controller::Local(0) });
            Some(n_ai)
        } else {
            None
        };
        let n = racers.len();
        let mut world = World::new(track.clone(), &racers, laps);
        let mut ais: Vec<Option<AiDriver>> = (0..n).map(|i| if Some(i) == ref_idx { None } else { Some(AiDriver::racer(i, difficulty, seed)) }).collect();
        if std::env::var("SIM_NOBAND").is_ok() {
            for a in ais.iter_mut().flatten() {
                a.band = Default::default();
            }
        }
        let mut autopilot = AiDriver::new(0.85, 0.0);
        let t_plan = Instant::now();
        let predicted: Vec<f32> = (0..n)
            .map(|i| match ais[i].as_mut() {
                Some(a) => a.predicted_lap(&world.cars[i], &track),
                None => autopilot.predicted_lap(&world.cars[i], &track),
            })
            .collect();
        let plan_ms = t_plan.elapsed().as_secs_f32() * 1000.0;
        let mut resets = vec![0; n];
        let (mut car_hits, mut scenery_hits) = (0, 0);
        let mut max_speed = vec![0.0f32; n];
        let mut airtime_max = 0.0f32;
        let mut nan = false;
        let t1 = Instant::now();
        let mut ticks = 0u64;
        let mut leader = usize::MAX;
        let mut lead_changes = 0;
        let mut max_spin = 0.0f32;
        let (mut start_hits, mut hard_hits) = (0, 0);
        let (mut rear_hits, mut side_hits) = (0, 0);
        while world.race.phase != RacePhase::Finished && ticks < 120 * 60 * 12 {
            if let Some(r) = ref_idx {
                // the "human" looks out for other cars too (no catch-up for it)
                let inp = autopilot.drive_world(r, &world, TICK);
                world.set_input(r, inp);
            }
            drive_all(&mut ais, &mut world, TICK);
            world.tick();
            ticks += 1;
            if let Some(w) = watch.filter(|w| *w < n && ticks % 30 == 0 && (world.race.racers[*w].off_track || (world.race.is_racing() && world.cars[*w].vel.length() < 4.0))) {
                let c = &world.cars[w];
                let r = &world.race.racers[w];
                let d = ais[w].as_ref().map(|a| a.debug).unwrap_or_default();
                println!(
                    "      watch t{:6.1} s{:6.1} lat{:5.1} v{:5.1} vt{:5.1} beta{:5.1} up{:.2} g{} in thr{:.2} br{:.2} st{:5.2} avoid{:4.1} band{:4.2} spin{:.2}",
                    world.race.clock, r.s, r.lateral, c.vel.length(), d.target_speed, c.telemetry.drift_angle.to_degrees(), c.up().y, c.gear, c.input.throttle, c.input.brake, c.input.steer, d.avoid, d.band, c.telemetry.wheelspin
                );
            }
            for (i, c) in world.cars.iter().enumerate() {
                max_speed[i] = max_speed[i].max(c.vel.length());
                airtime_max = airtime_max.max(c.telemetry.airtime);
                max_spin = max_spin.max(c.telemetry.wheelspin);
                if !c.pos.is_finite() || !c.vel.is_finite() {
                    nan = true;
                }
            }
            if world.race.is_racing() && world.race.clock > 5.0 {
                let l = world.race.standings()[0];
                if leader != usize::MAX && l != leader {
                    lead_changes += 1;
                }
                leader = l;
            }
            for e in world.drain_events() {
                match e {
                    WorldEvent::Race(RaceEvent::Reset { car }) => resets[car] += 1,
                    WorldEvent::Collision { car, other: Some(o), strength, .. } => {
                        car_hits += 1;
                        let d = world.cars[o].pos - world.cars[car].pos;
                        let f = world.cars[car].forward();
                        if d.dot(world.cars[car].right()).abs() < 1.3 {
                            rear_hits += 1;
                        } else {
                            side_hits += 1;
                        }
                        let _ = f;
                        if world.race.clock < 12.0 {
                            start_hits += 1;
                        }
                        if strength > 4.0 {
                            hard_hits += 1;
                        }
                    }
                    WorldEvent::Collision { other: None, .. } => scenery_hits += 1,
                    _ => {}
                }
            }
            if nan {
                break;
            }
        }
        let wall = t1.elapsed().as_millis().max(1);
        println!(
            "   sim {:.1}s game time in {wall} ms ({}x realtime), plan {plan_ms:.1}ms, car-car hits {car_hits} (first 12s {start_hits}, hard >4m/s {hard_hits}, rear {rear_hits} side {side_hits}), scenery hits {scenery_hits}, lead changes {lead_changes}, max air {:.2}s, max wheelspin {:.2}, NaN {nan}",
            ticks as f32 / 120.0,
            (ticks as f32 / 120.0 * 1000.0 / wall as f32) as i32,
            airtime_max,
            max_spin
        );
        let finish: Vec<f32> = world.race.racers.iter().filter_map(|r| r.finish_time).collect();
        let dnf = world.race.racers.iter().filter(|r| r.finish_time.is_none()).count();
        let spread = if finish.len() > 1 { finish.iter().cloned().fold(f32::MIN, f32::max) - finish.iter().cloned().fold(f32::MAX, f32::min) } else { 0.0 };
        let ref_best = ref_idx.and_then(|r| world.race.racers[r].best_lap);
        for i in world.race.standings() {
            let r = &world.race.racers[i];
            let laps: Vec<String> = r.lap_times.iter().map(|t| format_time(*t)).collect();
            let who = if Some(i) == ref_idx {
                "REF  autopilot                     ".to_string()
            } else {
                let a = ais[i].as_ref().unwrap();
                let p = a.personality;
                format!("AI   sk {:.2} ag {:.2} co {:.2} mis {:2} rec {:2}", p.skill, p.aggression, p.consistency, a.stats.mistakes, a.stats.recoveries)
            };
            let off = ais[i].as_ref().map(|a| a.stats.off_track_time).unwrap_or(0.0);
            let pace = match (ref_best, r.best_lap) {
                (Some(rb), Some(b)) if Some(i) != ref_idx => format!(" pace {:+.1}%", (b / rb - 1.0) * 100.0),
                _ => String::new(),
            };
            println!(
                "   P{} #{i} {:8} {who} finish {:>9} laps [{}] pred {} resets {} off {:.1}s vmax {:.0}{pace}",
                r.place,
                world.cars[i].spec.name,
                r.finish_time.map(format_time).unwrap_or("DNF".into()),
                laps.join(", "),
                format_time(predicted[i]),
                resets[i],
                off,
                max_speed[i] * 3.6,
            );
            if Some(i) != ref_idx {
                if let (Some(rb), Some(b)) = (ref_best, r.best_lap) {
                    tot.pace_ratio += b / rb;
                    tot.pace_n += 1;
                }
                tot.off_track += off;
            }
        }
        println!("   finishing spread {spread:.1}s, DNF {dnf}");
        tot.races += 1;
        tot.dnf += dnf as u32;
        tot.nan += nan as u32;
        tot.car_hits += car_hits;
        tot.scenery_hits += scenery_hits;
        tot.resets += resets.iter().sum::<u32>();
        tot.spread += spread;
        tot.lead_changes += lead_changes;
        if let Some(r) = ref_idx {
            tot.ref_place += world.race.racers[r].place as f32;
        }
    }
    let r = tot.races.max(1) as f32;
    println!(
        "SUMMARY {} races: DNF {} NaN {} | avg spread {:.1}s | car-car hits/race {:.1} scenery hits/race {:.1} resets/race {:.1} | lead changes/race {:.1} | AI off-track {:.1}s/race{}",
        tot.races,
        tot.dnf,
        tot.nan,
        tot.spread / r,
        tot.car_hits as f32 / r,
        tot.scenery_hits as f32 / r,
        tot.resets as f32 / r,
        tot.lead_changes as f32 / r,
        tot.off_track / r,
        if with_ref { format!(" | reference avg place {:.1}/{} | AI best lap vs reference {:+.1}%", tot.ref_place / r, n_ai + 1, (tot.pace_ratio / tot.pace_n.max(1) as f32 - 1.0) * 100.0) } else { String::new() }
    );
    let _ = car_specs;
}
