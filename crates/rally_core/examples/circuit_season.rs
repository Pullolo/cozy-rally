//! Headless run of the circuit campaign: every event is generated and driven.
//! * time trials: the reference run (medal times) must finish.
//! * races: 5 AI rivals at the event's skill + a "player" autopilot (skill from arg 1,
//!   default 0.85) starting from the back, like the game does. Reports the player's place,
//!   the field spread, resets, NaN and the weather at the finish.
//! cargo run --release -p rally_core --example circuit_season -- [player_skill] [event id filter]
use rally_core::ai::{drive_all, AiDriver};
use rally_core::campaign::{circuit_campaign, EventKind};
use rally_core::campaign_run::{medal_times, reference_run};
use rally_core::race::{format_time, RaceEvent, RacePhase};
use rally_core::track::Track;
use rally_core::world::{Controller, RacerSetup, World, WorldEvent, TICK};
use std::sync::Arc;

fn main() {
    let skill: f32 = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.85);
    let filter = std::env::args().nth(2).unwrap_or_default();
    let mut bad = 0;
    for ch in circuit_campaign() {
        println!("== {} (needs {} golds)", ch.name, ch.unlock_golds);
        for ev in ch.stages.iter().filter(|e| e.id.contains(&filter)) {
            let t0 = std::time::Instant::now();
            match ev.kind {
                EventKind::TimeTrial => match reference_run(ev) {
                    Some(r) => {
                        let m = medal_times(r.time);
                        println!("  {:32} TT {} laps  ref {}  gold {}  resets {}  ({:.1}s)", ev.id, ev.laps, format_time(r.time), format_time(m.gold), r.resets, t0.elapsed().as_secs_f32());
                    }
                    None => {
                        bad += 1;
                        println!("  {:32} TT  REFERENCE RUN FAILED", ev.id);
                    }
                },
                EventKind::Race { rivals, skill: ai_skill } => {
                    let track = Arc::new(Track::generate(&ev.params));
                    let car = ev.cars[0] as usize;
                    let mut racers: Vec<RacerSetup> = (0..rivals).map(|_| RacerSetup { car_spec: car, controller: Controller::Ai }).collect();
                    racers.push(RacerSetup { car_spec: car, controller: Controller::Local(0) });
                    let n = racers.len();
                    let me = n - 1;
                    let mut world = World::new(track.clone(), &racers, ev.laps);
                    let mut ais: Vec<Option<AiDriver>> = (0..n).map(|i| if i == me { None } else { Some(AiDriver::racer(i, ai_skill, ev.params.seed)) }).collect();
                    let mut pilot = AiDriver::new(skill, 0.0);
                    let mut resets = vec![0u32; n];
                    let mut nan = false;
                    let mut ticks = 0u64;
                    let mut margin = 0.0f32;
                    while world.race.phase != RacePhase::Finished && ticks < 120 * 60 * 40 {
                        let inp = pilot.drive_world(me, &world, TICK);
                        world.set_input(me, inp);
                        drive_all(&mut ais, &mut world, TICK);
                        world.tick();
                        ticks += 1;
                        nan |= world.cars.iter().any(|c| !c.pos.is_finite());
                        if std::env::var("WATCH").is_ok() && ticks % 24 == 0 && world.race.racers[0].off_track {
                            let c = &world.cars[0];
                            let d = ais[0].as_ref().map(|a| a.debug).unwrap_or_default();
                            println!("      off ai0 t {:6.1} s {:7.1} lat {:5.1} v {:5.1} vt {:5.1} beta {:6.1} thr {:.2} br {:.2} st {:5.2} spin {:.2}", world.race.clock, world.race.racers[0].s, world.race.racers[0].lateral, c.vel.length(), d.target_speed, c.telemetry.drift_angle.to_degrees(), c.input.throttle, c.input.brake, c.input.steer, c.telemetry.wheelspin);
                        }
                        for e in world.drain_events() {
                            match e {
                                WorldEvent::Race(RaceEvent::Reset { car }) => {
                                    resets[car] += 1;
                                    if std::env::var("WATCH").is_ok() && car == 0 {
                                        let d = ais[0].as_ref().map(|a| a.debug).unwrap_or_default();
                                        println!("      reset ai0 t {:6.1} s {:7.1} lap {} vt {:5.1}", world.race.clock, world.race.racers[0].s, world.race.racers[0].lap, d.target_speed);
                                    }
                                }
                                WorldEvent::Race(RaceEvent::Finish { car, place: 2, time }) if car != me && world.race.racers[me].place == 1 => {
                                    margin = time - world.race.racers[me].finish_time.unwrap_or(time);
                                }
                                _ => {}
                            }
                        }
                    }
                    let place = world.race.racers[me].place;
                    if place == 1 && world.race.racers.iter().filter(|r| r.finished).count() == 1 {
                        margin = 99.0;
                    }
                    let fin: Vec<f32> = world.race.racers.iter().filter_map(|r| r.finish_time).collect();
                    let dnf = n - fin.len();
                    let spread = fin.iter().cloned().fold(0.0, f32::max) - fin.iter().cloned().fold(f32::MAX, f32::min);
                    let w = world.track.weather_at(0.0);
                    let medal = ev.race_medal(place, margin);
                    let ai_resets: u32 = resets[..me].iter().sum();
                    println!(
                        "  {:32} RACE {} laps ai {:.2} | you P{} {:?} margin {:4.1}s | winner {} spread {:5.1}s DNF {} | resets ai {} you {} | end wx wet {:.2} water {:.2} fog {:.2} ice {:.2} | NaN {} ({:.1}s)",
                        ev.id, ev.laps, ai_skill, place, medal, margin, fin.iter().cloned().fold(f32::MAX, f32::min).min(9999.0), spread, dnf, ai_resets, resets[me], w.wetness, w.water, w.fog, w.ice, nan, t0.elapsed().as_secs_f32()
                    );
                    if nan || dnf > 1 {
                        bad += 1;
                    }
                }
            }
        }
    }
    println!("problems: {bad}");
}
