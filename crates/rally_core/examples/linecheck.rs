//! Print racing line / speed profile around an arc length: linecheck seed s_from s_to [spec]
use rally_core::racing_line::{RacingLine, SpeedParams, SpeedProfile};
use rally_core::track::{Track, TrackParams, SAMPLE_SPACING};
use rally_core::vehicle::car_specs;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let g = |i: usize, d: f32| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let track = Track::generate(&TrackParams { seed: g(1, 1.0) as u64, biome: None, length_class: 1, ..Default::default() }.with_env_overrides());
    let line = RacingLine::compute(&track, rally_core::racing_line::LINE_MARGIN);
    let spec = &car_specs()[g(4, 1.0) as usize];
    let p = SpeedProfile::build(&line, &SpeedParams::for_car(spec, 1.0, 0.9));
    println!("jumps at s {:?}", track.jumps.iter().map(|&j| j as f32 * SAMPLE_SPACING).collect::<Vec<_>>());
    let (from, to) = (g(2, 0.0), g(3, 100.0));
    let mut s = from;
    while s < to {
        let i = (s / SAMPLE_SPACING) as usize % line.offset.len();
        println!("s{:6.0} curv {:7.4} R {:6.0} vcurv {:7.4} hw {:4.1} off {:5.2} lim {:5.1} pred {:5.1} jump {}", s, line.curvature[i], 1.0 / line.curvature[i].abs().max(1e-4), line.vcurv[i], track.samples[i].half_width, line.offset[i], p.limit[i], p.predicted[i], line.jump_zone[i]);
        s += 4.0;
    }
}
