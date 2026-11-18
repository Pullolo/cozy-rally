//! Campaign check: generates all campaign stages, prints layout / conditions stats, and (with
//! `drive`) lets the AI autopilot run every stage.
//! cargo run --release -p rally_core --example campaign -- [drive] [stage-id-filter]
use rally_core::ai::AiDriver;
use rally_core::campaign::campaign;
use rally_core::layout::FeatureKind;
use rally_core::pacenotes::{notes_for, NoteKind};
use rally_core::race::{format_time, RaceEvent, RacePhase};
use rally_core::track::Track;
use rally_core::world::{Controller, RacerSetup, World, WorldEvent, TICK};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let drive = args.iter().any(|a| a == "drive");
    let filter = args.iter().skip(1).find(|a| *a != "drive").cloned();
    let car: usize = std::env::var("CAR").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let mut total_resets = 0;
    let mut dnf = 0;
    for ch in campaign() {
        println!("=== {} ({:?})", ch.name, ch.biome);
        for st in &ch.stages {
            if let Some(f) = &filter {
                if !st.id.contains(f.as_str()) {
                    continue;
                }
            }
            let t0 = Instant::now();
            let mut params = st.params.clone();
            if let Some(seed) = std::env::var("SEED").ok().and_then(|v| v.parse().ok()) {
                params.seed = seed;
            }
            let track = Arc::new(Track::generate(&params));
            let gen_ms = t0.elapsed().as_millis();
            let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
            for f in &track.features {
                if f.kind != FeatureKind::Straight {
                    *kinds.entry(format!("{:?}", f.kind)).or_default() += 1;
                }
            }
            let notes = notes_for(&track);
            if let Ok(dir) = std::env::var("DUMP") {
                use std::io::Write;
                let mut f = std::fs::File::create(format!("{dir}/track_{}.csv", st.id)).unwrap();
                writeln!(f, "s,x,z,y,curv,hw,bank,surface").unwrap();
                for s in &track.samples {
                    writeln!(f, "{},{},{},{},{},{},{},{}", s.s, s.pos.x, s.pos.z, s.pos.y, s.curvature, s.half_width, s.bank, s.surface.name()).unwrap();
                }
                let mut f = std::fs::File::create(format!("{dir}/notes_{}.tsv", st.id)).unwrap();
                for nt in &notes.notes {
                    writeln!(f, "{}\t{}\t{:?}\t{}\t{}", nt.s_start, nt.s_end, nt.kind, nt.severity, nt.text).unwrap();
                }
                let mut f = std::fs::File::create(format!("{dir}/wx_{}.csv", st.id)).unwrap();
                let mut s = 0.0;
                while s < track.length {
                    let w = track.conditions.at(s);
                    writeln!(f, "{},{},{},{}", s, w.precipitation, w.wetness, w.fog).unwrap();
                    s += 20.0;
                }
            }
            if std::env::var("NOTES").is_ok() {
                for nt in &notes.notes {
                    println!("      {:6.2} km  {:9} | {}", (nt.s_start - track.start_s) / 1000.0, format!("{:?}", nt.kind), nt.text);
                }
            }
            let mut nk: BTreeMap<String, usize> = BTreeMap::new();
            for n in &notes.notes {
                let k = match n.kind {
                    NoteKind::Corner => format!("{}", n.severity),
                    k => format!("{k:?}"),
                };
                *nk.entry(k).or_default() += 1;
            }
            let ys: Vec<f32> = track.samples.iter().map(|s| s.pos.y).collect();
            let (ymin, ymax) = ys.iter().fold((f32::MAX, f32::MIN), |a, &y| (a.0.min(y), a.1.max(y)));
            let climb: f32 = ys.windows(2).map(|w| (w[1] - w[0]).max(0.0)).sum();
            println!(
                "{:28} {:>5.0} m stage ({:.0} m road), gen {gen_ms} ms, terrain {}x{} ({:.1} MB), props {}, jumps {}, elev {:.0}..{:.0} m (climb {:.0} m), surface changes {}",
                st.id,
                track.stage_length(),
                track.length,
                track.terrain.nx,
                track.terrain.nz,
                track.terrain.heights.len() as f32 * 4.0 / 1e6,
                track.props.len(),
                track.jumps.len(),
                ymin,
                ymax,
                climb,
                track.surface_map.transitions.len() + track.surface_map.patches.len()
            );
            println!("   designed: {kinds:?}");
            println!("   notes: {nk:?}");
            let ev: Vec<String> = track.weather_events.iter().map(|e| format!("{:?} {:.1}-{:.1} km", e.2, (e.0 - track.start_s) / 1000.0, (e.1 - track.start_s).min(track.stage_length()) / 1000.0)).collect();
            println!("   weather: [{}]", ev.join(", "));
            let mut tl = String::new();
            let steps = 10;
            for k in 0..=steps {
                let s = track.start_s + track.stage_length() * k as f32 / steps as f32;
                let w = track.conditions.at(s);
                tl += &format!(" {:.1}km:p{:.1}/w{:.1}/f{:.1}{}", (s - track.start_s) / 1000.0, w.precipitation, w.wetness, w.fog, if w.snow { "*" } else { "" });
            }
            println!("   timeline:{tl}");
            if !drive {
                continue;
            }
            // ---- AI autopilot run
            let mut world = World::new(track.clone(), &[RacerSetup { car_spec: car, controller: Controller::Ai }], 1);
            let mut ai = AiDriver::new(0.9, 0.0);
            let mut ticks = 0u64;
            let mut resets = 0;
            let mut nan = false;
            let mut reset_at = Vec::new();
            while world.race.phase != RacePhase::Finished && ticks < 120 * 60 * 20 {
                let inp = ai.drive(&world.cars[0], &world.track, TICK);
                world.set_input(0, inp);
                world.tick();
                ticks += 1;
                if !world.cars[0].pos.is_finite() {
                    nan = true;
                    break;
                }
                for e in world.drain_events() {
                    if let WorldEvent::Race(RaceEvent::Reset { .. }) = e {
                        resets += 1;
                        reset_at.push(format!("{:.2}km", (world.race.racers[0].s - track.start_s) / 1000.0));
                    }
                }
            }
            let r = &world.race.racers[0];
            total_resets += resets;
            if r.finish_time.is_none() {
                dnf += 1;
            }
            let splits: Vec<String> = r.split_times.iter().map(|t| format_time(*t)).collect();
            println!(
                "   AI {}: {} splits [{}] resets {resets} {:?} avg {:.0} km/h NaN {nan}",
                world.cars[0].spec.name,
                r.finish_time.map(format_time).unwrap_or_else(|| format!("DNF at {:.0}%", r.progress * 100.0)),
                splits.join(", "),
                reset_at,
                r.finish_time.map(|t| track.stage_length() / t * 3.6).unwrap_or(0.0)
            );
        }
    }
    if drive {
        println!("TOTAL resets {total_resets}, DNF {dnf}");
    }
}
