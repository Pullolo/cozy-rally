//! Offline audio render + numeric checks (no device needed).
//!
//!   cargo run --release -p rally_game --example audio_render -- [out_dir]
//!
//! Renders engine sweeps for every car, traction-loss scenes, impacts, race cues, the finish
//! fanfare, 20 s of each music mood, a mood crossfade and a short real simulated race through
//! the same bridge the game uses; writes WAVs and prints/validates statistics
//! (NaN/inf, peak, RMS, DC, silences, spectral tracking of the engine firing frequency).
#![allow(dead_code)]

#[path = "../src/audio/synth/mod.rs"]
mod synth;
#[path = "../src/audio/bridge.rs"]
mod bridge;

use rally_core::glam::Vec3;
use synth::*;

const SR: f32 = 48000.0;

struct Clip {
    l: Vec<f32>,
    r: Vec<f32>,
}

impl Clip {
    fn new() -> Clip {
        Clip { l: Vec::new(), r: Vec::new() }
    }
    fn secs(&self) -> f32 {
        self.l.len() as f32 / SR
    }
    fn mono(&self) -> Vec<f32> {
        self.l.iter().zip(&self.r).map(|(a, b)| 0.5 * (a + b)).collect()
    }
}

/// Render `secs`, calling `ctl(synth, t)` once per block before rendering it.
fn render(s: &mut Synth, clip: &mut Clip, secs: f32, mut ctl: impl FnMut(&mut Synth, f32)) {
    let total = (secs * SR) as usize;
    let mut done = 0;
    while done < total {
        let n = (total - done).min(BLOCK);
        ctl(s, done as f32 / SR);
        s.render_block(n);
        clip.l.extend_from_slice(&s.out_l[..n]);
        clip.r.extend_from_slice(&s.out_r[..n]);
        done += n;
    }
}

fn write_wav(path: &std::path::Path, c: &Clip) {
    let mut data = Vec::with_capacity(c.l.len() * 4 + 44);
    let n = c.l.len() as u32 * 4;
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + n).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&(SR as u32).to_le_bytes());
    data.extend_from_slice(&(SR as u32 * 4).to_le_bytes());
    data.extend_from_slice(&4u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&n.to_le_bytes());
    for (a, b) in c.l.iter().zip(&c.r) {
        data.extend_from_slice(&((a.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        data.extend_from_slice(&((b.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, data).expect("write wav");
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Goertzel magnitude (normalised) of frequency f in x
fn goertzel(x: &[f32], f: f32) -> f32 {
    let w = std::f32::consts::TAU * f / SR;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    let n = x.len();
    for (i, &v) in x.iter().enumerate() {
        // Hann window
        let h = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
        let s0 = v * h + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / n as f32
}

struct Checks {
    fails: Vec<String>,
}

impl Checks {
    fn check(&mut self, ok: bool, what: String) {
        println!("    [{}] {}", if ok { " ok " } else { "FAIL" }, what);
        if !ok {
            self.fails.push(what);
        }
    }
}

/// Generic sanity: finite, peak, RMS, DC, longest silence
fn basic(ck: &mut Checks, name: &str, c: &Clip, rms_range: (f32, f32), max_silence: Option<f32>, pre_peak: f32) {
    let bad = c.l.iter().chain(&c.r).filter(|v| !v.is_finite()).count();
    let peak = c.l.iter().chain(&c.r).fold(0.0f32, |m, v| m.max(v.abs()));
    let m = c.mono();
    let r = rms(&m);
    let dc_l = c.l.iter().sum::<f32>() / c.l.len() as f32;
    let dc_r = c.r.iter().sum::<f32>() / c.r.len() as f32;
    // longest run of 50 ms windows below -60 dBFS
    let win = (SR * 0.05) as usize;
    let (mut run, mut longest) = (0usize, 0usize);
    for w in m.chunks(win) {
        if rms(w) < 0.001 {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let silence = longest as f32 * 0.05;
    println!(
        "  {name}: {:.1}s peak {:.3} ({:.1} dBFS) pre-limiter {:.3}  RMS {:.4} ({:.1} dBFS)  DC {:+.5}/{:+.5}  longest silence {:.2}s",
        c.secs(),
        peak,
        db(peak),
        pre_peak,
        r,
        db(r),
        dc_l,
        dc_r,
        silence
    );
    ck.check(bad == 0, format!("{name}: no NaN/inf ({bad} bad samples)"));
    ck.check(peak < 1.0, format!("{name}: peak {peak:.3} < 1.0"));
    ck.check(r >= rms_range.0 && r <= rms_range.1, format!("{name}: RMS {r:.4} in [{}, {}]", rms_range.0, rms_range.1));
    ck.check(dc_l.abs() < 0.005 && dc_r.abs() < 0.005, format!("{name}: |DC| < 0.005"));
    if let Some(ms) = max_silence {
        ck.check(silence <= ms, format!("{name}: longest silence {silence:.2}s <= {ms}s"));
    }
}

fn engine_params(kind: u8) -> EngineParams {
    let (idle, red) = match kind {
        0 => (900.0, 7200.0),
        1 => (850.0, 7000.0),
        2 => (900.0, 7200.0),
        _ => (750.0, 5600.0),
    };
    EngineParams {
        id: 0,
        kind,
        rpm: idle,
        idle_rpm: idle,
        redline: red,
        gain: 1.0,
        doppler: 1.0,
        local: true,
        ground: 1.0,
        s_hard: 1.0,
        ..Default::default()
    }
}

fn cars_cmd(p: EngineParams) -> Cmd {
    let mut cars = [EngineParams::default(); MAX_CARS];
    cars[0] = p;
    Cmd::Cars { n: 1, cars, paused: false }
}

/// Engine sweep timeline: rpm/throttle/shifting at time t
fn sweep_state(p: &mut EngineParams, t: f32) {
    let (idle, red) = (p.idle_rpm, p.redline);
    p.shifting = false;
    if t < 1.0 {
        p.rpm = idle;
        p.throttle = 0.0;
        p.load = 0.0;
    } else if t < 4.0 {
        let u = (t - 1.0) / 3.0;
        p.rpm = idle + (red - idle) * u;
        p.throttle = 1.0;
        p.load = 1.0 - 0.3 * p.rpm / red;
    } else if t < 4.8 {
        // on the limiter
        p.rpm = red + 20.0;
        p.throttle = 1.0;
        p.load = 0.0;
    } else if t < 5.0 {
        // upshift: revs drop, power cut
        p.rpm = red * 0.7;
        p.throttle = 1.0;
        p.load = 0.0;
        p.shifting = true;
    } else {
        // lift off: revs fall to idle
        let u = ((t - 5.0) / 0.6).min(1.0);
        p.rpm = red * 0.7 + (idle - red * 0.7) * u;
        p.throttle = 0.0;
        p.load = 0.0;
    }
}

fn engine_sweeps(ck: &mut Checks, dir: &std::path::Path) {
    let names = ["pip", "bramble", "thistle", "burrow"];
    let cyls = [3.0, 8.0, 4.0, 4.0];
    println!("\n== engine sweeps (6 s: idle 1s, full-throttle sweep to redline 3s, limiter 0.8s, shift 0.2s, lift 1s) ==");
    for kind in 0..4u8 {
        let mut s = Synth::new(SR);
        let mut clip = Clip::new();
        let mut p = engine_params(kind);
        let mut log: Vec<(f32, f32)> = Vec::new();
        render(&mut s, &mut clip, 6.0, |s, t| {
            sweep_state(&mut p, t);
            s.handle(cars_cmd(p));
            log.push((t, p.rpm));
        });
        let name = format!("engine_{}", names[kind as usize]);
        write_wav(&dir.join(format!("{name}.wav")), &clip);
        basic(ck, &name, &clip, (0.02, 0.3), None, s.pre_peak);
        let m = clip.mono();
        let cyl = cyls[kind as usize];
        // spectral tracking during the sweep: strongest harmonic of the cycle frequency
        let win = 4096;
        let mut hits = 0;
        let mut total = 0;
        let mut rows = Vec::new();
        let mut t = 1.15;
        while t + win as f32 / SR < 3.95 {
            let i0 = (t * SR) as usize;
            let x = &m[i0..i0 + win];
            let tc = t + win as f32 / SR * 0.5;
            let p_rpm = log.iter().min_by(|a, b| (a.0 - tc).abs().total_cmp(&(b.0 - tc).abs())).unwrap().1;
            let f0 = p_rpm / 120.0;
            let ffire = f0 * cyl;
            // strongest harmonic order in 1..=2*cyl (all orders that are physically present)
            let mut best = (0.0, 0usize);
            for k in 1..=(2.0 * cyl) as usize {
                let mg = goertzel(x, f0 * k as f32);
                if mg > best.0 {
                    best = (mg, k);
                }
            }
            if best.1 == cyl as usize {
                hits += 1;
            }
            total += 1;
            rows.push(format!("{:.0}rpm/{:.0}Hz:o{}", p_rpm, ffire, best.1));
            t += 0.35;
        }
        println!("    spectrum: {}", rows.join(" | "));
        ck.check(
            hits * 10 >= total * 7,
            format!("{name}: firing frequency (order {cyl}) is the strongest engine order in {hits}/{total} sweep windows"),
        );
        // steady state: the spectrum is harmonic to the cycle frequency and the firing
        // frequency stands well clear of the in-between (non-harmonic) frequencies
        let mut proms = Vec::new();
        for frac in [0.15f32, 0.4, 0.65, 0.9] {
            let mut s2 = Synth::new(SR);
            let mut c2 = Clip::new();
            let mut q = engine_params(kind);
            q.rpm = q.idle_rpm + (q.redline - q.idle_rpm) * frac;
            q.throttle = 0.6;
            q.load = 0.6;
            render(&mut s2, &mut c2, 1.5, |s, _| s.handle(cars_cmd(q)));
            let x = &c2.mono()[(0.4 * SR) as usize..(1.5 * SR) as usize];
            let f0 = q.rpm / 120.0;
            let ff = f0 * cyl;
            let at = goertzel(x, ff);
            let between = (goertzel(x, ff - 0.5 * f0) + goertzel(x, ff + 0.5 * f0)) * 0.5;
            proms.push((q.rpm, db(at) - db(between)));
        }
        let pmin = proms.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        ck.check(
            pmin > 15.0,
            format!(
                "{name}: steady-state firing peak above in-between frequencies: {}",
                proms.iter().map(|(r, d)| format!("{r:.0}rpm {d:+.0}dB")).collect::<Vec<_>>().join(", ")
            ),
        );
        // loudness rises with rpm+load
        let r_idle = rms(&m[(0.3 * SR) as usize..(0.95 * SR) as usize]);
        let r_high = rms(&m[(3.3 * SR) as usize..(3.95 * SR) as usize]);
        ck.check(r_high > r_idle * 1.5, format!("{name}: high-rpm full load {:.1} dB louder than idle", db(r_high) - db(r_idle)));
        // limiter bounce: envelope modulation at ~14 Hz
        let hop = (SR * 0.004) as usize;
        let env: Vec<f32> = m[(4.15 * SR) as usize..(4.8 * SR) as usize].chunks(hop).map(rms).collect();
        let env_mean = env.iter().sum::<f32>() / env.len() as f32;
        let env_ac: Vec<f32> = env.iter().map(|v| v - env_mean).collect();
        let er = 1.0 / 0.004;
        let mag = |f: f32| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in env_ac.iter().enumerate() {
                let ph = std::f32::consts::TAU * f * i as f32 / er;
                re += v * ph.cos();
                im += v * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        let m14 = mag(14.0);
        let m_ref = (mag(5.0) + mag(9.0) + mag(21.0) + mag(30.0)) / 4.0;
        ck.check(m14 > 3.0 * m_ref, format!("{name}: rev-limiter bounce at 14 Hz is {:.1} dB above other modulation rates", db(m14) - db(m_ref)));
        // shift dip
        let r_before = rms(&m[(4.5 * SR) as usize..(4.75 * SR) as usize]);
        let r_shift = rms(&m[(4.88 * SR) as usize..(4.98 * SR) as usize]);
        ck.check(r_shift < r_before, format!("{name}: gear-shift dip {:.1} dB", db(r_shift) - db(r_before)));
        if kind == 2 {
            // turbo whistle: narrow tone at ~4.4 kHz on full boost (high rpm, full load)
            let x = &m[(3.5 * SR) as usize..(3.5 * SR) as usize + 8192];
            let mut best = (0.0, 0.0);
            let mut f = 3000.0;
            while f < 5000.0 {
                let g = goertzel(x, f);
                if g > best.0 {
                    best = (g, f);
                }
                f += 10.0;
            }
            let side = (goertzel(x, best.1 - 150.0) + goertzel(x, best.1 + 150.0)) * 0.5;
            ck.check(db(best.0) - db(side) > 12.0, format!("thistle: turbo whistle at {:.0} Hz, {:.1} dB above its surroundings", best.1, db(best.0) - db(side)));
            // blow-off "pssh" right after the lift at 5.0 s
            let bov = band_rms(&m[(5.02 * SR) as usize..(5.25 * SR) as usize], 1500.0, 4000.0);
            let after = band_rms(&m[(5.6 * SR) as usize..(5.83 * SR) as usize], 1500.0, 4000.0);
            ck.check(db(bov) - db(after) > 8.0, format!("thistle: blow-off pssh on lift +{:.1} dB (1.5-4 kHz)", db(bov) - db(after)));
        }
    }
}

fn band_rms(x: &[f32], lo: f32, hi: f32) -> f32 {
    // sum of Goertzel magnitudes on a grid as a crude band energy
    let mut e = 0.0;
    let mut f = lo;
    while f <= hi {
        let g = goertzel(x, f);
        e += g * g;
        f += 25.0;
    }
    e.sqrt()
}

fn traction(ck: &mut Checks, dir: &std::path::Path) {
    println!("\n== traction loss (Thistle, 20 m/s) ==");
    // asphalt: roll 1s | lateral slide ramp 2s | brake lock 1s | wheelspin 1s
    let mut s = Synth::new(SR);
    let mut clip = Clip::new();
    let mut p = engine_params(2);
    render(&mut s, &mut clip, 5.0, |s, t| {
        p.speed = 20.0;
        p.rpm = 4200.0;
        p.throttle = 0.5;
        p.load = 0.5;
        p.slip_lat_hard = 0.0;
        p.lock_hard = 0.0;
        p.spin_hard = 0.0;
        if (1.0..3.0).contains(&t) {
            p.slip_lat_hard = ((t - 1.0) / 1.5).min(1.0);
        } else if (3.0..4.0).contains(&t) {
            p.lock_hard = 1.0;
            p.throttle = 0.0;
            p.load = 0.0;
            p.rpm = 2000.0;
            p.speed = 20.0 - (t - 3.0) * 10.0;
        } else if t >= 4.0 {
            p.spin_hard = 1.0;
            p.throttle = 1.0;
            p.load = 1.0;
            p.rpm = 6200.0;
            p.speed = 8.0;
        }
        s.handle(cars_cmd(p));
    });
    write_wav(&dir.join("skid_asphalt.wav"), &clip);
    basic(ck, "skid_asphalt", &clip, (0.02, 0.35), None, s.pre_peak);
    let m = clip.mono();
    let seg = |a: f32, b: f32| &m[(a * SR) as usize..(b * SR) as usize];
    let roll = band_rms(seg(0.4, 0.95), 600.0, 1600.0);
    let slide = band_rms(seg(2.4, 2.95), 600.0, 1600.0);
    let lock = band_rms(seg(3.3, 3.8), 500.0, 1400.0);
    let spin = band_rms(seg(4.3, 4.9), 600.0, 1600.0);
    ck.check(db(slide) - db(roll) > 10.0, format!("asphalt slide squeal +{:.1} dB over rolling (600-1600 Hz)", db(slide) - db(roll)));
    ck.check(db(lock) - db(roll) > 8.0, format!("brake lock-up squeal +{:.1} dB over rolling", db(lock) - db(roll)));
    ck.check(db(spin) - db(roll) > 8.0, format!("wheelspin squeal/whirr +{:.1} dB over rolling", db(spin) - db(roll)));
    // what the player hears: overall level rise over the engine+rolling mix (same engine state)
    let bb = db(rms(seg(2.4, 2.95))) - db(rms(seg(0.4, 0.95)));
    ck.check(bb > 4.0, format!("asphalt slide raises the overall mix by {bb:+.1} dB"));
    // squeal grows with slip amount
    let half = band_rms(seg(1.55, 1.8), 600.0, 1600.0);
    ck.check(slide > half && half > roll, format!("squeal scales with slip: {:.1} < {:.1} < {:.1} dB", db(roll), db(half), db(slide)));

    // gravel: roll 1s | lateral scrabble 1s | wheelspin roost 1s | lock 1s ; then snow & grass
    let mut s = Synth::new(SR);
    let mut clip = Clip::new();
    let mut p = engine_params(2);
    render(&mut s, &mut clip, 6.0, |s, t| {
        p.speed = 18.0;
        p.rpm = 4000.0;
        p.throttle = 0.5;
        p.load = 0.5;
        p.s_hard = 0.0;
        p.s_loose = 1.0;
        p.s_snow = 0.0;
        p.s_grass = 0.0;
        p.slip_lat_loose = 0.0;
        p.spin_loose = 0.0;
        p.lock_loose = 0.0;
        if (1.0..2.0).contains(&t) {
            p.slip_lat_loose = 0.9;
        } else if (2.0..3.0).contains(&t) {
            p.spin_loose = 1.0;
            p.throttle = 1.0;
            p.load = 1.0;
            p.rpm = 6000.0;
            p.speed = 8.0;
        } else if (3.0..4.0).contains(&t) {
            p.lock_loose = 1.0;
            p.throttle = 0.0;
            p.load = 0.0;
            p.rpm = 1800.0;
        } else if (4.0..5.0).contains(&t) {
            p.s_loose = 0.0;
            p.s_snow = 1.0;
            p.slip_lat_loose = if t > 4.5 { 0.8 } else { 0.0 };
        } else if t >= 5.0 {
            p.s_loose = 0.0;
            p.s_grass = 1.0;
        }
        s.handle(cars_cmd(p));
    });
    write_wav(&dir.join("traction_gravel_snow_grass.wav"), &clip);
    basic(ck, "traction_gravel_snow_grass", &clip, (0.02, 0.35), None, s.pre_peak);
    let m = clip.mono();
    let seg = |a: f32, b: f32| &m[(a * SR) as usize..(b * SR) as usize];
    let roll = band_rms(seg(0.4, 0.95), 800.0, 3500.0);
    let scr = band_rms(seg(1.3, 1.95), 800.0, 3500.0);
    let roost = band_rms(seg(2.3, 2.95), 800.0, 3500.0);
    let lock = band_rms(seg(3.3, 3.95), 800.0, 3500.0);
    let snow_roll = band_rms(seg(4.1, 4.45), 400.0, 1500.0);
    let snow_slide = band_rms(seg(4.6, 4.95), 400.0, 1500.0);
    ck.check(db(scr) - db(roll) > 6.0, format!("gravel scrabble +{:.1} dB over rolling crunch (0.8-3.5 kHz)", db(scr) - db(roll)));
    ck.check(db(roost) - db(roll) > 6.0, format!("gravel roost (wheelspin) +{:.1} dB over rolling crunch", db(roost) - db(roll)));
    ck.check(db(lock) - db(roll) > 6.0, format!("gravel lock-up slide +{:.1} dB over rolling crunch", db(lock) - db(roll)));
    let bb = db(rms(seg(1.3, 1.95))) - db(rms(seg(0.4, 0.95)));
    ck.check(bb > 4.0, format!("gravel scrabble raises the overall mix by {bb:+.1} dB"));
    let bb = db(rms(seg(4.6, 4.95))) - db(rms(seg(4.1, 4.45)));
    ck.check(bb > 3.0, format!("snow slide raises the overall mix by {bb:+.1} dB"));
    ck.check(db(snow_slide) - db(snow_roll) > 4.0, format!("snow slide +{:.1} dB over snow rolling", db(snow_slide) - db(snow_roll)));
    ck.check(roll > 0.0 && rms(seg(5.2, 5.9)) > 0.0, "gravel and grass rolling are audible".into());
}

fn oneshots(ck: &mut Checks, dir: &std::path::Path) {
    println!("\n== one-shots ==");
    // collisions + landings
    let mut s = Synth::new(SR);
    let mut clip = Clip::new();
    let hits = [(2.0, false), (6.0, false), (12.0, false), (20.0, false), (8.0, true), (16.0, true)];
    let mut fired = 0;
    let mut peaks = Vec::new();
    render(&mut s, &mut clip, 6.0 * 0.7 + 2.0 * 0.7 + 0.5, |s, t| {
        let slot = (t / 0.7) as usize;
        if slot >= fired && slot < hits.len() + 2 {
            fired = slot + 1;
            if slot < hits.len() {
                let (st, sc) = hits[slot];
                s.handle(Cmd::Sfx { kind: SfxKind::Collision { strength: st, scenery: sc }, gain: 1.0, pan: 0.0 });
            } else {
                let st = if slot == hits.len() { 5.0 } else { 12.0 };
                s.handle(Cmd::Sfx { kind: SfxKind::Landing { strength: st }, gain: 1.0, pan: 0.0 });
            }
        }
    });
    write_wav(&dir.join("collision.wav"), &clip);
    basic(ck, "collision", &clip, (0.01, 0.3), None, s.pre_peak);
    for k in 0..4 {
        let a = (k as f32 * 0.7 * SR) as usize;
        let b = ((k as f32 + 1.0) * 0.7 * SR) as usize;
        peaks.push(clip.l[a..b].iter().fold(0.0f32, |m, v| m.max(v.abs())));
    }
    ck.check(peaks.windows(2).all(|w| w[1] > w[0]), format!("collision loudness grows with strength: {:?}", peaks.iter().map(|p| format!("{:.2}", p)).collect::<Vec<_>>()));

    // race cues
    let mut s = Synth::new(SR);
    let mut clip = Clip::new();
    let cues: [(f32, SfxKind); 7] = [
        (0.2, SfxKind::CountdownTick(3)),
        (1.2, SfxKind::CountdownTick(2)),
        (2.2, SfxKind::CountdownTick(1)),
        (3.2, SfxKind::Go),
        (5.0, SfxKind::Checkpoint),
        (6.2, SfxKind::Lap),
        (7.6, SfxKind::FinalLap),
    ];
    let mut next = 0;
    render(&mut s, &mut clip, 9.5, |s, t| {
        while next < cues.len() && t >= cues[next].0 {
            s.handle(Cmd::Sfx { kind: cues[next].1, gain: 1.0, pan: 0.0 });
            next += 1;
        }
    });
    write_wav(&dir.join("countdown_go_cues.wav"), &clip);
    basic(ck, "countdown_go_cues", &clip, (0.005, 0.2), None, s.pre_peak);
    let m = clip.mono();
    // pitch of tick vs go (dominant frequency in the first 120 ms)
    let dom = |a: f32| {
        let x = &m[(a * SR) as usize..(a * SR) as usize + 4096];
        let mut best = (0.0, 0.0);
        let mut f = 150.0;
        while f < 3000.0 {
            let g = goertzel(x, f);
            if g > best.0 {
                best = (g, f);
            }
            f *= 1.01;
        }
        best.1
    };
    let (ft, fg) = (dom(0.21), dom(3.21));
    ck.check(fg > ft * 1.4, format!("countdown tick ~{ft:.0} Hz is lower than GO ~{fg:.0} Hz"));

    // fanfare (1st) then a 5th place finish, respawn, splash
    let mut s = Synth::new(SR);
    let mut clip = Clip::new();
    let mut st = 0;
    render(&mut s, &mut clip, 9.0, |s, t| {
        let ev = [(0.1, SfxKind::Finish { place: 1 }), (3.2, SfxKind::Finish { place: 5 }), (6.0, SfxKind::Respawn), (7.4, SfxKind::Splash { strength: 12.0 })];
        while st < ev.len() && t >= ev[st].0 {
            s.handle(Cmd::Sfx { kind: ev[st].1, gain: 1.0, pan: 0.0 });
            st += 1;
        }
    });
    write_wav(&dir.join("fanfare_respawn_splash.wav"), &clip);
    basic(ck, "fanfare_respawn_splash", &clip, (0.005, 0.2), None, s.pre_peak);

    // UI
    let mut s = Synth::new(SR);
    let mut clip = Clip::new();
    let mut k = 0;
    render(&mut s, &mut clip, 2.4, |s, t| {
        let ev = [UiKind::Move, UiKind::Move, UiKind::Move, UiKind::Select, UiKind::Back];
        if k < ev.len() && t >= 0.1 + k as f32 * 0.45 {
            s.handle(Cmd::Ui(ev[k]));
            k += 1;
        }
    });
    write_wav(&dir.join("ui.wav"), &clip);
    basic(ck, "ui", &clip, (0.002, 0.1), None, s.pre_peak);
}

fn music(ck: &mut Checks, dir: &std::path::Path) {
    println!("\n== music (20 s per mood, default volumes) ==");
    let mut levels = Vec::new();
    for (name, mood) in [("menu", Mood::Menu), ("race", Mood::Race), ("results", Mood::Results)] {
        let mut s = Synth::new(SR);
        s.handle(Cmd::Music(mood));
        let mut clip = Clip::new();
        render(&mut s, &mut clip, 20.0, |_, _| {});
        let name = format!("music_{name}");
        write_wav(&dir.join(format!("{name}.wav")), &clip);
        // skip the 0.25 s count-in when looking for silences
        let tail = Clip { l: clip.l[(0.5 * SR) as usize..].to_vec(), r: clip.r[(0.5 * SR) as usize..].to_vec() };
        basic(ck, &name, &tail, (0.01, 0.2), Some(0.3), s.pre_peak);
        let m = tail.mono();
        // loudness stability: per-second RMS spread
        let per_s: Vec<f32> = m.chunks(SR as usize).map(|c| db(rms(c))).collect();
        let lo = per_s.iter().cloned().fold(f32::MAX, f32::min);
        let hi = per_s.iter().cloned().fold(f32::MIN, f32::max);
        println!("    per-second RMS dB range {lo:.1} .. {hi:.1}");
        ck.check(hi - lo < 18.0, format!("{name}: per-second loudness within {:.1} dB", hi - lo));
        // stereo: L/R not identical (panned instruments)
        let side = rms(&tail.l.iter().zip(&tail.r).map(|(a, b)| 0.5 * (a - b)).collect::<Vec<_>>());
        ck.check(side > 0.05 * rms(&m), format!("{name}: stereo width side/mid {:.2}", side / rms(&m)));
        // harsh energy check: >6 kHz energy small vs total
        let (mut hf, mut lf) = (0.0, 0.0);
        for k in 0..8 {
            let x = &m[((1.0 + 2.2 * k as f32) * SR) as usize..((1.0 + 2.2 * k as f32) * SR) as usize + 8192];
            hf += band_rms(x, 6000.0, 12000.0).powi(2);
            lf += band_rms(x, 60.0, 2000.0).powi(2);
        }
        let (hf, lf) = (hf.sqrt(), lf.sqrt());
        ck.check(db(lf) - db(hf) > 20.0, format!("{name}: soft top end (60-2k vs 6-12k: {:.1} dB)", db(lf) - db(hf)));
        levels.push(rms(&m));
    }
    ck.check(levels[1] < levels[0] && levels[1] < levels[2], "race music sits quieter than menu/results".into());

    // crossfade menu -> race -> results -> silent
    let mut s = Synth::new(SR);
    let mut clip = Clip::new();
    let mut stage = 0;
    render(&mut s, &mut clip, 26.0, |s, t| {
        let plan = [(0.0, Mood::Menu), (8.0, Mood::Race), (16.0, Mood::Results), (24.0, Mood::Silent)];
        if stage < plan.len() && t >= plan[stage].0 {
            s.handle(Cmd::Music(plan[stage].1));
            stage += 1;
        }
    });
    write_wav(&dir.join("music_crossfade.wav"), &clip);
    let body = Clip { l: clip.l[(0.5 * SR) as usize..(24.0 * SR) as usize].to_vec(), r: clip.r[(0.5 * SR) as usize..(24.0 * SR) as usize].to_vec() };
    basic(ck, "music_crossfade (0.5-24 s)", &body, (0.01, 0.2), Some(0.3), s.pre_peak);
    let end = rms(&clip.mono()[(25.5 * SR) as usize..]);
    ck.check(end < 0.003, format!("music fades out on Music::None (end RMS {end:.5})"));
}

fn sim_race(ck: &mut Checks, dir: &std::path::Path) {
    use rally_core::ai::AiDriver;
    use rally_core::track::{Track, TrackParams};
    use rally_core::world::{Controller, RacerSetup, World, TICK};
    use std::sync::Arc;
    println!("\n== real simulation through the game bridge (5 cars, local = Thistle, 40 s) ==");
    let track = Arc::new(Track::generate(&TrackParams { seed: 3, biome: None, length_class: 0 }));
    let racers: Vec<RacerSetup> = (0..5)
        .map(|i| RacerSetup { car_spec: if i == 4 { 2 } else { i % 4 }, controller: if i == 4 { Controller::Local(0) } else { Controller::Ai } })
        .collect();
    let mut world = World::new(track.clone(), &racers, 2);
    let mut ais: Vec<AiDriver> = (0..5).map(|i| AiDriver::new(if i == 4 { 1.0 } else { 0.8 }, (i as f32 - 2.0) * 1.2)).collect();
    let mut br = bridge::Bridge::new();
    let mut s = Synth::new(SR);
    s.handle(Cmd::Music(Mood::Race));
    let mut clip = Clip::new();
    let frame = 1.0 / 60.0;
    let mut acc = 0.0;
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut max_slip = (0.0f32, 0.0f32, 0.0f32);
    let mut voices_max = 0;
    let mut eye = Vec3::ZERO;
    render(&mut s, &mut clip, 40.0, |s, _t| {
        acc += BLOCK as f32 / SR;
        if acc < frame {
            return;
        }
        acc -= frame;
        for i in 0..world.cars.len() {
            let inp = ais[i].drive(&world.cars[i], &world.track, frame);
            world.set_input(i, inp);
        }
        world.update(frame);
        let events = world.drain_events();
        let car = &world.cars[4];
        let fwd = car.rot * Vec3::NEG_Z;
        let want = car.pos - fwd * 6.5 + Vec3::Y * 2.4;
        eye = if eye == Vec3::ZERO { want } else { eye.lerp(want, 0.15) };
        let lis = [bridge::Listener { eye, target: car.pos + Vec3::Y }];
        let mut cmds = Vec::new();
        br.frame(&world, &[4], &lis, &events, false, frame, &mut |c| cmds.push(c));
        for c in cmds {
            match c {
                Cmd::Sfx { kind, .. } => {
                    let k = format!("{kind:?}");
                    let k = k.split([' ', '(', '{']).next().unwrap().to_string();
                    *counts.entry(k).or_default() += 1;
                }
                Cmd::Cars { n, cars, .. } => {
                    voices_max = voices_max.max(n as usize);
                    let p = cars[0];
                    max_slip.0 = max_slip.0.max(p.slip_lat_hard.max(p.slip_lat_loose));
                    max_slip.1 = max_slip.1.max(p.spin_hard.max(p.spin_loose));
                    max_slip.2 = max_slip.2.max(p.lock_hard.max(p.lock_loose));
                }
                _ => {}
            }
            s.handle(c);
        }
    });
    let _ = TICK;
    write_wav(&dir.join("sim_race.wav"), &clip);
    let (pi, pv) = clip.l.iter().zip(&clip.r).map(|(a, b)| a.abs().max(b.abs())).enumerate().fold((0, 0.0f32), |m, (i, v)| if v > m.1 { (i, v) } else { m });
    println!("    loudest moment at {:.2}s ({pv:.3})", pi as f32 / SR);
    basic(ck, "sim_race", &clip, (0.02, 0.35), None, s.pre_peak);
    println!("    events: {counts:?}  max engine voices {voices_max}  local max slip lat/spin/lock {:.2}/{:.2}/{:.2}", max_slip.0, max_slip.1, max_slip.2);
    ck.check(counts.get("CountdownTick").copied().unwrap_or(0) == 3 && counts.contains_key("Go"), "countdown ticks + GO reached the synth".into());
    ck.check(voices_max == 4, format!("local + 3 nearest AI engine voices ({voices_max})"));
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "audio_out".into());
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("out dir");
    let mut ck = Checks { fails: Vec::new() };
    let t0 = std::time::Instant::now();
    engine_sweeps(&mut ck, &dir);
    traction(&mut ck, &dir);
    oneshots(&mut ck, &dir);
    music(&mut ck, &dir);
    sim_race(&mut ck, &dir);
    // performance
    let perf = |voices: usize, full: bool, music: bool| -> f32 {
        let mut s = Synth::new(SR);
        if music {
            s.handle(Cmd::Music(Mood::Race));
        }
        let mut cars = [EngineParams::default(); MAX_CARS];
        for (i, c) in cars.iter_mut().enumerate() {
            *c = engine_params((i % 4) as u8);
            c.id = i as u32;
            c.rpm = 5000.0;
            c.throttle = 1.0;
            c.load = 1.0;
            c.speed = 25.0;
            c.s_hard = 0.5;
            c.s_loose = 0.5;
            if full {
                c.slip_lat_hard = 0.5;
                c.slip_lat_loose = 0.5;
            }
            c.local = i == 0;
            c.gain = 0.3;
        }
        s.handle(Cmd::Cars { n: voices as u8, cars, paused: false });
        let mut clip = Clip::new();
        let tp = std::time::Instant::now();
        render(&mut s, &mut clip, 10.0, |_, _| {});
        tp.elapsed().as_secs_f32() / 10.0 * 100.0
    };
    let p_music = perf(0, false, true);
    let p_typ = perf(4, false, true);
    let p_worst = perf(8, true, true);
    println!("\n== perf (% of one core): race music alone {p_music:.1}% | typical race (4 engines + music) {p_typ:.1}% | worst case (8 engines all sliding + music) {p_worst:.1}% ==");
    ck.check(p_typ < 8.0, format!("typical race DSP load {p_typ:.1}% < 8% of a core"));
    ck.check(p_worst < 15.0, format!("worst-case DSP load {p_worst:.1}% < 15% of a core"));
    println!("\nwrote WAVs to {} in {:.1}s", dir.display(), t0.elapsed().as_secs_f32());
    if ck.fails.is_empty() {
        println!("ALL CHECKS PASSED");
    } else {
        println!("{} CHECK(S) FAILED:", ck.fails.len());
        for f in &ck.fails {
            println!("  - {f}");
        }
        std::process::exit(1);
    }
}
