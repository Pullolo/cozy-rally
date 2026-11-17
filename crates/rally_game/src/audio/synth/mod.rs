//! The procedural synthesizer: engines, tyres, one-shot effects, generative music and the
//! master bus. Pure std (no audio backend, no game types) so it can also run offline
//! (see `examples/audio_render.rs`).
//!
//! The synth is driven by `Cmd`s (plain `Copy` data, safe to receive inside the real-time
//! callback) and pulled with `render_block`. Nothing here allocates after `Synth::new`.

pub mod dsp;
pub mod engine;
pub mod music;
pub mod notes;
pub mod reverb;
pub mod sfx;

use dsp::*;
use engine::EngineVoice;
use music::MusicPlayer;
use notes::NotePool;
use reverb::Reverb;

/// Max simultaneous engine voices (local players + nearest AI)
pub const MAX_CARS: usize = 8;
/// Internal processing block (frames)
pub const BLOCK: usize = 64;

/// Per-car parameters for one engine+tyre voice. Sent every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EngineParams {
    /// Stable key (car index in the world)
    pub id: u32,
    /// Engine character (CarSpec::id): 0 Pip, 1 Bramble, 2 Thistle, 3 Burrow
    pub kind: u8,
    pub rpm: f32,
    pub idle_rpm: f32,
    pub redline: f32,
    pub throttle: f32,
    /// 0..1 engine load (effective throttle)
    pub load: f32,
    pub shifting: bool,
    /// m/s
    pub speed: f32,
    /// Fraction of wheels touching the ground
    pub ground: f32,
    /// Surface mix of the touching wheels (each 0..1, sum ~1 when on the ground)
    pub s_hard: f32,
    pub s_loose: f32,
    pub s_snow: f32,
    pub s_grass: f32,
    /// Traction loss 0..1, split by kind (lateral slide / wheelspin / brake lock) and surface
    pub slip_lat_hard: f32,
    pub slip_lat_loose: f32,
    pub spin_hard: f32,
    pub spin_loose: f32,
    pub lock_hard: f32,
    pub lock_loose: f32,
    pub in_water: bool,
    /// Distance attenuation 0..1
    pub gain: f32,
    /// -1 left .. 1 right
    pub pan: f32,
    /// Pitch factor (1 = none)
    pub doppler: f32,
    /// 0 = close, 1 = far away (air absorption lowpass)
    pub muffle: f32,
    /// Local player car: gets wind noise and full detail
    pub local: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SfxKind {
    /// Car-car (`scenery` false) or car-scenery impact; strength in m/s
    Collision { strength: f32, scenery: bool },
    /// Suspension thump; strength = landing speed m/s
    Landing { strength: f32 },
    CountdownTick(u32),
    Go,
    Checkpoint,
    Lap,
    FinalLap,
    Finish { place: u32 },
    Respawn,
    Splash { strength: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKind {
    Move,
    Select,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Menu,
    Race,
    Results,
    Silent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    Cars { n: u8, cars: [EngineParams; MAX_CARS], paused: bool },
    StopCars,
    Sfx { kind: SfxKind, gain: f32, pan: f32 },
    Ui(UiKind),
    Music(Mood),
    Volumes { master: f32, music: f32, sfx: f32 },
}

/// Bus tags for notes
pub const BUS_SFX: u8 = 0;

pub struct Synth {
    pub sr: f32,
    engines: Vec<EngineVoice>,
    pub notes: NotePool,
    pub music: MusicPlayer,
    reverb: Reverb,
    rng: Rng,
    // volumes
    vol_master: f32,
    vol_music: f32,
    vol_sfx: f32,
    sm_master: f32,
    sm_music: f32,
    sm_sfx: f32,
    // engine watchdog: frames since the last Cars command
    cars_age: u64,
    cars_interval: f32,
    // master
    dc_l: DcBlock,
    dc_r: DcBlock,
    dc_r_coef: f32,
    lim_env: f32,
    lofi_l: OnePole,
    lofi_r: OnePole,
    lofi_a: f32,
    // block buffers
    mus_l: [f32; BLOCK],
    mus_r: [f32; BLOCK],
    sfx_l: [f32; BLOCK],
    sfx_r: [f32; BLOCK],
    rev_l: [f32; BLOCK],
    rev_r: [f32; BLOCK],
    pub out_l: [f32; BLOCK],
    pub out_r: [f32; BLOCK],
    /// Peak of the master bus before the limiter (for diagnostics)
    pub pre_peak: f32,
}

impl Synth {
    pub fn new(sr: f32) -> Synth {
        let sr = sr.clamp(8000.0, 384_000.0);
        Synth {
            sr,
            engines: (0..MAX_CARS).map(|i| EngineVoice::new(sr, 0x5eed + i as u32 * 977)).collect(),
            notes: NotePool::new(sr, 72),
            music: MusicPlayer::new(sr, 0xC0217),
            reverb: Reverb::new(sr),
            rng: Rng::new(0xA0D10),
            vol_master: 0.85,
            vol_music: 0.55,
            vol_sfx: 0.9,
            sm_master: 0.0,
            sm_music: 0.55,
            sm_sfx: 0.9,
            cars_age: u64::MAX / 2,
            cars_interval: sr / 60.0,
            dc_l: DcBlock::default(),
            dc_r: DcBlock::default(),
            dc_r_coef: dc_coef(8.0, sr),
            lim_env: 0.0,
            lofi_l: OnePole::default(),
            lofi_r: OnePole::default(),
            lofi_a: onepole_coef(7500.0, sr),
            mus_l: [0.0; BLOCK],
            mus_r: [0.0; BLOCK],
            sfx_l: [0.0; BLOCK],
            sfx_r: [0.0; BLOCK],
            rev_l: [0.0; BLOCK],
            rev_r: [0.0; BLOCK],
            out_l: [0.0; BLOCK],
            out_r: [0.0; BLOCK],
            pre_peak: 0.0,
        }
    }

    pub fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Cars { n, cars, paused } => {
                let n = (n as usize).min(MAX_CARS);
                // estimate the update interval so engine smoothing adapts to the frame rate
                if self.cars_age < (self.sr as u64) {
                    self.cars_interval += (self.cars_age as f32 - self.cars_interval) * 0.2;
                }
                self.cars_age = 0;
                let interval = self.cars_interval / self.sr;
                let mut seen = [false; MAX_CARS];
                for p in cars.iter().take(n) {
                    if let Some(slot) = self.engines.iter().position(|v| v.active && v.id == p.id) {
                        self.engines[slot].set(p, paused, interval);
                        seen[slot] = true;
                    } else if let Some(slot) = self.engines.iter().position(|v| !v.active) {
                        self.engines[slot].start(p);
                        self.engines[slot].set(p, paused, interval);
                        seen[slot] = true;
                    }
                }
                for (i, v) in self.engines.iter_mut().enumerate() {
                    if v.active && !seen[i] {
                        v.release();
                    }
                }
            }
            Cmd::StopCars => {
                for v in self.engines.iter_mut() {
                    v.release();
                }
            }
            Cmd::Sfx { kind, gain, pan } => sfx::play(&mut self.notes, &mut self.rng, kind, gain, pan),
            Cmd::Ui(k) => sfx::ui(&mut self.notes, &mut self.rng, k),
            Cmd::Music(m) => self.music.set_mood(m, &mut self.notes),
            Cmd::Volumes { master, music, sfx } => {
                self.vol_master = master.clamp(0.0, 1.0);
                self.vol_music = music.clamp(0.0, 1.0);
                self.vol_sfx = sfx.clamp(0.0, 1.0);
            }
        }
    }

    /// Render `n <= BLOCK` frames into `out_l` / `out_r`.
    pub fn render_block(&mut self, n: usize) {
        let n = n.min(BLOCK);
        self.mus_l[..n].fill(0.0);
        self.mus_r[..n].fill(0.0);
        self.sfx_l[..n].fill(0.0);
        self.sfx_r[..n].fill(0.0);
        self.rev_l[..n].fill(0.0);
        self.rev_r[..n].fill(0.0);

        // engines fade out on their own if the game stops sending updates (pause, menus)
        self.cars_age = self.cars_age.saturating_add(n as u64);
        let stale = self.cars_age as f32 > (self.sr * 0.25).max(self.cars_interval * 3.0);
        for v in self.engines.iter_mut() {
            if v.active {
                v.render(n, stale, &mut self.sfx_l, &mut self.sfx_r, &mut self.rev_l, &mut self.rev_r);
            }
        }

        // music sequencer schedules notes, then all notes render
        self.music.tick(n, &mut self.notes);
        let layer_gains = self.music.layer_gains();
        self.notes.render(
            n,
            &layer_gains,
            self.music.wow(),
            &mut self.sfx_l,
            &mut self.sfx_r,
            &mut self.mus_l,
            &mut self.mus_r,
            &mut self.rev_l,
            &mut self.rev_r,
        );
        self.music.render_crackle(n, &mut self.mus_l, &mut self.mus_r);

        let a = tau_coef(0.08, self.sr / n as f32);
        self.sm_master += (self.vol_master - self.sm_master) * a;
        self.sm_music += (self.vol_music - self.sm_music) * a;
        self.sm_sfx += (self.vol_sfx - self.sm_sfx) * a;
        let (gm, gs) = (self.sm_music, self.sm_sfx);
        // reverb send scaled like the buses feeding it (music dominates the send)
        for i in 0..n {
            self.rev_l[i] *= gs.max(gm);
            self.rev_r[i] *= gs.max(gm);
        }
        self.reverb.process(n, &mut self.rev_l, &mut self.rev_r);

        let rel = tau_coef(0.25, self.sr);
        let att = tau_coef(0.0015, self.sr);
        let mut peak = self.pre_peak;
        for i in 0..n {
            // gentle "lofi" lowpass on the music bus only
            let ml = self.lofi_l.lp(self.mus_l[i], self.lofi_a);
            let mr = self.lofi_r.lp(self.mus_r[i], self.lofi_a);
            let mut l = (ml * gm + self.sfx_l[i] * gs + self.rev_l[i]) * self.sm_master;
            let mut r = (mr * gm + self.sfx_r[i] * gs + self.rev_r[i]) * self.sm_master;
            l = self.dc_l.process(l, self.dc_r_coef);
            r = self.dc_r.process(r, self.dc_r_coef);
            if !(l.is_finite() && r.is_finite()) {
                l = 0.0;
                r = 0.0;
                self.dc_l = DcBlock::default();
                self.dc_r = DcBlock::default();
            }
            let pk = l.abs().max(r.abs());
            peak = peak.max(pk);
            // peak limiter (fast attack, slow release) towards 0.75 ...
            let c = if pk > self.lim_env { att } else { rel };
            self.lim_env += (pk - self.lim_env) * c;
            let g = if self.lim_env > 0.75 { 0.75 / self.lim_env } else { 1.0 };
            // ... then a soft clipper that can never exceed 0.99
            self.out_l[i] = soft_clip(l * g);
            self.out_r[i] = soft_clip(r * g);
        }
        self.pre_peak = peak;
    }

    /// Convenience: render interleaved stereo
    pub fn render_interleaved(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(BLOCK);
            self.render_block(n);
            for i in 0..n {
                out[(done + i) * 2] = self.out_l[i];
                out[(done + i) * 2 + 1] = self.out_r[i];
            }
            done += n;
        }
    }

    pub fn active_engines(&self) -> usize {
        self.engines.iter().filter(|v| v.active).count()
    }
}

/// Transparent below 0.7, smoothly saturating to at most 0.99 above.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    const K: f32 = 0.7;
    const H: f32 = 0.29;
    let a = x.abs();
    if a <= K {
        x
    } else {
        let t = (a - K) / H;
        // rational tanh approximation (monotonic, < 1)
        let s = t * (27.0 + t * t) / (27.0 + 9.0 * t * t);
        x.signum() * (K + H * s.min(1.0))
    }
}
