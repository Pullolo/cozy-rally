//! Surface map inspection: sections, patches, and grip continuity along the road.
//! cargo run -p rally_core --release --example surfaces -- [seed] [surface 0..4] [biome 1..5] [difficulty]
use rally_core::biome::ALL_BIOMES;
use rally_core::glam::Vec3;
use rally_core::pacenotes::notes_for;
use rally_core::surfacemap::SURFACE_MODES;
use rally_core::track::{Track, TrackParams};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let seed: u64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let mode = SURFACE_MODES[a.get(2).and_then(|s| s.parse().ok()).unwrap_or(4usize).min(4)];
    let biome = a.get(3).and_then(|s| s.parse::<usize>().ok()).filter(|&b| b > 0).map(|b| ALL_BIOMES[b - 1]);
    let diff: u8 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(1);
    let t = Track::generate(&TrackParams { seed, biome, difficulty: diff, surface: mode, ..Default::default() });
    let m = &t.surface_map;
    println!("{} {:?} len {:.0} base {:?} (AI main {:?})", t.style.name, mode, t.length, m.base, t.style.road_surface);
    for tr in &m.transitions {
        println!("  transition at {:7.1} over {:4.1} m: {:?} -> {:?}", tr.s, tr.len, tr.from, tr.to);
    }
    for p in &m.patches {
        println!("  patch {:?} at {:7.1} (+-{:.1} m) lat {:.1} half-width {:.1}", p.surface, p.s, p.half_len, p.lat, p.half_width);
    }
    // grip continuity: walk the centre line (and two lateral offsets) in 0.25 m steps
    let mut worst = 0.0f32;
    let mut worst_at = (0.0, 0.0);
    for lat in [-2.5f32, 0.0, 2.0] {
        let mut prev: Option<f32> = None;
        let mut hint = None;
        let steps = (t.length / 0.25) as usize;
        for k in 0..steps {
            let s = k as f32 * 0.25;
            let (p, _, r, _) = t.frame_at(s);
            let g = t.ground_at(p + r * lat + Vec3::Y * 0.1, hint);
            hint = Some(g.query.index);
            if let Some(pg) = prev {
                let d = (g.props.grip - pg).abs();
                if d > worst {
                    worst = d;
                    worst_at = (s, lat);
                }
            }
            prev = Some(g.props.grip);
        }
    }
    println!("  max grip change per 0.25 m step: {:.4} (at s {:.1}, lat {:.1})", worst, worst_at.0, worst_at.1);
    if let Some(tr) = m.transitions.first() {
        println!("  grip through the first transition (centre line):");
        let mut line = String::new();
        for k in -4..=((tr.len / 2.0) as i32 + 6) {
            let s = tr.s + k as f32 * 2.0;
            let (p, _, _, _) = t.frame_at(s);
            let g = t.ground_at(p + Vec3::Y * 0.1, None);
            line += &format!(" {:.0}:{:.3}", k * 2, g.props.grip);
        }
        println!("   {line}");
    }
    let notes = notes_for(&t);
    for nt in notes.notes.iter().filter(|n| n.surface_change().is_some()) {
        println!("  note at {:7.1}: {}", nt.s_start, nt.text);
    }
}
