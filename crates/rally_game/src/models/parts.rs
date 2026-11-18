//! Modelling helpers shared by cars and props: lofts, lathes, bevelled boxes, inset panels,
//! beams and position-driven wind sway. Kept here (not in render/mesh.rs) so the renderer's
//! MeshBuilder stays small; everything goes through the public MeshBuilder API.
use crate::render::mesh::{rgb, sway, MeshBuilder};
use macroquad::math::{Mat4, Quat, Vec3};

/// A position in a MeshBuilder's vertex stream (chunk index, vertex index).
#[derive(Clone, Copy)]
pub struct Mark(usize, usize);

#[allow(dead_code)]
pub trait MeshExt {
    fn mark(&self) -> Mark;
    /// Set the wind-sway weight of every vertex appended since `mark` from its position.
    /// Weights must be a pure function of position so shared corners never crack apart.
    fn sway_since(&mut self, mark: Mark, f: impl Fn(Vec3) -> f32);
    /// Recolour every triangle appended since `mark` from its centroid (keeps sway alpha)
    fn recolor_tris_since(&mut self, mark: Mark, f: impl Fn(Vec3) -> [u8; 4]);
    /// Transform every vertex appended since `mark` (positions + normals)
    fn transform_since(&mut self, mark: Mark, m: Mat4);
    /// Convex planar polygon (fan triangulated)
    fn poly(&mut self, pts: &[Vec3], color: [u8; 4]);
    /// Convex planar polygon shrunk by `inset` (metres, edge offset) and pushed `lift` along its normal.
    /// The normal is taken to point away from `outside_hint` reversed, i.e. towards `out`.
    fn panel(&mut self, pts: &[Vec3], inset: f32, lift: f32, out: Vec3, color: [u8; 4]);
    /// Connect rings of equal length with quads; `color(segment, quad)`; optional end caps.
    fn loft(&mut self, rings: &[Vec<Vec3>], color: impl Fn(usize, usize) -> [u8; 4], cap0: Option<[u8; 4]>, cap1: Option<[u8; 4]>);
    /// Surface of revolution around local X: profile of (x, radius) pairs
    fn lathe_x(&mut self, m: Mat4, profile: &[(f32, f32)], segs: usize, color: impl Fn(usize, usize) -> [u8; 4]);
    /// Box with bevelled edges (bevel in metres), transformed by m
    fn bevel_box(&mut self, m: Mat4, half: Vec3, bevel: f32, color: [u8; 4]);
    /// Square-section beam from a to b
    fn beam(&mut self, a: Vec3, b: Vec3, thick: f32, color: [u8; 4]);
    /// Flat beam from a to b whose wide side faces `up`
    fn plank(&mut self, a: Vec3, b: Vec3, width: f32, thick: f32, up: Vec3, color: [u8; 4]);
    /// Double-sided flat ring (annulus) in the plane with normal `n`
    fn disc(&mut self, center: Vec3, n: Vec3, radius: f32, segs: usize, color: [u8; 4]);
}

impl MeshExt for MeshBuilder {
    fn mark(&self) -> Mark {
        match self.chunks.last() {
            None => Mark(0, 0),
            Some(c) => Mark(self.chunks.len() - 1, c.vertices.len()),
        }
    }

    fn sway_since(&mut self, mark: Mark, f: impl Fn(Vec3) -> f32) {
        for (ci, ch) in self.chunks.iter_mut().enumerate().skip(mark.0) {
            let start = if ci == mark.0 { mark.1 } else { 0 };
            for v in ch.vertices.iter_mut().skip(start) {
                let w = f(Vec3::from(v.pos));
                v.color = sway(v.color, w);
            }
        }
    }

    fn recolor_tris_since(&mut self, mark: Mark, f: impl Fn(Vec3) -> [u8; 4]) {
        for (ci, ch) in self.chunks.iter_mut().enumerate().skip(mark.0) {
            let start = if ci == mark.0 { mark.1 } else { 0 };
            // flat-shaded triangles are 3 consecutive vertices
            let mut k = start;
            while k + 2 < ch.vertices.len() {
                let cen = (Vec3::from(ch.vertices[k].pos) + Vec3::from(ch.vertices[k + 1].pos) + Vec3::from(ch.vertices[k + 2].pos)) / 3.0;
                let c = f(cen);
                for v in &mut ch.vertices[k..k + 3] {
                    v.color = [c[0], c[1], c[2], v.color[3]];
                }
                k += 3;
            }
        }
    }

    fn transform_since(&mut self, mark: Mark, m: Mat4) {
        for (ci, ch) in self.chunks.iter_mut().enumerate().skip(mark.0) {
            let start = if ci == mark.0 { mark.1 } else { 0 };
            if start >= ch.vertices.len() {
                continue;
            }
            let mut min = Vec3::splat(f32::MAX);
            let mut max = Vec3::splat(f32::MIN);
            for (k, v) in ch.vertices.iter_mut().enumerate() {
                if k >= start {
                    let p = m.transform_point3(Vec3::from(v.pos));
                    v.pos = p.to_array();
                    v.normal = m.transform_vector3(Vec3::from(v.normal)).normalize_or_zero().to_array();
                }
                let p = Vec3::from(v.pos);
                min = min.min(p);
                max = max.max(p);
            }
            ch.min = min;
            ch.max = max;
        }
    }

    fn poly(&mut self, pts: &[Vec3], color: [u8; 4]) {
        for i in 1..pts.len().saturating_sub(1) {
            self.tri(pts[0], pts[i], pts[i + 1], color);
        }
    }

    fn panel(&mut self, pts: &[Vec3], inset: f32, lift: f32, out: Vec3, color: [u8; 4]) {
        let n = pts.len();
        if n < 3 {
            return;
        }
        let mut nrm = Vec3::ZERO;
        for i in 0..n {
            nrm += pts[i].cross(pts[(i + 1) % n]);
        }
        let mut nrm = nrm.normalize_or_zero();
        // polygon winding decides the inward direction; make it consistent with nrm
        let flip = nrm.dot(out) < 0.0;
        let face_n = if flip { -nrm } else { nrm };
        let res: Vec<Vec3> = (0..n)
            .map(|i| {
                let p = pts[i];
                let e0 = (p - pts[(i + n - 1) % n]).normalize_or_zero();
                let e1 = (pts[(i + 1) % n] - p).normalize_or_zero();
                let i0 = nrm.cross(e0);
                let i1 = nrm.cross(e1);
                let d = 1.0 + i0.dot(i1);
                let off = if d.abs() > 1e-3 { (i0 + i1) / d } else { i0 };
                p + off * inset + face_n * lift
            })
            .collect();
        nrm = face_n;
        let _ = nrm;
        self.poly(&res, color);
    }

    fn loft(&mut self, rings: &[Vec<Vec3>], color: impl Fn(usize, usize) -> [u8; 4], cap0: Option<[u8; 4]>, cap1: Option<[u8; 4]>) {
        for s in 0..rings.len().saturating_sub(1) {
            let a = &rings[s];
            let b = &rings[s + 1];
            let k = a.len();
            for i in 0..k {
                let j = (i + 1) % k;
                let c = color(s, i);
                // alpha 0 = hidden face, skip it
                if c[3] == 0 {
                    continue;
                }
                // skip degenerate quads (collapsed rings)
                if a[i].distance_squared(a[j]) < 1e-10 && b[i].distance_squared(b[j]) < 1e-10 {
                    continue;
                }
                if a[i].distance_squared(a[j]) < 1e-10 {
                    self.tri(a[i], b[j], b[i], c);
                } else if b[i].distance_squared(b[j]) < 1e-10 {
                    self.tri(a[i], a[j], b[i], c);
                } else {
                    self.quad(a[i], a[j], b[j], b[i], c);
                }
            }
        }
        if let (Some(c), Some(r)) = (cap0, rings.first()) {
            let cen = r.iter().copied().sum::<Vec3>() / r.len() as f32;
            for i in 0..r.len() {
                self.tri(cen, r[(i + 1) % r.len()], r[i], c);
            }
        }
        if let (Some(c), Some(r)) = (cap1, rings.last()) {
            let cen = r.iter().copied().sum::<Vec3>() / r.len() as f32;
            for i in 0..r.len() {
                self.tri(cen, r[i], r[(i + 1) % r.len()], c);
            }
        }
    }

    fn lathe_x(&mut self, m: Mat4, profile: &[(f32, f32)], segs: usize, color: impl Fn(usize, usize) -> [u8; 4]) {
        let rings: Vec<Vec<Vec3>> = profile
            .iter()
            .map(|&(x, r)| {
                (0..segs)
                    .map(|i| {
                        let a = i as f32 / segs as f32 * std::f32::consts::TAU;
                        m.transform_point3(Vec3::new(x, a.cos() * r, a.sin() * r))
                    })
                    .collect()
            })
            .collect();
        self.loft(&rings, |s, i| color(s, i), None, None);
    }

    fn bevel_box(&mut self, m: Mat4, half: Vec3, bevel: f32, color: [u8; 4]) {
        let b = bevel.min(half.x * 0.9).min(half.y * 0.9).min(half.z * 0.9).max(0.0);
        let ring = |z: f32, inset: f32| -> Vec<Vec3> {
            let (hx, hy) = (half.x - inset, half.y - inset);
            let bb = (b - inset).max(0.0);
            [
                (hx - bb, -hy),
                (hx, -hy + bb),
                (hx, hy - bb),
                (hx - bb, hy),
                (-hx + bb, hy),
                (-hx, hy - bb),
                (-hx, -hy + bb),
                (-hx + bb, -hy),
            ]
            .iter()
            .map(|&(x, y)| m.transform_point3(Vec3::new(x, y, z)))
            .collect()
        };
        let rings = vec![ring(-half.z, b), ring(-half.z + b, 0.0), ring(half.z - b, 0.0), ring(half.z, b)];
        self.loft(&rings, |_, _| color, Some(color), Some(color));
    }

    fn beam(&mut self, a: Vec3, b: Vec3, thick: f32, color: [u8; 4]) {
        let d = b - a;
        let len = d.length();
        if len < 1e-5 {
            return;
        }
        let rot = Quat::from_rotation_arc(Vec3::Y, d / len);
        self.cube(Mat4::from_rotation_translation(rot, (a + b) * 0.5), Vec3::new(thick * 0.5, len * 0.5, thick * 0.5), color);
    }

    fn plank(&mut self, a: Vec3, b: Vec3, width: f32, thick: f32, up: Vec3, color: [u8; 4]) {
        let d = b - a;
        let len = d.length();
        if len < 1e-5 {
            return;
        }
        let x = d / len;
        let z = x.cross(up).normalize_or_zero();
        let y = z.cross(x);
        let m = Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), ((a + b) * 0.5).extend(1.0));
        self.cube(m, Vec3::new(len * 0.5, thick * 0.5, width * 0.5), color);
    }

    fn disc(&mut self, center: Vec3, n: Vec3, radius: f32, segs: usize, color: [u8; 4]) {
        let n = n.normalize_or_zero();
        let t = if n.y.abs() < 0.9 { n.cross(Vec3::Y).normalize() } else { n.cross(Vec3::X).normalize() };
        let b = n.cross(t);
        for i in 0..segs {
            let a0 = i as f32 / segs as f32 * std::f32::consts::TAU;
            let a1 = (i + 1) as f32 / segs as f32 * std::f32::consts::TAU;
            self.tri(center, center + (t * a0.cos() + b * a0.sin()) * radius, center + (t * a1.cos() + b * a1.sin()) * radius, color);
        }
    }
}

/// Seven-segment digit made of flat quads lying in the plane spanned by `right`/`up`,
/// centred at `c`, `h` tall. Used for race numbers on doors and roofs.
pub fn digit(m: &mut MeshBuilder, c: Vec3, right: Vec3, up: Vec3, h: f32, d: u8, color: [u8; 4]) {
    // segments: a top, b top-right, c bottom-right, d bottom, e bottom-left, f top-left, g middle
    const SEGS: [u8; 10] = [0b0111111, 0b0000110, 0b1011011, 0b1001111, 0b1100110, 0b1101101, 0b1111101, 0b0000111, 0b1111111, 0b1101111];
    let bits = SEGS[(d % 10) as usize];
    let w = h * 0.5;
    let t = h * 0.13;
    let hw = w * 0.5;
    let hh = h * 0.5;
    let bar = |m: &mut MeshBuilder, x0: f32, y0: f32, x1: f32, y1: f32| {
        let p = |x: f32, y: f32| c + right * x + up * y;
        m.quad(p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1), color);
    };
    let segs: [(f32, f32, f32, f32); 7] = [
        (-hw, hh - t, hw, hh),
        (hw - t, 0.0, hw, hh),
        (hw - t, -hh, hw, 0.0),
        (-hw, -hh, hw, -hh + t),
        (-hw, -hh, -hw + t, 0.0),
        (-hw, 0.0, -hw + t, hh),
        (-hw, -t * 0.5, hw, t * 0.5),
    ];
    for (k, s) in segs.iter().enumerate() {
        if bits & (1 << k) != 0 {
            bar(m, s.0, s.1, s.2, s.3);
        }
    }
}

/// Number roundel: cream disc + (one or two) digits, facing `n`.
pub fn roundel(m: &mut MeshBuilder, c: Vec3, n: Vec3, up: Vec3, r: f32, number: u8, disc_c: [f32; 3], ink: [f32; 3]) {
    let n = n.normalize();
    m.disc(c, n, r, 12, rgb(disc_c));
    let right = up.cross(n).normalize();
    // digits sit 1.2 cm off the disc: no coplanar overlap even when the car is far away
    let c2 = c + n * 0.012;
    if number >= 10 {
        digit(m, c2 - right * r * 0.3, right, up, r * 1.05, number / 10, rgb(ink));
        digit(m, c2 + right * r * 0.3, right, up, r * 1.05, number % 10, rgb(ink));
    } else {
        digit(m, c2, right, up, r * 1.15, number, rgb(ink));
    }
}

/// Deterministic small hash -> [0,1)
pub fn h01(a: f32, b: f32) -> f32 {
    let v = (a * 12.9898 + b * 78.233).sin() * 43758.547;
    v - v.floor()
}

/// 5x5 block-letter glyph rows (bit 4 = leftmost); unknown chars render blank.
fn glyph(c: char) -> [u8; 5] {
    match c {
        'A' => [0b01110, 0b10001, 0b11111, 0b10001, 0b10001],
        'F' => [0b11111, 0b10000, 0b11110, 0b10000, 0b10000],
        'H' => [0b10001, 0b10001, 0b11111, 0b10001, 0b10001],
        'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b11111],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001],
        'O' | '0' => [0b01110, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b11110, 0b10000, 0b10000],
        'R' => [0b11110, 0b10001, 0b11110, 0b10100, 0b10011],
        'S' => [0b01111, 0b10000, 0b01110, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b01110],
        '2' => [0b11110, 0b00001, 0b01110, 0b10000, 0b11111],
        '3' => [0b11110, 0b00001, 0b01110, 0b00001, 0b11110],
        '4' => [0b10010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b11110],
        '6' => [0b01110, 0b10000, 0b11110, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b00100],
        '8' => [0b01110, 0b10001, 0b01110, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b01111, 0b00001, 0b01110],
        '!' => [0b00100, 0b00100, 0b00100, 0b00000, 0b00100],
        _ => [0; 5],
    }
}

/// Block text centred at `c` on a plane (right/up in-plane axes), letter height `h`.
/// Pixel runs are merged into one quad per row so text stays cheap.
pub fn block_text(m: &mut MeshBuilder, c: Vec3, right: Vec3, up: Vec3, h: f32, text: &str, color: [u8; 4]) {
    let px = h / 5.0;
    let adv = px * 6.0;
    let n = text.chars().count() as f32;
    let x0 = -(n * adv - px) * 0.5;
    for (i, ch) in text.chars().enumerate() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            let y1 = h * 0.5 - row as f32 * px;
            let y0 = y1 - px;
            let mut col = 0;
            while col < 5 {
                if bits & (1 << (4 - col)) != 0 {
                    let start = col;
                    while col < 5 && bits & (1 << (4 - col)) != 0 {
                        col += 1;
                    }
                    let xa = x0 + i as f32 * adv + start as f32 * px;
                    let xb = x0 + i as f32 * adv + col as f32 * px;
                    let p = |x: f32, y: f32| c + right * x + up * y;
                    m.quad(p(xa, y0), p(xb, y0), p(xb, y1), p(xa, y1), color);
                } else {
                    col += 1;
                }
            }
        }
    }
}
