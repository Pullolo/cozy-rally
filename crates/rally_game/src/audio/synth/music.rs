//! Generative cozy lofi music. A 16th-note sequencer per "layer" picks chord progressions
//! (maj7 / add9 / sus / 6-9 colours), plays a soft FM e-piano, round bass, Karplus-Strong
//! plucks, a gentle brushed kit and a pentatonic melody that phrases in two-bar calls and
//! answers. Two layers allow crossfading between moods. Vinyl crackle + slight tape wow on top.
use super::dsp::*;
use super::notes::{Inst, Note, NotePool};
use super::{Mood, BLOCK};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Q {
    Maj7,
    Maj9,
    Min7,
    Min9,
    Sus7,
    Add9,
    SixNine,
}

fn intervals(q: Q) -> &'static [i32] {
    match q {
        Q::Maj7 => &[0, 4, 7, 11],
        Q::Maj9 => &[0, 4, 7, 11, 14],
        Q::Min7 => &[0, 3, 7, 10],
        Q::Min9 => &[0, 3, 7, 10, 14],
        Q::Sus7 => &[0, 5, 7, 10, 14],
        Q::Add9 => &[0, 4, 7, 14],
        Q::SixNine => &[0, 4, 9, 14],
    }
}

type Prog = &'static [(i32, Q)];

const MENU_PROGS: &[Prog] = &[
    &[(0, Q::Maj9), (9, Q::Min7), (2, Q::Min9), (7, Q::Sus7)],
    &[(5, Q::Maj7), (4, Q::Min7), (2, Q::Min9), (0, Q::Maj9)],
    &[(0, Q::Maj7), (5, Q::Maj9), (9, Q::Min9), (5, Q::SixNine)],
    &[(2, Q::Min9), (7, Q::Sus7), (0, Q::Maj9), (9, Q::Min7)],
];
const RACE_PROGS: &[Prog] = &[
    &[(9, Q::Min9), (5, Q::Maj7), (0, Q::Add9), (7, Q::Sus7)],
    &[(0, Q::Maj7), (4, Q::Min7), (5, Q::Maj9), (7, Q::Sus7)],
    &[(5, Q::Maj9), (7, Q::Add9), (4, Q::Min7), (9, Q::Min9)],
];
const RESULTS_PROGS: &[Prog] = &[
    &[(0, Q::Add9), (7, Q::Add9), (9, Q::Min7), (5, Q::Maj7)],
    &[(5, Q::Maj7), (7, Q::Sus7), (4, Q::Min7), (9, Q::Min7)],
    &[(0, Q::SixNine), (5, Q::Maj9), (0, Q::Add9), (7, Q::Sus7)],
];

/// Rhythm cells for the melody: (step, length in steps)
const CELLS: &[&[(u32, u32)]] = &[
    &[(0, 3), (3, 3), (6, 2), (8, 6)],
    &[(0, 2), (2, 2), (4, 4), (10, 2), (12, 4)],
    &[(2, 2), (4, 2), (6, 6), (12, 4)],
    &[(0, 6), (6, 2), (8, 8)],
    &[(4, 2), (6, 2), (8, 2), (10, 6)],
    &[(0, 4), (6, 4), (12, 4)],
];
const PENTA: [i32; 5] = [0, 2, 4, 7, 9];

struct Style {
    bpm: f32,
    swing: f32,
    key: i32,
    level: f32,
    progs: &'static [Prog],
    int_min: i32,
    int_max: i32,
    mel_prob: f32,
}

fn style(m: Mood) -> Style {
    match m {
        Mood::Menu | Mood::Silent => Style { bpm: 78.0, swing: 0.17, key: 53, level: 0.9, progs: MENU_PROGS, int_min: 0, int_max: 2, mel_prob: 0.6 },
        Mood::Race => Style { bpm: 94.0, swing: 0.12, key: 50, level: 0.55, progs: RACE_PROGS, int_min: 1, int_max: 2, mel_prob: 0.45 },
        Mood::Results => Style { bpm: 100.0, swing: 0.14, key: 55, level: 0.95, progs: RESULTS_PROGS, int_min: 1, int_max: 2, mel_prob: 0.8 },
    }
}

struct Layer {
    bus: u8,
    mood: Mood,
    playing: bool,
    gain: f32,
    target: f32,
    next_at: f64,
    step: u32,
    prog: Prog,
    intensity: i32,
    mel_on: bool,
    mel_idx: i32,
    mel_cell: usize,
    mel_inst: u8,
    rng: Rng,
}

impl Layer {
    fn new(bus: u8, seed: u32) -> Layer {
        Layer {
            bus,
            mood: Mood::Silent,
            playing: false,
            gain: 0.0,
            target: 0.0,
            next_at: 0.0,
            step: 0,
            prog: MENU_PROGS[0],
            intensity: 0,
            mel_on: false,
            mel_idx: 5,
            mel_cell: 0,
            mel_inst: 0,
            rng: Rng::new(seed),
        }
    }
}

pub struct MusicPlayer {
    sr: f32,
    clock: u64,
    layers: [Layer; 2],
    active: Option<usize>,
    rng: Rng,
    wow_ph: [f32; 2],
    wow: f32,
    crackle_env: f32,
    crackle_lp: OnePole,
    hiss_lp: OnePole,
    crackle_pan: f32,
}

impl MusicPlayer {
    pub fn new(sr: f32, seed: u32) -> MusicPlayer {
        MusicPlayer {
            sr,
            clock: 0,
            layers: [Layer::new(1, seed ^ 0x51), Layer::new(2, seed ^ 0x9e3)],
            active: None,
            rng: Rng::new(seed),
            wow_ph: [0.0; 2],
            wow: 1.0,
            crackle_env: 0.0,
            crackle_lp: OnePole::default(),
            hiss_lp: OnePole::default(),
            crackle_pan: 0.0,
        }
    }

    pub fn mood(&self) -> Mood {
        self.active.map(|i| self.layers[i].mood).unwrap_or(Mood::Silent)
    }

    pub fn set_mood(&mut self, m: Mood, pool: &mut NotePool) {
        if m == Mood::Silent {
            for l in self.layers.iter_mut() {
                l.target = 0.0;
            }
            self.active = None;
            return;
        }
        if let Some(a) = self.active {
            if self.layers[a].mood == m && self.layers[a].target > 0.0 {
                return;
            }
            self.layers[a].target = 0.0;
        }
        // take the quieter layer for the new mood
        let idx = if self.layers[0].gain <= self.layers[1].gain { 0 } else { 1 };
        let idx = if Some(idx) == self.active { 1 - idx } else { idx };
        let seed = self.rng.next_u32();
        let st = style(m);
        let l = &mut self.layers[idx];
        if l.playing {
            pool.release_bus(l.bus);
        }
        l.mood = m;
        l.playing = true;
        l.gain = 0.0;
        l.target = st.level;
        l.next_at = self.clock as f64 + self.sr as f64 * 0.25;
        l.step = 0;
        l.rng = Rng::new(seed);
        l.prog = st.progs[(l.rng.next_u32() as usize) % st.progs.len()];
        l.intensity = if m == Mood::Menu { 0 } else { 1 };
        l.mel_on = false;
        l.mel_idx = 5;
        l.mel_inst = 0;
        self.active = Some(idx);
    }

    /// Gain per note bus (index = bus)
    pub fn layer_gains(&self) -> [f32; 4] {
        [1.0, self.layers[0].gain, self.layers[1].gain, 0.0]
    }

    pub fn wow(&self) -> f32 {
        self.wow
    }

    pub fn tick(&mut self, n: usize, pool: &mut NotePool) {
        let sr = self.sr;
        let end = (self.clock + n as u64) as f64;
        for li in 0..2 {
            // fades: ~2.2 s crossfade
            let l = &mut self.layers[li];
            let step = n as f32 / (2.2 * sr);
            if l.gain < l.target {
                l.gain = (l.gain + step).min(l.target);
            } else if l.gain > l.target {
                l.gain = (l.gain - step * 1.3).max(l.target);
            }
            if !l.playing {
                continue;
            }
            if l.target == 0.0 && l.gain < 0.3 {
                // fading out: stop starting new notes
                l.playing = false;
                continue;
            }
            let st = style(l.mood);
            let step_len = 60.0 / st.bpm as f64 / 4.0 * sr as f64;
            while l.next_at < end {
                let offset = ((l.next_at - self.clock as f64) / sr as f64).max(0.0) as f32;
                play_step(l, &st, offset, step_len as f32 / sr, pool);
                l.step += 1;
                l.next_at += step_len;
            }
        }
        self.clock += n as u64;
        // tape wow (control rate)
        let dt = n as f32 / sr;
        self.wow_ph[0] = (self.wow_ph[0] + 0.33 * dt).fract();
        self.wow_ph[1] = (self.wow_ph[1] + 0.87 * dt).fract();
        self.wow = 1.0 + 0.0016 * sin_turns(self.wow_ph[0]) + 0.0006 * sin_turns(self.wow_ph[1]);
    }

    /// Vinyl crackle + faint hiss, following the music level
    pub fn render_crackle(&mut self, n: usize, l: &mut [f32; BLOCK], r: &mut [f32; BLOCK]) {
        let lvl = self.layers[0].gain.max(self.layers[1].gain);
        if lvl < 1e-3 {
            return;
        }
        let rate = 9.0 / self.sr;
        let dec = (-1.0 / (0.0004 * self.sr)).exp();
        let a = onepole_coef(3200.0, self.sr);
        let ha = onepole_coef(4500.0, self.sr);
        for i in 0..n {
            if self.rng.chance(rate) {
                let x = self.rng.f();
                self.crackle_env = x * x * x * if self.rng.chance(0.08) { 0.12 } else { 0.05 };
                self.crackle_pan = self.rng.bi() * 0.6;
            }
            let nz = self.rng.bi();
            let c = self.crackle_lp.lp(nz * self.crackle_env, a) * 2.5;
            self.crackle_env = flush(self.crackle_env * dec);
            let h = self.hiss_lp.lp(self.rng.bi(), ha) * 0.004;
            let v = lvl;
            l[i] += (c * (1.0 - self.crackle_pan) + h) * v;
            r[i] += (c * (1.0 + self.crackle_pan) + h) * v;
        }
    }
}

fn chord_tones(key: i32, root: i32, q: Q, center: i32, rootless: bool) -> ([i32; 6], usize) {
    let ivs = intervals(q);
    let mut out = [0i32; 6];
    let mut k = 0;
    for &iv in ivs {
        if rootless && iv == 0 && ivs.len() >= 4 {
            continue;
        }
        let mut p = key + root + iv;
        while p < center - 6 {
            p += 12;
        }
        while p >= center + 6 {
            p -= 12;
        }
        out[k] = p;
        k += 1;
    }
    out[..k].sort_unstable();
    (out, k)
}

fn has_pc(key: i32, root: i32, q: Q, pitch: i32) -> bool {
    let pc = (pitch - key - root).rem_euclid(12);
    intervals(q).iter().any(|iv| iv % 12 == pc)
}

fn bass_note(key: i32, root: i32) -> i32 {
    let mut b = key - 12 + root;
    while b > 47 {
        b -= 12;
    }
    while b < 36 {
        b += 12;
    }
    b
}

fn play_step(l: &mut Layer, st: &Style, offset: f32, step_s: f32, pool: &mut NotePool) {
    let s = l.step % 16;
    let bar = l.step / 16;
    let bus = l.bus;
    let mood = l.mood;
    let rng = &mut l.rng;

    // ---- bar-level decisions ----
    if s == 0 {
        if bar > 0 && bar % 8 == 0 && rng.chance(0.5) {
            let next = st.progs[(rng.next_u32() as usize) % st.progs.len()];
            l.prog = next;
        }
        if bar % 4 == 0 && bar > 0 {
            let d = rng.pick(&[-1, 0, 0, 1, 1]);
            l.intensity = (l.intensity + d).clamp(st.int_min, st.int_max);
        }
        if mood == Mood::Menu && bar < 4 {
            l.intensity = 0;
        }
        if bar % 2 == 0 {
            l.mel_on = bar >= 2 && rng.chance(st.mel_prob);
            l.mel_inst = (rng.next_u32() % 2) as u8;
        }
        l.mel_cell = (rng.next_u32() as usize) % CELLS.len();
    }
    let ci = (bar % 4) as usize % l.prog.len();
    let (root, q) = l.prog[ci];
    let (nroot, _) = l.prog[(ci + 1) % l.prog.len()];
    let key = st.key;
    let int = l.intensity;
    // swing on the off 16ths + a little human timing
    let mut t = offset;
    if s % 2 == 1 {
        t += st.swing * step_s;
    }
    let hum = |rng: &mut Rng| rng.bi() * 0.004;

    // ---- drums ----
    let kick = |pool: &mut NotePool, rng: &mut Rng, v: f32| {
        pool.play(Note::new(Inst::Kick, 105.0, 0.30 * v).glide_to(47.0, 0.03).bus(bus).at(t + hum(rng)));
    };
    let brush = |pool: &mut NotePool, rng: &mut Rng, v: f32| {
        pool.play(Note::new(Inst::Brush, 2100.0, 0.12 * v).q(0.7).bus(bus).pan(0.15).send(0.15).at(t + hum(rng)));
    };
    let hat = |pool: &mut NotePool, rng: &mut Rng, v: f32| {
        pool.play(Note::new(Inst::Hat, 7600.0, 0.04 * v).env(0.001, 0.028).bus(bus).pan(-0.25).at(t + hum(rng)));
    };
    let shaker = |pool: &mut NotePool, rng: &mut Rng, v: f32| {
        pool.play(Note::new(Inst::Shaker, 4800.0, 0.04 * v).bus(bus).pan(0.3).at(t + hum(rng)));
    };
    let fill_bar = bar % 8 == 7 && int >= 1;
    if int >= 1 {
        match mood {
            Mood::Race => {
                if s == 0 || s == 8 {
                    kick(pool, rng, 1.0);
                }
                if (s == 6 && rng.chance(0.5)) || (s == 11 && rng.chance(0.35)) || (s == 14 && int >= 2 && rng.chance(0.3) && !fill_bar) {
                    kick(pool, rng, 0.7);
                }
                if s == 4 || s == 12 {
                    brush(pool, rng, 1.1);
                } else if s % 2 == 1 && rng.chance(0.18) {
                    brush(pool, rng, 0.35);
                }
                if s % 2 == 0 {
                    hat(pool, rng, if s % 4 == 2 { 1.0 } else { 0.65 });
                } else if int >= 2 {
                    hat(pool, rng, 0.4);
                }
            }
            Mood::Results => {
                if s == 0 || s == 8 || (s == 6 && rng.chance(0.5)) || (s == 10 && rng.chance(0.4)) {
                    kick(pool, rng, if s % 8 == 0 { 1.0 } else { 0.7 });
                }
                if s == 4 || s == 12 {
                    brush(pool, rng, 1.0);
                }
                shaker(pool, rng, [0.9, 0.4, 0.7, 0.4][(s % 4) as usize]);
            }
            _ => {
                if s == 0 || (s == 10 && rng.chance(0.6)) || (s == 7 && int >= 2 && rng.chance(0.2)) {
                    kick(pool, rng, if s == 0 { 1.0 } else { 0.65 });
                }
                if s == 4 || s == 12 {
                    brush(pool, rng, 1.0);
                } else if s % 2 == 1 && rng.chance(0.12) {
                    brush(pool, rng, 0.35);
                }
                if (int >= 2 && s % 2 == 0) || s % 4 == 2 {
                    shaker(pool, rng, if s % 4 == 2 { 0.9 } else { 0.5 });
                }
            }
        }
        if fill_bar && s >= 13 {
            brush(pool, rng, 0.4 + 0.2 * (s - 12) as f32);
        }
    } else if mood == Mood::Menu && s % 8 == 6 && bar >= 2 {
        shaker(pool, rng, 0.5);
    }

    // ---- bass ----
    let b = bass_note(key, root);
    let bass = |pool: &mut NotePool, rng: &mut Rng, m: i32, len: u32, v: f32| {
        pool.play(Note::new(Inst::Bass, midi_hz(m as f32), 0.27 * v).hold(len as f32 * step_s * 0.95, 0.07).bus(bus).at(t + hum(rng)));
    };
    let approach = |rng: &mut Rng| bass_note(key, nroot) + if rng.chance(0.5) { -1 } else { 2 };
    match mood {
        Mood::Race => match s {
            0 => bass(pool, rng, b, 3, 1.0),
            3 if rng.chance(0.5) => bass(pool, rng, b, 1, 0.6),
            6 => bass(pool, rng, b + 12, 2, 0.75),
            8 => bass(pool, rng, b + 7, 3, 0.85),
            11 if rng.chance(0.5) => bass(pool, rng, b, 1, 0.6),
            14 => {
                let a = approach(rng);
                bass(pool, rng, a, 2, 0.7)
            }
            _ => {}
        },
        Mood::Results => match s {
            0 => bass(pool, rng, b, 4, 1.0),
            6 => bass(pool, rng, b + 7, 2, 0.8),
            8 => bass(pool, rng, b + 12, 3, 0.8),
            12 => bass(pool, rng, b, 2, 0.8),
            14 if rng.chance(0.6) => {
                let a = approach(rng);
                bass(pool, rng, a, 2, 0.7)
            }
            _ => {}
        },
        _ => {
            if int == 0 {
                if s == 0 && bar >= 1 {
                    bass(pool, rng, b, 14, 0.8);
                }
            } else {
                match s {
                    0 => bass(pool, rng, b, 6, 1.0),
                    10 if rng.chance(0.6) => {
                        let m = if rng.chance(0.5) { b + 7 } else { b + 12 };
                        bass(pool, rng, m, 2, 0.7)
                    }
                    14 if rng.chance(0.35) => {
                        let a = approach(rng);
                        bass(pool, rng, a, 2, 0.6)
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- keys (e-piano) ----
    let (tones, nt) = chord_tones(key + 12, root, q, 64, true);
    let keys = |pool: &mut NotePool, rng: &mut Rng, len: u32, v: f32, top: usize| {
        let start = nt.saturating_sub(top);
        for (j, &p) in tones[start..nt].iter().enumerate() {
            let strum = j as f32 * 0.014;
            pool.play(
                Note::new(Inst::EPiano, midi_hz(p as f32), 0.075 * v * rng.range(0.85, 1.0))
                    .bright(0.3 + 0.15 * v)
                    .hold(len as f32 * step_s, 0.3)
                    .env(0.003, 1.5)
                    .pan(-0.35 + 0.7 * j as f32 / nt.max(2) as f32)
                    .bus(bus)
                    .send(0.25)
                    .at(t + strum + hum(rng)),
            );
        }
    };
    match mood {
        Mood::Race => match s {
            0 => keys(pool, rng, 3, 1.0, 6),
            6 => keys(pool, rng, 2, 0.75, 3),
            10 if rng.chance(0.6) => keys(pool, rng, 2, 0.7, 3),
            14 if rng.chance(0.3) => keys(pool, rng, 2, 0.6, 3),
            _ => {}
        },
        Mood::Results => match s {
            0 => keys(pool, rng, 7, 1.0, 6),
            8 => keys(pool, rng, 6, 0.8, 4),
            14 if rng.chance(0.3) => keys(pool, rng, 2, 0.6, 3),
            _ => {}
        },
        _ => match s {
            0 => keys(pool, rng, 14, 1.0, 6),
            10 if int >= 1 && rng.chance(0.35) => keys(pool, rng, 4, 0.6, 3),
            _ => {}
        },
    }

    // ---- pad ----
    let pad_lvl = match mood {
        Mood::Menu => {
            if int == 0 {
                1.0
            } else {
                0.6
            }
        }
        _ => 0.0,
    };
    if s == 0 && pad_lvl > 0.0 {
        let (pt, pn) = chord_tones(key + 12, root, q, 60, false);
        for (j, &p) in pt[..pn].iter().enumerate() {
            pool.play(
                Note::new(Inst::Pad, midi_hz(p as f32), 0.04 * pad_lvl)
                    .bright(0.2)
                    .env(0.7, 8.0)
                    .hold(16.0 * step_s, 0.9)
                    .pan(-0.5 + j as f32 * 0.25)
                    .bus(bus)
                    .send(0.35)
                    .at(t),
            );
        }
    }

    // ---- plucked guitar ----
    let (pt, pn) = chord_tones(key + 12, root, q, 67, false);
    let pluck = |pool: &mut NotePool, rng: &mut Rng, p: i32, v: f32, d: f32| {
        pool.play(
            Note::new(Inst::Pluck, midi_hz(p as f32), 0.13 * v)
                .bright(0.35)
                .env(0.001, 1.6)
                .hold(if mood == Mood::Results { 0.6 } else { 1.0 }, 0.15)
                .pan(rng.bi() * 0.4)
                .bus(bus)
                .send(0.2)
                .at(t + d + hum(rng)),
        );
    };
    match mood {
        Mood::Menu if int >= 1 && s % 2 == 0 && rng.chance(0.85) => {
            const ARP: [usize; 8] = [0, 1, 2, 3, 2, 1, 2, 3];
            let k = ARP[(s / 2) as usize] % pn.max(1);
            pluck(pool, rng, pt[k], 0.9, 0.0);
        }
        Mood::Results if int >= 1 && s % 4 == 2 => {
            for j in 0..pn.min(3) {
                pluck(pool, rng, pt[j], 0.6, j as f32 * 0.016);
            }
            // (strums are short: hold ends with the next one)
        }
        Mood::Race if int >= 2 && s % 2 == 1 && rng.chance(0.25) => {
            let k = (rng.next_u32() as usize) % pn.max(1);
            pluck(pool, rng, pt[k] + 12, 0.45, 0.0);
        }
        _ => {}
    }

    // ---- melody ----
    if l.mel_on {
        let cell = CELLS[l.mel_cell];
        if let Some(&(_, len)) = cell.iter().find(|(st_, _)| *st_ == s) {
            // random walk on the pentatonic scale, landing on chord tones on strong steps
            let d = rng.pick(&[-2, -1, -1, 0, 1, 1, 2]);
            l.mel_idx = (l.mel_idx + d).clamp(0, 10);
            let pitch_of = |i: i32| key + 24 + 12 * (i / 5) + PENTA[(i % 5) as usize];
            let mut idx = l.mel_idx;
            if s % 8 == 0 || (bar % 2 == 1 && s >= 8) {
                for dd in [0, 1, -1, 2, -2] {
                    let c = (l.mel_idx + dd).clamp(0, 10);
                    if has_pc(key, root, q, pitch_of(c)) {
                        idx = c;
                        break;
                    }
                }
                l.mel_idx = idx;
            }
            let f = midi_hz(pitch_of(idx) as f32);
            let hold = len as f32 * step_s;
            let v = rng.range(0.8, 1.0);
            match mood {
                Mood::Race => {
                    pool.play(Note::new(Inst::Marimba, f, 0.10 * v).env(0.001, 0.35).bus(bus).pan(0.2).send(0.25).at(t + hum(rng)));
                }
                Mood::Results => {
                    pool.play(Note::new(Inst::Glock, f, 0.06 * v).env(0.001, 0.6).bus(bus).pan(0.15).send(0.3).at(t + hum(rng)));
                    pool.play(Note::new(Inst::Marimba, f * 0.5, 0.07 * v).env(0.001, 0.3).bus(bus).pan(-0.1).send(0.2).at(t));
                }
                _ => {
                    if l.mel_inst == 0 {
                        pool.play(Note::new(Inst::Glock, f, 0.05 * v).env(0.001, 0.7).bus(bus).pan(0.2).send(0.35).at(t + hum(rng)));
                    } else {
                        pool.play(
                            Note::new(Inst::EPiano, f, 0.08 * v)
                                .bright(0.6)
                                .env(0.003, 1.2)
                                .hold(hold, 0.25)
                                .bus(bus)
                                .pan(0.2)
                                .send(0.3)
                                .at(t + hum(rng)),
                        );
                    }
                }
            }
        }
    }
}
