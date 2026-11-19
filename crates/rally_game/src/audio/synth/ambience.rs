//! Weather ambience around the listener: rain (drizzle -> downpour, with drops pattering on
//! the car roof), soft snowfall air, gusty wind, and the trigger for rare distant thunder.
use super::dsp::*;
use super::BLOCK;

/// Weather around the (first) local listener. All 0..1 unless noted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AmbienceParams {
    /// Rain intensity (0 when it snows)
    pub rain: f32,
    /// Snowfall intensity
    pub snow: f32,
    /// Wind speed (m/s)
    pub wind: f32,
    /// Fog density
    pub fog: f32,
    /// Storminess (enables rare distant thunder)
    pub storm: f32,
}

struct Side {
    bed_hi: Svf,
    bed_lo: Svf,
    drop_env: [f32; 2],
    drop_f: [Svf; 2],
    roof_env: f32,
    roof_f: Svf,
    rumble: Svf,
    snow_f: Svf,
    wind_f: Svf,
    whistle_f: Svf,
}

impl Side {
    fn new() -> Side {
        Side {
            bed_hi: Svf::default(),
            bed_lo: Svf::default(),
            drop_env: [0.0; 2],
            drop_f: [Svf::default(); 2],
            roof_env: 0.0,
            roof_f: Svf::default(),
            rumble: Svf::default(),
            snow_f: Svf::default(),
            wind_f: Svf::default(),
            whistle_f: Svf::default(),
        }
    }
}

pub struct Ambience {
    sr: f32,
    target: AmbienceParams,
    rain: f32,
    snow: f32,
    wind: f32,
    storm: f32,
    pub hush: f32,
    fade: f32,
    rng: Rng,
    sides: [Side; 2],
    c_bed_hi: SvfCoef,
    c_bed_lo: SvfCoef,
    c_drop: [SvfCoef; 2],
    c_roof: SvfCoef,
    c_rumble: SvfCoef,
    c_snow: SvfCoef,
    c_whistle: SvfCoef,
    gust: Wander,
    gust_pan: Wander,
    gust_burst: f32,
    thunder_cooldown: f32,
}

impl Ambience {
    pub fn new(sr: f32) -> Ambience {
        Ambience {
            sr,
            target: AmbienceParams::default(),
            rain: 0.0,
            snow: 0.0,
            wind: 0.0,
            storm: 0.0,
            hush: 0.0,
            fade: 0.0,
            rng: Rng::new(0xA3B1E),
            sides: [Side::new(), Side::new()],
            c_bed_hi: SvfCoef::new(3400.0, 0.45, sr),
            c_bed_lo: SvfCoef::new(700.0, 0.6, sr),
            c_drop: [SvfCoef::new(2500.0, 2.2, sr), SvfCoef::new(4100.0, 2.5, sr)],
            c_roof: SvfCoef::new(540.0, 3.0, sr),
            c_rumble: SvfCoef::new(170.0, 0.7, sr),
            c_snow: SvfCoef::new(1300.0, 0.5, sr),
            c_whistle: SvfCoef::new(1700.0, 5.0, sr),
            gust: Wander::default(),
            gust_pan: Wander::default(),
            gust_burst: 0.0,
            thunder_cooldown: 8.0,
        }
    }

    pub fn set(&mut self, p: AmbienceParams) {
        self.target = p;
    }

    /// Render; returns Some(strength) when a distant thunder should start.
    pub fn render(&mut self, n: usize, stale: bool, l: &mut [f32; BLOCK], r: &mut [f32; BLOCK]) -> Option<f32> {
        let sr = self.sr;
        let bs = n as f32 / sr;
        let t = self.target;
        let a = tau_coef(1.2, 1.0 / bs);
        self.rain += (t.rain.clamp(0.0, 1.0) - self.rain) * a;
        self.snow += (t.snow.clamp(0.0, 1.0) - self.snow) * a;
        self.wind += (t.wind.clamp(0.0, 25.0) - self.wind) * a;
        self.storm += (t.storm.clamp(0.0, 1.0) - self.storm) * a;
        let hush_t = (t.fog * 0.6).max(self.snow * 0.9).clamp(0.0, 1.0);
        self.hush += (hush_t - self.hush) * a;
        let fade_t = if stale { 0.0 } else { 1.0 };
        self.fade += (fade_t - self.fade) * tau_coef(if stale { 0.5 } else { 0.8 }, 1.0 / bs);
        self.fade = flush(self.fade);
        let fade = self.fade;
        if fade < 1e-4 {
            return None;
        }
        let rain = self.rain;
        let w = clamp01(self.wind / 12.0);

        // ---- control ----
        let bed_amt = (0.075 * rain + 0.04 * rain.powi(3)) * fade;
        let lo_amt = 0.6 * bed_amt;
        let drop_rate = if rain > 0.01 { (25.0 + 1300.0 * rain.powf(1.4)) / sr } else { 0.0 };
        let drop_amp = 0.35 + 0.5 * rain;
        let roof_rate = if rain > 0.01 { (4.0 + 200.0 * rain) / sr } else { 0.0 };
        let rumble_amt = 0.035 * rain * rain * fade;
        let snow_amt = 0.012 * self.snow * fade;
        let drop_dec = (-1.0 / (0.0016 * sr)).exp();
        let roof_dec = (-1.0 / (0.009 * sr)).exp();
        // wind: slow gust envelope + occasional bursts
        if self.rng.chance(bs * w * 0.35) {
            self.gust_burst = self.gust_burst.max(self.rng.range(0.4, 1.0));
        }
        self.gust_burst *= (-bs / 1.1).exp();
        let g = clamp01(0.4 + 0.45 * self.gust.step(&mut self.rng, 0.6, sr / n as f32) + 0.7 * self.gust_burst);
        let wind_amt = (0.008 + 0.28 * w.powf(1.3)) * (0.15 + 0.85 * g * g) * fade;
        let wind_c = SvfCoef::new(220.0 + 520.0 * g * (0.5 + 0.5 * w), 0.7, sr);
        let whistle_amt = 0.03 * (w * g).powi(2) * fade;
        let wpan = 0.5 * self.gust_pan.step(&mut self.rng, 0.15, sr / n as f32);
        let (wl, wr) = pan_gains(wpan);

        for i in 0..n {
            let mut out = [0.0f32; 2];
            for ch in 0..2 {
                let s = &mut self.sides[ch];
                let nz = self.rng.bi();
                let mut v = 0.0;
                if bed_amt > 1e-5 {
                    v += s.bed_hi.bp(nz, &self.c_bed_hi) * bed_amt + s.bed_lo.lp(nz, &self.c_bed_lo) * lo_amt;
                    if self.rng.chance(drop_rate) {
                        let k = (self.rng.next_u32() & 1) as usize;
                        let x = self.rng.f();
                        s.drop_env[k] = s.drop_env[k].max(x * x * drop_amp);
                    }
                    if self.rng.chance(roof_rate) {
                        s.roof_env = s.roof_env.max(self.rng.range(0.3, 1.0));
                    }
                }
                for k in 0..2 {
                    if s.drop_env[k] > 1e-5 {
                        v += s.drop_f[k].bp(nz, &self.c_drop[k]) * s.drop_env[k] * 0.15 * fade;
                        s.drop_env[k] *= drop_dec;
                    }
                }
                if s.roof_env > 1e-5 {
                    v += s.roof_f.bp(nz, &self.c_roof) * s.roof_env * 0.07 * fade;
                    s.roof_env *= roof_dec;
                }
                if rumble_amt > 1e-5 {
                    v += s.rumble.lp(nz, &self.c_rumble) * rumble_amt;
                }
                if snow_amt > 1e-5 {
                    v += s.snow_f.lp(nz, &self.c_snow) * snow_amt;
                }
                let wn = self.rng.bi();
                v += s.wind_f.bp(wn, &wind_c) * wind_amt * if ch == 0 { wl } else { wr } * 1.4;
                if whistle_amt > 1e-5 {
                    v += s.whistle_f.bp(wn, &self.c_whistle) * whistle_amt;
                }
                out[ch] = v;
            }
            l[i] += out[0];
            r[i] += out[1];
        }
        for s in self.sides.iter_mut() {
            s.drop_env = [flush(s.drop_env[0]), flush(s.drop_env[1])];
            s.roof_env = flush(s.roof_env);
        }

        // rare, soft distant thunder in storms (at least ~20 s apart)
        self.thunder_cooldown -= bs;
        if self.storm > 0.05 && self.thunder_cooldown <= 0.0 && self.rng.chance(bs * self.storm / 30.0) {
            self.thunder_cooldown = 20.0;
            return Some(self.rng.range(0.5, 1.0) * self.storm * fade);
        }
        None
    }
}
