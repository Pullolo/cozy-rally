//! Turns particles into camera-facing / tumbling geometry, batched into a few streamed
//! draw calls (each well below the 60k-vertex stream buffer).
use super::particle::{Particle, Shape};
use crate::render::gpu::{DrawParams, Environment, Gpu, View};
use crate::render::mesh::Vertex;
use macroquad::math::{Mat4, Vec3};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Vertices per streamed draw call (stream buffer holds 60k; u16 indices)
pub const BATCH_VERTS: usize = 30_000;

#[derive(Default)]
pub struct Batch {
    pub v: Vec<Vertex>,
    pub i: Vec<u16>,
}

impl Batch {
    #[inline]
    fn vert(&mut self, p: Vec3, n: Vec3, c: [u8; 4]) -> u16 {
        let k = self.v.len() as u16;
        self.v.push(Vertex { pos: p.to_array(), normal: n.to_array(), color: c });
        k
    }
    #[inline]
    fn tri(&mut self, a: u16, b: u16, c: u16) {
        self.i.extend_from_slice(&[a, b, c]);
    }
}

/// A growable list of batches; `room(n)` makes sure the current one can take n vertices.
#[derive(Default)]
pub struct Batches {
    list: Vec<Batch>,
    used: usize,
}

impl Batches {
    pub fn clear(&mut self) {
        for b in &mut self.list {
            b.v.clear();
            b.i.clear();
        }
        self.used = 0;
    }

    #[inline]
    pub fn room(&mut self, n: usize) -> &mut Batch {
        if self.list.is_empty() {
            self.list.push(Batch::default());
        }
        if self.list[self.used].v.len() + n > BATCH_VERTS {
            self.used += 1;
            if self.used == self.list.len() {
                self.list.push(Batch::default());
            }
        }
        &mut self.list[self.used]
    }

    pub fn vertex_count(&self) -> usize {
        self.list.iter().map(|b| b.v.len()).sum()
    }

    pub fn draw(&self, gpu: &mut Gpu, view: &View, env: &Environment, p: &DrawParams) {
        for b in self.list.iter().take(self.used + 1) {
            if !b.i.is_empty() {
                gpu.draw_dynamic(&b.v, &b.i, view, env, Mat4::IDENTITY, p, 1);
            }
        }
    }
}

/// Camera basis for billboards
#[derive(Clone, Copy)]
pub struct Basis {
    pub right: Vec3,
    pub up: Vec3,
    /// Points from the scene towards the camera
    pub toward: Vec3,
    pub eye: Vec3,
}

impl Basis {
    pub fn from_view(view: &View) -> Basis {
        let m = view.view_proj;
        let right = m.row(0).truncate().normalize_or_zero();
        let up = m.row(1).truncate().normalize_or_zero();
        let toward = right.cross(up).normalize_or_zero();
        Basis { right, up, toward, eye: view.eye }
    }
}

#[inline]
pub fn col(c: [f32; 3], a: f32) -> [u8; 4] {
    [
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
        (a.clamp(0.0, 1.0) * 255.0) as u8,
    ]
}

/// Precomputed unit circle (12 steps) so puffs don't call sin/cos per vertex.
pub struct Circle {
    pub pts: [(f32, f32); 12],
}

impl Circle {
    pub fn new() -> Circle {
        let mut pts = [(0.0, 0.0); 12];
        for (k, p) in pts.iter_mut().enumerate() {
            let a = k as f32 / 12.0 * TAU;
            *p = (a.cos(), a.sin());
        }
        Circle { pts }
    }
}

/// Fade particles that are very close to the camera (avoid screen-filling blobs).
#[inline]
fn near_fade(dist: f32, size: f32) -> f32 {
    ((dist - 0.8 - size * 0.6) / (1.5 + size)).clamp(0.0, 1.0)
}

/// Soft lit cloud: centre -> inner ring -> transparent rim, normals bulge like a sphere.
pub fn puff(b: &mut Batches, bs: &Basis, circ: &Circle, p: &Particle, dist: f32) {
    let r = p.size();
    let a = p.opacity() * near_fade(dist, r);
    if a < 0.01 {
        return;
    }
    // pull towards the camera so clouds don't slice into the ground behind them
    let c = p.pos + bs.toward * (r * 0.55).min(dist * 0.3);
    // spin the billboard slowly so the low-poly rim never looks static
    let (sr, cr) = p.rot.sin_cos();
    let ax = bs.right * cr + bs.up * sr;
    let ay = bs.up * cr - bs.right * sr;
    let detailed = r / dist.max(0.1) > 0.012;
    if detailed {
        let batch = b.room(13);
        let c0 = col(p.color, a);
        let c1 = col(p.color, a * 0.62);
        let c2 = col(p.color, 0.0);
        // normals bulge like a sphere but lean upwards, so clouds are lit from the sky
        // like the ground they sit on (never darker than the road behind them)
        let fwd = (bs.toward + Vec3::Y * 0.9).normalize();
        let base = batch.vert(c, fwd, c0);
        // 6 inner + 6 outer, irregular cauliflower rim
        for k in 0..6 {
            let (x, y) = circ.pts[k * 2];
            let d = ax * x + ay * y;
            let wob = 1.0 + 0.13 * ((k as f32) * 2.3 + p.seed * 31.0).sin();
            batch.vert(c + d * (r * 0.5 * wob), (fwd + d * 0.8).normalize(), c1);
        }
        for k in 0..6 {
            let (x, y) = circ.pts[k * 2 + 1];
            let d = ax * x + ay * y;
            let wob = 1.0 + 0.16 * ((k as f32) * 1.7 + p.seed * 17.0).sin();
            batch.vert(c + d * (r * wob), (fwd * 0.4 + d).normalize(), c2);
        }
        for k in 0..6u16 {
            let i0 = base + 1 + k;
            let i1 = base + 1 + (k + 1) % 6;
            let o0 = base + 7 + k;
            let o_prev = base + 7 + (k + 5) % 6;
            batch.tri(base, i0, i1);
            // outer ring: each outer vertex sits between two inner ones
            batch.tri(i0, o0, i1);
            batch.tri(i0, o_prev, o0);
        }
    } else {
        let batch = b.room(7);
        let fwd = (bs.toward + Vec3::Y * 0.9).normalize();
        let base = batch.vert(c, fwd, col(p.color, a * 0.8));
        let c2 = col(p.color, 0.0);
        for k in 0..6 {
            let (x, y) = circ.pts[k * 2];
            let d = ax * x + ay * y;
            batch.vert(c + d * r, (fwd * 0.4 + d).normalize(), c2);
        }
        for k in 0..6u16 {
            batch.tri(base, base + 1 + k, base + 1 + (k + 1) % 6);
        }
    }
}

/// Tumbling axes for flat bits: `a` spins around Y with yaw, `b` tumbles around `a`.
#[inline]
fn tumble_axes(p: &Particle) -> (Vec3, Vec3) {
    let (sy, cy) = p.yaw.sin_cos();
    let a = Vec3::new(cy, 0.0, sy);
    let side = Vec3::new(-sy, 0.0, cy); // a x Y (up to sign)
    let (st, ct) = p.rot.sin_cos();
    let bb = Vec3::Y * ct + side * st;
    (a, bb)
}

/// Flakes, leaves, chunks, drops (lit)
pub fn bit(b: &mut Batches, bs: &Basis, p: &Particle, dist: f32) {
    let a = p.opacity() * near_fade(dist, 0.8);
    if a < 0.02 {
        return;
    }
    let s = p.size();
    let c = col(p.color, a);
    match p.shape {
        Shape::Chunk => {
            let (ax, by) = tumble_axes(p);
            let n = ax.cross(by);
            let batch = b.room(3);
            let i0 = batch.vert(p.pos + ax * s, n, c);
            let i1 = batch.vert(p.pos - ax * (s * 0.5) + by * (s * 0.85), n, c);
            let i2 = batch.vert(p.pos - ax * (s * 0.6) - by * (s * 0.75), n, c);
            batch.tri(i0, i1, i2);
        }
        Shape::Flake => {
            let (ax, by) = tumble_axes(p);
            let n = ax.cross(by);
            let w = ax * s;
            let h = by * (s * p.aspect);
            let batch = b.room(4);
            let i0 = batch.vert(p.pos - w - h, n, c);
            let i1 = batch.vert(p.pos + w - h, n, c);
            let i2 = batch.vert(p.pos + w + h, n, c);
            let i3 = batch.vert(p.pos - w + h, n, c);
            batch.tri(i0, i1, i2);
            batch.tri(i0, i2, i3);
        }
        Shape::Leaf => {
            let (ax, by) = tumble_axes(p);
            let n = ax.cross(by);
            let w = ax * (s * p.aspect);
            let h = by * s;
            // a little paler towards the tip for a soft gradient
            let tip = col([p.color[0] * 1.12 + 0.04, p.color[1] * 1.12 + 0.04, p.color[2] * 1.12 + 0.04], a);
            let batch = b.room(5);
            let i0 = batch.vert(p.pos - h, n, c);
            let i1 = batch.vert(p.pos + w * 0.9 + h * 0.1, n, c);
            let i2 = batch.vert(p.pos + h, n, tip);
            let i3 = batch.vert(p.pos - w * 0.9 + h * 0.1, n, c);
            batch.tri(i0, i1, i2);
            batch.tri(i0, i2, i3);
        }
        _ => {
            // Drop: small round billboard, bright centre
            let batch = b.room(7);
            let base = batch.vert(p.pos, bs.toward, c);
            let rim = col(p.color, a * 0.5);
            for k in 0..6 {
                let ang = k as f32 / 6.0 * TAU;
                let d = bs.right * ang.cos() + bs.up * ang.sin();
                batch.vert(p.pos + d * s, (bs.toward * 0.4 + d).normalize(), rim);
            }
            for k in 0..6u16 {
                batch.tri(base, base + 1 + k, base + 1 + (k + 1) % 6);
            }
        }
    }
}

/// Unlit glowy things: soft dots, stars, sparks, twinkles
pub fn glow(b: &mut Batches, bs: &Basis, circ: &Circle, p: &Particle, dist: f32, time: f32) {
    let mut a = p.opacity() * near_fade(dist, 0.1);
    let mut s = p.size();
    match p.shape {
        Shape::Twinkle => {
            let tw = 0.5 + 0.5 * (time * 7.0 + p.seed * 60.0).sin();
            s *= 0.5 + tw;
            a *= 0.4 + 0.6 * tw;
        }
        Shape::Glow if p.ambient => {
            // firefly pulse
            let pulse = (time * 2.2 + p.seed * 40.0).sin().max(0.0);
            a *= 0.15 + 0.85 * pulse * pulse;
        }
        _ => {}
    }
    if a < 0.02 {
        return;
    }
    // keep tiny things at least ~1.5px-ish visible at distance
    s = s.max(dist * 0.0025);
    let c = col(p.color, a);
    let n = bs.toward;
    match p.shape {
        Shape::Star => {
            let batch = b.room(11);
            let base = batch.vert(p.pos, n, col([1.0, 1.0, 0.95], a));
            for k in 0..10 {
                let ang = p.rot + k as f32 / 10.0 * TAU + FRAC_PI_2;
                let rr = if k % 2 == 0 { s } else { s * 0.42 };
                batch.vert(p.pos + (bs.right * ang.cos() + bs.up * ang.sin()) * rr, n, c);
            }
            for k in 0..10u16 {
                batch.tri(base, base + 1 + k, base + 1 + (k + 1) % 10);
            }
        }
        Shape::Spark => {
            let tail = p.vel * 0.045;
            let side = p.vel.cross(bs.toward).normalize_or_zero() * s;
            let batch = b.room(4);
            let i0 = batch.vert(p.pos + side, n, c);
            let i1 = batch.vert(p.pos - side, n, c);
            let i2 = batch.vert(p.pos - tail - side * 0.3, n, col(p.color, 0.0));
            let i3 = batch.vert(p.pos - tail + side * 0.3, n, col(p.color, 0.0));
            batch.tri(i0, i1, i2);
            batch.tri(i0, i2, i3);
        }
        Shape::Twinkle => {
            // 4-point sparkle: two thin crossed diamonds + soft core
            let batch = b.room(9);
            let base = batch.vert(p.pos, n, c);
            let rim = col(p.color, 0.0);
            for k in 0..8 {
                let ang = k as f32 / 8.0 * TAU + p.rot;
                let rr = if k % 2 == 0 { s } else { s * 0.18 };
                batch.vert(p.pos + (bs.right * ang.cos() + bs.up * ang.sin()) * rr, n, if k % 2 == 0 { rim } else { c });
            }
            for k in 0..8u16 {
                batch.tri(base, base + 1 + k, base + 1 + (k + 1) % 8);
            }
        }
        _ => {
            // soft glow dot: hot core + halo
            let batch = b.room(13);
            let base = batch.vert(p.pos, n, col([1.0, 1.0, 0.92], a));
            let mid = col(p.color, a * 0.55);
            let rim = col(p.color, 0.0);
            for k in 0..6 {
                let (x, y) = circ.pts[k * 2];
                batch.vert(p.pos + (bs.right * x + bs.up * y) * (s * 0.35), n, mid);
            }
            for k in 0..6 {
                let (x, y) = circ.pts[k * 2 + 1];
                batch.vert(p.pos + (bs.right * x + bs.up * y) * s, n, rim);
            }
            for k in 0..6u16 {
                let i0 = base + 1 + k;
                let i1 = base + 1 + (k + 1) % 6;
                batch.tri(base, i0, i1);
                batch.tri(i0, base + 7 + k, i1);
                batch.tri(i0, base + 7 + (k + 5) % 6, base + 7 + k);
            }
        }
    }
}

/// Butterflies and gulls: a body with hinged, flapping wings (lit, two-sided)
pub fn critter(b: &mut Batches, p: &Particle, dist: f32) {
    let a = p.opacity() * near_fade(dist, 0.2);
    if a < 0.02 {
        return;
    }
    let h = Vec3::new(p.vel.x, 0.0, p.vel.z).normalize_or(Vec3::X);
    let side = Vec3::new(-h.z, 0.0, h.x);
    let s = p.size();
    let flap = p.rot.sin();
    let c = col(p.color, a);
    if p.shape == Shape::Butterfly {
        let ang = flap * 1.1 + 0.2;
        let batch = b.room(14);
        for sgn in [-1.0f32, 1.0] {
            let w = (side * (sgn * ang.cos()) + Vec3::Y * ang.sin()) * s;
            let n = h.cross(w).normalize_or(Vec3::Y);
            // fore wing (bigger) + hind wing
            let b0 = batch.vert(p.pos + h * (s * 0.15), n, c);
            let b1 = batch.vert(p.pos - h * (s * 0.1), n, c);
            let t0 = batch.vert(p.pos + w + h * (s * 0.55), n, c);
            let t1 = batch.vert(p.pos + w * 0.85 - h * (s * 0.05), n, c);
            batch.tri(b0, b1, t1);
            batch.tri(b0, t1, t0);
            let hc = col([p.color[0] * 0.85, p.color[1] * 0.85, p.color[2] * 0.85], a);
            let t2 = batch.vert(p.pos + w * 0.6 - h * (s * 0.5), n, hc);
            let b2 = batch.vert(p.pos - h * (s * 0.12), n, hc);
            let b3 = batch.vert(p.pos + w * 0.75 - h * (s * 0.05), n, hc);
            batch.tri(b2, t2, b3);
        }
        // body
        let dark = col([0.25, 0.2, 0.22], a);
        let n = Vec3::Y;
        let i0 = batch.vert(p.pos + h * (s * 0.3), n, dark);
        let i1 = batch.vert(p.pos - h * (s * 0.35) + side * (s * 0.04), n, dark);
        let i2 = batch.vert(p.pos - h * (s * 0.35) - side * (s * 0.04), n, dark);
        batch.tri(i0, i1, i2);
    } else {
        // gull: gull-wing "M" with dihedral inner panel and drooping tips
        let ang = flap * 0.55 + 0.15;
        let grey = col([0.62, 0.66, 0.72], a);
        let batch = b.room(16);
        for sgn in [-1.0f32, 1.0] {
            let inner = (side * (sgn * ang.cos()) + Vec3::Y * ang.sin()) * (s * 0.5);
            let tip_ang = ang - 0.6 - flap * 0.3;
            let outer = (side * (sgn * tip_ang.cos()) + Vec3::Y * tip_ang.sin()) * (s * 0.55);
            let n = h.cross(inner).normalize_or(Vec3::Y);
            let r0 = batch.vert(p.pos + h * (s * 0.18), n, c);
            let r1 = batch.vert(p.pos - h * (s * 0.12), n, c);
            let e0 = batch.vert(p.pos + inner + h * (s * 0.12), n, c);
            let e1 = batch.vert(p.pos + inner - h * (s * 0.08), n, c);
            batch.tri(r0, r1, e1);
            batch.tri(r0, e1, e0);
            let n2 = h.cross(outer).normalize_or(Vec3::Y);
            let f0 = batch.vert(p.pos + inner + h * (s * 0.12), n2, c);
            let f1 = batch.vert(p.pos + inner - h * (s * 0.08), n2, c);
            let tip = batch.vert(p.pos + inner + outer - h * (s * 0.12), n2, grey);
            batch.tri(f0, f1, tip);
        }
        // body + tail
        let n = Vec3::Y;
        let i0 = batch.vert(p.pos + h * (s * 0.35), n, c);
        let i1 = batch.vert(p.pos - h * (s * 0.4) + side * (s * 0.08), n, c);
        let i2 = batch.vert(p.pos - h * (s * 0.4) - side * (s * 0.08), n, c);
        batch.tri(i0, i1, i2);
    }
}

/// Tumbleweed: two jagged, counter-rotating twiggy discs facing the camera
pub fn tumbleweed(b: &mut Batches, bs: &Basis, p: &Particle, dist: f32) {
    let a = p.opacity() * near_fade(dist, p.size0);
    if a < 0.02 {
        return;
    }
    let r = p.size0;
    let c = p.pos + bs.toward * (r * 0.3);
    for (layer, (shade_k, rot, rr)) in [(0.82f32, p.rot, 1.0f32), (1.08, -p.rot * 0.7 + 1.0, 0.82)].into_iter().enumerate() {
        let colr = [p.color[0] * shade_k, p.color[1] * shade_k, p.color[2] * shade_k];
        let batch = b.room(15);
        let base = batch.vert(c + bs.toward * (layer as f32 * 0.02), bs.toward, col(colr, a * 0.55));
        for k in 0..14 {
            let ang = rot + k as f32 / 14.0 * TAU;
            let d = bs.right * ang.cos() + bs.up * ang.sin();
            let jag = if k % 2 == 0 { 1.0 } else { 0.72 + 0.1 * ((k as f32) * 3.7 + p.seed * 9.0).sin() };
            batch.vert(c + d * (r * rr * jag), (bs.toward * 0.5 + d).normalize(), col(colr, a * if k % 2 == 0 { 0.25 } else { 0.85 }));
        }
        for k in 0..14u16 {
            batch.tri(base, base + 1 + k, base + 1 + (k + 1) % 14);
        }
    }
}
