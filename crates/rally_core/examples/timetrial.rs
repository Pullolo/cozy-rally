//! Headless time-trial check: an autopilot drives laps with a ghost recorder; prints lap
//! times, ghost updates and the live delta.
//! cargo run --release -p rally_core --example timetrial -- [seed] [laps] [out.bin] [car] [lane]
//! (writes the best lap as a ghost file usable by the game: ghost_s{seed}_b0_l1.bin)
use rally_core::ai::AiDriver;
use rally_core::ghost::{GhostLap, TimeTrial};
use rally_core::track::{Track, TrackParams};
use rally_core::world::{Controller, RacerSetup, World, TICK};
use std::sync::Arc;
fn main() {
    let seed: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let laps: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let track = Arc::new(Track::generate(&TrackParams { seed, biome: None, length_class: 1, ..Default::default() }.with_env_overrides()));
    let mut world = World::new(track.clone(), &[RacerSetup { car_spec: std::env::args().nth(4).and_then(|s| s.parse().ok()).unwrap_or(2), controller: Controller::Local(0) }], laps);
    let mut tt = TimeTrial::new(&[0], None);
    // vary the driver a little per lap so laps differ
    let mut ai = AiDriver::new(0.8, std::env::args().nth(5).and_then(|s| s.parse().ok()).unwrap_or(0.0));
    let mut frame = 0u32;
    let mut lap_seen = 0;
    let mut deltas = Vec::new();
    while !world.race.racers[0].finished && frame < 120 * 600 {
        let lap = world.race.racers[0].lap;
        ai.skill = 0.8;
        let mut inp = ai.drive(&world.cars[0], &world.track, TICK * 2.0);
        if lap == 3 {
            inp.throttle *= 0.9; // a deliberately slower lap
        }
        world.set_input(0, inp);
        world.update(TICK * 2.0); // 60 fps frames
        tt.update(&world);
        frame += 1;
        if let Some(d) = tt.deltas[0] {
            if frame % 600 == 0 {
                deltas.push(format!("{:+.2}", d));
            }
        }
        if world.race.racers[0].lap_times.len() > lap_seen {
            lap_seen = world.race.racers[0].lap_times.len();
            let (t, vs) = tt.last_lap[0].unwrap();
            println!("lap {lap_seen}: {:.3}s vs ghost {:?} new best {} ghost {:.3}s frames {} | deltas during lap [{}]", t, vs.map(|v| (v * 1000.0).round() / 1000.0), tt.new_best, tt.best.as_ref().unwrap().time, tt.best.as_ref().unwrap().frames.len(), deltas.join(" "));
            tt.new_best = false;
            deltas.clear();
        }
        if frame == 120 * 20 {
            if let Some((p, _)) = tt.ghost_pose(&world, 0) {
                println!("  ghost pose sample at 40s: {:?} car {:?}", p, world.cars[0].pos);
            }
        }
    }
    let g = tt.best.unwrap();
    let bytes = g.to_bytes();
    let back = GhostLap::from_bytes(&bytes).unwrap();
    if let Some(out) = std::env::args().nth(3) {
        std::fs::write(&out, &bytes).expect("write ghost");
        println!("wrote {out}");
    }
    println!("ghost blob {} bytes, roundtrip ok {}", bytes.len(), back.time == g.time && back.frames.len() == g.frames.len() && back.frames.iter().zip(&g.frames).all(|(a, b)| a.pos == b.pos && a.t == b.t && a.rot.dot(b.rot) > 0.9999));
}
