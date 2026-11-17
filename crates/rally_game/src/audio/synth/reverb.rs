//! Small Freeverb-style stereo reverb (4 damped combs + 2 allpasses per side).
use super::BLOCK;

struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}

impl Comb {
    #[inline]
    fn process(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let y = self.buf[self.i];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.i] = x + self.store * fb;
        self.i += 1;
        if self.i >= self.buf.len() {
            self.i = 0;
        }
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    i: usize,
}

impl Allpass {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        let y = b - x;
        self.buf[self.i] = x + b * 0.5;
        self.i += 1;
        if self.i >= self.buf.len() {
            self.i = 0;
        }
        y
    }
}

pub struct Reverb {
    combs: [Vec<Comb>; 2],
    aps: [Vec<Allpass>; 2],
    fb: f32,
    damp: f32,
}

impl Reverb {
    pub fn new(sr: f32) -> Reverb {
        let s = sr / 44100.0;
        let mk = |spread: usize| -> (Vec<Comb>, Vec<Allpass>) {
            let combs = [1116usize, 1277, 1422, 1557]
                .iter()
                .map(|&l| Comb { buf: vec![0.0; ((l + spread) as f32 * s * 1.15) as usize + 1], i: 0, store: 0.0 })
                .collect();
            let aps = [556usize, 341].iter().map(|&l| Allpass { buf: vec![0.0; ((l + spread) as f32 * s) as usize + 1], i: 0 }).collect();
            (combs, aps)
        };
        let (cl, al) = mk(0);
        let (cr, ar) = mk(23);
        Reverb { combs: [cl, cr], aps: [al, ar], fb: 0.8, damp: 0.4 }
    }

    /// In-place: the buffers hold the send on entry and the wet signal on exit
    pub fn process(&mut self, n: usize, l: &mut [f32; BLOCK], r: &mut [f32; BLOCK]) {
        for i in 0..n {
            let x = (l[i] + r[i]) * 0.5 * 0.35;
            let mut outs = [0.0f32; 2];
            for ch in 0..2 {
                let mut acc = 0.0;
                for c in self.combs[ch].iter_mut() {
                    acc += c.process(x, self.fb, self.damp);
                }
                for a in self.aps[ch].iter_mut() {
                    acc = a.process(acc);
                }
                outs[ch] = acc;
            }
            l[i] = outs[0];
            r[i] = outs[1];
        }
    }
}
