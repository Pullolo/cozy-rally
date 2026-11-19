//! One-shot effects and UI sounds, built from notes. Musical cues use C major pentatonic so
//! they always sit nicely over the music.
use super::dsp::{clamp01, midi_hz, Rng};
use super::notes::{Inst, Note, NotePool};
use super::{SfxKind, UiKind, BUS_SFX};

fn m(n: f32) -> f32 {
    midi_hz(n)
}

pub fn ui(pool: &mut NotePool, rng: &mut Rng, k: UiKind) {
    let det = 1.0 + rng.bi() * 0.004;
    match k {
        UiKind::Move => {
            pool.play(Note::new(Inst::Marimba, m(79.0) * det, 0.09).env(0.001, 0.09).send(0.08).bus(BUS_SFX));
            pool.play(Note::new(Inst::Wood, m(91.0) * det, 0.025).send(0.05));
        }
        UiKind::Select => {
            pool.play(Note::new(Inst::Marimba, m(72.0), 0.11).env(0.001, 0.14).send(0.12));
            pool.play(Note::new(Inst::Marimba, m(79.0), 0.11).env(0.001, 0.2).at(0.065).send(0.12));
            pool.play(Note::new(Inst::Glock, m(91.0), 0.035).env(0.001, 0.25).at(0.065).send(0.2));
        }
        UiKind::Back => {
            pool.play(Note::new(Inst::Marimba, m(79.0), 0.09).env(0.001, 0.12).send(0.1));
            pool.play(Note::new(Inst::Marimba, m(72.0), 0.09).env(0.001, 0.18).at(0.07).send(0.1));
        }
    }
}

pub fn play(pool: &mut NotePool, rng: &mut Rng, kind: SfxKind, gain: f32, pan: f32) {
    let g = gain.clamp(0.0, 1.5);
    if g < 1e-3 {
        return;
    }
    let p = pan;
    match kind {
        SfxKind::Collision { strength, scenery } => {
            let k = clamp01((strength - 1.0) / 12.0);
            let (f0, f1) = if scenery { (95.0, 40.0) } else { (125.0, 52.0) };
            pool.play(Note::new(Inst::Thump, f0, (0.12 + 0.32 * k) * g).glide_to(f1, 0.04).env(0.001, 0.11 + 0.09 * k).pan(p).send(0.05));
            pool.play(Note::new(Inst::NoiseLp, 1400.0, (0.08 + 0.18 * k) * g).glide_to(500.0, 0.05).env(0.001, 0.05 + 0.04 * k).pan(p));
            // a bit of cute clatter (bonks of loose bits), more with harder hits
            let count = 1 + (k * 5.0) as usize;
            for _ in 0..count {
                let f = rng.range(380.0, 1150.0);
                let d = rng.range(0.015, 0.06 + 0.12 * k);
                pool.play(Note::new(Inst::Wood, f, rng.range(0.025, 0.05 + 0.06 * k) * g).at(d).pan(p + rng.bi() * 0.2).send(0.06));
            }
        }
        SfxKind::Landing { strength } => {
            let k = clamp01((strength - 3.0) / 8.0);
            pool.play(Note::new(Inst::Thump, 88.0, (0.08 + 0.26 * k) * g).glide_to(42.0, 0.05).env(0.001, 0.13 + 0.06 * k).pan(p));
            pool.play(Note::new(Inst::NoiseLp, 420.0, 0.12 * k * g).env(0.002, 0.05).pan(p));
            // springy suspension "boing"
            pool.play(Note::new(Inst::Sine, 215.0, (0.025 + 0.04 * k) * g).glide_to(150.0, 0.08).env(0.002, 0.09).at(0.02).pan(p));
        }
        SfxKind::CountdownTick(_) => {
            pool.play(Note::new(Inst::Marimba, m(69.0), 0.20 * g).env(0.001, 0.22).send(0.15));
            pool.play(Note::new(Inst::Sine, m(57.0), 0.10 * g).env(0.003, 0.12).hold(0.12, 0.05));
        }
        SfxKind::Go => {
            for (i, n) in [72.0, 76.0, 79.0, 84.0].iter().enumerate() {
                pool.play(Note::new(Inst::Marimba, m(*n), 0.11 * g).env(0.001, 0.35).at(i as f32 * 0.006).pan((i as f32 - 1.5) * 0.2).send(0.2));
            }
            pool.play(Note::new(Inst::Bell, m(84.0), 0.10 * g).env(0.001, 0.6).bright(0.6).send(0.3));
            pool.play(Note::new(Inst::Glock, m(96.0), 0.05 * g).env(0.001, 0.5).at(0.03).send(0.3));
        }
        SfxKind::Checkpoint => {
            pool.play(Note::new(Inst::Glock, m(84.0), 0.11 * g).env(0.001, 0.25).send(0.25).pan(-0.1));
            pool.play(Note::new(Inst::Glock, m(91.0), 0.11 * g).env(0.001, 0.45).at(0.07).send(0.3).pan(0.1));
            pool.play(Note::new(Inst::Bell, m(91.0), 0.035 * g).env(0.001, 0.4).at(0.07).bright(0.3).send(0.3));
        }
        SfxKind::Lap => {
            for (i, n) in [84.0, 88.0, 91.0, 96.0].iter().enumerate() {
                let t = i as f32 * 0.085;
                pool.play(Note::new(Inst::Glock, m(*n), 0.09 * g).env(0.001, 0.4 + 0.1 * i as f32).at(t).pan(-0.3 + 0.2 * i as f32).send(0.3));
                pool.play(Note::new(Inst::Marimba, m(*n - 12.0), 0.07 * g).env(0.001, 0.25).at(t).send(0.15));
            }
        }
        SfxKind::FinalLap => {
            let run = [79.0, 81.0, 84.0, 86.0, 88.0];
            for (i, n) in run.iter().enumerate() {
                let t = i as f32 * 0.09;
                pool.play(Note::new(Inst::Marimba, m(*n), 0.10 * g).env(0.001, 0.2).at(t).pan(-0.4 + 0.2 * i as f32).send(0.15));
            }
            let t = 0.5;
            for (i, n) in [88.0, 91.0, 96.0].iter().enumerate() {
                pool.play(Note::new(Inst::Bell, m(*n), 0.06 * g).env(0.002, 0.9).bright(0.4).at(t + i as f32 * 0.01).pan((i as f32 - 1.0) * 0.3).send(0.35));
                pool.play(Note::new(Inst::Glock, m(*n), 0.06 * g).env(0.001, 0.8).at(t).send(0.3));
            }
            pool.play(Note::new(Inst::Bass, m(48.0), 0.12 * g).env(0.005, 0.5).hold(0.6, 0.15).at(t));
        }
        SfxKind::Finish { place } => {
            let big = place <= 3;
            let lv = if big { 1.0 } else { 0.75 };
            // triplet pickup -> warm chord
            for (i, n) in [72.0, 76.0, 79.0].iter().enumerate() {
                let t = i as f32 * 0.11;
                pool.play(Note::new(Inst::Marimba, m(*n), 0.12 * g * lv).env(0.001, 0.18).at(t).send(0.15));
                pool.play(Note::new(Inst::Glock, m(*n + 12.0), 0.05 * g * lv).env(0.001, 0.2).at(t).send(0.25));
            }
            let t = 0.36;
            for (i, n) in [72.0, 76.0, 79.0, 83.0, 86.0].iter().enumerate() {
                pool.play(
                    Note::new(Inst::EPiano, m(*n), 0.075 * g * lv)
                        .env(0.004, 1.6)
                        .hold(1.2, 0.4)
                        .bright(0.6)
                        .at(t + i as f32 * 0.012)
                        .pan((i as f32 - 2.0) * 0.18)
                        .send(0.35),
                );
            }
            pool.play(Note::new(Inst::Bell, m(84.0), 0.08 * g * lv).env(0.002, 1.2).bright(0.5).at(t).send(0.4));
            pool.play(Note::new(Inst::Bass, m(48.0), 0.16 * g * lv).env(0.005, 0.9).hold(1.0, 0.2).at(t));
            pool.play(Note::new(Inst::Kick, 100.0, 0.18 * g * lv).glide_to(48.0, 0.03).at(t));
            if big {
                // sparkle run on top for the podium
                for (i, n) in [91.0, 93.0, 96.0, 98.0, 100.0, 103.0, 108.0].iter().enumerate() {
                    let tt = t + 0.25 + i as f32 * 0.055;
                    pool.play(Note::new(Inst::Glock, m(*n), 0.045 * g).env(0.001, 0.5).at(tt).pan(-0.5 + 0.16 * i as f32).send(0.45));
                }
            }
            if place == 1 {
                pool.play(Note::new(Inst::Bell, m(96.0), 0.06 * g).env(0.002, 1.4).bright(0.5).at(t + 0.7).send(0.5));
                pool.play(Note::new(Inst::Bell, m(100.0), 0.05 * g).env(0.002, 1.4).bright(0.5).at(t + 0.72).send(0.5));
            }
        }
        SfxKind::Respawn => {
            pool.play(Note::new(Inst::NoiseBp, 300.0, 0.30 * g).glide_to(2400.0, 0.14).q(1.2).env(0.12, 0.18).hold(0.14, 0.12).pan(p).send(0.3));
            pool.play(Note::new(Inst::Sine, 520.0, 0.04 * g).glide_to(1150.0, 0.12).env(0.08, 0.2).hold(0.15, 0.1).pan(p).send(0.3));
            pool.play(Note::new(Inst::Glock, m(91.0), 0.05 * g).env(0.001, 0.35).at(0.18).pan(p).send(0.4));
            pool.play(Note::new(Inst::Glock, m(96.0), 0.04 * g).env(0.001, 0.35).at(0.24).pan(p).send(0.4));
        }
        SfxKind::Splash { strength } => {
            let k = clamp01(strength / 15.0) * 0.7 + 0.3;
            pool.play(Note::new(Inst::NoiseLp, 4200.0, 0.34 * k * g).glide_to(380.0, 0.16).q(0.9).env(0.004, 0.32).pan(p).send(0.15));
            pool.play(Note::new(Inst::NoiseBp, 1300.0, 0.20 * k * g).q(0.8).env(0.003, 0.14).pan(p));
            for _ in 0..7 {
                let f = rng.range(320.0, 760.0);
                pool.play(Note::new(Inst::Sine, f, rng.range(0.025, 0.06) * k * g).glide_to(f * 1.9, 0.025).env(0.002, 0.035).hold(0.05, 0.02).at(rng.range(0.04, 0.45)).pan(p + rng.bi() * 0.3));
            }
        }
        SfxKind::Puddle { strength } => {
            let k = clamp01(strength / 15.0) * 0.7 + 0.3;
            pool.play(Note::new(Inst::NoiseLp, 3400.0, 0.17 * k * g).glide_to(650.0, 0.06).q(0.9).env(0.003, 0.11).pan(p));
            pool.play(Note::new(Inst::NoiseBp, 1700.0, 0.09 * k * g).q(0.9).env(0.002, 0.06).pan(p));
            for _ in 0..2 {
                let f = rng.range(380.0, 700.0);
                pool.play(Note::new(Inst::Sine, f, 0.025 * k * g).glide_to(f * 1.8, 0.02).env(0.002, 0.03).hold(0.04, 0.02).at(rng.range(0.03, 0.15)).pan(p));
            }
        }
        SfxKind::Thunder { strength } => {
            // a soft, far rumble that rolls a few times
            let st = strength.clamp(0.0, 1.0);
            let rolls = 3 + (rng.next_u32() % 3) as usize;
            let mut t = 0.0;
            for i in 0..rolls {
                let f = rng.range(140.0, 260.0);
                let lvl = if i == 0 { 0.22 } else { rng.range(0.1, 0.2) };
                pool.play(
                    Note::new(Inst::NoiseLp, f, lvl * st * g)
                        .glide_to(f * 0.45, 1.2)
                        .q(0.7)
                        .env(rng.range(0.25, 0.7), rng.range(1.0, 2.0))
                        .hold(0.5, 1.2)
                        .at(t)
                        .pan(p + rng.bi() * 0.25)
                        .send(0.2),
                );
                t += rng.range(0.35, 1.1);
            }
            pool.play(Note::new(Inst::Thump, 45.0, 0.06 * st * g).glide_to(32.0, 0.8).env(0.4, 1.2).hold(0.6, 0.8).pan(p));
        }
        SfxKind::StageBeep(k) => {
            if k > 0 {
                pool.play(Note::new(Inst::Sine, m(81.0), 0.13 * g).env(0.004, 0.5).hold(0.11, 0.03).send(0.1));
                pool.play(Note::new(Inst::Marimba, m(81.0), 0.07 * g).env(0.001, 0.12).send(0.1));
            } else {
                pool.play(Note::new(Inst::Sine, m(88.0), 0.13 * g).env(0.004, 2.0).hold(0.55, 0.08).send(0.15));
                pool.play(Note::new(Inst::Bell, m(88.0), 0.07 * g).env(0.001, 0.7).bright(0.5).send(0.3));
                for (i, n) in [91.0, 96.0, 100.0].iter().enumerate() {
                    pool.play(Note::new(Inst::Glock, m(*n), 0.04 * g).env(0.001, 0.4).at(0.05 + i as f32 * 0.05).pan(-0.3 + 0.3 * i as f32).send(0.35));
                }
            }
        }
        SfxKind::Split { faster } => {
            if faster {
                pool.play(Note::new(Inst::Glock, m(88.0), 0.11 * g).env(0.001, 0.3).send(0.3).pan(-0.1));
                pool.play(Note::new(Inst::Glock, m(93.0), 0.11 * g).env(0.001, 0.5).at(0.06).send(0.35).pan(0.1));
                pool.play(Note::new(Inst::Bell, m(93.0), 0.04 * g).env(0.001, 0.5).bright(0.4).at(0.06).send(0.35));
            } else {
                pool.play(Note::new(Inst::Marimba, m(69.0), 0.09 * g).env(0.001, 0.25).send(0.15));
                pool.play(Note::new(Inst::Marimba, m(64.0), 0.08 * g).env(0.001, 0.35).at(0.09).send(0.15));
            }
        }
        SfxKind::Medal(tier) => medal(pool, tier, g),
        SfxKind::NewRecord => {
            // "ta-ta-taaa!" in F, warm chord, sparkle run
            for (i, (n, t)) in [(72.0, 0.0), (72.0, 0.12), (77.0, 0.24)].iter().enumerate() {
                pool.play(Note::new(Inst::Marimba, m(*n), 0.12 * g).env(0.001, if i == 2 { 0.4 } else { 0.12 }).at(*t).send(0.2));
                pool.play(Note::new(Inst::Glock, m(*n + 12.0), 0.05 * g).env(0.001, 0.25).at(*t).send(0.3));
            }
            let t = 0.48;
            for (i, n) in [65.0, 69.0, 72.0, 77.0, 79.0].iter().enumerate() {
                pool.play(Note::new(Inst::EPiano, m(*n), 0.075 * g).env(0.004, 1.8).hold(1.3, 0.4).bright(0.65).at(t + i as f32 * 0.012).pan((i as f32 - 2.0) * 0.2).send(0.35));
            }
            pool.play(Note::new(Inst::Bell, m(89.0), 0.08 * g).env(0.002, 1.3).bright(0.5).at(t).send(0.45));
            pool.play(Note::new(Inst::Bass, m(41.0), 0.17 * g).env(0.005, 1.0).hold(1.1, 0.25).at(t));
            pool.play(Note::new(Inst::Kick, 100.0, 0.18 * g).glide_to(48.0, 0.03).at(t));
            pool.play(Note::new(Inst::Brush, 3000.0, 0.10 * g).q(0.5).env(0.25, 0.4).at(t - 0.2).send(0.3));
            for (i, n) in [89.0, 91.0, 93.0, 96.0, 101.0, 105.0].iter().enumerate() {
                pool.play(Note::new(Inst::Glock, m(*n), 0.045 * g).env(0.001, 0.5).at(t + 0.3 + i as f32 * 0.06).pan(-0.5 + 0.2 * i as f32).send(0.45));
            }
            pool.play(Note::new(Inst::Bell, m(101.0), 0.045 * g).env(0.002, 1.4).bright(0.4).at(t + 0.75).send(0.5));
        }
    }
}

/// Medal jingles: richer with each tier, gold and platinum get sparkle on top
fn medal(pool: &mut NotePool, tier: u8, g: f32) {
    match tier {
        0 => {
            pool.play(Note::new(Inst::Marimba, m(79.0), 0.09 * g).env(0.001, 0.3).send(0.15));
            pool.play(Note::new(Inst::Marimba, m(84.0), 0.09 * g).env(0.001, 0.5).at(0.12).send(0.2));
        }
        _ => {
            let run: &[f32] = match tier {
                1 => &[72.0, 76.0, 79.0],
                2 => &[72.0, 76.0, 79.0, 84.0],
                _ => &[72.0, 76.0, 79.0, 84.0, 88.0],
            };
            let step = if tier >= 3 { 0.08 } else { 0.1 };
            for (i, n) in run.iter().enumerate() {
                let t = i as f32 * step;
                pool.play(Note::new(Inst::Marimba, m(*n), 0.11 * g).env(0.001, 0.2).at(t).pan(-0.3 + 0.15 * i as f32).send(0.15));
                if tier >= 2 {
                    pool.play(Note::new(Inst::Glock, m(*n + 12.0), 0.04 * g).env(0.001, 0.25).at(t).send(0.3));
                }
            }
            let t = run.len() as f32 * step + 0.02;
            let chord: &[f32] = match tier {
                1 => &[60.0, 64.0, 67.0],
                2 => &[60.0, 64.0, 67.0, 71.0],
                _ => &[60.0, 64.0, 67.0, 71.0, 74.0],
            };
            for (i, n) in chord.iter().enumerate() {
                pool.play(Note::new(Inst::EPiano, m(*n + 12.0), 0.065 * g).env(0.004, 1.4).hold(0.5 + 0.2 * tier as f32, 0.35).bright(0.5).at(t + i as f32 * 0.012).pan((i as f32 - 2.0) * 0.18).send(0.3));
            }
            pool.play(Note::new(Inst::Bass, m(48.0), 0.13 * g).env(0.005, 0.8).hold(0.5 + 0.2 * tier as f32, 0.2).at(t));
            if tier >= 3 {
                pool.play(Note::new(Inst::Bell, m(84.0), 0.07 * g).env(0.002, 1.1).bright(0.5).at(t).send(0.4));
                for (i, n) in [91.0, 93.0, 96.0, 98.0, 100.0].iter().enumerate() {
                    pool.play(Note::new(Inst::Glock, m(*n), 0.04 * g).env(0.001, 0.45).at(t + 0.15 + i as f32 * 0.05).pan(-0.4 + 0.2 * i as f32).send(0.45));
                }
            }
            if tier >= 4 {
                // platinum: a second, higher shimmer and a bright bell chord
                for (i, n) in [100.0, 103.0, 105.0, 108.0].iter().enumerate() {
                    pool.play(Note::new(Inst::Glock, m(*n), 0.03 * g).env(0.001, 0.5).at(t + 0.45 + i as f32 * 0.045).pan(0.4 - 0.25 * i as f32).send(0.5));
                }
                for (i, n) in [88.0, 91.0, 96.0].iter().enumerate() {
                    pool.play(Note::new(Inst::Bell, m(*n), 0.04 * g).env(0.002, 1.3).bright(0.45).at(t + 0.7 + i as f32 * 0.015).pan((i as f32 - 1.0) * 0.35).send(0.5));
                }
            }
        }
    }
}
