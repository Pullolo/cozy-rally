//! Reproducible render benchmark: `COZY_BENCH=1` flies a chase-style camera around the whole
//! track (one pose every `COZY_BENCH_STEP` samples, default 12 = 24 m), renders a few frames
//! per pose with synchronous per-section timing (see `perf`), then prints the worst areas and
//! a summary and exits. `COZY_BENCH_OUT=<file.csv>` also writes every pose.
//!
//! Combine with `COZY_AUTOPLAY=1 COZY_SEED=.. COZY_BIOME=..` to pick the track.
use super::perf::{FrameStats, SECS, SEC_NAMES};
use macroquad::math::Vec3;
use rally_core::track::Track;
use std::cell::RefCell;

const FRAMES_PER_POSE: usize = 4;
/// frames recorded per pose (the last ones; the first ones warm up caches/targets)
const RECORD: usize = 2;
/// frames to skip after the bench starts (loading, first-frame shader compiles)
const WARMUP: usize = 10;

#[derive(Clone, Debug)]
struct Row {
    sample: usize,
    pos: Vec3,
    total: f32,
    cpu: [f32; SECS],
    draws: u32,
    verts: u64,
    shadow_verts: u64,
    dyn_calls: u32,
    dyn_kb: u64,
}

#[derive(Clone, Debug, Default)]
struct Bin {
    frames: u32,
    sum: f32,
    worst: f32,
    worst_cpu: [f32; SECS],
    max_dyn_kb: u64,
    max_dyn_calls: u32,
    max_verts: u64,
    max_draws: u32,
}

struct Bench {
    /// drive mode: real race, frames binned by the player's track position
    drive: bool,
    bins: Vec<Bin>,
    cur_sample: usize,
    race_time: f32,
    time_limit: f32,
    poses: Vec<(usize, Vec3, Vec3)>,
    pose: usize,
    frame: usize,
    warm: usize,
    acc: Vec<FrameStats>,
    rows: Vec<Row>,
    label: String,
}

thread_local! {
    static BENCH: RefCell<Option<Bench>> = const { RefCell::new(None) };
}

pub fn enabled() -> bool {
    std::env::var("COZY_BENCH").is_ok()
}

/// Build the camera path for `track` (call when a race session starts).
pub fn setup(track: &Track, label: &str) {
    if !enabled() {
        return;
    }
    let step: usize = std::env::var("COZY_BENCH_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(12).max(1);
    let n = track.samples.len();
    let only: Vec<usize> = std::env::var("COZY_BENCH_ONLY").ok().map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
    let mut poses = Vec::new();
    for i in (0..n).step_by(step) {
        if !only.is_empty() && !only.contains(&i) {
            continue;
        }
        let s = &track.samples[i];
        let fwd = Vec3::new(s.tangent.x, 0.0, s.tangent.z).normalize_or_zero();
        let car = s.pos + Vec3::Y * 0.6;
        let mut eye = car - fwd * 6.8 + Vec3::Y * 2.4;
        eye.y = eye.y.max(track.visual_height(eye.x, eye.z) + 1.0);
        let ahead = &track.samples[(i + 5) % n];
        poses.push((i, eye, ahead.pos + Vec3::Y * 1.0));
    }
    let drive = std::env::var("COZY_BENCH").is_ok_and(|v| v == "drive");
    if drive {
        poses.clear();
    }
    let time_limit = std::env::var("COZY_BENCH_TIME").ok().and_then(|v| v.parse().ok()).unwrap_or(90.0);
    BENCH.with(|b| *b.borrow_mut() = Some(Bench { drive, bins: vec![Bin::default(); n / BIN + 1], cur_sample: 0, race_time: 0.0, time_limit, poses, pose: 0, frame: 0, warm: 0, acc: Vec::new(), rows: Vec::new(), label: label.to_string() }));
}

/// Samples per drive-mode bin (50 m)
const BIN: usize = 25;

/// Drive mode: the player's current track sample and race time
pub fn set_position(sample: usize, race_time: f32) {
    BENCH.with(|b| {
        if let Some(b) = b.borrow_mut().as_mut() {
            b.cur_sample = sample;
            b.race_time = race_time;
        }
    })
}

/// Camera pose to render this frame, if benchmarking
pub fn pose() -> Option<(Vec3, Vec3)> {
    BENCH.with(|b| b.borrow().as_ref().and_then(|b| b.poses.get(b.pose).map(|p| (p.1, p.2))))
}

/// Called by `perf::frame_end` with the completed frame's stats.
pub fn record(st: &FrameStats) {
    let finished = BENCH.with(|b| {
        let mut b = b.borrow_mut();
        let Some(bench) = b.as_mut() else { return false };
        if bench.warm < WARMUP {
            bench.warm += 1;
            return false;
        }
        if bench.drive {
            let total: f32 = st.cpu_ms.iter().sum();
            let bi = (bench.cur_sample / BIN).min(bench.bins.len() - 1);
            let bin = &mut bench.bins[bi];
            bin.frames += 1;
            bin.sum += total;
            if total > bin.worst {
                bin.worst = total;
                bin.worst_cpu = st.cpu_ms;
            }
            bin.max_dyn_kb = bin.max_dyn_kb.max(st.c.dyn_bytes / 1024);
            bin.max_dyn_calls = bin.max_dyn_calls.max(st.c.dyn_calls);
            bin.max_verts = bin.max_verts.max(st.c.verts);
            bin.max_draws = bin.max_draws.max(st.c.draws);
            return bench.race_time > bench.time_limit;
        }
        bench.frame += 1;
        if bench.frame > FRAMES_PER_POSE - RECORD {
            bench.acc.push(*st);
        }
        if bench.frame >= FRAMES_PER_POSE {
            let (sample, pos, _) = bench.poses[bench.pose];
            let k = bench.acc.len().max(1) as f32;
            let mut cpu = [0.0; SECS];
            for f in &bench.acc {
                for i in 0..SECS {
                    cpu[i] += f.cpu_ms[i] / k;
                }
            }
            let last = bench.acc.last().copied().unwrap_or_default();
            // the 3D part of the frame (update + render passes); HUD varies with the race state
            let total = cpu[0] + cpu[1] + cpu[2] + cpu[3] + cpu[4];
            bench.rows.push(Row { sample, pos, total, cpu, draws: last.c.draws, verts: last.c.verts, shadow_verts: last.c.shadow_verts, dyn_calls: last.c.dyn_calls, dyn_kb: last.c.dyn_bytes / 1024 });
            bench.acc.clear();
            bench.frame = 0;
            bench.pose += 1;
        }
        bench.pose >= bench.poses.len()
    });
    if finished {
        report();
        std::process::exit(0);
    }
}

fn report() {
    BENCH.with(|b| {
        let b = b.borrow();
        let Some(bench) = b.as_ref() else { return };
        if bench.drive {
            report_drive(bench);
            return;
        }
        let rows = &bench.rows;
        if rows.is_empty() {
            return;
        }
        let mut sorted: Vec<&Row> = rows.iter().collect();
        sorted.sort_by(|a, b| b.total.partial_cmp(&a.total).unwrap());
        let mean = rows.iter().map(|r| r.total).sum::<f32>() / rows.len() as f32;
        let median = sorted[sorted.len() / 2].total;
        println!("== bench {} : {} poses  mean {:.2} ms  median {:.2} ms  worst {:.2} ms", bench.label, rows.len(), mean, median, sorted[0].total);
        let hdr: Vec<String> = SEC_NAMES[..5].iter().map(|s| format!("{s:>7}")).collect();
        println!("  sample        pos         total {}   draws  verts(k) shadowv(k) stream(calls/KB)", hdr.join(""));
        for r in sorted.iter().take(8) {
            let secs: Vec<String> = r.cpu[..5].iter().map(|v| format!("{v:7.2}")).collect();
            println!("  {:6} ({:7.0},{:7.0}) {:7.2} {}  {:6} {:8} {:9}   {}/{}", r.sample, r.pos.x, r.pos.z, r.total, secs.join(""), r.draws, r.verts / 1000, r.shadow_verts / 1000, r.dyn_calls, r.dyn_kb);
        }
        let avg = |f: &dyn Fn(&Row) -> f32| rows.iter().map(f).sum::<f32>() / rows.len() as f32;
        println!(
            "  mean per section: update {:.2} shadow {:.2} scene {:.2} fx {:.2} post {:.2} | draws {:.0} verts {:.0}k shadow verts {:.0}k",
            avg(&|r| r.cpu[0]),
            avg(&|r| r.cpu[1]),
            avg(&|r| r.cpu[2]),
            avg(&|r| r.cpu[3]),
            avg(&|r| r.cpu[4]),
            avg(&|r| r.draws as f32),
            avg(&|r| r.verts as f32 / 1000.0),
            avg(&|r| r.shadow_verts as f32 / 1000.0)
        );
        if let Ok(path) = std::env::var("COZY_BENCH_OUT") {
            let mut s = String::from("sample,x,z,total,update,shadow,scene,fx,post,draws,verts,shadow_verts,dyn_calls,dyn_kb\n");
            for r in rows {
                s += &format!("{},{:.1},{:.1},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{},{},{}\n", r.sample, r.pos.x, r.pos.z, r.total, r.cpu[0], r.cpu[1], r.cpu[2], r.cpu[3], r.cpu[4], r.draws, r.verts, r.shadow_verts, r.dyn_calls, r.dyn_kb);
            }
            let _ = std::fs::write(&path, s);
        }
    });
}

fn report_drive(bench: &Bench) {
    let mut bins: Vec<(usize, &Bin)> = bench.bins.iter().enumerate().filter(|(_, b)| b.frames > 0).collect();
    let frames: u32 = bins.iter().map(|b| b.1.frames).sum();
    let mean = bins.iter().map(|b| b.1.sum).sum::<f32>() / frames.max(1) as f32;
    bins.sort_by(|a, b| b.1.worst.partial_cmp(&a.1.worst).unwrap());
    println!("== bench drive {} : {} frames over {} areas (50 m)  mean {:.2} ms  worst frame {:.2} ms", bench.label, frames, bins.len(), mean, bins.first().map_or(0.0, |b| b.1.worst));
    println!("  area(m)  frames   mean   worst | worst frame: update shadow scene fx post hud | max draws verts(k) stream calls/KB");
    for (i, b) in bins.iter().take(10) {
        let c = b.worst_cpu;
        println!(
            "  {:6}  {:6} {:7.2} {:7.2} | {:6.2} {:6.2} {:6.2} {:6.2} {:6.2} {:6.2} | {:5} {:6} {:4}/{}",
            i * BIN * 2,
            b.frames,
            b.sum / b.frames as f32,
            b.worst,
            c[0], c[1], c[2], c[3], c[4], c[5],
            b.max_draws,
            b.max_verts / 1000,
            b.max_dyn_calls,
            b.max_dyn_kb
        );
    }
}
