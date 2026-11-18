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
        let first: u64 = std::env::var("SEED_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        for seed in first..=seeds {
            let t0 = Instant::now();
            let track = Arc::new(Track::generate(&TrackParams { seed, biome, length_class: 1, difficulty: diff, surface, ..Default::default() }));
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
            let mut last_s = vec![0.0f32; 4];
            let mut slow: Vec<BTreeMap<i32, f32>> = vec![BTreeMap::new(); 4];
            while world.race.phase != RacePhase::Finished && ticks < 120 * 60 * 10 {
                for i in 0..4 {
                    let inp = ais[i].drive(&world.cars[i], &world.track, rally_core::world::TICK);
                    world.set_input(i, inp);
                }
                for i in 0..4 {
                    last_s[i] = world.race.racers[i].s;
                }
                world.tick();
                ticks += 1;
                if let Ok(tc) = std::env::var("TRACE_CAR") {
                    let i: usize = tc.parse().unwrap_or(0);
                    let r = &world.race.racers[i];
                    let (a, b): (f32, f32) = std::env::var("TRACE_S").ok().and_then(|v| { let p: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect(); Some((p[0], p[1])) }).unwrap_or((0.0, 1e9));
                    if ticks % 15 == 0 && r.s >= a && r.s <= b && r.lap == 1 {
                        let c = &world.cars[i];
                        let smp = &world.track.samples[r.track_index];
                        println!("t {:.2} s {:.0} v {:.1} lat {:.1} hw {:.1} k {:.4} bank {:.3} surf {:?} drift {:.2} yaw {:.2} in {:.2}/{:.2}/{:.2}", world.race.clock, r.s, c.vel.length(), r.lateral, smp.half_width, smp.curvature, smp.bank, smp.surface, c.telemetry.drift_angle, c.ang_vel.y, c.input.throttle, c.input.brake, c.input.steer);
                    }
                }
                if world.race.is_racing() && world.race.clock > 3.0 {
                    for i in 0..4 {
                        if world.cars[i].vel.length() < 4.0 && !world.race.racers[i].finished {
                            let e = slow[i].entry((world.race.racers[i].s / 20.0) as i32 * 20).or_default();
                            *e += rally_core::world::TICK;
                            if std::env::var("DEBUG_STUCK").is_ok() && (*e - 6.0).abs() < 0.005 {
                                let c = &world.cars[i];
                                let q = world.track.query(c.pos, None);
                                let mut near = Vec::new();
                                world.track.for_each_prop_near(c.pos, 6.0, |p| near.push(format!("{:?} d{:.1} r{:.1}", p.kind, (p.pos - c.pos).length(), p.radius)));
                                let (_, nrm) = world.track.terrain.sample(c.pos.x, c.pos.z);
                                println!("STUCK car {i} s {:.0} dist {:.1} lat {:.1} hw {:.1} up {:.2} gear {} wheels {} terrain_n.y {:.2} input {:?} props {:?}", q.s, q.dist, q.lateral, world.track.samples[q.index].half_width, c.up().y, c.gear, c.telemetry.wheels_on_ground, nrm.y, c.input, near);
                            }
                        }
                    }
                }
                if world.cars.iter().any(|c| !c.pos.is_finite()) {
                    nan = true;
                    break;
                }
                for e in world.drain_events() {
                    if let WorldEvent::Race(RaceEvent::Reset { car }) = e {
                        r_here += 1;
                        if verbose {
                            let up = rally_core::pacenotes::upcoming(&notes, last_s[car] + 40.0, 1);
                            let prev = up.first().map(|u| (u.index + notes.notes.len() - 1) % notes.notes.len()).map(|i| notes.notes[i].text.clone()).unwrap_or_default();
                            println!("     reset car {car} {} from s {:.0} (t {:.1}) near note [{prev}]", world.cars[car].spec.name, last_s[car], world.race.clock);
                        }
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
                for i in 0..4 {
                    let r = &world.race.racers[i];
                    let worst: Vec<String> = { let mut v: Vec<(&i32, &f32)> = slow[i].iter().filter(|(_, t)| **t > 1.5).collect(); v.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap()); v.iter().take(3).map(|(s, t)| format!("s{s}:{t:.0}s")).collect() };
                    println!("     car {i} {} fin {:?} laps {:?} lap {} s {:.0} slow {:?}", world.cars[i].spec.name, r.finish_time, r.lap_times, r.lap, r.s, worst);
                }
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
