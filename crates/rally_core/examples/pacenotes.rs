//! Print the pace notes for a few seeds (sanity check against the curvature profile).
//! cargo run -p rally_core --release --example pacenotes -- [seed_from] [seed_to] [difficulty] [dump_dir]
//! With a dump dir, writes `<dir>/track_<seed>_<diff>.csv` (samples) and `notes_<seed>_<diff>.json`.
use rally_core::pacenotes::{notes_for, NoteKind};
use rally_core::track::{Track, TrackParams, DIFFICULTY_NAMES};
use std::collections::BTreeMap;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let from: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let to: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    let diff: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let dump = args.get(4).cloned();
    for seed in from..=to {
        let t = Track::generate(&TrackParams { seed, difficulty: diff, ..Default::default() });
        let t0 = std::time::Instant::now();
        let notes = notes_for(&t);
        let us = t0.elapsed().as_micros();
        println!("=== seed {seed} {} / {} ({:.0} m, {} notes, {us} us)", t.style.name, DIFFICULTY_NAMES[diff as usize], t.length, notes.notes.len());
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for nt in &notes.notes {
            let key = match nt.kind {
                NoteKind::Corner => format!("{}", nt.severity),
                k => format!("{k:?}"),
            };
            *kinds.entry(key).or_default() += 1;
            println!(
                "{:7.1}-{:7.1}  {:9} ang {:5.0} Rmin {:6.1} v {:5.0}km/h  | {}",
                nt.s_start,
                nt.s_end,
                format!("{:?}", nt.kind),
                nt.angle_deg,
                nt.min_radius,
                nt.speed_kmh,
                nt.text
            );
        }
        println!("   counts: {kinds:?}");
        if let Some(dir) = &dump {
            let mut f = std::fs::File::create(format!("{dir}/track_{seed}_{diff}.csv")).unwrap();
            writeln!(f, "s,x,z,y,curv,hw,bank,surface").unwrap();
            for s in &t.samples {
                writeln!(f, "{},{},{},{},{},{},{},{}", s.s, s.pos.x, s.pos.z, s.pos.y, s.curvature, s.half_width, s.bank, s.surface.name()).unwrap();
            }
            let mut f = std::fs::File::create(format!("{dir}/notes_{seed}_{diff}.tsv")).unwrap();
            for nt in &notes.notes {
                writeln!(f, "{}\t{}\t{:?}\t{}\t{}", nt.s_start, nt.s_end, nt.kind, nt.severity, nt.text).unwrap();
            }
        }
    }
}
