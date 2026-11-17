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
            pool.play(Note::new(Inst::Thump, f0, (0.16 + 0.42 * k) * g).glide_to(f1, 0.04).env(0.001, 0.11 + 0.09 * k).pan(p).send(0.05));
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
            pool.play(Note::new(Inst::Thump, 88.0, (0.12 + 0.36 * k) * g).glide_to(42.0, 0.05).env(0.001, 0.13 + 0.06 * k).pan(p));
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
    }
}
