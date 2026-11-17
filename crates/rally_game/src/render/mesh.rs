//! CPU-side mesh building with flat-shaded low-poly primitives.
//! Meshes are automatically split into chunks of < 65k vertices (u16 indices, GLES2 safe).
use macroquad::math::{Mat4, Quat, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [u8; 4],
}

pub const MAX_CHUNK_VERTS: usize = 60_000;

#[derive(Clone, Default)]
pub struct MeshChunk {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub min: Vec3,
    pub max: Vec3,
}

#[derive(Clone, Default)]
pub struct MeshBuilder {
    pub chunks: Vec<MeshChunk>,
}

pub fn rgb(c: [f32; 3]) -> [u8; 4] {
    [(c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8, 255]
}

pub fn rgba(c: [f32; 3], a: f32) -> [u8; 4] {
    let mut v = rgb(c);
    v[3] = (a.clamp(0.0, 1.0) * 255.0) as u8;
    v
}

pub fn shade(c: [f32; 3], f: f32) -> [f32; 3] {
    [c[0] * f, c[1] * f, c[2] * f]
}

pub fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

impl MeshBuilder {
    pub fn new() -> Self {
        MeshBuilder { chunks: Vec::new() }
    }

    fn chunk_for(&mut self, extra: usize) -> &mut MeshChunk {
        let need_new = match self.chunks.last() {
            None => true,
            Some(c) => c.vertices.len() + extra > MAX_CHUNK_VERTS,
        };
        if need_new {
            self.chunks.push(MeshChunk { min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN), ..Default::default() });
        }
        self.chunks.last_mut().unwrap()
    }

    /// Force subsequent geometry into a new chunk (spatial tiling for culling)
    pub fn begin_chunk(&mut self) {
        if self.chunks.last().map_or(false, |c| !c.vertices.is_empty()) {
            self.chunks.push(MeshChunk { min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN), ..Default::default() });
        }
    }

    pub fn vertex_count(&self) -> usize {
        self.chunks.iter().map(|c| c.vertices.len()).sum()
    }

    /// Flat-shaded triangle (counter-clockwise front face)
    pub fn tri(&mut self, a: Vec3, b: Vec3, c: Vec3, color: [u8; 4]) {
        let n = (b - a).cross(c - a).normalize_or_zero();
        self.tri_n(a, b, c, n, color);
    }

    pub fn tri_n(&mut self, a: Vec3, b: Vec3, c: Vec3, n: Vec3, color: [u8; 4]) {
        let ch = self.chunk_for(3);
        let base = ch.vertices.len() as u16;
        for p in [a, b, c] {
            ch.vertices.push(Vertex { pos: p.to_array(), normal: n.to_array(), color });
            ch.min = ch.min.min(p);
            ch.max = ch.max.max(p);
        }
        ch.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Triangle with per-vertex colours & normals (smooth)
    pub fn tri_smooth(&mut self, v: [(Vec3, Vec3, [u8; 4]); 3]) {
        let ch = self.chunk_for(3);
        let base = ch.vertices.len() as u16;
        for (p, n, c) in v {
            ch.vertices.push(Vertex { pos: p.to_array(), normal: n.to_array(), color: c });
            ch.min = ch.min.min(p);
            ch.max = ch.max.max(p);
        }
        ch.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Quad a-b-c-d counter-clockwise
    pub fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, color: [u8; 4]) {
        self.tri(a, b, c, color);
        self.tri(a, c, d, color);
    }

    /// Axis-aligned box transformed by `m`
    pub fn cube(&mut self, m: Mat4, half: Vec3, color: [u8; 4]) {
        self.box_tapered(m, half, 1.0, color);
    }

    /// Box whose top face is scaled by `top_scale` in X/Z (nice for car cabins, rocks)
    pub fn box_tapered(&mut self, m: Mat4, half: Vec3, top_scale: f32, color: [u8; 4]) {
        let h = half;
        let t = top_scale;
        let p = [
            Vec3::new(-h.x, -h.y, -h.z),
            Vec3::new(h.x, -h.y, -h.z),
            Vec3::new(h.x, -h.y, h.z),
            Vec3::new(-h.x, -h.y, h.z),
            Vec3::new(-h.x * t, h.y, -h.z * t),
            Vec3::new(h.x * t, h.y, -h.z * t),
            Vec3::new(h.x * t, h.y, h.z * t),
            Vec3::new(-h.x * t, h.y, h.z * t),
        ];
        let p: Vec<Vec3> = p.iter().map(|v| m.transform_point3(*v)).collect();
        let faces = [[0, 1, 2, 3], [7, 6, 5, 4], [3, 2, 6, 7], [1, 0, 4, 5], [0, 3, 7, 4], [2, 1, 5, 6]];
        for f in faces {
            self.quad(p[f[0]], p[f[1]], p[f[2]], p[f[3]], color);
        }
    }

    /// Box defined by 8 explicit local corners: bottom (-z..+z ccw) then top, transformed by m
    pub fn hexa(&mut self, m: Mat4, p: [Vec3; 8], color: [u8; 4]) {
        let p: Vec<Vec3> = p.iter().map(|v| m.transform_point3(*v)).collect();
        let faces = [[0, 1, 2, 3], [7, 6, 5, 4], [3, 2, 6, 7], [1, 0, 4, 5], [0, 3, 7, 4], [2, 1, 5, 6]];
        for f in faces {
            self.quad(p[f[0]], p[f[1]], p[f[2]], p[f[3]], color);
        }
    }

    /// Cylinder along local Y from -h to +h
    pub fn cylinder(&mut self, m: Mat4, radius: f32, half_h: f32, segments: usize, color: [u8; 4], cap_color: [u8; 4]) {
        self.frustum(m, radius, radius, half_h, segments, color, cap_color);
    }

    /// Cone-frustum along local Y: bottom radius r0 at -h, top r1 at +h
    pub fn frustum(&mut self, m: Mat4, r0: f32, r1: f32, half_h: f32, segments: usize, color: [u8; 4], cap_color: [u8; 4]) {
        let mut bottom = Vec::with_capacity(segments);
        let mut top = Vec::with_capacity(segments);
        for i in 0..segments {
            let a = i as f32 / segments as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            bottom.push(m.transform_point3(Vec3::new(c * r0, -half_h, s * r0)));
            top.push(m.transform_point3(Vec3::new(c * r1, half_h, s * r1)));
        }
        let cb = m.transform_point3(Vec3::new(0.0, -half_h, 0.0));
        let ct = m.transform_point3(Vec3::new(0.0, half_h, 0.0));
        for i in 0..segments {
            let j = (i + 1) % segments;
            if r1 > 1e-4 {
                self.quad(bottom[j], bottom[i], top[i], top[j], color);
                self.tri(ct, top[j], top[i], cap_color);
            } else {
                self.tri(bottom[j], bottom[i], ct, color);
            }
            if r0 > 1e-4 {
                self.tri(cb, bottom[i], bottom[j], cap_color);
            }
        }
    }

    /// Low-poly sphere (UV sphere with few rings), scaled by `scale`
    pub fn blob(&mut self, m: Mat4, scale: Vec3, rings: usize, segments: usize, color: [u8; 4]) {
        let mut pts = Vec::new();
        for r in 0..=rings {
            let phi = r as f32 / rings as f32 * std::f32::consts::PI;
            let mut row = Vec::new();
            for s in 0..segments {
                let th = s as f32 / segments as f32 * std::f32::consts::TAU;
                let p = Vec3::new(phi.sin() * th.cos(), phi.cos(), phi.sin() * th.sin()) * scale;
                row.push(m.transform_point3(p));
            }
            pts.push(row);
        }
        for r in 0..rings {
            for s in 0..segments {
                let t = (s + 1) % segments;
                let a = pts[r][s];
                let b = pts[r][t];
                let c = pts[r + 1][t];
                let d = pts[r + 1][s];
                if r != 0 {
                    self.tri(a, b, d, color);
                }
                if r != rings - 1 {
                    self.tri(b, c, d, color);
                }
            }
        }
    }

    pub fn append(&mut self, other: &MeshBuilder) {
        for ch in &other.chunks {
            let dst = self.chunk_for(ch.vertices.len());
            let base = dst.vertices.len() as u16;
            dst.vertices.extend_from_slice(&ch.vertices);
            dst.indices.extend(ch.indices.iter().map(|i| i + base));
            dst.min = dst.min.min(ch.min);
            dst.max = dst.max.max(ch.max);
        }
    }

    /// Append another mesh transformed by m (normals rotated)
    pub fn append_transformed(&mut self, other: &MeshBuilder, m: Mat4) {
        for ch in &other.chunks {
            let dst = self.chunk_for(ch.vertices.len());
            let base = dst.vertices.len() as u16;
            for v in &ch.vertices {
                let p = m.transform_point3(Vec3::from(v.pos));
                let n = m.transform_vector3(Vec3::from(v.normal)).normalize_or_zero();
                dst.vertices.push(Vertex { pos: p.to_array(), normal: n.to_array(), color: v.color });
                dst.min = dst.min.min(p);
                dst.max = dst.max.max(p);
            }
            dst.indices.extend(ch.indices.iter().map(|i| i + base));
        }
    }
}

pub fn trs(t: Vec3, r: Quat, s: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(s, r, t)
}

/// Vertex-colour convention for OPAQUE meshes: alpha encodes wind-sway weight
/// (255 = rigid, lower = sways more; the opaque shader treats output alpha as 1).
/// Use for foliage tips, grass blades, flags. Never use on alpha-blended meshes.
pub fn sway(c: [u8; 4], weight: f32) -> [u8; 4] {
    [c[0], c[1], c[2], (255.0 - weight.clamp(0.0, 1.0) * 200.0) as u8]
}
