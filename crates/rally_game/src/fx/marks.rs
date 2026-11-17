//! Tyre marks: ground-hugging ribbons stored in two ring buffers (dark skid marks that
//! linger, and faint tracks on loose surfaces that fade sooner).
use super::render::{col, Batches};
use macroquad::math::Vec3;
use rally_core::track::Track;

const LIFT: f32 = 0.085;

#[derive(Clone, Copy)]
pub struct MarkSeg {
    pub a: [Vec3; 2],
    pub b: [Vec3; 2],
    pub alpha: [f32; 2],
    pub color: [f32; 3],
    pub born: f32,
}

pub struct MarkRing {
    pub segs: Vec<MarkSeg>,
    pub head: usize,
    pub cap: usize,
    pub life: f32,
}

impl MarkRing {
    pub fn new(cap: usize, life: f32) -> MarkRing {
        MarkRing { segs: Vec::with_capacity(cap), head: 0, cap, life }
    }

    pub fn push(&mut self, s: MarkSeg) {
        if self.segs.len() < self.cap {
            self.segs.push(s);
        } else {
            self.segs[self.head] = s;
            self.head = (self.head + 1) % self.cap;
        }
    }

    /// Fade multiplier for a segment of the given age (also fades the oldest ring entries
    /// so overwriting never pops).
    #[inline]
    fn fade(&self, age: f32) -> f32 {
        let t = age / self.life;
        if t >= 1.0 {
            0.0
        } else {
            let k = ((1.0 - t) / 0.4).min(1.0);
            k * k
        }
    }

    pub fn build(&self, b: &mut Batches, now: f32, eye: Vec3, max_dist: f32) {
        let md2 = max_dist * max_dist;
        for s in &self.segs {
            let f = self.fade(now - s.born);
            if f <= 0.003 || s.a[0].distance_squared(eye) > md2 {
                continue;
            }
            quad(b, s.a, s.b, [s.alpha[0] * f, s.alpha[1] * f], s.color);
        }
    }
}

fn quad(b: &mut Batches, a: [Vec3; 2], bb: [Vec3; 2], alpha: [f32; 2], color: [f32; 3]) {
    let batch = b.room(4);
    let n = Vec3::Y;
    let base = batch.v.len() as u16;
    for (p, al) in [(a[0], alpha[0]), (a[1], alpha[0]), (bb[1], alpha[1]), (bb[0], alpha[1])] {
        batch.v.push(crate::render::mesh::Vertex { pos: p.to_array(), normal: n.to_array(), color: col(color, al) });
    }
    // keep the winding facing up so the two-sided lighting never flips it dark
    if (a[1] - a[0]).cross(bb[1] - a[0]).y >= 0.0 {
        batch.i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    } else {
        batch.i.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
}

/// Per-wheel ribbon builder
#[derive(Clone, Copy, Default)]
pub struct MarkChain {
    pub active: bool,
    pub last: [Vec3; 2],
    pub last_alpha: f32,
    pub last_centre: Vec3,
    /// live head (drawn every frame up to the wheel)
    pub head: [Vec3; 2],
    pub head_alpha: f32,
    pub color: [f32; 3],
    pub skid: bool,
}

fn ground_pt(track: &Track, p: Vec3) -> Vec3 {
    Vec3::new(p.x, track.visual_height(p.x, p.z) + LIFT, p.z)
}

impl MarkChain {
    /// Feed the wheel's contact point this frame. `strength` 0..1 (0 = no mark).
    /// Returns a finished segment when one should be committed.
    pub fn feed(&mut self, track: &Track, centre: Vec3, side: Vec3, width: f32, strength: f32, color: [f32; 3], skid: bool, seg_len: f32) -> Option<MarkSeg> {
        let edge = [ground_pt(track, centre - side * (width * 0.5)), ground_pt(track, centre + side * (width * 0.5))];
        if strength <= 0.01 {
            let out = if self.active && self.last_alpha > 0.01 {
                // close the ribbon with a fading tail
                Some(MarkSeg { a: self.last, b: edge, alpha: [self.last_alpha, 0.0], color: self.color, born: 0.0 })
            } else {
                None
            };
            self.active = false;
            return out.filter(|s| s.a[0].distance(s.b[0]) < seg_len * 3.0);
        }
        if !self.active || centre.distance(self.last_centre) > 6.0 {
            self.active = true;
            self.last = edge;
            self.last_centre = centre;
            // start from zero so the ribbon fades in
            self.last_alpha = 0.0;
            self.color = color;
            self.skid = skid;
            self.head = edge;
            self.head_alpha = strength;
            return None;
        }
        self.head = edge;
        self.head_alpha = strength;
        if centre.distance(self.last_centre) >= seg_len {
            let seg = MarkSeg { a: self.last, b: edge, alpha: [self.last_alpha, strength], color: self.color, born: 0.0 };
            self.last = edge;
            self.last_centre = centre;
            self.last_alpha = strength;
            self.color = color;
            return Some(seg);
        }
        None
    }

    pub fn build_head(&self, b: &mut Batches) {
        if self.active && (self.last_alpha > 0.01 || self.head_alpha > 0.01) {
            quad(b, self.last, self.head, [self.last_alpha, self.head_alpha], self.color);
        }
    }
}
