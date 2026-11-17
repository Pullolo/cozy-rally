//! Small, allocation-free DSP building blocks used by every synth voice.

pub const TAU: f32 = std::f32::consts::TAU;
pub const PI: f32 = std::f32::consts::PI;

/// Fast sine of a phase given in turns (1.0 = one full cycle). Max error ~4e-6.
#[inline]
pub fn sin_turns(p: f32) -> f32 {
    // map to y in [-1, 1) where sin(2*pi*p) = -sin(pi*y)
    let x = p - p.floor();
    let mut y = 2.0 * x - 1.0;
    // fold to [-0.5, 0.5] using sin(pi*(1-y)) = sin(pi*y)
    if y > 0.5 {
        y = 1.0 - y;
    } else if y < -0.5 {
        y = -1.0 - y;
    }
    let y2 = y * y;
    let s = y * (PI + y2 * (-5.167_712_8 + y2 * (2.550_164 + y2 * (-0.599_264_5 + y2 * 0.082_145_89))));
    -s
}

#[inline]
pub fn cos_turns(p: f32) -> f32 {
    sin_turns(p + 0.25)
}

#[inline]
pub fn midi_hz(m: f32) -> f32 {
    440.0 * ((m - 69.0) / 12.0).exp2()
}

#[inline]
pub fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp01((x - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// One-pole coefficient for a time constant in seconds
#[inline]
pub fn tau_coef(tau: f32, sr: f32) -> f32 {
    if tau <= 0.0 { 1.0 } else { 1.0 - (-1.0 / (tau * sr)).exp() }
}

/// Equal-power pan: -1 = left, 1 = right
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * 0.125; // turns, 0..0.25
    (cos_turns(a), sin_turns(a))
}

/// xorshift32 noise / random source
#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(seed.max(1).wrapping_mul(2_654_435_761) | 1)
    }
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// [0, 1)
    #[inline]
    pub fn f(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
    /// [-1, 1)
    #[inline]
    pub fn bi(&mut self) -> f32 {
        self.f() * 2.0 - 1.0
    }
    #[inline]
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[(self.next_u32() as usize) % xs.len()]
    }
}

/// One-pole lowpass
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    pub z: f32,
}

impl OnePole {
    #[inline]
    pub fn lp(&mut self, x: f32, a: f32) -> f32 {
        self.z += (x - self.z) * a;
        self.z
    }
    #[inline]
    pub fn hp(&mut self, x: f32, a: f32) -> f32 {
        x - self.lp(x, a)
    }
}

/// Coefficient of a one-pole lowpass for a cutoff frequency
#[inline]
pub fn onepole_coef(fc: f32, sr: f32) -> f32 {
    1.0 - (-TAU * fc.max(1.0) / sr).exp()
}

/// DC blocker (leaky differentiator)
#[derive(Clone, Copy, Debug, Default)]
pub struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline]
    pub fn process(&mut self, x: f32, r: f32) -> f32 {
        let y = x - self.x1 + r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

pub fn dc_coef(fc: f32, sr: f32) -> f32 {
    (-TAU * fc / sr).exp()
}

/// Coefficients for a TPT state-variable filter (Simper)
#[derive(Clone, Copy, Debug)]
pub struct SvfCoef {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl SvfCoef {
    pub fn new(fc: f32, q: f32, sr: f32) -> SvfCoef {
        let fc = fc.clamp(10.0, sr * 0.45);
        let g = (PI * fc / sr).tan();
        let k = 1.0 / q.max(0.05);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        SvfCoef { a1, a2, a3, k }
    }
}

impl Default for SvfCoef {
    fn default() -> Self {
        SvfCoef::new(1000.0, 0.7, 48000.0)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SvfOut {
    pub lp: f32,
    pub bp: f32,
    pub hp: f32,
}

impl Svf {
    #[inline]
    pub fn process(&mut self, x: f32, c: &SvfCoef) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { lp: v2, bp: v1, hp: x - c.k * v1 - v2 }
    }
    /// Band-pass normalised to unity peak gain
    #[inline]
    pub fn bp(&mut self, x: f32, c: &SvfCoef) -> f32 {
        self.process(x, c).bp * c.k
    }
    #[inline]
    pub fn lp(&mut self, x: f32, c: &SvfCoef) -> f32 {
        self.process(x, c).lp
    }
    #[inline]
    pub fn hp(&mut self, x: f32, c: &SvfCoef) -> f32 {
        self.process(x, c).hp
    }
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

/// Exponential parameter smoother
#[derive(Clone, Copy, Debug, Default)]
pub struct Smooth {
    pub v: f32,
}

impl Smooth {
    #[inline]
    pub fn step(&mut self, target: f32, a: f32) -> f32 {
        self.v += (target - self.v) * a;
        self.v
    }
}

/// Slowly wandering random value in [-1, 1] (smoothed sample & hold)
#[derive(Clone, Copy, Debug, Default)]
pub struct Wander {
    target: f32,
    v: f32,
    t: f32,
}

impl Wander {
    /// `rate`: new targets per second
    #[inline]
    pub fn step(&mut self, rng: &mut Rng, rate: f32, sr: f32) -> f32 {
        self.t += rate / sr;
        if self.t >= 1.0 {
            self.t -= 1.0;
            self.target = rng.bi();
        }
        self.v += (self.target - self.v) * (rate * 3.0 / sr).min(1.0);
        self.v
    }
}
