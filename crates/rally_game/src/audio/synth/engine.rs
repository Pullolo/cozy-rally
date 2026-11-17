//! Engine + tyre + wind voice for one car.
//!
//! Engine model: the 4-stroke cycle phase (f0 = rpm / 120) drives one smooth pressure pulse per
//! cylinder at its firing offset. The pulse train (with per-cylinder imbalance and per-cycle
//! jitter) runs through a small bank of resonant "exhaust" formants and a load-dependent
//! lowpass. All engine sound is harmonic to f0 and its strongest component is the firing
//! frequency f0 * cylinders, so pitch follows rpm exactly.
use super::dsp::*;
use super::{EngineParams, BLOCK};

#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub cyl: usize,
    /// Firing offset of each cylinder within the 720 deg cycle (0..1)
    pub offsets: [f32; 8],
    /// Static per-cylinder strength (bank imbalance -> burble)
    pub amps: [f32; 8],
    /// Pulse width as a fraction of the firing interval
    pub width: f32,
    /// 0 = round pulses, 1 = snappy/buzzy pulses
    pub snap: f32,
    /// (freq Hz, Q, gain)
    pub formants: [(f32, f32, f32); 3],
    /// Direct (lowpassed) path level
    pub dry: f32,
    pub lp_base: f32,
    pub lp_rpm: f32,
    /// Combustion noise riding on the pulses
    pub grit: f32,
    /// Random per-cycle amplitude variation
    pub jitter: f32,
    /// Idle rpm wobble (V8 cam lope)
    pub lope: f32,
    /// Overrun pops
    pub pops: f32,
    /// Diesel knock clatter
    pub knock: f32,
    pub turbo: bool,
    pub gain: f32,
}

pub fn profile(kind: u8) -> Profile {
    let even = |n: usize| {
        let mut o = [0.0; 8];
        for (i, v) in o.iter_mut().enumerate().take(n) {
            *v = i as f32 / n as f32;
        }
        o
    };
    match kind {
        // Pip: buzzy little 3-cylinder hatch
        0 => Profile {
            cyl: 3,
            offsets: even(3),
            amps: [1.0, 0.82, 0.92, 0.0, 0.0, 0.0, 0.0, 0.0],
            width: 0.36,
            snap: 0.75,
            formants: [(560.0, 2.5, 0.55), (1450.0, 3.5, 0.32), (2700.0, 4.0, 0.10)],
            dry: 0.8,
            lp_base: 1300.0,
            lp_rpm: 3200.0,
            grit: 0.30,
            jitter: 0.07,
            lope: 0.0,
            pops: 0.0,
            knock: 0.0,
            turbo: false,
            gain: 0.85,
        },
        // Bramble: burbly cross-plane V8 coupe (bank pattern A B A A B B B A)
        1 => Profile {
            cyl: 8,
            offsets: even(8),
            amps: [1.0, 0.66, 1.0, 0.95, 0.70, 0.62, 0.68, 0.97],
            width: 0.80,
            snap: 0.15,
            formants: [(130.0, 1.4, 0.75), (340.0, 2.0, 0.45), (820.0, 2.5, 0.15)],
            dry: 1.0,
            lp_base: 600.0,
            lp_rpm: 2200.0,
            grit: 0.16,
            jitter: 0.12,
            lope: 0.035,
            pops: 1.0,
            knock: 0.0,
            turbo: false,
            gain: 0.95,
        },
        // Thistle: turbo 4-cylinder rally car
        2 => Profile {
            cyl: 4,
            offsets: even(4),
            amps: [1.0, 0.93, 0.97, 0.9, 0.0, 0.0, 0.0, 0.0],
            width: 0.5,
            snap: 0.45,
            formants: [(400.0, 2.0, 0.55), (1100.0, 3.0, 0.35), (2300.0, 4.0, 0.12)],
            dry: 0.9,
            lp_base: 1000.0,
            lp_rpm: 2800.0,
            grit: 0.26,
            jitter: 0.06,
            lope: 0.0,
            pops: 0.25,
            knock: 0.0,
            turbo: true,
            gain: 0.85,
        },
        // Burrow: chuggy low diesel camper
        _ => Profile {
            cyl: 4,
            offsets: [0.0, 0.255, 0.5, 0.745, 0.0, 0.0, 0.0, 0.0],
            amps: [1.0, 0.78, 0.94, 0.84, 0.0, 0.0, 0.0, 0.0],
            width: 0.62,
            snap: 0.55,
            formants: [(105.0, 1.2, 0.8), (250.0, 2.0, 0.5), (650.0, 2.5, 0.15)],
            dry: 1.0,
            lp_base: 420.0,
            lp_rpm: 1300.0,
            grit: 0.22,
            jitter: 0.16,
            lope: 0.0,
            pops: 0.0,
            knock: 0.55,
            turbo: false,
            gain: 1.0,
        },
    }
}

/// Smoothed control values (per block)
#[derive(Clone, Copy, Debug, Default)]
struct Ctl {
    speed: f32,
    ground: f32,
    s_hard: f32,
    s_loose: f32,
    s_snow: f32,
    s_grass: f32,
    slip_lat_hard: f32,
    slip_lat_loose: f32,
    spin_hard: f32,
    spin_loose: f32,
    lock_hard: f32,
    lock_loose: f32,
    water: f32,
    pan: f32,
    muffle: f32,
    doppler: f32,
}

pub struct EngineVoice {
    pub active: bool,
    pub id: u32,
    sr: f32,
    prof: Profile,
    p: EngineParams,
    paused: bool,
    releasing: bool,
    rng: Rng,
    // smoothed per-sample
    rpm: f32,
    rpm_a: f32,
    load: Smooth,
    amp: Smooth,
    fade: f32,
    cut: Smooth,
    shift: Smooth,
    gain: Smooth,
    // control
    ctl: Ctl,
    // engine state
    phase: f32,
    jit: [f32; 8],
    lim_phase: f32,
    lope: Wander,
    dc: DcBlock,
    dc_r: f32,
    fm: [Svf; 3],
    fm_c: [SvfCoef; 3],
    lp: Svf,
    lp_c: SvfCoef,
    knock_env: f32,
    knock_f: Svf,
    knock_c: SvfCoef,
    pop_env: f32,
    pop_f: Svf,
    pop_c: SvfCoef,
    // turbo
    boost: f32,
    whistle_ph: f32,
    bov_env: f32,
    bov_t: f32,
    bov_f: Svf,
    prev_thr: f32,
    spool_f: Svf,
    spool_c: SvfCoef,
    // tyres
    rumble_f: Svf,
    rumble_c: SvfCoef,
    road_f: Svf,
    road_c: SvfCoef,
    grain_env: [f32; 3],
    grain_f: [Svf; 3],
    grain_c: [SvfCoef; 3],
    bed_f: Svf,
    bed_c: SvfCoef,
    spray_f: Svf,
    spray_c: SvfCoef,
    grass_f: Svf,
    grass_c: SvfCoef,
    grass_w: Wander,
    sq_ph: f32,
    sq_vib: f32,
    sq_w: Wander,
    sq_rough: OnePole,
    sq_f: Svf,
    judder_ph: f32,
    whirr_f: Svf,
    wind_f: Svf,
    wind_w: Wander,
    water_f: Svf,
    water_w: Wander,
    muffle: OnePole,
    tyre_soft: Svf,
    tyre_soft_c: SvfCoef,
    out_dc: DcBlock,
}

impl EngineVoice {
    pub fn new(sr: f32, seed: u32) -> EngineVoice {
        let c = SvfCoef::new(1000.0, 0.7, sr);
        EngineVoice {
            active: false,
            id: u32::MAX,
            sr,
            prof: profile(0),
            p: EngineParams::default(),
            paused: false,
            releasing: false,
            rng: Rng::new(seed),
            rpm: 900.0,
            rpm_a: 0.01,
            load: Smooth::default(),
            amp: Smooth::default(),
            fade: 0.0,
            cut: Smooth::default(),
            shift: Smooth::default(),
            gain: Smooth::default(),
            ctl: Ctl::default(),
            phase: 0.0,
            jit: [1.0; 8],
            lim_phase: 0.0,
            lope: Wander::default(),
            dc: DcBlock::default(),
            dc_r: dc_coef(12.0, sr),
            fm: [Svf::default(); 3],
            fm_c: [c; 3],
            lp: Svf::default(),
            lp_c: c,
            knock_env: 0.0,
            knock_f: Svf::default(),
            knock_c: SvfCoef::new(1900.0, 1.6, sr),
            pop_env: 0.0,
            pop_f: Svf::default(),
            pop_c: SvfCoef::new(420.0, 0.9, sr),
            boost: 0.0,
            whistle_ph: 0.0,
            bov_env: 0.0,
            bov_t: 0.0,
            bov_f: Svf::default(),
            prev_thr: 0.0,
            spool_f: Svf::default(),
            spool_c: SvfCoef::new(2600.0, 3.0, sr),
            rumble_f: Svf::default(),
            rumble_c: SvfCoef::new(150.0, 0.8, sr),
            road_f: Svf::default(),
            road_c: SvfCoef::new(420.0, 0.7, sr),
            grain_env: [0.0; 3],
            grain_f: [Svf::default(); 3],
            grain_c: [SvfCoef::new(2500.0, 1.6, sr), SvfCoef::new(1100.0, 1.3, sr), SvfCoef::new(620.0, 1.0, sr)],
            bed_f: Svf::default(),
            bed_c: SvfCoef::new(1300.0, 0.6, sr),
            spray_f: Svf::default(),
            spray_c: SvfCoef::new(3800.0, 0.6, sr),
            grass_f: Svf::default(),
            grass_c: SvfCoef::new(3300.0, 0.5, sr),
            grass_w: Wander::default(),
            sq_ph: 0.0,
            sq_vib: 0.0,
            sq_w: Wander::default(),
            sq_rough: OnePole::default(),
            sq_f: Svf::default(),
            judder_ph: 0.0,
            whirr_f: Svf::default(),
            wind_f: Svf::default(),
            wind_w: Wander::default(),
            water_f: Svf::default(),
            water_w: Wander::default(),
            muffle: OnePole::default(),
            tyre_soft: Svf::default(),
            tyre_soft_c: SvfCoef::new(5200.0, 0.6, sr),
            out_dc: DcBlock::default(),
        }
    }

    pub fn start(&mut self, p: &EngineParams) {
        let seed = self.rng.next_u32();
        let sr = self.sr;
        *self = EngineVoice::new(sr, seed);
        self.active = true;
        self.id = p.id;
        self.prof = profile(p.kind);
        self.p = *p;
        self.rpm = p.rpm.max(100.0);
        self.load.v = p.load;
        self.gain.v = p.gain;
        self.ctl.pan = p.pan;
        self.ctl.doppler = p.doppler;
        self.ctl.muffle = p.muffle;
        self.ctl.speed = p.speed;
        self.ctl.ground = p.ground;
        self.prev_thr = p.throttle;
        for (i, c) in self.fm_c.iter_mut().enumerate() {
            let (f, q, _) = self.prof.formants[i];
            *c = SvfCoef::new(f, q, sr);
        }
    }

    pub fn set(&mut self, p: &EngineParams, paused: bool, interval: f32) {
        // turbo blow-off on a sharp lift
        if self.prof.turbo && self.prev_thr > 0.55 && p.throttle < 0.2 && self.boost > 0.3 {
            self.bov_env = 0.55 + 0.45 * self.boost;
            self.bov_t = 0.0;
            self.boost *= 0.35;
        }
        self.prev_thr = p.throttle;
        if p.kind != self.p.kind {
            self.prof = profile(p.kind);
        }
        self.p = *p;
        self.paused = paused;
        self.releasing = false;
        // follow rpm a bit slower than the update interval so low frame rates don't step
        let tau = (interval * 0.8).clamp(0.025, 0.25);
        self.rpm_a = tau_coef(tau, self.sr);
    }

    pub fn release(&mut self) {
        self.releasing = true;
    }

    /// Render n samples, adding to the stereo buses.
    pub fn render(&mut self, n: usize, stale: bool, out_l: &mut [f32; BLOCK], out_r: &mut [f32; BLOCK], rev_l: &mut [f32; BLOCK], rev_r: &mut [f32; BLOCK]) {
        let sr = self.sr;
        let p = self.p;
        let pr = self.prof;
        let silent = self.releasing || self.paused || stale;
        let fade_target = if silent { 0.0 } else { 1.0 };
        let fade_a = tau_coef(if silent { 0.12 } else { 0.08 }, sr);

        // ---- control rate ----
        let bs = n as f32 / sr;
        let ca = tau_coef(0.035, 1.0 / bs);
        let cs = tau_coef(0.06, 1.0 / bs);
        let c = &mut self.ctl;
        c.speed += (p.speed - c.speed) * ca;
        c.ground += (p.ground - c.ground) * tau_coef(0.02, 1.0 / bs);
        c.s_hard += (p.s_hard - c.s_hard) * cs;
        c.s_loose += (p.s_loose - c.s_loose) * cs;
        c.s_snow += (p.s_snow - c.s_snow) * cs;
        c.s_grass += (p.s_grass - c.s_grass) * cs;
        c.slip_lat_hard += (p.slip_lat_hard - c.slip_lat_hard) * ca;
        c.slip_lat_loose += (p.slip_lat_loose - c.slip_lat_loose) * ca;
        c.spin_hard += (p.spin_hard - c.spin_hard) * ca;
        c.spin_loose += (p.spin_loose - c.spin_loose) * ca;
        c.lock_hard += (p.lock_hard - c.lock_hard) * ca;
        c.lock_loose += (p.lock_loose - c.lock_loose) * ca;
        c.water += ((p.in_water as u8 as f32) - c.water) * cs;
        c.pan += (p.pan - c.pan) * cs;
        c.muffle += (p.muffle - c.muffle) * cs;
        c.doppler += (p.doppler - c.doppler) * cs;
        let c = *c;

        let span = (p.redline - p.idle_rpm).max(500.0);
        let rpm_n = clamp01((self.rpm - p.idle_rpm) / span);
        let spin = c.spin_hard.max(c.spin_loose);
        // wheelspin flares the revs a little beyond what the drivetrain reports
        let target_rpm = (p.rpm * (1.0 + 0.07 * spin)).min(p.redline + 150.0);
        let limiter = p.rpm >= p.redline - 60.0 && p.throttle > 0.4;
        let target_load = clamp01(p.load.max(p.throttle * 0.85) + 0.3 * spin);

        // brightness / formants follow load and rpm
        let lp_fc = pr.lp_base + pr.lp_rpm * rpm_n * (0.55 + 0.45 * self.load.v);
        self.lp_c = SvfCoef::new(lp_fc, 0.6, sr);
        let fgain = 0.6 + 0.4 * self.load.v;

        // turbo
        if pr.turbo {
            let bt = clamp01(self.load.v * smoothstep(0.2, 0.75, rpm_n) * 1.1);
            let tau = if bt > self.boost { 0.7 } else { 0.25 };
            self.boost += (bt - self.boost) * tau_coef(tau, 1.0 / bs);
        }

        // tyre amounts
        let sp = clamp01(c.speed / 35.0);
        let g = c.ground;
        let loose = (c.s_loose + 0.4 * c.s_grass) * g;
        let snow = c.s_snow * g;
        let slip_l = c.slip_lat_loose.max(c.lock_loose * 0.9);
        let rumble_amt = g * sp.powf(0.8) * (0.25 * c.s_hard + 0.85 * c.s_loose + 0.45 * c.s_snow + 0.4 * c.s_grass) * 0.55;
        let road_amt = g * c.s_hard * sp.powf(1.4) * 0.16;
        let grain_rate = (loose * (25.0 + 950.0 * sp) + (c.s_loose + c.s_grass) * g * (slip_l * 1900.0 + c.spin_loose * 1700.0)) / sr;
        let grain_amp = 0.25 + 0.6 * sp + 0.7 * slip_l + 0.6 * c.spin_loose;
        let snow_rate = (snow * (18.0 + 380.0 * sp) + c.s_snow * g * (slip_l + c.spin_loose) * 2600.0) / sr;
        let bed_amt = (c.s_loose * g) * (sp * 0.07 + slip_l * 0.6 + c.spin_loose * 0.38) + c.s_snow * g * (sp * 0.03 + slip_l * 0.55 + c.spin_loose * 0.3);
        let spray_amt = (c.s_loose * g) * (c.spin_loose * 0.22 + slip_l * 0.06);
        let grass_amt = c.s_grass * g * (sp * 0.12 + slip_l * 0.3);
        // asphalt squeal: lateral scrub, brake lock or wheelspin beyond grip
        let sq_lat = smoothstep(0.05, 0.9, c.slip_lat_hard);
        let sq_lock = smoothstep(0.05, 0.9, c.lock_hard);
        let sq_spin = smoothstep(0.05, 0.9, c.spin_hard);
        let sq = (sq_lat.max(sq_lock).max(sq_spin) * g).min(1.0);
        let sq_amt = 0.25 * sq.powf(1.2) * smoothstep(1.0, 5.0, c.speed + 4.0 * sq_spin);
        let sq_freq = 700.0 + 170.0 * sq_lat - 120.0 * sq_lock + 200.0 * sq_spin;
        let judder = 0.55 * sq_lock;
        let whirr_amt = 0.09 * sq_spin + 0.05 * c.spin_loose;
        let whirr_c = SvfCoef::new(280.0 + 450.0 * spin, 4.0, sr);
        let sq_c = SvfCoef::new(sq_freq * 1.6, 1.5, sr);
        let wind_amt = if p.local { 0.11 * sp * sp } else { 0.0 };
        let wind_c = SvfCoef::new(220.0 + 1000.0 * sp, 0.55, sr);
        let water_amt = c.water * (0.06 + 0.35 * sp);
        let water_c = SvfCoef::new(700.0, 0.7, sr);
        let muffle_a = onepole_coef(18000.0 * (1.0 - c.muffle).powi(2) + 700.0, sr);
        let (pl, prr) = pan_gains(c.pan);
        let grain_dec = (-1.0 / (0.0013 * sr)).exp();
        let snow_dec = (-1.0 / (0.004 * sr)).exp();
        let mut bov_c = SvfCoef::new(3600.0, 1.4, sr);
        if self.bov_env > 1e-4 {
            bov_c = SvfCoef::new(1500.0 + 2300.0 * (-self.bov_t * 6.0).exp(), 1.4, sr);
        }

        // ---- per sample ----
        let load_a = tau_coef(0.03, sr);
        let fx_a = tau_coef(0.006, sr);
        let gain_a = tau_coef(0.05, sr);
        let knock_dec = (-1.0 / (0.0025 * sr)).exp();
        let pop_dec = (-1.0 / (0.014 * sr)).exp();
        let bov_dec = (-1.0 / (0.16 * sr)).exp();
        let wp = pr.width / pr.cyl as f32;
        // quadratic phase warp: snappier attack for buzzy engines (cheap stand-in for u^a)
        let warp = 0.9 * pr.snap;
        let lope_amt = pr.lope * (1.0 - rpm_n) * (1.0 - self.load.v * 0.7);
        let base_amp = pr.gain * 0.5;

        for i in 0..n {
            self.fade += (fade_target - self.fade) * fade_a;
            self.rpm += (target_rpm - self.rpm) * self.rpm_a;
            let load = self.load.step(target_load, load_a);
            let shift = self.shift.step(if p.shifting { 1.0 } else { 0.0 }, fx_a);
            // rev limiter: ignition cut bouncing at ~14 Hz
            let mut cut_t = 0.0;
            if limiter {
                self.lim_phase += 14.0 / sr;
                if self.lim_phase >= 1.0 {
                    self.lim_phase -= 1.0;
                }
                if self.lim_phase < 0.42 {
                    cut_t = 1.0;
                }
            }
            let cut = self.cut.step(cut_t, fx_a);
            let lope = self.lope.step(&mut self.rng, 2.2, sr) * lope_amt;
            let rpm_eff = self.rpm * (1.0 + lope) * (1.0 - 0.035 * cut - 0.02 * shift) * c.doppler;
            let eff_load = load * (1.0 - cut) * (1.0 - shift);

            // cycle phase
            let prev = self.phase;
            self.phase += rpm_eff / 120.0 / sr;
            let wrapped = self.phase >= 1.0;
            if wrapped {
                self.phase -= 1.0;
                for k in 0..pr.cyl {
                    self.jit[k] = 1.0 + pr.jitter * self.rng.bi() * (1.2 - 0.6 * rpm_n);
                }
            }
            let mut s = 0.0;
            for k in 0..pr.cyl {
                let off = pr.offsets[k];
                let mut q = self.phase - off;
                if q < 0.0 {
                    q += 1.0;
                }
                if q < wp {
                    let u0 = q / wp;
                    let u = u0 * (1.0 + warp * (1.0 - u0));
                    s += pr.amps[k] * self.jit[k] * (0.5 - 0.5 * cos_turns(u));
                }
                // firing event (crossing the offset)
                let crossed = if wrapped { prev < off || self.phase >= off } else { prev < off && self.phase >= off };
                if crossed {
                    if pr.knock > 0.0 {
                        let k_amt = pr.knock * (1.0 - 0.7 * rpm_n) * (0.4 + 0.6 * eff_load.max(0.3)) * self.rng.range(0.6, 1.0);
                        self.knock_env = self.knock_env.max(k_amt);
                    }
                    if pr.pops > 0.0 && load < 0.08 && rpm_n > 0.3 && !p.shifting && self.rng.chance(pr.pops * 0.05) {
                        self.pop_env = self.pop_env.max(self.rng.range(0.4, 1.0) * pr.pops);
                    }
                }
            }
            let noise = self.rng.bi();
            let grit = pr.grit * (0.25 + 0.75 * eff_load);
            let x = s + noise * s * grit;
            let x = self.dc.process(x, self.dc_r);
            let mut y = self.lp.lp(x, &self.lp_c) * pr.dry;
            for k in 0..3 {
                y += self.fm[k].bp(x, &self.fm_c[k]) * pr.formants[k].2 * fgain;
            }
            let amp_t = base_amp * (0.42 + 0.58 * eff_load) * (0.65 + 0.35 * rpm_n) * (1.0 - 0.3 * cut);
            let amp = self.amp.step(amp_t, load_a);
            let mut eng = y * amp;
            // diesel clatter
            if self.knock_env > 1e-4 {
                eng += self.knock_f.bp(noise, &self.knock_c) * self.knock_env * amp * 0.9;
                self.knock_env *= knock_dec;
            }
            // overrun pops
            if self.pop_env > 1e-4 {
                eng += self.pop_f.lp(noise, &self.pop_c) * self.pop_env * base_amp * 1.2;
                self.pop_env *= pop_dec;
            }
            if pr.turbo {
                let b = self.boost;
                if b > 1e-3 {
                    self.whistle_ph += (1500.0 + 3000.0 * b) * c.doppler / sr;
                    if self.whistle_ph >= 1.0 {
                        self.whistle_ph -= 1.0;
                    }
                    eng += sin_turns(self.whistle_ph) * 0.022 * b * b * pr.gain;
                    eng += self.spool_f.bp(noise, &self.spool_c) * 0.025 * b * pr.gain;
                }
                if self.bov_env > 1e-4 {
                    let atk = (self.bov_t / 0.008).min(1.0);
                    eng += self.bov_f.bp(noise, &bov_c) * self.bov_env * atk * 0.16;
                    self.bov_env *= bov_dec;
                    self.bov_t += 1.0 / sr;
                }
            }

            // ---- tyres & environment ----
            let mut ty = 0.0;
            let n2 = self.rng.bi();
            if rumble_amt > 1e-4 {
                ty += self.rumble_f.lp(n2, &self.rumble_c) * rumble_amt * 1.6;
            }
            if road_amt > 1e-4 {
                ty += self.road_f.bp(noise, &self.road_c) * road_amt;
            }
            if grain_rate > 0.0 || snow_rate > 0.0 || self.grain_env[0] + self.grain_env[1] + self.grain_env[2] > 1e-4 {
                if self.rng.chance(grain_rate) {
                    let a = self.rng.f();
                    let k = if self.rng.chance(0.45) { 0 } else { 1 };
                    self.grain_env[k] = self.grain_env[k].max(a * a * grain_amp);
                }
                if self.rng.chance(snow_rate) {
                    let a = self.rng.range(0.3, 1.0);
                    self.grain_env[2] = self.grain_env[2].max(a * (0.3 + 0.5 * sp + 0.6 * slip_l));
                }
                let mut gr = 0.0;
                for k in 0..3 {
                    if self.grain_env[k] > 1e-5 {
                        gr += self.grain_f[k].bp(n2, &self.grain_c[k]) * self.grain_env[k];
                        self.grain_env[k] *= if k == 2 { snow_dec } else { grain_dec };
                    }
                }
                ty += gr * 0.6;
            }
            if bed_amt > 1e-4 {
                ty += self.bed_f.bp(n2, &self.bed_c) * bed_amt;
            }
            if spray_amt > 1e-4 {
                ty += self.spray_f.bp(noise, &self.spray_c) * spray_amt;
            }
            if grass_amt > 1e-4 {
                let w = 0.6 + 0.4 * self.grass_w.step(&mut self.rng, 3.0, sr);
                ty += self.grass_f.bp(n2, &self.grass_c) * grass_amt * w;
            }
            if sq_amt > 1e-4 {
                self.sq_vib += 6.5 / sr;
                if self.sq_vib >= 1.0 {
                    self.sq_vib -= 1.0;
                }
                let wob = self.sq_w.step(&mut self.rng, 5.0, sr);
                let f = sq_freq * (1.0 + 0.012 * sin_turns(self.sq_vib) + 0.025 * wob);
                self.sq_ph += f / sr;
                if self.sq_ph >= 1.0 {
                    self.sq_ph -= 1.0;
                }
                let tone = sin_turns(self.sq_ph) + 0.28 * sin_turns(2.0 * self.sq_ph) + 0.1 * sin_turns(3.0 * self.sq_ph);
                let rough = 0.75 + 0.5 * self.sq_rough.lp(noise, 0.02);
                let mut sqv = tone * rough + self.sq_f.bp(noise, &sq_c) * 0.5;
                if judder > 0.0 {
                    self.judder_ph += 17.0 / sr;
                    if self.judder_ph >= 1.0 {
                        self.judder_ph -= 1.0;
                    }
                    sqv *= 1.0 - judder * (0.5 + 0.5 * sin_turns(self.judder_ph));
                }
                ty += sqv * sq_amt;
            }
            if whirr_amt > 1e-4 {
                ty += self.whirr_f.bp(n2, &whirr_c) * whirr_amt;
            }
            if wind_amt > 1e-4 {
                let w = 0.8 + 0.2 * self.wind_w.step(&mut self.rng, 0.7, sr);
                ty += self.wind_f.bp(noise, &wind_c) * wind_amt * w;
            }
            if water_amt > 1e-4 {
                let w = 0.5 + 0.5 * self.water_w.step(&mut self.rng, 14.0, sr);
                ty += self.water_f.bp(n2, &water_c) * water_amt * w;
            }

            let gain = self.gain.step(p.gain, gain_a) * self.fade;
            // keep the tyre/wind hiss soft: gentle 12 dB/oct rolloff above ~5 kHz
            let ty = self.tyre_soft.lp(ty, &self.tyre_soft_c);
            let mono = self.out_dc.process(eng + ty, self.dc_r);
            let v = self.muffle.lp(mono, muffle_a) * gain;
            out_l[i] += v * pl;
            out_r[i] += v * prr;
            rev_l[i] += v * pl * 0.04;
            rev_r[i] += v * prr * 0.04;
        }
        self.fade = flush(self.fade);
        if self.releasing && self.fade < 1e-4 {
            self.active = false;
            self.id = u32::MAX;
        }
    }
}
