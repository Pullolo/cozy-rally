#![allow(dead_code)]
#[path = "../src/audio/synth/mod.rs"]
mod synth;
use synth::*;
fn cpu() -> f32 {
    let s = std::fs::read_to_string("/proc/thread-self/stat").unwrap_or_default();
    let f: Vec<&str> = s.rsplit(')').next().unwrap_or("").split_whitespace().collect();
    (f.get(11).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0) + f.get(12).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0)) / 100.0
}
fn main() {
    bench();
    for mood in [Mood::Menu, Mood::Race, Mood::Results] {
        let mut s = Synth::new(48000.0);
        s.handle(Cmd::Music(mood));
        let mut sum = 0usize; let mut mx = 0; let mut n = 0;
        let c0 = cpu();
        for _ in 0..(48000 * 20 / BLOCK) { s.render_block(BLOCK); let a = s.notes.active(); sum += a; mx = mx.max(a); n += 1; }
        println!("{mood:?}: avg active {:.1} max {mx}  {:.2}% core", sum as f32 / n as f32, (cpu() - c0) / 20.0 * 100.0);
    }
    let mut s = Synth::new(48000.0);
    let c0 = cpu();
    for _ in 0..(48000 * 20 / BLOCK) { s.render_block(BLOCK); }
    println!("idle synth: {:.2}% core", (cpu() - c0) / 20.0 * 100.0);
}
#[allow(unused)]
fn bench() {
    use synth::notes::{Inst, Note};
    for inst in [Inst::Sine, Inst::Marimba, Inst::Glock, Inst::EPiano, Inst::Pluck, Inst::Bass, Inst::Pad, Inst::Shaker, Inst::Hat, Inst::Kick] {
        let mut s = Synth::new(48000.0);
        for k in 0..20 { s.notes.play(Note::new(inst, 200.0 + k as f32 * 20.0, 0.01).env(0.001, 100.0).hold(100.0, 1.0).bus(1)); }
        let c0 = cpu();
        for _ in 0..(48000 * 10 / BLOCK) { s.render_block(BLOCK); }
        let dt = cpu() - c0;
        println!("{inst:?}: {:.2} ns/voice-sample", dt / (10.0 * 48000.0 * 20.0) * 1e9);
    }
}
