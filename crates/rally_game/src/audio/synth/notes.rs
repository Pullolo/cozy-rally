//! A pool of short synthesized notes: the instruments of the music and of all one-shot
//! effects (marimba, glockenspiel, FM e-piano, Karplus-Strong pluck, round bass, soft drums,
//! filtered noise bursts, thumps). Voices are preallocated; nothing allocates while playing.
use super::dsp::*;
use super::BLOCK;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inst {
    Sine,
    Marimba,
    Wood,
    Glock,
    Bell,
    EPiano,
    Pluck,
    Bass,
    Kick,
    Brush,
    Hat,
    Shaker,
    /// Filtered noise: lowpass / bandpass / highpass with a gliding cutoff (freq -> freq_end)
    NoiseLp,
    NoiseBp,
    NoiseHp,
    /// Sine with fast pitch drop + a little low noise (impacts, landings)
    Thump,
    Pad,
}

#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub inst: Inst,
    pub freq: f32,
    pub freq_end: f32,
    /// Glide time constant (s)
    pub glide: f32,
    pub gain: f32,
    pub pan: f32,
    pub attack: f32,
    /// Exponential decay time constant while held (s)
    pub decay: f32,
    /// Time before release (s)
    pub hold: f32,
    /// Release time constant (s)
    pub release: f32,
    /// Start delay (s)
    pub delay: f32,
    pub q: f32,
    /// 0 = sfx, 1.. = music layer
    pub bus: u8,
    /// Reverb send
    pub send: f32,
    /// Instrument-specific brightness 0..1
    pub bright: f32,
}

impl Note {
    pub fn new(inst: Inst, freq: f32, gain: f32) -> Note {
        let (attack, decay, hold, release) = match inst {
            Inst::Sine => (0.004, 0.3, 0.3, 0.05),
            Inst::Marimba => (0.001, 0.35, 2.0, 0.05),
            Inst::Wood => (0.0005, 0.05, 0.4, 0.03),
            Inst::Glock => (0.001, 0.9, 1.6, 0.12),
            Inst::Bell => (0.001, 0.8, 2.0, 0.12),
            Inst::EPiano => (0.003, 1.4, 1.0, 0.25),
            Inst::Pluck => (0.001, 2.5, 1.5, 0.12),
            Inst::Bass => (0.006, 0.9, 0.5, 0.07),
            Inst::Kick => (0.001, 0.16, 0.6, 0.05),
            Inst::Brush => (0.012, 0.11, 0.5, 0.05),
            Inst::Hat => (0.001, 0.035, 0.3, 0.02),
            Inst::Shaker => (0.009, 0.045, 0.3, 0.02),
            Inst::NoiseLp | Inst::NoiseBp | Inst::NoiseHp => (0.005, 0.2, 2.0, 0.05),
            Inst::Thump => (0.001, 0.12, 1.0, 0.05),
            Inst::Pad => (0.6, 6.0, 2.0, 0.8),
        };
        Note { inst, freq, freq_end: freq, glide: 0.0, gain, pan: 0.0, attack, decay, hold, release, delay: 0.0, q: 0.8, bus: 0, send: 0.0, bright: 0.5 }
    }
    pub fn pan(mut self, p: f32) -> Note {
        self.pan = p;
        self
    }
    pub fn at(mut self, delay: f32) -> Note {
        self.delay = delay.max(0.0);
        self
    }
    pub fn glide_to(mut self, f: f32, tau: f32) -> Note {
        self.freq_end = f;
        self.glide = tau;
        self
    }
    pub fn env(mut self, attack: f32, decay: f32) -> Note {
        self.attack = attack;
        self.decay = decay;
        self
    }
    pub fn hold(mut self, hold: f32, release: f32) -> Note {
        self.hold = hold;
        self.release = release;
        self
    }
    pub fn q(mut self, q: f32) -> Note {
        self.q = q;
        self
    }
    pub fn bus(mut self, b: u8) -> Note {
        self.bus = b;
        self
    }
    pub fn send(mut self, s: f32) -> Note {
        self.send = s;
        self
    }
    pub fn bright(mut self, b: f32) -> Note {
        self.bright = b;
        self
    }
}

const KS_LEN: usize = 4096;

struct Voice {
    n: Note,
    active: bool,
    /// samples since start (negative while waiting for the delay)
    t: i64,
    lvl: f32,
    aux: [f32; 3],
    aux_mul: [f32; 3],
    ph: [f32; 3],
    freq: f32,
    glide_a: f32,
    svf: Svf,
    svf2: Svf,
    coef: SvfCoef,
    coef2: SvfCoef,
    lp: OnePole,
    rng: Rng,
    dmul: f32,
    rmul: f32,
    atk: f32,
    hold_s: i64,
    ks: Box<[f32]>,
    ks_w: usize,
    ks_last: f32,
    ks_damp: f32,
    order: u64,
}

pub struct NotePool {
    /// Added to every note's delay (used to queue result cues)
    pub base_delay: f32,
    sr: f32,
    voices: Vec<Voice>,
    counter: u64,
}

impl NotePool {
    pub fn new(sr: f32, n: usize) -> NotePool {
        let voices = (0..n)
            .map(|i| Voice {
                n: Note::new(Inst::Sine, 440.0, 0.0),
                active: false,
                t: 0,
                lvl: 0.0,
                aux: [0.0; 3],
                aux_mul: [0.0; 3],
                ph: [0.0; 3],
                freq: 440.0,
                glide_a: 0.0,
                svf: Svf::default(),
                svf2: Svf::default(),
                coef: SvfCoef::new(1000.0, 0.7, sr),
                coef2: SvfCoef::new(1000.0, 0.7, sr),
                lp: OnePole::default(),
                rng: Rng::new(0x1234 + i as u32 * 7919),
                dmul: 1.0,
                rmul: 1.0,
                atk: 1.0,
                hold_s: 0,
                ks: vec![0.0; KS_LEN].into_boxed_slice(),
                ks_w: 0,
                ks_last: 0.0,
                ks_damp: 0.99,
                order: 0,
            })
            .collect();
        NotePool { base_delay: 0.0, sr, voices, counter: 0 }
    }

    pub fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Stop scheduling-delayed notes and fade everything on a music layer
    pub fn release_bus(&mut self, bus: u8) {
        for v in self.voices.iter_mut() {
            if v.active && v.n.bus == bus {
                if v.t < 0 {
                    v.active = false;
                } else {
                    v.hold_s = v.hold_s.min(v.t);
                }
            }
        }
    }

    pub fn play(&mut self, mut note: Note) {
        if note.gain <= 1e-5 || !note.freq.is_finite() {
            return;
        }
        note.delay += self.base_delay;
        let sr = self.sr;
        // free voice, else steal the quietest (preferring music over effects)
        let idx = match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            None => {
                let mut best = 0;
                let mut best_score = f32::MAX;
                for (i, v) in self.voices.iter().enumerate() {
                    let mut score = v.lvl * v.n.gain;
                    if v.t < 0 {
                        score = 10.0 + v.n.gain;
                    }
                    if v.n.bus == 0 {
                        score += 0.5;
                    }
                    if score < best_score {
                        best_score = score;
                        best = i;
                    }
                }
                best
            }
        };
        self.counter += 1;
        let v = &mut self.voices[idx];
        v.n = note;
        v.active = true;
        v.t = -((note.delay * sr) as i64);
        v.lvl = 1.0;
        v.ph = [0.0; 3];
        v.freq = note.freq;
        v.glide_a = if note.glide > 0.0 { tau_coef(note.glide, sr) } else { 0.0 };
        v.svf.reset();
        v.svf2.reset();
        v.lp = OnePole::default();
        v.dmul = (-1.0 / (note.decay.max(0.001) * sr)).exp();
        v.rmul = (-1.0 / (note.release.max(0.001) * sr)).exp();
        v.atk = (note.attack * sr).max(1.0);
        v.hold_s = (note.hold * sr) as i64;
        v.order = self.counter;
        let f = note.freq;
        let dm = |tau: f32| (-1.0 / (tau * sr)).exp();
        v.aux = [1.0; 3];
        v.aux_mul = [1.0; 3];
        match note.inst {
            Inst::Marimba => {
                v.aux_mul = [dm(0.045), dm(0.012), dm(0.0025)];
            }
            Inst::Wood => {
                v.aux_mul = [dm(0.02), dm(0.0025), 1.0];
                v.coef = SvfCoef::new((f * 3.0).min(sr * 0.4), 1.5, sr);
            }
            Inst::Glock => {
                v.aux_mul = [dm(0.35), dm(0.12), dm(0.003)];
            }
            Inst::Bell => {
                v.aux_mul = [dm(0.25), dm(0.004), 1.0];
            }
            Inst::EPiano => {
                v.aux_mul = [dm(0.5 + 0.3 * (1.0 - note.bright)), dm(0.06), dm(0.004)];
            }
            Inst::Bass => {
                v.aux_mul = [dm(0.05), 1.0, 1.0];
                v.coef = SvfCoef::new(f * 5.0, 0.6, sr);
            }
            Inst::Kick => {
                v.aux_mul = [dm(0.004), 1.0, 1.0];
                v.coef = SvfCoef::new(2500.0, 0.7, sr);
            }
            Inst::Brush => {
                v.coef = SvfCoef::new(f, note.q, sr);
                v.coef2 = SvfCoef::new(f * 2.4, 1.0, sr);
                v.aux_mul = [dm(0.03), 1.0, 1.0];
            }
            Inst::Hat => v.coef = SvfCoef::new(f, 0.7, sr),
            Inst::Shaker => v.coef = SvfCoef::new(f, 1.2, sr),
            Inst::NoiseLp | Inst::NoiseBp | Inst::NoiseHp => v.coef = SvfCoef::new(f, note.q, sr),
            Inst::Thump => {
                v.coef = SvfCoef::new(350.0, 0.7, sr);
                v.aux_mul = [dm(0.03), 1.0, 1.0];
            }
            Inst::Pad => v.coef = SvfCoef::new(f * (2.0 + 3.0 * note.bright), 0.6, sr),
            Inst::Pluck => {
                // excitation: soft (lowpassed) noise burst
                let len = (sr / f).clamp(2.0, (KS_LEN - 4) as f32);
                let fill = len.ceil() as usize + 2;
                let a = 0.25 + 0.6 * note.bright;
                let mut z = 0.0;
                let mut z2 = 0.0;
                for k in 0..fill {
                    z += (v.rng.bi() - z) * a;
                    z2 += (z - z2) * a;
                    v.ks[k] = z2 * 2.0;
                }
                // remove DC of the excitation
                let mean = v.ks[..fill].iter().sum::<f32>() / fill as f32;
                for k in 0..fill {
                    v.ks[k] -= mean;
                }
                v.ks_w = fill;
                v.ks_last = 0.0;
                // per-period loss -> decay time scaled by note.decay (seconds to -60dB)
                let periods = note.decay.max(0.05) * f;
                v.ks_damp = (0.001f32).powf(1.0 / periods).min(0.9995);
                // the string itself decays; the envelope only shapes the release
                v.dmul = 1.0;
            }
            _ => {}
        }
    }

    /// Render all voices, adding into the buses. `layer_gains[bus]` scales music layers.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        n: usize,
        layer_gains: &[f32; 4],
        wow: f32,
        send_gain: [f32; 2],
        sfx_l: &mut [f32; BLOCK],
        sfx_r: &mut [f32; BLOCK],
        mus_l: &mut [f32; BLOCK],
        mus_r: &mut [f32; BLOCK],
        rev_l: &mut [f32; BLOCK],
        rev_r: &mut [f32; BLOCK],
    ) {
        let sr = self.sr;
        let inv_sr = 1.0 / sr;
        for v in self.voices.iter_mut() {
            if !v.active {
                continue;
            }
            // still waiting for its start time: skip the whole block
            if v.t + (n as i64) <= 0 {
                v.t += n as i64;
                continue;
            }
            let bus = v.n.bus as usize;
            let lg = if bus == 0 { 1.0 } else { layer_gains[bus.min(3)] };
            let (pl, pr) = pan_gains(v.n.pan);
            let gl = v.n.gain * pl * lg;
            let gr = v.n.gain * pr * lg;
            let send = v.n.send * send_gain[(bus != 0) as usize];
            let pitch = if bus == 0 { 1.0 } else { wow };
            let (ol, or) = if bus == 0 { (&mut *sfx_l, &mut *sfx_r) } else { (&mut *mus_l, &mut *mus_r) };
            // filter cutoff glide for noise voices (control rate)
            if matches!(v.n.inst, Inst::NoiseLp | Inst::NoiseBp | Inst::NoiseHp) && v.glide_a > 0.0 {
                v.coef = SvfCoef::new(v.freq, v.n.q, sr);
            }
            for i in 0..n {
                if v.t < 0 {
                    v.t += 1;
                    continue;
                }
                if v.glide_a > 0.0 {
                    v.freq += (v.n.freq_end - v.freq) * v.glide_a;
                }
                let f = v.freq * pitch;
                let inc = f * inv_sr;
                let x = render_inst(v, inc, f, sr);
                // envelope
                let a = if (v.t as f32) < v.atk {
                    let u = v.t as f32 / v.atk;
                    u * u * (3.0 - 2.0 * u)
                } else {
                    v.lvl *= if v.t < v.hold_s { v.dmul } else { v.rmul };
                    1.0
                };
                let y = x * a * v.lvl;
                ol[i] += y * gl;
                or[i] += y * gr;
                if send > 0.0 {
                    rev_l[i] += y * gl * send;
                    rev_r[i] += y * gr * send;
                }
                v.t += 1;
                // done once inaudible: ~-85 dBFS before bus volumes
                if v.lvl * v.n.gain < 6e-5 {
                    v.active = false;
                    break;
                }
            }
        }
    }
}

#[inline]
fn adv(ph: &mut f32, inc: f32) -> f32 {
    *ph += inc;
    if *ph >= 1.0 {
        *ph -= ph.floor();
    }
    *ph
}

#[inline]
fn render_inst(v: &mut Voice, inc: f32, f: f32, sr: f32) -> f32 {
    match v.n.inst {
        Inst::Sine => sin_turns(adv(&mut v.ph[0], inc)),
        Inst::Marimba => {
            let p = adv(&mut v.ph[0], inc);
            let click = v.rng.bi() * v.aux[2] * 0.25;
            let x = sin_turns(p) + 0.32 * v.aux[0] * sin_turns(4.0 * p) + 0.12 * v.aux[1] * sin_turns(9.9 * p) + click;
            for k in 0..3 {
                v.aux[k] *= v.aux_mul[k];
            }
            x
        }
        Inst::Wood => {
            let p = adv(&mut v.ph[0], inc);
            let p2 = adv(&mut v.ph[1], inc * 2.72);
            let click = v.svf.bp(v.rng.bi(), &v.coef) * v.aux[1];
            let x = sin_turns(p) * 0.8 + 0.5 * v.aux[0] * sin_turns(p2) + click * 0.6;
            v.aux[0] *= v.aux_mul[0];
            v.aux[1] *= v.aux_mul[1];
            x
        }
        Inst::Glock => {
            let p = adv(&mut v.ph[0], inc);
            let p2 = adv(&mut v.ph[1], inc * 2.756);
            let p3 = adv(&mut v.ph[2], inc * 5.404);
            let x = sin_turns(p) + 0.3 * v.aux[0] * sin_turns(p2) + 0.08 * v.aux[1] * sin_turns(p3) + 0.1 * v.aux[2] * v.rng.bi();
            for k in 0..3 {
                v.aux[k] *= v.aux_mul[k];
            }
            x
        }
        Inst::Bell => {
            // FM bell: ratio 3.5, decaying index
            let p = adv(&mut v.ph[0], inc);
            let m = adv(&mut v.ph[1], inc * 3.5);
            let idx = (0.35 + 1.6 * v.aux[0] * v.n.bright) / TAU;
            let x = sin_turns(p + idx * sin_turns(m)) + 0.1 * v.aux[1] * v.rng.bi();
            v.aux[0] *= v.aux_mul[0];
            v.aux[1] *= v.aux_mul[1];
            x
        }
        Inst::EPiano => {
            // Rhodes-ish: 1:1 FM body with decaying index + a quick tine partial
            let p = adv(&mut v.ph[0], inc);
            let idx = (0.15 + (0.6 + 0.9 * v.n.bright) * v.aux[0]) / TAU;
            let body = sin_turns(p + idx * sin_turns(p));
            let tine = if f * 7.0 < sr * 0.45 { sin_turns(7.0 * p) * v.aux[1] * 0.12 * v.n.bright } else { 0.0 };
            for k in 0..3 {
                v.aux[k] *= v.aux_mul[k];
            }
            body + tine
        }
        Inst::Pluck => {
            let len = (sr / f).clamp(2.0, (KS_LEN - 4) as f32) - 0.5;
            let mut rp = v.ks_w as f32 - len;
            if rp < 0.0 {
                rp += KS_LEN as f32;
            }
            let i0 = rp as usize;
            let fr = rp - i0 as f32;
            let a = v.ks[i0 % KS_LEN];
            let b = v.ks[(i0 + 1) % KS_LEN];
            let y = a + (b - a) * fr;
            let fb = v.ks_damp * 0.5 * (y + v.ks_last);
            v.ks_last = y;
            v.ks[v.ks_w] = fb;
            v.ks_w = (v.ks_w + 1) % KS_LEN;
            y
        }
        Inst::Bass => {
            let p = adv(&mut v.ph[0], inc);
            let s = sin_turns(p) + 0.22 * sin_turns(2.0 * p) + 0.18 * v.aux[0] * sin_turns(3.0 * p);
            v.aux[0] *= v.aux_mul[0];
            let s = s / (1.0 + 0.25 * s.abs());
            v.svf.lp(s, &v.coef)
        }
        Inst::Kick => {
            let p = adv(&mut v.ph[0], inc);
            let click = v.svf.lp(v.rng.bi(), &v.coef) * v.aux[0] * 0.5;
            v.aux[0] *= v.aux_mul[0];
            sin_turns(p) + click
        }
        Inst::Brush => {
            let nz = v.rng.bi();
            let body = sin_turns(adv(&mut v.ph[0], 190.0 / sr)) * v.aux[0] * 0.35;
            v.aux[0] *= v.aux_mul[0];
            v.svf.bp(nz, &v.coef) * 0.8 + v.svf2.bp(nz, &v.coef2) * 0.3 + body
        }
        Inst::Hat => v.svf.hp(v.rng.bi(), &v.coef) * 0.7,
        Inst::Shaker => v.svf.bp(v.rng.bi(), &v.coef),
        Inst::NoiseLp => v.svf.lp(v.rng.bi(), &v.coef),
        Inst::NoiseBp => v.svf.bp(v.rng.bi(), &v.coef),
        Inst::NoiseHp => v.svf.hp(v.rng.bi(), &v.coef),
        Inst::Thump => {
            let p = adv(&mut v.ph[0], inc);
            let nz = v.svf.lp(v.rng.bi(), &v.coef) * (0.3 + 0.7 * v.aux[0]);
            v.aux[0] *= v.aux_mul[0];
            sin_turns(p) + nz * 0.6
        }
        Inst::Pad => {
            let p = adv(&mut v.ph[0], inc);
            let p2 = adv(&mut v.ph[1], inc * 1.0042);
            let p3 = adv(&mut v.ph[2], inc * 0.9961);
            let s = sin_turns(p) + 0.7 * sin_turns(p2) + 0.7 * sin_turns(p3) + 0.25 * sin_turns(2.0 * p2) + 0.12 * sin_turns(3.0 * p3);
            v.svf.lp(s * 0.4, &v.coef)
        }
    }
}
