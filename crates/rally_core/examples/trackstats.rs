//! Track generator statistics per difficulty (no driving).
//! cargo run -p rally_core --release --example trackstats -- [seeds] [length_class]
use rally_core::layout::FeatureKind;
use rally_core::track::{Track, TrackParams, DIFFICULTY_NAMES};
use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seeds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let lc: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    for diff in 0..3u8 {
        let mut counts: BTreeMap<String, f32> = BTreeMap::new();
        let (mut len_sum, mut min_r, mut max_ms, mut ms_sum) = (0.0, f32::MAX, 0u128, 0u128);
        let mut hw_min = f32::MAX;
        let mut surf: BTreeMap<String, f32> = BTreeMap::new();
        let mut jumps = 0;
        for seed in 1..=seeds {
            let t0 = Instant::now();
            let t = Track::generate(&TrackParams { seed, biome: None, length_class: lc, difficulty: diff, ..Default::default() }.with_env_overrides());
            let ms = t0.elapsed().as_millis();
            max_ms = max_ms.max(ms);
            ms_sum += ms;
            len_sum += t.length;
            jumps += t.jumps.len();
            for s in &t.samples {
                min_r = min_r.min(1.0 / s.curvature.abs().max(1e-6));
                hw_min = hw_min.min(s.half_width);
                *surf.entry(s.surface.name().to_string()).or_default() += 2.0 / seeds as f32;
            }
            for f in &t.features {
                if f.kind != FeatureKind::Straight {
                    *counts.entry(format!("{:?}", f.kind)).or_default() += 1.0 / seeds as f32;
                }
            }
        }
        println!(
            "{} ({}): avg len {:.0} m, min R {:.1} m, min width {:.1} m, jumps/track {:.1}, gen avg {} ms max {} ms",
            DIFFICULTY_NAMES[diff as usize],
            diff,
            len_sum / seeds as f32,
            min_r,
            hw_min * 2.0,
            jumps as f32 / seeds as f32,
            ms_sum / seeds as u128,
            max_ms
        );
        let c: Vec<String> = counts.iter().map(|(k, v)| format!("{k} {v:.1}")).collect();
        println!("   designed corners/track: {}", c.join(", "));
        let s: Vec<String> = surf.iter().map(|(k, v)| format!("{k} {v:.0}m")).collect();
        println!("   surface metres/track: {}", s.join(", "));
    }
}
