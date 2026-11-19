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

    /// Number of draw calls `draw` will issue
    pub fn calls(&self) -> usize {
        self.list.iter().take(self.used + 1).filter(|b| !b.i.is_empty()).count()
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
    pub pts16: [(f32, f32); 16],
}

impl Circle {
    pub fn new() -> Circle {
        let mut pts = [(0.0, 0.0); 12];
        for (k, p) in pts.iter_mut().enumerate() {
            let a = k as f32 / 12.0 * TAU;
            *p = (a.cos(), a.sin());
        }
        let mut pts16 = [(0.0, 0.0); 16];
        for (k, p) in pts16.iter_mut().enumerate() {
            let a = k as f32 / 16.0 * TAU;
            *p = (a.cos(), a.sin());
        }
        Circle { pts, pts16 }
    }
}

/// Fade particles that are very close to the camera (never block the player's view of
/// the road; big clouds start fading further away).
#[inline]
fn near_fade(dist: f32, size: f32) -> f32 {
    ((dist - 1.0 - size * 0.5) / (1.0 + size * 0.8)).clamp(0.0, 1.0)
}

/// Fill-rate guard for clouds: anything within ~3-4 m of the camera, or whose billboard would
/// cover a large part of the screen, fades out (and its drawn radius is clamped), so big dust
/// or spray puffs never become full-screen overdraw. The chase-cam look is unaffected: clouds
/// around and behind the car are 6+ m away.
/// `COZY_FX_NOGUARD=1` disables the guard (for before/after fill-rate measurements)
pub static NO_GUARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[inline]
pub fn cloud_guard(dist: f32, r: f32) -> (f32, f32) {
    if NO_GUARD.load(std::sync::atomic::Ordering::Relaxed) {
        return (1.0, r);
    }
    let near = ((dist - 3.0) / 2.5).clamp(0.0, 1.0);
    // r/dist = angular radius; 0.3 ~ a puff spanning a third of the screen height
    let cover = r / dist.max(0.1);
    let big = ((0.55 - cover) / 0.25).clamp(0.0, 1.0);
    (near * big, r.min(dist * 0.6))
}

/// Squash the lower half of a cloud billboard: dust sits on the ground with a soft flat
/// base instead of slicing into it (no hard depth-clipped edges).
#[inline]
fn flat_bottom(d: Vec3, up: Vec3) -> Vec3 {
    let y = d.dot(up);
    if y < 0.0 {
        d - up * (y * 0.5)
    } else {
        d
    }
}

/// Keep the followed car readable: clouds between the camera and the car fade where they
/// overlap the car on screen; clouds beside/above it (and everything beyond it) stay.
#[inline]
fn car_clearance(bs: &Basis, focus: Vec3, pos: Vec3, r: f32) -> f32 {
    let to_car = focus - bs.eye;
    let fd = to_car.length();
    if fd < 0.1 {
        return 1.0;
    }
    let dir = to_car / fd;
    let v = pos - bs.eye;
    let along = v.dot(dir);
    if along > fd + 1.5 || along < 0.1 {
        return 1.0;
    }
    // where the puff lands on screen, measured in metres at the car's distance
    let q = v * (fd / along) - to_car;
    let lateral = (q - dir * q.dot(dir)).length();
    let r_proj = r * (fd / along);
    ((lateral - 0.9) / (r_proj * 0.7 + 0.6)).clamp(0.12, 1.0)
}

/// Soft lit cloud: a dense rounded core that rolls off to a transparent, slightly lumpy rim.
/// Vertex colours are brighter on top and a touch shaded underneath so every puff reads as
/// a soft round volume rather than a flat smear.
pub fn puff(b: &mut Batches, bs: &Basis, circ: &Circle, p: &Particle, dist: f32, focus: Option<Vec3>) -> f32 {
    let (guard, r) = cloud_guard(dist, p.size());
    let mut a = p.opacity() * near_fade(dist, r) * guard;
    if let Some(f) = focus {
        a *= car_clearance(bs, f, p.pos, r);
    }
    if a < 0.01 {
        return 0.0;
    }
    // pull towards the camera so clouds don't slice into the ground behind them
    let c = p.pos + bs.toward * (r * 0.25).min(dist * 0.15);
    // spin the billboard slowly so the low-poly rim never looks static
    let (sr, cr) = p.rot.sin_cos();
    let ax = bs.right * cr + bs.up * sr;
    let ay = bs.up * cr - bs.right * sr;
    // normals bulge like a sphere but lean upwards, so clouds are lit from the sky
    let fwd = (bs.toward + Vec3::Y * 0.9).normalize();
    let tone = |d: Vec3, k: f32| {
        let y = d.dot(bs.up);
        let f = 1.0 + (0.08 * y.max(0.0) - 0.16 * (-y).max(0.0)) * k;
        [p.color[0] * f, p.color[1] * f, p.color[2] * f]
    };
    let detailed = r / dist.max(0.1) > 0.012;
    if detailed {
        let batch = b.room(17);
        let base = batch.vert(c + bs.up * (r * 0.06), fwd, col(tone(bs.up, 0.6), a));
        // 8 inner (dense core, 92% opacity) + 8 outer (transparent, lumpy) vertices
        for k in 0..8 {
            let (x, y) = circ.pts16[k * 2];
            let d = flat_bottom(ax * x + ay * y, bs.up);
            let wob = 1.0 + 0.1 * ((k as f32) * 2.3 + p.seed * 31.0).sin();
            batch.vert(c + d * (r * 0.58 * wob), (fwd + d * 0.3).normalize(), col(tone(d, 1.0), a * 0.92));
        }
        for k in 0..8 {
            let (x, y) = circ.pts16[k * 2 + 1];
            let d = flat_bottom(ax * x + ay * y, bs.up);
            let wob = 1.0 + 0.14 * ((k as f32) * 1.7 + p.seed * 17.0).sin();
            batch.vert(c + d * (r * wob), (fwd + d * 0.5).normalize(), col(tone(d, 1.0), 0.0));
        }
        for k in 0..8u16 {
            let i0 = base + 1 + k;
            let i1 = base + 1 + (k + 1) % 8;
            let o0 = base + 9 + k;
            let o_prev = base + 9 + (k + 7) % 8;
            batch.tri(base, i0, i1);
            // outer ring: each outer vertex sits between two inner ones
            batch.tri(i0, o0, i1);
            batch.tri(i0, o_prev, o0);
        }
    } else {
        let batch = b.room(9);
        let base = batch.vert(c, fwd, col(p.color, a * 0.85));
        for k in 0..8 {
            let (x, y) = circ.pts16[k * 2];
            let d = flat_bottom(ax * x + ay * y, bs.up);
            batch.vert(c + d * r, (fwd + d * 0.5).normalize(), col(tone(d, 1.0), 0.0));
        }
        for k in 0..8u16 {
            batch.tri(base, base + 1 + k, base + 1 + (k + 1) % 8);
        }
    }
    // screen-coverage estimate (angular area x alpha) for the fill-rate stat
    let c = r / dist.max(0.1);
    c * c * a
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
    let a = p.opacity() * near_fade(dist, 2.5);
    if a < 0.02 {
        return;
    }
    let s = p.size();
    let c = col(p.color, a);
    match p.shape {
        Shape::Chunk => {
            // a small rounded pebble: lumpy octagon facing the camera, sphere-ish normals and
            // a lit top / shaded underside; the lumpy rim spins so it reads as tumbling
            let r = s.max(dist * 0.0018);
            let (sr, cr) = p.rot.sin_cos();
            let ax = bs.right * cr + bs.up * sr;
            let ay = bs.up * cr - bs.right * sr;
            let batch = b.room(9);
            let top = [p.color[0] * 1.08 + 0.03, p.color[1] * 1.08 + 0.03, p.color[2] * 1.08 + 0.03];
            let base = batch.vert(p.pos, (bs.toward + Vec3::Y * 0.6).normalize(), col(top, a));
            for k in 0..8 {
                let ang = k as f32 / 8.0 * TAU;
                let lump = 0.82 + 0.18 * ((k as f32) * 2.7 + p.seed * 23.0).sin();
                let d = ax * (ang.cos() * lump) + ay * (ang.sin() * lump * 0.8);
                let y = d.dot(bs.up);
                let f = if y < 0.0 { 1.0 + y * 0.35 } else { 1.0 + y * 0.08 };
                batch.vert(p.pos + d * r, (bs.toward * 0.5 + d).normalize(), col([p.color[0] * f, p.color[1] * f, p.color[2] * f], a));
            }
            for k in 0..8u16 {
                batch.tri(base, base + 1 + k, base + 1 + (k + 1) % 8);
            }
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
        // rounded fore + hind wings fanned from the body, paler towards the tips
        const FORE: [(f32, f32); 5] = [(0.3, 0.5), (0.7, 0.62), (0.98, 0.42), (1.0, 0.1), (0.72, -0.08)];
        const HIND: [(f32, f32); 4] = [(0.6, -0.12), (0.7, -0.4), (0.52, -0.62), (0.22, -0.66)];
        // readability floor: far butterflies never shrink below a few pixels
        let s = s.max(dist * 0.007);
        let ang = flap * 1.15 + 0.3;
        // deeper wing edges + pale wing roots give a crisp two-tone silhouette on grass
        let tip = col([p.color[0] * 0.78, p.color[1] * 0.74, p.color[2] * 0.8], a);
        let c = col([p.color[0] * 0.5 + 0.5, p.color[1] * 0.5 + 0.5, p.color[2] * 0.5 + 0.5], a);
        let hind_c = col([p.color[0] * 0.9, p.color[1] * 0.9, p.color[2] * 0.9], a);
        let batch = b.room(24);
        for sgn in [-1.0f32, 1.0] {
            let w = side * (sgn * ang.cos()) + Vec3::Y * ang.sin();
            let n = h.cross(w).normalize_or(Vec3::Y);
            let root = batch.vert(p.pos + h * (s * 0.1), n, c);
            let mut prev: Option<u16> = None;
            for &(o, f) in FORE.iter() {
                let v = batch.vert(p.pos + w * (o * s) + h * (f * s), n, tip);
                if let Some(pv) = prev {
                    batch.tri(root, pv, v);
                }
                prev = Some(v);
            }
            let root2 = batch.vert(p.pos - h * (s * 0.1), n, hind_c);
            let mut prev: Option<u16> = None;
            for &(o, f) in HIND.iter() {
                let v = batch.vert(p.pos + w * (o * s) + h * (f * s), n, hind_c);
                if let Some(pv) = prev {
                    batch.tri(root2, pv, v);
                }
                prev = Some(v);
            }
        }
        // slim body
        let dark = col([0.32, 0.26, 0.28], a);
        let n = Vec3::Y;
        let i0 = batch.vert(p.pos + h * (s * 0.32), n, dark);
        let i1 = batch.vert(p.pos - h * (s * 0.4) + side * (s * 0.05), n, dark);
        let i2 = batch.vert(p.pos - h * (s * 0.4) - side * (s * 0.05), n, dark);
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
