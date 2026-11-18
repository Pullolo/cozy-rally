//! AI races per track difficulty with aggregate stats (corner mix from pace notes, min radius,
//! lap spreads, resets). cargo run -p rally_core --release --example stagesim -- [seeds] [laps] [surface 0..4] [diffs e.g. 012] [biome 0..5]
use rally_core::ai::AiDriver;
use rally_core::biome::ALL_BIOMES;
use rally_core::pacenotes::{notes_for, NoteKind};
use rally_core::race::{RaceEvent, RacePhase};
use rally_core::surfacemap::SURFACE_MODES;
use rally_core::track::{Track, TrackParams, DIFFICULTY_NAMES};
use rally_core::world::{Controller, RacerSetup, World, WorldEvent};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seeds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let laps: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let surface = SURFACE_MODES[args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0usize).min(4)];
    let diffs: Vec<u8> = args.get(4).map(|s| s.bytes().map(|b| b - b'0').collect()).unwrap_or(vec![0, 1, 2]);
    let biome = args.get(5).and_then(|s| s.parse::<usize>().ok()).filter(|&b| b > 0).map(|b| ALL_BIOMES[b - 1]);
    let verbose = std::env::var("VERBOSE").is_ok();
    for &diff in &diffs {
        let mut kinds: BTreeMap<String, f32> = BTreeMap::new();
        let (mut min_r, mut len_sum, mut gen_max) = (f32::MAX, 0.0, 0u128);
        let (mut resets, mut dnf, mut nan, mut races) = (0usize, 0usize, false, 0usize);
        let mut spreads = Vec::new();
        let mut best_laps = Vec::new();
        let mut per_seed_resets = Vec::new();
        let mut changes = 0usize;
        for seed in 1..=seeds {
            let t0 = Instant::now();
            let track = Arc::new(Track::generate(&TrackParams { seed, biome, length_class: 1, difficulty: diff, surface }));
            gen_max = gen_max.max(t0.elapsed().as_millis());
            len_sum += track.length;
            changes += track.surface_map.transitions.len() + track.surface_map.patches.len();
            for s in &track.samples {
                min_r = min_r.min(1.0 / s.curvature.abs().max(1e-6));
            }
            let notes = notes_for(&track);
            for nt in &notes.notes {
                let key = match nt.kind {
                    NoteKind::Corner => format!("{}", nt.severity),
                    k => format!("{k:?}"),
                };
                *kinds.entry(key).or_default() += 1.0 / seeds as f32;
            }
            let racers: Vec<RacerSetup> = (0..4).map(|i| RacerSetup { car_spec: i, controller: Controller::Ai }).collect();
            let mut world = World::new(track.clone(), &racers, laps);
            let mut ais: Vec<AiDriver> = (0..4).map(|i| AiDriver::new(0.9, (i as f32 - 1.5) * 1.2)).collect();
            let mut ticks = 0u64;
            let mut r_here = 0;
            while world.race.phase != RacePhase::Finished && ticks < 120 * 60 * 10 {
                for i in 0..4 {
                    let inp = ais[i].drive(&world.cars[i], &world.track, rally_core::world::TICK);
                    world.set_input(i, inp);
                }
                world.tick();
                ticks += 1;
                if world.cars.iter().any(|c| !c.pos.is_finite()) {
                    nan = true;
                    break;
                }
                for e in world.drain_events() {
                    if let WorldEvent::Race(RaceEvent::Reset { .. }) = e {
                        r_here += 1;
                    }
                }
            }
            races += 1;
            resets += r_here;
            per_seed_resets.push(r_here);
            let fin: Vec<f32> = world.race.racers.iter().filter_map(|r| r.finish_time).collect();
            dnf += 4 - fin.len();
            if fin.len() >= 2 {
                let lo = fin.iter().cloned().fold(f32::MAX, f32::min);
                let hi = fin.iter().cloned().fold(0.0, f32::max);
                spreads.push(hi - lo);
            }
            if let Some(b) = world.race.racers.iter().filter_map(|r| r.best_lap).fold(None, |a: Option<f32>, b| Some(a.map_or(b, |a| a.min(b)))) {
                best_laps.push(b);
            }
            if verbose {
                println!("  seed {seed} {} {:.0}m resets {r_here} finished {} best lap {:?}", track.style.name, track.length, fin.len(), best_laps.last());
            }
        }
        let avg = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
        println!(
            "{} [{}]: {races} races, avg len {:.0} m, min R {:.1} m, gen max {gen_max} ms, surface changes/track {:.1}",
            DIFFICULTY_NAMES[diff as usize],
            surface.name(),
            len_sum / seeds as f32,
            min_r,
            changes as f32 / seeds as f32
        );
        let k: Vec<String> = kinds.iter().map(|(k, v)| format!("{k}:{v:.1}")).collect();
        println!("   notes/track: {}", k.join(" "));
        println!(
            "   best lap avg {:.1}s, finish spread avg {:.1}s (max {:.1}s), resets/race {:.2} (per seed {:?}), DNF {dnf}, NaN {nan}",
            avg(&best_laps),
            avg(&spreads),
            spreads.iter().cloned().fold(0.0, f32::max),
            resets as f32 / races as f32,
            per_seed_resets
        );
    }
}
