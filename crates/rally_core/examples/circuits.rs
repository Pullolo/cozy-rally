//! Circuit check: generates the named circuits, prints layout stats and (with `drive`) runs
//! an AI race on each. cargo run --release -p rally_core --example circuits -- [drive] [id-filter]
use rally_core::ai::AiDriver;
use rally_core::circuits::circuits;
use rally_core::layout::FeatureKind;
use rally_core::pacenotes::{notes_for, NoteKind};
use rally_core::race::{format_time, RaceEvent, RacePhase};
use rally_core::track::{RunoffKind, Track, SAMPLE_SPACING};
use rally_core::world::{Controller, RacerSetup, World, WorldEvent, TICK};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let drive = args.iter().any(|a| a == "drive");
    let filter = args.iter().skip(1).find(|a| *a != "drive").cloned();
    for c in circuits() {
        if filter.as_ref().is_some_and(|f| !c.id.contains(f.as_str())) {
            continue;
        }
        let mut params = c.params.clone();
        if let Some(seed) = std::env::var("SEED").ok().and_then(|v| v.parse().ok()) {
            params.seed = seed;
        }
        let t0 = Instant::now();
        let track = Arc::new(Track::generate(&params));
        let gen_ms = t0.elapsed().as_millis();
        let len = track.length;
        let mut straights: Vec<f32> = track.features.iter().filter(|f| f.kind == FeatureKind::Straight).map(|f| f.len(len)).collect();
        straights.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for f in track.features.iter().filter(|f| f.kind != FeatureKind::Straight) {
            *kinds.entry(format!("{:?}", f.kind)).or_default() += 1;
        }
        let notes = notes_for(&track);
        let mut speeds: Vec<f32> = notes.notes.iter().filter(|n| matches!(n.kind, NoteKind::Corner | NoteKind::Hairpin | NoteKind::Square)).map(|n| n.speed_kmh).collect();
        speeds.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let grades: Vec<String> = notes.notes.iter().filter(|n| n.is_corner()).map(|n| match n.kind {
            NoteKind::Corner => n.severity.to_string(),
            NoteKind::Hairpin => "H".into(),
            NoteKind::Square => "Sq".into(),
            _ => "K".into(),
        }).collect();
        let n = track.samples.len();
        let kerb = (0..n).filter(|&i| { let e = track.edge(i); e.kerb_left > 0.0 || e.kerb_right > 0.0 }).count() as f32 / n as f32;
        let (mut gravel, mut tarmac) = (0.0, 0.0);
        for i in 0..n {
            let e = track.edge(i);
            for r in [e.runoff_left, e.runoff_right] {
                if r.width > 3.0 {
                    match r.kind {
                        RunoffKind::Gravel => gravel += SAMPLE_SPACING,
                        RunoffKind::Tarmac => tarmac += SAMPLE_SPACING,
                        _ => {}
                    }
                }
            }
        }
        let ys: Vec<f32> = track.samples.iter().map(|s| s.pos.y).collect();
        let (ymin, ymax) = ys.iter().fold((f32::MAX, f32::MIN), |a, &y| (a.0.min(y), a.1.max(y)));
        let widths = track.samples.iter().fold((f32::MAX, f32::MIN), |a, s| (a.0.min(s.half_width * 2.0), a.1.max(s.half_width * 2.0)));
        println!(
            "{:24} {:24} {:>6.0} m  gen {gen_ms} ms  terrain {}x{} ({:.1} MB)  props {}  width {:.1}-{:.1} m  elev {:.0} m range",
            c.id, c.name, len, track.terrain.nx, track.terrain.nz, track.terrain.heights.len() as f32 * 4e-6, track.props.len(), widths.0, widths.1, ymax - ymin
        );
        println!("   straights (m): {:?}", straights.iter().take(4).map(|s| s.round()).collect::<Vec<_>>());
        println!("   corners: {kinds:?}");
        println!("   ref corner speeds km/h: min {:.0} median {:.0} max {:.0}   grades: {}", speeds.first().unwrap_or(&0.0), speeds.get(speeds.len() / 2).unwrap_or(&0.0), speeds.last().unwrap_or(&0.0), grades.join(" "));
        println!("   kerbs on {:.0}% of samples, run-off: gravel {:.0} m, tarmac {:.0} m (both sides)", kerb * 100.0, gravel, tarmac);
        if !drive {
            continue;
        }
        let laps = if track.length > 7000.0 { 1 } else { 2 };
        let racers: Vec<RacerSetup> = (0..4).map(|i| RacerSetup { car_spec: i, controller: Controller::Ai }).collect();
        let mut world = World::new(track.clone(), &racers, laps);
        let mut ais: Vec<AiDriver> = (0..4).map(|i| AiDriver::new(0.9, (i as f32 - 1.5) * 1.2)).collect();
        let mut ticks = 0u64;
        let mut resets = 0;
        let mut vmax = 0.0f32;
        let mut nan = false;
        while world.race.phase != RacePhase::Finished && ticks < 120 * 60 * 20 {
            for i in 0..4 {
                let inp = ais[i].drive(&world.cars[i], &world.track, TICK);
                world.set_input(i, inp);
            }
            world.tick();
            ticks += 1;
            for car in &world.cars {
                vmax = vmax.max(car.vel.length());
                nan |= !car.pos.is_finite();
            }
            if nan {
                break;
            }
            for e in world.drain_events() {
                if let WorldEvent::Race(RaceEvent::Reset { .. }) = e {
                    resets += 1;
                }
            }
        }
        let best = world.race.racers.iter().filter_map(|r| r.best_lap).fold(f32::MAX, f32::min);
        let dnf = world.race.racers.iter().filter(|r| r.finish_time.is_none()).count();
        println!("   AI ({laps} laps, rally cars on tarmac): best lap {} avg {:.0} km/h, vmax {:.0} km/h, resets {resets}, DNF {dnf}, NaN {nan}", format_time(best), len / best * 3.6, vmax * 3.6);
    }
}
