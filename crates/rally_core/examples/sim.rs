//! Headless playtest: generate tracks, race AI cars, report stats.
//! cargo run -p rally_core --release --example sim -- [seeds] [laps]
use rally_core::ai::AiDriver;
use rally_core::race::{format_time, RaceEvent, RacePhase};
use rally_core::track::{Track, TrackParams};
use rally_core::world::{Controller, RacerSetup, World, WorldEvent};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seeds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let laps: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    for seed in 1..=seeds {
        let t0 = Instant::now();
        let track = Arc::new(Track::generate(&TrackParams { seed, biome: None, length_class: 1, ..Default::default() }.with_env_overrides()));
        let gen_ms = t0.elapsed().as_millis();
        let max_jump = track.samples.windows(2).map(|w| (w[1].pos.y - w[0].pos.y).abs()).fold(0.0f32, f32::max);
        println!(
            "seed {seed}: {} len {:.0}m samples {} terrain {}x{} props {} jumps {} water {:?} gen {gen_ms}ms maxdy {:.2}",
            track.style.name, track.length, track.samples.len(), track.terrain.nx, track.terrain.nz, track.props.len(), track.jumps.len(), track.water_level.map(|w| w as i32), max_jump
        );
        let racers: Vec<RacerSetup> = (0..4).map(|i| RacerSetup { car_spec: i, controller: Controller::Ai }).collect();
        let mut world = World::new(track.clone(), &racers, laps);
        let mut ais: Vec<AiDriver> = (0..4).map(|i| AiDriver::new(0.9, (i as f32 - 1.5) * 1.2)).collect();
        let mut resets = vec![0; 4];
        let mut collisions = 0;
        let mut max_speed = vec![0.0f32; 4];
        let mut airtime_max = 0.0f32;
        let mut nan = false;
        let t1 = Instant::now();
        let mut ticks = 0u64;
        while world.race.phase != RacePhase::Finished && ticks < 120 * 60 * 8 {
            for i in 0..4 {
                let inp = ais[i].drive(&world.cars[i], &world.track, rally_core::world::TICK);
                world.set_input(i, inp);
            }
            world.tick();
            ticks += 1;
            for (i, c) in world.cars.iter().enumerate() {
                max_speed[i] = max_speed[i].max(c.vel.length());
                airtime_max = airtime_max.max(c.telemetry.airtime);
                if !c.pos.is_finite() {
                    nan = true;
                }
            }
            for e in world.drain_events() {
                match e {
                    WorldEvent::Race(RaceEvent::Reset { car }) => resets[car] += 1,
                    WorldEvent::Collision { .. } => collisions += 1,
                    _ => {}
                }
            }
            if nan {
                break;
            }
        }
        println!("   sim {:.1}s game time in {} ms ({}x realtime), collisions {collisions}, max air {:.2}s, NaN {nan}", ticks as f32 / 120.0, t1.elapsed().as_millis(), (ticks as f32 / 120.0 * 1000.0 / t1.elapsed().as_millis().max(1) as f32) as i32, airtime_max);
        for i in world.race.standings() {
            let r = &world.race.racers[i];
            let laps: Vec<String> = r.lap_times.iter().map(|t| format_time(*t)).collect();
            println!(
                "   P{} {:8} finish {:>9} laps [{}] resets {} vmax {:.0} km/h lap {} cp {}",
                r.place,
                world.cars[i].spec.name,
                r.finish_time.map(format_time).unwrap_or("DNF".into()),
                laps.join(", "),
                resets[i],
                max_speed[i] * 3.6,
                r.lap,
                r.next_cp
            );
        }
    }
}
