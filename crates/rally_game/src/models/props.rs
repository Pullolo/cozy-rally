//! Props: trees, rocks, buildings, critters, signs and festive bits, per biome.
//! Everything is appended in world space; foliage, flags and grass get a wind-sway weight
//! (vertex alpha, see `mesh::sway`) that depends only on position so shared corners never crack.
use super::parts::{h01, MeshExt};
use crate::render::mesh::{mix, rgb, shade, MeshBuilder};
use macroquad::math::{Mat4, Quat, Vec3};
use rally_core::biome::{Biome, BiomeStyle};
use rally_core::track::{Prop, PropKind};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

const WOOD: [f32; 3] = [0.62, 0.45, 0.30];
const WOOD_D: [f32; 3] = [0.45, 0.32, 0.22];
const CREAM: [f32; 3] = [0.98, 0.94, 0.84];
const WHITE: [f32; 3] = [0.97, 0.97, 0.95];
const DARK: [f32; 3] = [0.22, 0.21, 0.24];
const SNOW: [f32; 3] = [0.96, 0.97, 1.0];
const STONE: [f32; 3] = [0.62, 0.6, 0.58];
const PASTELS: [[f32; 3]; 6] = [[0.95, 0.45, 0.42], [0.40, 0.65, 0.92], [0.98, 0.82, 0.35], [0.50, 0.78, 0.50], [0.78, 0.55, 0.90], [0.98, 0.62, 0.35]];

fn biome_of(style: &BiomeStyle) -> Biome {
    match style.name {
        "Maple Woods" => Biome::Autumn,
        "Frosty Peaks" => Biome::Snow,
        "Canyon Dunes" => Biome::Desert,
        "Seaside Cliffs" => Biome::Coast,
        _ => Biome::Meadow,
    }
}

/// Local frame of a prop: uniform scale, yaw, position.
struct Frame {
    base: Mat4,
    pos: Vec3,
    s: f32,
    seed: f32,
}

impl Frame {
    fn at(&self, x: f32, y: f32, z: f32) -> Mat4 {
        self.base * Mat4::from_translation(Vec3::new(x, y, z))
    }
    fn atr(&self, x: f32, y: f32, z: f32, r: Quat) -> Mat4 {
        self.base * Mat4::from_rotation_translation(r, Vec3::new(x, y, z))
    }
    fn p(&self, x: f32, y: f32, z: f32) -> Vec3 {
        self.base.transform_point3(Vec3::new(x, y, z))
    }
    fn r(&self, k: f32) -> f32 {
        h01(self.seed, k)
    }
}

/// Sway weight growing with height above the prop base (0 at `y0`, `w` at `y0 + h`).
fn sway_up(m: &mut MeshBuilder, mk: super::parts::Mark, base_y: f32, y0: f32, h: f32, w: f32) {
    m.sway_since(mk, move |p| (((p.y - base_y) - y0) / h).clamp(0.0, 1.0) * w);
}

/// Gabled roof along local X over walls of half size (hx, hz), eaves at y0, ridge at y1.
fn gable(m: &mut MeshBuilder, f: &Frame, hx: f32, hz: f32, y0: f32, y1: f32, o: f32, roof: [f32; 3], wall: [f32; 3]) {
    let t = 0.14;
    for sz in [-1.0f32, 1.0] {
        let e0 = f.p(-hx - o, y0 - o * 0.5, sz * (hz + o));
        let e1 = f.p(hx + o, y0 - o * 0.5, sz * (hz + o));
        let r0 = f.p(-hx - o, y1, 0.0);
        let r1 = f.p(hx + o, y1, 0.0);
        let up = Vec3::Y * t;
        m.quad(e0 + up, e1 + up, r1 + up, r0 + up, rgb(roof));
        m.quad(e0, e1, r1, r0, rgb(shade(roof, 0.8)));
        m.quad(e0, e1, e1 + up, e0 + up, rgb(shade(roof, 0.75)));
        for (a, b) in [(e0, r0), (e1, r1)] {
            m.quad(a, b, b + up, a + up, rgb(shade(roof, 0.75)));
        }
    }
    for sx in [-1.0f32, 1.0] {
        m.tri(f.p(sx * hx, y0, -hz), f.p(sx * hx, y0, hz), f.p(sx * hx, y1, 0.0), rgb(wall));
    }
    // ridge cap
    m.beam(f.p(-hx - o, y1 + t, 0.0), f.p(hx + o, y1 + t, 0.0), 0.16, rgb(shade(roof, 0.7)));
}

/// Window with frame + cross on a wall facing local `n` (x/z), centred at local c.
fn window(m: &mut MeshBuilder, f: &Frame, c: Vec3, n: Vec3, w: f32, h: f32, frame: [f32; 3], glass: [f32; 3], shutters: Option<[f32; 3]>) {
    let right = Vec3::Y.cross(n).normalize();
    let q = |m: &mut MeshBuilder, cc: Vec3, hw: f32, hh: f32, lift: f32, col: [f32; 3]| {
        let o = cc + n * lift;
        m.quad(f.p_v(o - right * hw - Vec3::Y * hh), f.p_v(o + right * hw - Vec3::Y * hh), f.p_v(o + right * hw + Vec3::Y * hh), f.p_v(o - right * hw + Vec3::Y * hh), rgb(col));
    };
    q(m, c, w * 0.5 + 0.08, h * 0.5 + 0.08, 0.02, frame);
    q(m, c, w * 0.5, h * 0.5, 0.04, glass);
    q(m, c, 0.03, h * 0.5, 0.05, frame);
    q(m, c, w * 0.5, 0.03, 0.05, frame);
    if let Some(sh) = shutters {
        for s in [-1.0f32, 1.0] {
            q(m, c + right * s * (w * 0.5 + 0.2), 0.16, h * 0.5 + 0.04, 0.03, sh);
        }
    }
}

trait FrameV {
    fn p_v(&self, v: Vec3) -> Vec3;
}
impl FrameV for Frame {
    fn p_v(&self, v: Vec3) -> Vec3 {
        self.base.transform_point3(v)
    }
}

fn flower_box(m: &mut MeshBuilder, f: &Frame, c: Vec3, n: Vec3, w: f32) {
    let right = Vec3::Y.cross(n).normalize();
    let o = c + n * 0.15;
    m.cube(f.base * Mat4::from_translation(o) * Mat4::from_quat(Quat::from_rotation_arc(Vec3::Z, n)), Vec3::new(w * 0.5, 0.1, 0.12), rgb(WOOD));
    let mk = m.mark();
    for k in 0..5 {
        let x = (k as f32 / 4.0 - 0.5) * w * 0.85;
        let col = PASTELS[(k + (f.seed * 7.0) as usize) % 6];
        m.blob(f.base * Mat4::from_translation(o + right * x + Vec3::Y * 0.16), Vec3::new(0.11, 0.09, 0.11), 2, 5, rgb(col));
    }
    m.sway_since(mk, |_| 0.15);
}

fn leaf_c(style: &BiomeStyle, k: usize, f: &Frame) -> [f32; 3] {
    shade(style.leaves[k % 3], 0.9 + 0.2 * f.r(1.0))
}

// ------------------------------------------------------------------ vegetation

fn tree(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let trunk = rgb(style.trunk);
    let b = biome_of(style);
    m.frustum(f.at(0.0, 0.9, 0.0), 0.28, 0.16, 0.9, 5, trunk, trunk);
    m.beam(f.p(0.0, 1.4, 0.0), f.p(0.55, 2.2, 0.1), 0.16, trunk);
    let ci = if b == Biome::Autumn { p.variant as usize % 3 } else { p.variant as usize % 2 };
    let c = leaf_c(style, ci, f);
    let mk = m.mark();
    match p.variant % 3 {
        1 => {
            m.blob(f.at(0.0, 3.3, 0.0), Vec3::new(1.25, 1.9, 1.25), 3, 7, rgb(c));
            m.blob(f.at(0.35, 4.7, 0.1), Vec3::new(0.7, 0.75, 0.7), 2, 6, rgb(shade(c, 1.08)));
        }
        _ => {
            m.blob(f.at(0.0, 2.9, 0.0), Vec3::new(1.6, 1.35, 1.6), 3, 7, rgb(c));
            m.blob(f.at(0.55, 3.75, 0.2), Vec3::new(1.05, 0.9, 1.05), 3, 6, rgb(shade(c, 1.08)));
            m.blob(f.at(-0.7, 2.5, -0.45), Vec3::new(0.95, 0.8, 0.95), 2, 6, rgb(shade(c, 0.94)));
        }
    }
    if p.variant % 3 == 2 && b != Biome::Autumn {
        // fruit / blossom dots
        let fc = if b == Biome::Coast { [1.0, 0.72, 0.3] } else { [0.92, 0.3, 0.3] };
        for k in 0..6 {
            let a = k as f32 * 2.1 + f.seed;
            let v = Vec3::new(a.cos() * 1.45, 2.6 + (k % 3) as f32 * 0.5, a.sin() * 1.45);
            m.blob(f.at(v.x, v.y, v.z), Vec3::splat(0.14), 2, 4, rgb(fc));
        }
    }
    sway_up(m, mk, f.pos.y, 1.8 * f.s, 3.5 * f.s, 0.55);
    if b == Biome::Autumn {
        // fallen leaves
        m.disc(f.p(0.2, 0.04, 0.1), Vec3::Y, 1.7 * f.s, 9, rgb(shade(c, 0.85)));
    }
}

fn birch(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let b = biome_of(style);
    let bark = rgb([0.94, 0.93, 0.88]);
    m.frustum(f.at(0.0, 1.6, 0.0), 0.15, 0.09, 1.6, 6, bark, bark);
    for k in 0..4 {
        let y = 0.5 + k as f32 * 0.65 + f.r(k as f32) * 0.2;
        m.cube(f.atr(0.0, y, 0.0, Quat::from_rotation_y(k as f32 * 1.3)), Vec3::new(0.15 - y * 0.015, 0.035, 0.05), rgb(DARK));
    }
    let c = match b {
        Biome::Autumn => [0.98, 0.78, 0.25],
        Biome::Snow => [0.85, 0.75, 0.35],
        _ => mix(style.leaves[1], [0.75, 0.85, 0.35], 0.4),
    };
    let mk = m.mark();
    m.blob(f.at(0.0, 3.4, 0.0), Vec3::new(0.95, 1.4, 0.95), 3, 6, rgb(c));
    m.blob(f.at(0.3, 2.6, 0.25), Vec3::new(0.7, 0.85, 0.7), 2, 6, rgb(shade(c, 0.92)));
    if p.variant % 2 == 0 {
        m.blob(f.at(-0.3, 4.4, -0.1), Vec3::new(0.5, 0.65, 0.5), 2, 5, rgb(shade(c, 1.08)));
    }
    sway_up(m, mk, f.pos.y, 1.8 * f.s, 3.2 * f.s, 0.7);
}

fn blossom(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let trunk = rgb([0.42, 0.28, 0.24]);
    m.frustum(f.at(0.0, 0.8, 0.0), 0.24, 0.16, 0.8, 5, trunk, trunk);
    m.beam(f.p(0.0, 1.4, 0.0), f.p(0.9, 2.4, 0.2), 0.16, trunk);
    m.beam(f.p(0.0, 1.4, 0.0), f.p(-0.8, 2.3, -0.3), 0.15, trunk);
    let pinks = [[0.99, 0.78, 0.85], [0.97, 0.66, 0.78], [1.0, 0.88, 0.92]];
    let mk = m.mark();
    for (k, (x, y, z, r)) in [(0.0f32, 2.9f32, 0.0f32, 1.4f32), (1.0, 2.6, 0.3, 1.0), (-0.9, 2.5, -0.3, 1.0), (0.2, 3.6, -0.2, 0.9)].iter().enumerate() {
        let c = pinks[(k + p.variant as usize) % 3];
        m.blob(f.at(*x, *y, *z), Vec3::new(*r, r * 0.8, *r), 3, 7, rgb(c));
    }
    sway_up(m, mk, f.pos.y, 1.6 * f.s, 3.0 * f.s, 0.55);
    m.disc(f.p(0.1, 0.04, 0.0), Vec3::Y, 1.9 * f.s, 9, rgb([0.98, 0.8, 0.86]));
}

fn pine(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let trunk = rgb(style.trunk);
    let snowy = biome_of(style) == Biome::Snow;
    m.frustum(f.at(0.0, 0.6, 0.0), 0.22, 0.16, 0.6, 5, trunk, trunk);
    let c = shade(style.leaves[0], 0.85 + 0.1 * (p.variant % 3) as f32);
    let mk = m.mark();
    let tiers: &[(f32, f32)] = if p.variant % 2 == 0 { &[(1.6, 1.5), (2.7, 1.15), (3.7, 0.8)] } else { &[(1.5, 1.3), (2.4, 1.05), (3.25, 0.82), (4.05, 0.55)] };
    for (i, (y, r)) in tiers.iter().enumerate() {
        let col = shade(c, 1.0 + i as f32 * 0.06);
        m.frustum(f.at(0.0, *y, 0.0), *r, 0.0, 0.75, 7, rgb(col), rgb(shade(col, 0.7)));
        if snowy {
            m.frustum(f.at(0.0, y + 0.42, 0.0), r * 0.55, 0.0, 0.34, 7, rgb(SNOW), rgb(SNOW));
        }
    }
    sway_up(m, mk, f.pos.y, 1.0 * f.s, 3.5 * f.s, 0.4);
}

fn palm(m: &mut MeshBuilder, f: &Frame, style: &BiomeStyle) {
    let trunk = rgb(style.trunk);
    let mut prev = Vec3::ZERO;
    for i in 0..5 {
        let y = i as f32 * 0.9;
        let p2 = Vec3::new((y * 0.15).powi(2), y + 0.9, 0.0);
        let mid = (prev + p2) * 0.5;
        m.frustum(f.base * Mat4::from_translation(mid), 0.22 - i as f32 * 0.02, 0.2 - i as f32 * 0.02, 0.45, 5, if i % 2 == 0 { trunk } else { rgb(shade(style.trunk, 0.85)) }, trunk);
        prev = p2;
    }
    let leaf = style.leaves[1];
    let mk = m.mark();
    for k in 0..7 {
        let a = k as f32 / 7.0 * TAU + f.seed;
        let rot = Quat::from_rotation_y(a) * Quat::from_rotation_z(-0.45);
        m.box_tapered(f.base * Mat4::from_translation(prev + Vec3::new(a.cos() * 1.1, 0.0, -a.sin() * 1.1)) * Mat4::from_quat(rot), Vec3::new(1.3, 0.04, 0.32), 0.35, rgb(shade(leaf, 0.9 + 0.05 * (k % 3) as f32)));
    }
    let top_y = f.pos.y + prev.y * f.s;
    m.sway_since(mk, move |q| ((q.y - top_y + 1.0) / 1.5).clamp(0.0, 1.0) * 0.6);
    for k in 0..3 {
        let a = k as f32 * 2.1;
        m.blob(f.base * Mat4::from_translation(prev + Vec3::new(a.cos() * 0.25, -0.25, a.sin() * 0.25)), Vec3::splat(0.17), 2, 5, rgb([0.55, 0.4, 0.25]));
    }
}

fn bush(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let b = biome_of(style);
    let base = if b == Biome::Snow { [0.35, 0.5, 0.42] } else { style.leaves[p.variant as usize % 2] };
    let c = rgb(shade(base, 0.85));
    let mk = m.mark();
    m.blob(f.at(0.0, 0.45, 0.0), Vec3::new(0.9, 0.6, 0.9), 2, 6, c);
    m.blob(f.at(0.55, 0.35, 0.3), Vec3::new(0.6, 0.45, 0.6), 2, 5, rgb(shade(base, 0.95)));
    m.blob(f.at(-0.4, 0.3, 0.4), Vec3::new(0.5, 0.38, 0.5), 2, 5, rgb(shade(base, 0.8)));
    if b == Biome::Snow {
        m.blob(f.at(0.0, 0.85, 0.0), Vec3::new(0.65, 0.22, 0.65), 2, 6, rgb(SNOW));
    }
    sway_up(m, mk, f.pos.y, 0.2 * f.s, 0.8 * f.s, 0.25);
    if p.variant % 3 == 1 && b != Biome::Desert {
        let berry = if b == Biome::Snow { [0.85, 0.2, 0.25] } else { [[0.9, 0.25, 0.35], [0.45, 0.35, 0.8], [0.98, 0.95, 0.98]][(f.seed * 3.0) as usize % 3] };
        for k in 0..7 {
            let a = k as f32 * 2.4 + f.seed;
            m.blob(f.at(a.cos() * 0.75, 0.45 + (k % 3) as f32 * 0.15, a.sin() * 0.75), Vec3::splat(0.09), 1, 4, rgb(berry));
        }
    }
}

fn rock(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let b = biome_of(style);
    let c = shade(style.cliff, 0.85 + 0.1 * (p.variant % 3) as f32);
    m.box_tapered(f.base * Mat4::from_translation(Vec3::new(0.0, 0.35, 0.0)) * Mat4::from_rotation_x(0.2), Vec3::new(0.9, 0.6, 0.75), 0.6, rgb(c));
    m.box_tapered(f.base * Mat4::from_translation(Vec3::new(0.5, 0.25, 0.4)) * Mat4::from_rotation_z(0.3), Vec3::new(0.45, 0.35, 0.4), 0.5, rgb(shade(style.cliff, 1.05)));
    match b {
        Biome::Meadow | Biome::Autumn | Biome::Coast if p.variant % 2 == 0 => {
            m.blob(f.at(-0.05, 0.92, 0.05), Vec3::new(0.6, 0.12, 0.5), 2, 6, rgb([0.42, 0.62, 0.3]));
        }
        Biome::Snow => {
            m.blob(f.at(-0.05, 0.9, 0.05), Vec3::new(0.6, 0.14, 0.5), 2, 6, rgb(SNOW));
        }
        _ => {}
    }
}

fn cactus(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let c = rgb([0.36, 0.62, 0.38]);
    let c2 = rgb([0.42, 0.7, 0.42]);
    m.frustum(f.at(0.0, 1.1, 0.0), 0.3, 0.27, 1.1, 8, c, c);
    m.blob(f.at(0.0, 2.2, 0.0), Vec3::new(0.27, 0.22, 0.27), 2, 8, c2);
    for (sx, h) in [(1.0f32, 1.1f32), (-1.0, 1.5)] {
        if sx < 0.0 && p.variant % 2 == 1 {
            continue;
        }
        m.cylinder(f.at(sx * 0.42, h, 0.0) * Mat4::from_rotation_z(FRAC_PI_2), 0.15, 0.16, 6, c, c);
        m.cylinder(f.at(sx * 0.58, h + 0.38, 0.0), 0.15, 0.38, 6, c, c);
        m.blob(f.at(sx * 0.58, h + 0.76, 0.0), Vec3::splat(0.15), 2, 6, c2);
    }
    if p.variant % 3 != 2 {
        m.blob(f.at(0.0, 2.43, 0.0), Vec3::new(0.12, 0.08, 0.12), 2, 5, rgb(style.leaves[2]));
    }
}

fn flowers(m: &mut MeshBuilder, f: &Frame, style: &BiomeStyle) {
    let mk = m.mark();
    for k in 0..7 {
        let a = k as f32 * 1.7 + f.seed;
        let r = 0.25 + (k as f32 * 0.37) % 0.7;
        let pos = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
        let c = [style.leaves[2], [1.0, 0.95, 0.6], [0.95, 0.95, 1.0], [0.75, 0.6, 0.95]][k % 4];
        m.tri(f.p_v(pos - Vec3::X * 0.03), f.p_v(pos + Vec3::X * 0.03), f.p_v(pos + Vec3::Y * 0.34), rgb([0.3, 0.55, 0.25]));
        m.blob(f.base * Mat4::from_translation(pos + Vec3::Y * 0.36), Vec3::new(0.1, 0.06, 0.1), 2, 5, rgb(c));
        m.blob(f.base * Mat4::from_translation(pos + Vec3::Y * 0.4), Vec3::splat(0.04), 1, 4, rgb([0.98, 0.85, 0.3]));
    }
    sway_up(m, mk, f.pos.y, 0.0, 0.4 * f.s, 0.6);
}

fn grass(m: &mut MeshBuilder, f: &Frame, style: &BiomeStyle) {
    let b = biome_of(style);
    let g = match b {
        Biome::Autumn => [0.72, 0.62, 0.3],
        Biome::Desert => [0.78, 0.7, 0.42],
        Biome::Snow => [0.55, 0.62, 0.5],
        _ => shade(style.ground_a, 0.85),
    };
    let mk = m.mark();
    for k in 0..6 {
        let a = k as f32 * 1.1 + f.seed;
        let r = 0.1 + (k % 3) as f32 * 0.12;
        let p0 = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
        let lean = Vec3::new(a.cos(), 0.0, a.sin()) * 0.18;
        let h = 0.45 + (k % 3) as f32 * 0.15;
        let side = Vec3::new(-a.sin(), 0.0, a.cos()) * 0.05;
        m.tri(f.p_v(p0 - side), f.p_v(p0 + side), f.p_v(p0 + lean + Vec3::Y * h), rgb(shade(g, 0.9 + 0.08 * (k % 3) as f32)));
    }
    sway_up(m, mk, f.pos.y, 0.0, 0.6 * f.s, 0.8);
}

fn mushroom(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let caps = [[0.9, 0.25, 0.22], [0.95, 0.6, 0.3], [0.85, 0.7, 0.55]];
    let cap = caps[p.variant as usize % 3];
    for (k, (x, z, s)) in [(0.0f32, 0.0f32, 1.0f32), (0.35, 0.2, 0.65), (-0.25, 0.3, 0.5)].iter().enumerate() {
        m.frustum(f.at(*x, 0.15 * s, *z), 0.07 * s, 0.06 * s, 0.15 * s, 6, rgb(CREAM), rgb(CREAM));
        m.blob(f.at(*x, 0.3 * s, *z), Vec3::new(0.22 * s, 0.13 * s, 0.22 * s), 2, 7, rgb(cap));
        if p.variant % 3 == 0 && k == 0 {
            for d in 0..4 {
                let a = d as f32 * 1.6;
                m.blob(f.at(x + a.cos() * 0.12 * s, 0.39 * s, z + a.sin() * 0.12 * s), Vec3::splat(0.035 * s), 1, 4, rgb(WHITE));
            }
        }
    }
}

fn flower_field(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let palettes: [[[f32; 3]; 2]; 4] = [
        [[0.68, 0.55, 0.9], [0.6, 0.45, 0.85]],  // lavender
        [[0.95, 0.35, 0.4], [0.98, 0.85, 0.35]], // tulips
        [[0.98, 0.8, 0.25], [0.95, 0.7, 0.2]],   // sunflowers
        [[0.97, 0.97, 1.0], [0.98, 0.65, 0.75]], // daisies & pinks
    ];
    let pal = palettes[p.variant as usize % 4];
    // soft soil rows
    for row in 0..5 {
        let z = (row as f32 - 2.0) * 1.2;
        m.cube(f.at(0.0, 0.02, z), Vec3::new(4.2, 0.04, 0.4), rgb(shade(style.ground_a, 0.82)));
    }
    let mk = m.mark();
    for row in 0..5 {
        for k in 0..12 {
            let x = (k as f32 - 5.5) * 0.7 + (f.r(k as f32 + row as f32 * 13.0) - 0.5) * 0.2;
            let z = (row as f32 - 2.0) * 1.2;
            let c = pal[(k + row) % 2];
            let h = 0.45 + f.r(k as f32 * 3.1 + row as f32) * 0.2;
            let b = Vec3::new(x, 0.0, z);
            m.tri(f.p_v(b - Vec3::X * 0.06), f.p_v(b + Vec3::X * 0.06), f.p_v(b + Vec3::Y * h), rgb([0.32, 0.56, 0.26]));
            let t = b + Vec3::Y * h;
            let s = 0.13;
            let pts = [t + Vec3::new(-s, 0.0, 0.0), t + Vec3::new(0.0, 0.0, -s), t + Vec3::new(s, 0.0, 0.0), t + Vec3::new(0.0, 0.0, s)];
            let tip = t + Vec3::Y * s * 1.2;
            for i in 0..4 {
                m.tri(f.p_v(pts[i]), f.p_v(pts[(i + 1) % 4]), f.p_v(tip), rgb(shade(c, 0.92 + 0.08 * (i % 2) as f32)));
            }
        }
    }
    sway_up(m, mk, f.pos.y, 0.0, 0.6 * f.s, 0.5);
}

// ------------------------------------------------------------------ farm & village

fn plinth(m: &mut MeshBuilder, f: &Frame, hx: f32, hz: f32) {
    m.cube(f.at(0.0, -1.0, 0.0), Vec3::new(hx + 0.15, 1.2, hz + 0.15), rgb(STONE));
}

fn cottage(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let b = biome_of(style);
    let walls = [[0.98, 0.94, 0.84], [0.96, 0.88, 0.8], [0.9, 0.92, 0.96]][p.variant as usize % 3];
    let roof = match b {
        Biome::Coast => [0.35, 0.55, 0.8],
        Biome::Autumn => [0.65, 0.35, 0.25],
        Biome::Snow => SNOW,
        _ => [[0.85, 0.42, 0.35], [0.82, 0.68, 0.42], [0.45, 0.6, 0.82]][p.variant as usize % 3],
    };
    let (hx, hz, h) = (3.0, 2.3, 2.6);
    plinth(m, f, hx, hz);
    m.cube(f.at(0.0, h * 0.5, 0.0), Vec3::new(hx, h * 0.5, hz), rgb(walls));
    // timber corners
    for (sx, sz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        m.cube(f.at(sx * hx, h * 0.5, sz * hz), Vec3::new(0.12, h * 0.5, 0.12), rgb(WOOD_D));
    }
    gable(m, f, hx, hz, h, h + 2.0, 0.45, roof, walls);
    // chimney
    m.cube(f.at(1.6, h + 2.0, 0.9), Vec3::new(0.3, 0.9, 0.3), rgb([0.75, 0.45, 0.38]));
    m.cube(f.at(1.6, h + 2.9, 0.9), Vec3::new(0.36, 0.08, 0.36), rgb(STONE));
    // door + windows front (-Z) and back
    let door = PASTELS[(p.variant as usize + 1) % 6];
    m.cube(f.at(0.0, 0.95, -hz - 0.03), Vec3::new(0.5, 0.95, 0.05), rgb(door));
    m.blob(f.at(0.32, 0.95, -hz - 0.1), Vec3::splat(0.06), 1, 4, rgb([0.95, 0.85, 0.4]));
    m.cube(f.at(0.0, 2.0, -hz - 0.2), Vec3::new(0.75, 0.05, 0.3), rgb(roof));
    let sh = Some(shade(door, 0.9));
    for sx in [-1.0f32, 1.0] {
        window(m, f, Vec3::new(sx * 1.8, 1.45, -hz), Vec3::NEG_Z, 0.8, 0.8, WHITE, [0.55, 0.72, 0.85], sh);
        flower_box(m, f, Vec3::new(sx * 1.8, 0.9, -hz), Vec3::NEG_Z, 1.0);
        window(m, f, Vec3::new(sx * 1.4, 1.45, hz), Vec3::Z, 0.8, 0.8, WHITE, [0.55, 0.72, 0.85], sh);
    }
    window(m, f, Vec3::new(hx, 1.45, 0.0), Vec3::X, 0.7, 0.7, WHITE, [0.55, 0.72, 0.85], None);
    window(m, f, Vec3::new(-hx, 3.3, 0.0), Vec3::NEG_X, 0.6, 0.6, WHITE, [0.55, 0.72, 0.85], None);
    // front path stones
    for k in 0..3 {
        m.cube(f.at(0.1 * (k % 2) as f32, 0.03, -hz - 0.9 - k as f32 * 0.8), Vec3::new(0.35, 0.05, 0.3), rgb(STONE));
    }
}

fn barn(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let red = [[0.78, 0.3, 0.26], [0.85, 0.55, 0.3], [0.55, 0.6, 0.75]][p.variant as usize % 3];
    let (hx, hz, h) = (3.2, 4.0, 3.2);
    plinth(m, f, hx, hz);
    m.cube(f.at(0.0, h * 0.5, 0.0), Vec3::new(hx, h * 0.5, hz), rgb(red));
    // gambrel roof (ridge along Z)
    let roof = rgb([0.4, 0.38, 0.42]);
    let prof = [(-hx - 0.4, h - 0.2), (-hx * 0.65, h + 1.6), (0.0, h + 2.4), (hx * 0.65, h + 1.6), (hx + 0.4, h - 0.2)];
    for k in 0..4 {
        let (a, b) = (prof[k], prof[k + 1]);
        m.quad(f.p(a.0, a.1, -hz - 0.3), f.p(b.0, b.1, -hz - 0.3), f.p(b.0, b.1, hz + 0.3), f.p(a.0, a.1, hz + 0.3), roof);
    }
    for sz in [-1.0f32, 1.0] {
        let z = sz * hz;
        m.poly(&[f.p(-hx, h, z), f.p(-hx * 0.65, h + 1.6, z), f.p(0.0, h + 2.4, z), f.p(hx * 0.65, h + 1.6, z), f.p(hx, h, z)], rgb(red));
        // big doors with white X trim
        let zz = z + sz * 0.04;
        m.quad(f.p(-1.4, 0.0, zz), f.p(1.4, 0.0, zz), f.p(1.4, 2.6, zz), f.p(-1.4, 2.6, zz), rgb(shade(red, 0.85)));
        let zt = z + sz * 0.07;
        for (a, b) in [((-1.4, 0.0), (1.4, 2.6)), ((1.4, 0.0), (-1.4, 2.6)), ((-1.4, 2.6), (1.4, 2.6)), ((-1.4, 0.0), (-1.4, 2.6)), ((1.4, 0.0), (1.4, 2.6)), ((0.0, 0.0), (0.0, 2.6))] {
            m.beam(f.p(a.0, a.1, zt), f.p(b.0, b.1, zt), 0.14, rgb(WHITE));
        }
        window(m, f, Vec3::new(0.0, h + 1.0, z), Vec3::Z * sz, 0.8, 0.7, WHITE, [0.35, 0.3, 0.3], None);
    }
    // weathervane
    m.beam(f.p(0.0, h + 2.4, 0.0), f.p(0.0, h + 3.4, 0.0), 0.06, rgb(DARK));
    m.tri(f.p(0.0, h + 3.3, -0.4), f.p(0.0, h + 3.3, 0.4), f.p(0.0, h + 3.6, 0.3), rgb(DARK));
}

fn windmill(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    plinth(m, f, 2.0, 2.0);
    let body = [CREAM, [0.92, 0.88, 0.82], WHITE][p.variant as usize % 3];
    m.frustum(f.at(0.0, 3.5, 0.0), 2.1, 1.35, 3.5, 8, rgb(body), rgb(body));
    m.frustum(f.at(0.0, 7.6, 0.0), 1.55, 0.2, 0.9, 8, rgb([0.75, 0.38, 0.3]), rgb([0.75, 0.38, 0.3]));
    m.cube(f.at(0.0, 1.0, -1.95), Vec3::new(0.5, 1.0, 0.1), rgb(WOOD));
    window(m, f, Vec3::new(0.0, 4.5, -1.6), Vec3::NEG_Z, 0.6, 0.7, WOOD_D, [0.55, 0.72, 0.85], None);
    // sails
    let hub = Vec3::new(0.0, 7.2, -1.7);
    m.cylinder(f.at(hub.x, hub.y, hub.z) * Mat4::from_rotation_x(FRAC_PI_2), 0.28, 0.25, 8, rgb(WOOD_D), rgb(WOOD_D));
    let spin = p.variant as f32 * 0.4 + 0.3;
    for k in 0..4 {
        let a = spin + k as f32 * FRAC_PI_2;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        let side = Vec3::new(-a.sin(), a.cos(), 0.0);
        let h = hub - Vec3::Z * 0.3;
        m.beam(f.p_v(h), f.p_v(h + d * 5.0), 0.16, rgb(WOOD_D));
        let q = [h + d * 1.0, h + d * 4.8, h + d * 4.8 + side * 1.0, h + d * 1.0 + side * 1.0];
        m.quad(f.p_v(q[0]), f.p_v(q[1]), f.p_v(q[2]), f.p_v(q[3]), rgb(WHITE));
        for t in [0.33f32, 0.66] {
            m.beam(f.p_v(h + d * (1.0 + 3.8 * t) - Vec3::Z * 0.05), f.p_v(h + d * (1.0 + 3.8 * t) + side * 1.0 - Vec3::Z * 0.05), 0.06, rgb(WOOD_D));
        }
    }
}

fn fence(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let b = biome_of(style);
    let picket = b == Biome::Coast || (b == Biome::Meadow && p.variant % 2 == 1);
    let col = if picket { WHITE } else if b == Biome::Snow { [0.5, 0.38, 0.28] } else { WOOD };
    for x in [-2.0f32, 0.0, 2.0] {
        m.cube(f.at(x, 0.55, 0.0), Vec3::new(0.08, 0.55, 0.08), rgb(shade(col, 0.9)));
    }
    if picket {
        m.cube(f.at(0.0, 0.35, 0.0), Vec3::new(2.0, 0.05, 0.03), rgb(col));
        m.cube(f.at(0.0, 0.75, 0.0), Vec3::new(2.0, 0.05, 0.03), rgb(col));
        for k in 0..12 {
            let x = -1.85 + k as f32 * 0.335;
            let z = -0.05;
            m.quad(f.p(x - 0.07, 0.05, z), f.p(x + 0.07, 0.05, z), f.p(x + 0.07, 0.85, z), f.p(x - 0.07, 0.85, z), rgb(col));
            m.tri(f.p(x - 0.07, 0.85, z), f.p(x + 0.07, 0.85, z), f.p(x, 0.97, z), rgb(col));
        }
    } else {
        for y in [0.45f32, 0.85] {
            m.beam(f.p(-2.05, y, 0.0), f.p(2.05, y + 0.04, 0.0), 0.11, rgb(col));
        }
        if b == Biome::Snow {
            m.cube(f.at(0.0, 0.92, 0.0), Vec3::new(2.0, 0.04, 0.07), rgb(SNOW));
        }
    }
}

fn hay_bale(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let c = [0.93, 0.80, 0.45];
    if p.variant % 2 == 0 {
        m.cylinder(f.at(0.0, 0.5, 0.0) * Mat4::from_rotation_z(FRAC_PI_2), 0.5, 0.95, 9, rgb(c), rgb([0.85, 0.72, 0.38]));
        m.cylinder(f.at(0.0, 0.5, 0.0) * Mat4::from_rotation_z(FRAC_PI_2), 0.505, 0.06, 9, rgb([0.8, 0.5, 0.3]), rgb([0.8, 0.5, 0.3]));
    } else {
        m.bevel_box(f.at(0.0, 0.42, 0.0), Vec3::new(1.0, 0.42, 0.5), 0.08, rgb(c));
        for x in [-0.5f32, 0.5] {
            m.cube(f.at(x, 0.42, 0.0), Vec3::new(0.03, 0.43, 0.51), rgb([0.8, 0.5, 0.3]));
        }
    }
}

fn picnic(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let c = PASTELS[p.variant as usize % 6];
    for i in 0..4 {
        for j in 0..4 {
            let col = if (i + j) % 2 == 0 { c } else { WHITE };
            let (x, z) = (-0.9 + i as f32 * 0.6, -0.9 + j as f32 * 0.6);
            m.quad(f.p(x - 0.3, 0.04, z - 0.3), f.p(x + 0.3, 0.04, z - 0.3), f.p(x + 0.3, 0.04, z + 0.3), f.p(x - 0.3, 0.04, z + 0.3), rgb(col));
        }
    }
    // basket with handle
    m.bevel_box(f.at(0.5, 0.2, 0.3), Vec3::new(0.3, 0.17, 0.22), 0.04, rgb([0.78, 0.55, 0.32]));
    m.beam(f.p(0.25, 0.37, 0.3), f.p(0.5, 0.55, 0.3), 0.04, rgb(WOOD_D));
    m.beam(f.p(0.5, 0.55, 0.3), f.p(0.75, 0.37, 0.3), 0.04, rgb(WOOD_D));
    // plates + cake
    for (x, z) in [(-0.5f32, -0.4f32), (-0.3, 0.45)] {
        m.cylinder(f.at(x, 0.06, z), 0.18, 0.015, 8, rgb(WHITE), rgb(WHITE));
    }
    m.cylinder(f.at(-0.1, 0.14, 0.0), 0.2, 0.09, 8, rgb([0.98, 0.85, 0.75]), rgb([0.98, 0.6, 0.7]));
    m.blob(f.at(-0.1, 0.27, 0.0), Vec3::splat(0.05), 1, 4, rgb([0.9, 0.2, 0.25]));
}

fn camper(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let c = PASTELS[(p.variant as usize + 2) % 6];
    m.bevel_box(f.at(0.0, 1.35, 0.0), Vec3::new(1.05, 0.95, 2.3), 0.3, rgb(CREAM));
    m.cube(f.at(0.0, 0.95, 0.0), Vec3::new(1.07, 0.5, 2.31), rgb(c));
    m.cube(f.at(0.0, 1.5, 0.0), Vec3::new(1.075, 0.06, 2.315), rgb(shade(c, 0.85)));
    for z in [-1.4f32, 1.4] {
        for sx in [-1.0f32, 1.0] {
            m.cylinder(f.at(sx * 0.92, 0.38, z) * Mat4::from_rotation_z(FRAC_PI_2), 0.38, 0.14, 10, rgb(DARK), rgb([0.85, 0.85, 0.85]));
        }
    }
    // windscreen + side windows
    m.quad(f.p(-0.9, 1.6, -2.32), f.p(0.9, 1.6, -2.32), f.p(0.85, 2.05, -2.2), f.p(-0.85, 2.05, -2.2), rgb([0.55, 0.72, 0.85]));
    for z in [-0.6f32, 0.9] {
        window(m, f, Vec3::new(1.05, 1.8, z), Vec3::X, 0.9, 0.45, WHITE, [0.55, 0.72, 0.85], None);
    }
    // striped awning + chairs
    for k in 0..6 {
        let z0 = -1.5 + k as f32 * 0.5;
        let col = if k % 2 == 0 { c } else { WHITE };
        m.quad(f.p(1.08, 2.15, z0), f.p(1.08, 2.15, z0 + 0.5), f.p(2.4, 1.85, z0 + 0.5), f.p(2.4, 1.85, z0), rgb(col));
    }
    for z in [-1.5f32, 1.5] {
        m.beam(f.p(2.4, 0.0, z), f.p(2.4, 1.85, z), 0.05, rgb(DARK));
    }
    for (k, z) in [-0.5f32, 0.6].iter().enumerate() {
        let cc = PASTELS[(p.variant as usize + 4 + k) % 6];
        m.cube(f.at(2.0, 0.45, *z), Vec3::new(0.25, 0.04, 0.25), rgb(cc));
        m.cube(f.atr(2.22, 0.75, *z, Quat::from_rotation_z(0.2)), Vec3::new(0.03, 0.3, 0.25), rgb(cc));
        for (dx, dz) in [(-0.2f32, -0.2f32), (0.2, -0.2), (0.2, 0.2), (-0.2, 0.2)] {
            m.beam(f.p(2.0 + dx, 0.0, z + dz), f.p(2.0 + dx * 0.6, 0.45, z + dz * 0.6), 0.03, rgb(DARK));
        }
    }
}

fn tent(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let c = PASTELS[p.variant as usize % 6];
    // A-frame ridge tent with a striped side
    let (hl, hw, h) = (1.4, 1.2, 1.5);
    for sx in [-1.0f32, 1.0] {
        for k in 0..4 {
            let z0 = -hl + k as f32 * hl * 0.5;
            let col = if k % 2 == 0 { c } else { WHITE };
            m.quad(f.p(sx * hw, 0.0, z0), f.p(sx * hw, 0.0, z0 + hl * 0.5), f.p(0.0, h, z0 + hl * 0.5), f.p(0.0, h, z0), rgb(col));
        }
    }
    m.tri(f.p(-hw, 0.0, hl), f.p(hw, 0.0, hl), f.p(0.0, h, hl), rgb(shade(c, 0.9)));
    m.tri(f.p(-hw, 0.0, -hl), f.p(hw, 0.0, -hl), f.p(0.0, h, -hl), rgb(shade(c, 0.9)));
    m.tri(f.p(-0.45, 0.0, -hl - 0.01), f.p(0.45, 0.0, -hl - 0.01), f.p(0.0, 1.0, -hl - 0.01), rgb(DARK));
    m.beam(f.p(0.0, 0.0, -hl - 0.05), f.p(0.0, h + 0.3, -hl - 0.05), 0.05, rgb(WOOD_D));
    let mk = m.mark();
    m.tri(f.p(0.0, h + 0.3, -hl - 0.05), f.p(0.0, h + 0.05, -hl - 0.05), f.p(0.45, h + 0.2, -hl - 0.05), rgb(PASTELS[(p.variant as usize + 3) % 6]));
    sway_up(m, mk, f.pos.y, h, 0.5, 0.7);
}

fn hay_wagon(m: &mut MeshBuilder, f: &Frame, _p: &Prop) {
    m.cube(f.at(0.0, 0.85, 0.0), Vec3::new(1.0, 0.1, 1.8), rgb(WOOD));
    for sx in [-1.0f32, 1.0] {
        m.cube(f.at(sx * 1.0, 1.1, 0.0), Vec3::new(0.05, 0.25, 1.8), rgb(WOOD_D));
        for z in [-1.2f32, 1.2] {
            m.cylinder(f.at(sx * 1.08, 0.5, z) * Mat4::from_rotation_z(FRAC_PI_2), 0.5, 0.06, 10, rgb(WOOD_D), rgb([0.8, 0.3, 0.25]));
        }
    }
    m.bevel_box(f.at(0.0, 1.45, 0.0), Vec3::new(0.95, 0.5, 1.7), 0.25, rgb([0.95, 0.82, 0.45]));
    m.blob(f.at(0.0, 1.95, 0.2), Vec3::new(0.8, 0.35, 1.2), 2, 7, rgb([0.97, 0.86, 0.5]));
    m.beam(f.p(0.0, 0.8, -1.8), f.p(0.0, 0.4, -3.0), 0.08, rgb(WOOD_D));
    for k in 0..3 {
        m.blob(f.at(-0.4 + k as f32 * 0.4, 2.2, -0.6 + k as f32 * 0.5), Vec3::new(0.25, 0.12, 0.35), 1, 5, rgb([0.95, 0.6, 0.25]));
    }
}

fn beehive(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    for (k, x) in [-0.8f32, 0.8].iter().enumerate() {
        if k == 1 && p.variant % 2 == 1 {
            continue;
        }
        m.cube(f.at(*x, 0.15, 0.0), Vec3::new(0.35, 0.15, 0.35), rgb(WOOD_D));
        for j in 0..3 {
            let col = if j % 2 == 0 { WHITE } else { [0.98, 0.85, 0.4] };
            m.cube(f.at(*x, 0.42 + j as f32 * 0.26, 0.0), Vec3::new(0.32, 0.12, 0.32), rgb(col));
        }
        m.box_tapered(f.at(*x, 1.22, 0.0), Vec3::new(0.4, 0.1, 0.4), 0.6, rgb([0.75, 0.42, 0.32]));
        m.cube(f.at(*x, 0.34, -0.33), Vec3::new(0.12, 0.02, 0.02), rgb(DARK));
    }
    for k in 0..4 {
        let a = k as f32 * 1.7 + f.seed;
        let v = Vec3::new(a.cos() * 0.9, 1.0 + (k % 2) as f32 * 0.4, a.sin() * 0.6);
        m.blob(f.at(v.x, v.y, v.z), Vec3::new(0.07, 0.05, 0.05), 1, 4, rgb([0.98, 0.8, 0.2]));
    }
}

fn pumpkin(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let n = 2 + p.variant as usize % 3;
    for k in 0..n {
        let a = k as f32 * 2.3 + f.seed;
        let r = if k == 0 { 0.0 } else { 0.7 };
        let s = if k == 0 { 1.0 } else { 0.7 };
        let (x, z) = (a.cos() * r, a.sin() * r);
        let col = if k == 2 { [0.95, 0.85, 0.6] } else { [0.95, 0.52, 0.18] };
        for j in 0..4 {
            let ang = j as f32 * PI / 4.0;
            m.blob(f.atr(x, 0.3 * s, z, Quat::from_rotation_y(ang)), Vec3::new(0.42 * s, 0.32 * s, 0.22 * s), 2, 6, rgb(shade(col, 0.95 + 0.05 * (j % 2) as f32)));
        }
        m.frustum(f.at(x, 0.66 * s, z), 0.05 * s, 0.035 * s, 0.07 * s, 5, rgb([0.4, 0.5, 0.22]), rgb([0.4, 0.5, 0.22]));
        m.blob(f.atr(x + 0.12 * s, 0.6 * s, z, Quat::from_rotation_y(a)), Vec3::new(0.16 * s, 0.03, 0.1 * s), 1, 5, rgb([0.35, 0.55, 0.25]));
    }
}

fn scarecrow(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    m.beam(f.p(0.0, 0.0, 0.0), f.p(0.0, 2.0, 0.0), 0.1, rgb(WOOD_D));
    m.beam(f.p(-0.9, 1.45, 0.0), f.p(0.9, 1.45, 0.0), 0.08, rgb(WOOD_D));
    let shirt = [[0.75, 0.3, 0.28], [0.35, 0.5, 0.75], [0.45, 0.6, 0.35]][p.variant as usize % 3];
    let mk = m.mark();
    m.box_tapered(f.at(0.0, 1.2, 0.0), Vec3::new(0.38, 0.4, 0.2), 1.2, rgb(shirt));
    m.cube(f.at(0.0, 1.47, 0.0), Vec3::new(0.82, 0.1, 0.15), rgb(shade(shirt, 0.85)));
    for sx in [-1.0f32, 1.0] {
        m.tri(f.p(sx * 0.82, 1.38, 0.0), f.p(sx * 1.05, 1.25, 0.08), f.p(sx * 0.95, 1.5, -0.05), rgb([0.95, 0.85, 0.5]));
    }
    m.sway_since(mk, |_| 0.12);
    m.blob(f.at(0.0, 1.85, 0.0), Vec3::new(0.22, 0.25, 0.22), 2, 7, rgb([0.85, 0.75, 0.55]));
    m.blob(f.at(-0.07, 1.9, -0.2), Vec3::splat(0.03), 1, 4, rgb(DARK));
    m.blob(f.at(0.07, 1.9, -0.2), Vec3::splat(0.03), 1, 4, rgb(DARK));
    m.cylinder(f.at(0.0, 2.03, 0.0), 0.42, 0.02, 9, rgb([0.9, 0.78, 0.45]), rgb([0.9, 0.78, 0.45]));
    m.frustum(f.at(0.0, 2.15, 0.0), 0.22, 0.15, 0.12, 9, rgb([0.9, 0.78, 0.45]), rgb([0.88, 0.75, 0.42]));
    m.cylinder(f.at(0.0, 2.08, 0.0), 0.225, 0.03, 9, rgb([0.8, 0.3, 0.3]), rgb([0.8, 0.3, 0.3]));
    // a crow friend
    m.blob(f.at(0.75, 1.62, 0.0), Vec3::new(0.08, 0.08, 0.13), 2, 5, rgb(DARK));
    m.blob(f.at(0.75, 1.72, -0.1), Vec3::splat(0.06), 1, 5, rgb(DARK));
}

// ------------------------------------------------------------------ critters

fn legs(m: &mut MeshBuilder, f: &Frame, hx: f32, hz: f32, top: f32, col: [f32; 3]) {
    for (sx, sz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        m.beam(f.p(sx * hx, top, sz * hz), f.p(sx * hx, 0.0, sz * hz), 0.1, rgb(col));
    }
}

fn sheep(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let wool = if p.variant % 4 == 3 { [0.45, 0.4, 0.42] } else { [0.97, 0.96, 0.92] };
    legs(m, f, 0.2, 0.3, 0.45, DARK);
    m.blob(f.at(0.0, 0.62, 0.0), Vec3::new(0.42, 0.33, 0.55), 3, 7, rgb(wool));
    for k in 0..4 {
        let a = k as f32 * 1.57 + 0.4;
        m.blob(f.at(a.cos() * 0.25, 0.85, a.sin() * 0.35), Vec3::splat(0.2), 2, 5, rgb(shade(wool, 1.02)));
    }
    let graze = p.variant % 2 == 0;
    let (hy, hz) = if graze { (0.3, -0.65) } else { (0.75, -0.62) };
    m.blob(f.at(0.0, hy, hz), Vec3::new(0.14, 0.15, 0.2), 2, 6, rgb([0.3, 0.28, 0.3]));
    m.blob(f.at(0.0, hy + 0.12, hz + 0.08), Vec3::new(0.16, 0.08, 0.12), 2, 5, rgb(wool));
    for sx in [-1.0f32, 1.0] {
        m.blob(f.at(sx * 0.17, hy + 0.04, hz + 0.06), Vec3::new(0.09, 0.03, 0.05), 1, 4, rgb([0.3, 0.28, 0.3]));
    }
}

fn cow(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let brown = p.variant % 3 == 2;
    let body = if brown { [0.75, 0.52, 0.35] } else { [0.97, 0.96, 0.93] };
    legs(m, f, 0.3, 0.6, 0.7, if brown { [0.6, 0.42, 0.3] } else { [0.95, 0.94, 0.9] });
    m.bevel_box(f.at(0.0, 0.95, 0.0), Vec3::new(0.42, 0.36, 0.8), 0.18, rgb(body));
    if !brown {
        for (x, y, z, r) in [(0.43f32, 1.0f32, 0.2f32, 0.22f32), (-0.43, 0.9, -0.3, 0.25), (0.43, 0.85, -0.4, 0.15), (0.0, 1.32, 0.3, 0.2)] {
            let n = if y > 1.3 { Vec3::Y } else { Vec3::X * x.signum() };
            m.disc(f.p(x, y, z), f.base.transform_vector3(n), r * f.s, 7, rgb(DARK));
        }
    }
    for k in 0..4 {
        m.beam(f.p(-0.2 + (k % 2) as f32 * 0.4, 0.0, 0.62 * if k < 2 { -1.0 } else { 1.0 }), f.p(-0.2 + (k % 2) as f32 * 0.4, 0.05, 0.62 * if k < 2 { -1.0 } else { 1.0 }), 0.13, rgb(DARK));
    }
    let graze = p.variant % 2 == 0;
    let (hy, hz) = if graze { (0.45, -0.95) } else { (1.15, -0.95) };
    m.bevel_box(f.at(0.0, hy, hz), Vec3::new(0.2, 0.22, 0.25), 0.08, rgb(body));
    m.bevel_box(f.at(0.0, hy - 0.1, hz - 0.22), Vec3::new(0.18, 0.12, 0.08), 0.05, rgb([0.95, 0.7, 0.7]));
    for sx in [-1.0f32, 1.0] {
        m.beam(f.p(sx * 0.15, hy + 0.2, hz + 0.05), f.p(sx * 0.3, hy + 0.32, hz), 0.05, rgb(CREAM));
        m.blob(f.at(sx * 0.28, hy + 0.12, hz + 0.1), Vec3::new(0.1, 0.04, 0.06), 1, 4, rgb(body));
    }
    let mk = m.mark();
    m.beam(f.p(0.0, 1.2, 0.8), f.p(0.05, 0.6, 0.9), 0.04, rgb(body));
    sway_up(m, mk, f.pos.y + 1.2 * f.s, -10.0, 1.0, 0.0);
}

fn deer(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let fur = [0.72, 0.5, 0.32];
    legs(m, f, 0.14, 0.35, 0.65, shade(fur, 0.85));
    m.blob(f.at(0.0, 0.85, 0.0), Vec3::new(0.25, 0.25, 0.5), 3, 7, rgb(fur));
    for k in 0..3 {
        m.disc(f.p(0.25, 0.9, -0.15 + k as f32 * 0.2), f.base.transform_vector3(Vec3::X), 0.04 * f.s, 5, rgb(WHITE));
    }
    m.beam(f.p(0.0, 0.95, -0.38), f.p(0.0, 1.35, -0.5), 0.14, rgb(fur));
    m.blob(f.at(0.0, 1.42, -0.58), Vec3::new(0.12, 0.12, 0.2), 2, 6, rgb(fur));
    m.blob(f.at(0.0, 1.4, -0.77), Vec3::splat(0.04), 1, 4, rgb(DARK));
    m.blob(f.at(0.0, 0.95, 0.5), Vec3::new(0.08, 0.1, 0.05), 1, 4, rgb(WHITE));
    for sx in [-1.0f32, 1.0] {
        m.blob(f.at(sx * 0.12, 1.55, -0.52), Vec3::new(0.1, 0.04, 0.05), 1, 4, rgb(fur));
        if p.variant % 2 == 0 {
            let b = f.p(sx * 0.06, 1.52, -0.55);
            let t = f.p(sx * 0.25, 1.95, -0.5);
            m.beam(b, t, 0.04, rgb([0.85, 0.75, 0.6]));
            m.beam(f.p(sx * 0.16, 1.73, -0.53), f.p(sx * 0.12, 1.9, -0.68), 0.035, rgb([0.85, 0.75, 0.6]));
        }
    }
}

// ------------------------------------------------------------------ race-side furniture

fn chevron(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    for x in [-0.6f32, 0.6] {
        m.cube(f.at(x, 0.7, 0.05), Vec3::new(0.05, 0.7, 0.05), rgb(WHITE));
    }
    let (hw, y0, y1) = (0.85, 1.0, 1.65);
    m.bevel_box(f.at(0.0, (y0 + y1) * 0.5, 0.0), Vec3::new(hw, (y1 - y0) * 0.5, 0.04), 0.03, rgb(WHITE));
    let red = rgb([0.92, 0.3, 0.28]);
    // board front is local -Z, so the viewer's right is local -X
    let dir = if p.variant % 2 == 0 { -1.0 } else { 1.0 };
    let z = -0.05;
    for k in 0..3 {
        let x = (k as f32 - 1.0) * 0.5 * dir;
        let (yc, hh, w, t) = ((y0 + y1) * 0.5, 0.24, 0.18, 0.14);
        let tip = Vec3::new(x + w * dir, yc, z);
        let top = Vec3::new(x - w * dir, yc + hh, z);
        let bot = Vec3::new(x - w * dir, yc - hh, z);
        let tip_i = tip - Vec3::X * t * dir;
        let top_i = top - Vec3::X * t * dir;
        let bot_i = bot - Vec3::X * t * dir;
        m.quad(f.p_v(top_i), f.p_v(top), f.p_v(tip), f.p_v(tip_i), red);
        m.quad(f.p_v(tip_i), f.p_v(tip), f.p_v(bot), f.p_v(bot_i), red);
    }
    // back of the board: wooden brace
    m.cube(f.at(0.0, 1.33, 0.06), Vec3::new(hw * 0.9, 0.05, 0.02), rgb(WOOD_D));
}

fn arrow_sign(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    m.cube(f.at(0.0, 0.9, 0.0), Vec3::new(0.07, 0.9, 0.07), rgb(WOOD_D));
    // front is local -Z: viewer's right is local -X
    let dir = if p.variant % 2 == 0 { -1.0 } else { 1.0 };
    let col = rgb([0.98, 0.88, 0.5]);
    let z = -0.08;
    let y = 1.45;
    let pts = [Vec3::new(-0.6 * dir, y - 0.18, z), Vec3::new(0.25 * dir, y - 0.18, z), Vec3::new(0.25 * dir, y - 0.32, z), Vec3::new(0.7 * dir, y, z), Vec3::new(0.25 * dir, y + 0.32, z), Vec3::new(0.25 * dir, y + 0.18, z), Vec3::new(-0.6 * dir, y + 0.18, z)];
    let w: Vec<Vec3> = pts.iter().map(|v| f.p_v(*v)).collect();
    m.poly(&[w[0], w[1], w[5], w[6]], col);
    m.poly(&[w[2], w[3], w[4]], col);
    let back: Vec<Vec3> = pts.iter().map(|v| f.p_v(*v + Vec3::Z * 0.06)).collect();
    m.poly(&[back[0], back[1], back[5], back[6]], rgb(WOOD));
    m.poly(&[back[2], back[3], back[4]], rgb(WOOD));
    m.cube(f.at(-0.1 * dir, y, z - 0.01), Vec3::new(0.32, 0.03, 0.005), rgb([0.85, 0.4, 0.3]));
}

fn flag_pole(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    m.cylinder(f.at(0.0, 2.5, 0.0), 0.05, 2.5, 5, rgb(WHITE), rgb(WHITE));
    m.blob(f.at(0.0, 5.05, 0.0), Vec3::splat(0.09), 1, 5, rgb([0.98, 0.82, 0.35]));
    let c = PASTELS[p.variant as usize % 6];
    let c2 = PASTELS[(p.variant as usize + 2) % 6];
    let mk = m.mark();
    m.quad(f.p(0.0, 4.9, 0.0), f.p(1.3, 4.85, 0.0), f.p(1.3, 4.45, 0.0), f.p(0.0, 4.4, 0.0), rgb(c));
    m.quad(f.p(0.0, 4.4, 0.0), f.p(1.3, 4.45, 0.0), f.p(1.3, 4.05, 0.0), f.p(0.0, 4.0, 0.0), rgb(c2));
    let pole = f.p(0.0, 0.0, 0.0);
    let s = f.s;
    m.sway_since(mk, move |q| ((Vec3::new(q.x - pole.x, 0.0, q.z - pole.z).length()) / (1.3 * s)).clamp(0.0, 1.0) * 0.9);
}

fn bunting(m: &mut MeshBuilder, p: &Prop) {
    let len = 10.0 * p.scale;
    let dir = Vec3::new(p.rot_y.cos(), 0.0, -p.rot_y.sin());
    let a = p.pos;
    let b = p.pos + dir * len;
    // poles: b's foot uses the same base height (props are on gentle ground)
    for q in [a, b] {
        m.cylinder(Mat4::from_translation(q + Vec3::Y * 1.6), 0.06, 1.7, 5, rgb(WOOD), rgb(WOOD));
        m.blob(Mat4::from_translation(q + Vec3::Y * 3.35), Vec3::splat(0.1), 1, 5, rgb([0.98, 0.82, 0.35]));
    }
    let top = 3.2;
    let rope = |t: f32| -> Vec3 { a.lerp(b, t) + Vec3::Y * (top - 0.7 * 4.0 * t * (1.0 - t)) };
    let segs = 8;
    for i in 0..segs {
        m.beam(rope(i as f32 / segs as f32), rope((i + 1) as f32 / segs as f32), 0.025, rgb(WHITE));
    }
    let n = (len / 0.7) as usize;
    let mk = m.mark();
    for k in 0..n {
        let t0 = (k as f32 + 0.15) / n as f32;
        let t1 = (k as f32 + 0.85) / n as f32;
        let (p0, p1) = (rope(t0), rope(t1));
        let tip = (p0 + p1) * 0.5 - Vec3::Y * 0.42;
        m.tri(p0, p1, tip, rgb(PASTELS[(k + p.variant as usize) % 6]));
    }
    let (ay, by) = (a.y, b.y);
    m.sway_since(mk, move |q| {
        let t = ((q - a).dot(dir) / len).clamp(0.0, 1.0);
        let ry = ay + (by - ay) * t + top - 0.7 * 4.0 * t * (1.0 - t);
        ((ry - q.y) / 0.42).clamp(0.0, 1.0) * 0.7
    });
}

fn spectator(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let shirts = [[0.9, 0.38, 0.38], [0.38, 0.58, 0.9], [0.95, 0.8, 0.3], [0.45, 0.75, 0.45], [0.8, 0.5, 0.85], [0.95, 0.6, 0.3], [0.3, 0.75, 0.78]];
    let skins = [[0.98, 0.82, 0.68], [0.85, 0.65, 0.48], [0.6, 0.42, 0.3], [0.95, 0.75, 0.6]];
    let hairs = [[0.3, 0.22, 0.15], [0.85, 0.65, 0.3], [0.15, 0.12, 0.12], [0.7, 0.35, 0.2]];
    let v = p.variant as usize;
    let k = (f.r(5.0) * 100.0) as usize;
    let shirt = rgb(shirts[(v + k) % 7]);
    let skin = rgb(skins[k % 4]);
    let kid = k % 5 == 0;
    let s = if kid { 0.72 } else { 1.0 };
    let at = |x: f32, y: f32, z: f32| f.at(x * s, y * s, z * s);
    m.frustum(at(0.0, 0.38, 0.0), 0.17 * s, 0.15 * s, 0.38 * s, 6, rgb([0.3, 0.34, 0.5]), rgb([0.3, 0.34, 0.5]));
    m.blob(at(0.0, 1.0, 0.0), Vec3::new(0.25, 0.32, 0.2) * s, 2, 7, shirt);
    m.blob(at(0.0, 1.5, 0.0), Vec3::splat(0.17 * s), 3, 7, skin);
    m.blob(at(0.0, 1.58, 0.04), Vec3::new(0.18, 0.12, 0.17) * s, 2, 7, rgb(hairs[(k / 3) % 4]));
    m.blob(at(-0.06, 1.52, -0.16), Vec3::splat(0.022 * s), 1, 4, rgb(DARK));
    m.blob(at(0.06, 1.52, -0.16), Vec3::splat(0.022 * s), 1, 4, rgb(DARK));
    match v % 6 {
        0 | 3 => {
            // cheering, arms up
            for sx in [-1.0f32, 1.0] {
                m.beam(f.p(sx * 0.2 * s, 1.18 * s, 0.0), f.p(sx * 0.38 * s, 1.75 * s, -0.05 * s), 0.09 * s, shirt);
                m.blob(at(sx * 0.39, 1.8, -0.05), Vec3::splat(0.06 * s), 1, 4, skin);
            }
            if v % 6 == 3 {
                m.frustum(at(0.0, 1.7, 0.0), 0.2 * s, 0.14 * s, 0.06 * s, 7, shirt, shirt);
            }
        }
        1 | 4 => {
            // waving a little flag
            m.beam(f.p(-0.2 * s, 1.18 * s, 0.0), f.p(-0.25 * s, 0.75 * s, 0.0), 0.09 * s, shirt);
            m.beam(f.p(0.2 * s, 1.18 * s, 0.0), f.p(0.38 * s, 1.6 * s, -0.1 * s), 0.09 * s, shirt);
            m.beam(f.p(0.38 * s, 1.5 * s, -0.1 * s), f.p(0.38 * s, 2.3 * s, -0.1 * s), 0.03, rgb(WOOD_D));
            let fc = PASTELS[(v + k + 1) % 6];
            let mk = m.mark();
            m.quad(f.p(0.38 * s, 2.3 * s, -0.1 * s), f.p(0.38 * s + 0.55, 2.25 * s, -0.1 * s), f.p(0.38 * s + 0.55, 1.95 * s, -0.1 * s), f.p(0.38 * s, 1.95 * s, -0.1 * s), rgb(fc));
            let px = f.p(0.38 * s, 0.0, -0.1 * s);
            m.sway_since(mk, move |q| (Vec3::new(q.x - px.x, 0.0, q.z - px.z).length() / 0.55).clamp(0.0, 1.0) * 0.8);
        }
        2 => {
            // umbrella
            m.beam(f.p(0.2 * s, 1.18 * s, 0.0), f.p(0.3 * s, 1.3 * s, -0.15 * s), 0.09 * s, shirt);
            m.beam(f.p(0.3 * s, 1.0 * s, -0.15 * s), f.p(0.3 * s, 2.3 * s, -0.15 * s), 0.03, rgb(DARK));
            let uc = PASTELS[(k + 2) % 6];
            let c = f.p(0.3 * s, 2.35 * s, -0.15 * s);
            let segs = 8;
            for i in 0..segs {
                let a0 = i as f32 / segs as f32 * TAU;
                let a1 = (i + 1) as f32 / segs as f32 * TAU;
                let r = 0.75;
                let e = |a: f32| c + Vec3::new(a.cos() * r, -0.32, a.sin() * r);
                m.tri(c + Vec3::Y * 0.1, e(a0), e(a1), rgb(if i % 2 == 0 { uc } else { WHITE }));
            }
            m.beam(f.p(-0.2 * s, 1.18 * s, 0.0), f.p(-0.25 * s, 0.75 * s, 0.0), 0.09 * s, shirt);
        }
        _ => {
            // camera / hands on hips
            for sx in [-1.0f32, 1.0] {
                m.beam(f.p(sx * 0.2 * s, 1.18 * s, 0.0), f.p(sx * 0.3 * s, 0.85 * s, -0.05 * s), 0.09 * s, shirt);
            }
            m.cube(at(0.0, 1.25, -0.25), Vec3::new(0.1, 0.07, 0.05) * s, rgb(DARK));
            m.frustum(at(0.0, 1.7, 0.0), 0.24 * s, 0.17 * s, 0.03 * s, 8, rgb([0.95, 0.9, 0.7]), rgb([0.95, 0.9, 0.7]));
            m.frustum(at(0.0, 1.76, 0.0), 0.15 * s, 0.13 * s, 0.07 * s, 8, rgb([0.95, 0.9, 0.7]), rgb([0.9, 0.4, 0.4]));
        }
    }
}

fn cone(m: &mut MeshBuilder, f: &Frame) {
    m.cube(f.at(0.0, 0.03, 0.0), Vec3::new(0.26, 0.03, 0.26), rgb([1.0, 0.5, 0.15]));
    m.frustum(f.at(0.0, 0.38, 0.0), 0.2, 0.03, 0.34, 8, rgb([1.0, 0.5, 0.15]), rgb([1.0, 0.5, 0.15]));
    m.frustum(f.at(0.0, 0.4, 0.0), 0.125, 0.105, 0.06, 8, rgb(WHITE), rgb(WHITE));
}

// ------------------------------------------------------------------ biome specials

fn lighthouse(m: &mut MeshBuilder, f: &Frame) {
    plinth(m, f, 2.2, 2.2);
    let bands = 5;
    let h = 11.0;
    for k in 0..bands {
        let y0 = k as f32 * h / bands as f32;
        let y1 = y0 + h / bands as f32;
        let r0 = 2.0 - y0 * 0.07;
        let r1 = 2.0 - y1 * 0.07;
        let c = if k % 2 == 0 { WHITE } else { [0.9, 0.32, 0.3] };
        m.frustum(f.at(0.0, (y0 + y1) * 0.5, 0.0), r0, r1, (y1 - y0) * 0.5, 10, rgb(c), rgb(c));
    }
    m.cylinder(f.at(0.0, h + 0.1, 0.0), 1.75, 0.1, 10, rgb(DARK), rgb(DARK));
    for k in 0..12 {
        let a = k as f32 / 12.0 * TAU;
        m.beam(f.p(a.cos() * 1.65, h + 0.2, a.sin() * 1.65), f.p(a.cos() * 1.65, h + 0.8, a.sin() * 1.65), 0.05, rgb(DARK));
    }
    m.cylinder(f.at(0.0, h + 0.8, 0.0), 1.68, 0.03, 12, rgb(DARK), rgb(DARK));
    m.cylinder(f.at(0.0, h + 1.0, 0.0), 1.0, 0.8, 10, rgb([1.0, 0.95, 0.7]), rgb([1.0, 0.95, 0.7]));
    m.frustum(f.at(0.0, h + 2.2, 0.0), 1.25, 0.15, 0.45, 10, rgb([0.9, 0.32, 0.3]), rgb([0.9, 0.32, 0.3]));
    m.blob(f.at(0.0, h + 2.75, 0.0), Vec3::splat(0.2), 2, 6, rgb([0.98, 0.82, 0.35]));
    m.cube(f.at(0.0, 0.9, -1.95), Vec3::new(0.45, 0.9, 0.1), rgb([0.35, 0.5, 0.75]));
    for y in [3.5f32, 6.5, 9.0] {
        let r = 2.0 - y * 0.07;
        window(m, f, Vec3::new(0.0, y, -r), Vec3::NEG_Z, 0.4, 0.6, DARK, [0.55, 0.72, 0.85], None);
    }
    // keeper's hut
    let hf = Frame { base: f.base * Mat4::from_translation(Vec3::new(3.6, 0.0, 0.5)), pos: f.pos, s: f.s, seed: f.seed };
    hf_cottage_small(m, &hf);
}

fn hf_cottage_small(m: &mut MeshBuilder, f: &Frame) {
    m.cube(f.at(0.0, -0.5, 0.0), Vec3::new(1.7, 0.7, 1.5), rgb(STONE));
    m.cube(f.at(0.0, 1.0, 0.0), Vec3::new(1.5, 1.0, 1.3), rgb(WHITE));
    gable(m, f, 1.5, 1.3, 2.0, 3.1, 0.3, [0.35, 0.55, 0.8], WHITE);
    m.cube(f.at(0.0, 0.8, -1.33), Vec3::new(0.35, 0.8, 0.04), rgb([0.9, 0.32, 0.3]));
}

fn beach_umbrella(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let c = PASTELS[p.variant as usize % 6];
    m.beam(f.p(0.0, 0.0, 0.0), f.p(0.1, 2.3, 0.0), 0.06, rgb(WHITE));
    let top = Vec3::new(0.1, 2.45, 0.0);
    let segs = 10;
    let mk = m.mark();
    for i in 0..segs {
        let a0 = i as f32 / segs as f32 * TAU;
        let a1 = (i + 1) as f32 / segs as f32 * TAU;
        let e = |a: f32| top + Vec3::new(a.cos() * 1.5, -0.55, a.sin() * 1.5);
        m.tri(f.p_v(top), f.p_v(e(a0)), f.p_v(e(a1)), rgb(if i % 2 == 0 { c } else { WHITE }));
    }
    sway_up(m, mk, f.pos.y, 1.9 * f.s, 0.5 * f.s, 0.15);
    // towel + beach ball
    let tc = PASTELS[(p.variant as usize + 3) % 6];
    m.quad(f.p(0.6, 0.05, -0.5), f.p(1.4, 0.05, -0.5), f.p(1.4, 0.05, 1.3), f.p(0.6, 0.05, 1.3), rgb(tc));
    m.quad(f.p(0.6, 0.06, -0.2), f.p(1.4, 0.06, -0.2), f.p(1.4, 0.06, 0.0), f.p(0.6, 0.06, 0.0), rgb(WHITE));
    if p.variant % 2 == 0 {
        for (k, c) in [[0.95, 0.4, 0.4], WHITE, [0.4, 0.6, 0.95], [0.98, 0.85, 0.35]].iter().enumerate() {
            m.blob(f.atr(-0.9, 0.25, 0.6, Quat::from_rotation_y(k as f32 * FRAC_PI_2)), Vec3::new(0.25, 0.25, 0.1), 2, 6, rgb(*c));
        }
    }
}

fn boat(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let c = PASTELS[p.variant as usize % 6];
    // hull: loft of U-shaped rings along Z
    let rings: Vec<Vec<Vec3>> = [(-2.0f32, 0.05f32, 0.25f32), (-1.4, 0.55, 0.05), (0.0, 0.85, 0.0), (1.4, 0.75, 0.05), (1.9, 0.6, 0.15)]
        .iter()
        .map(|&(z, w, lift)| {
            [(-w, 0.7), (-w * 0.9, 0.25), (-w * 0.5, 0.0), (w * 0.5, 0.0), (w * 0.9, 0.25), (w, 0.7)]
                .iter()
                .map(|&(x, y)| f.p(x, y + lift - 0.25, z))
                .collect()
        })
        .collect();
    m.loft(&rings, |_, k| rgb(if k == 0 || k == 4 { WHITE } else { c }), Some(rgb(c)), Some(rgb(c)));
    m.cube(f.at(0.0, 0.32, 0.0), Vec3::new(0.75, 0.04, 1.6), rgb(WOOD));
    m.cube(f.at(0.0, 0.4, 0.2), Vec3::new(0.7, 0.05, 0.15), rgb(WOOD_D));
    if p.variant % 2 == 0 {
        m.beam(f.p(0.0, 0.3, -0.4), f.p(0.0, 3.6, -0.4), 0.08, rgb(WOOD_D));
        let mk = m.mark();
        m.tri(f.p(0.0, 3.5, -0.35), f.p(0.0, 0.7, -0.35), f.p(0.0, 0.7, 1.4), rgb(WHITE));
        m.tri(f.p(0.0, 3.3, -0.45), f.p(0.0, 0.9, -0.45), f.p(0.0, 0.9, -1.5), rgb(shade(c, 1.1)));
        sway_up(m, mk, f.pos.y, 0.5, 3.0, 0.25);
    } else {
        for sx in [-1.0f32, 1.0] {
            m.beam(f.p(sx * 0.6, 0.45, 0.2), f.p(sx * 1.6, -0.1, 0.5), 0.05, rgb(WOOD));
        }
    }
}

fn mesa(m: &mut MeshBuilder, f: &Frame, p: &Prop, style: &BiomeStyle) {
    let layers = [style.cliff, shade(style.cliff, 1.12), mix(style.cliff, [0.95, 0.8, 0.6], 0.4), shade(style.cliff, 0.92)];
    let mut y = -2.0;
    let mut r = 1.0;
    let n = 3 + p.variant as usize % 2;
    for k in 0..n {
        let h = 2.4 + f.r(k as f32) * 1.4;
        let rot = Quat::from_rotation_y(f.r(k as f32 + 5.0) * 0.8);
        m.box_tapered(f.base * Mat4::from_rotation_translation(rot, Vec3::new((f.r(k as f32 + 9.0) - 0.5) * 0.6, y + h * 0.5, 0.0)), Vec3::new(4.5 * r, h * 0.5, 3.6 * r), 0.9, rgb(layers[k % 4]));
        y += h;
        r *= 0.9;
    }
    m.box_tapered(f.at(0.0, y + 0.2, 0.0), Vec3::new(3.6 * r, 0.25, 2.9 * r), 0.92, rgb(shade(style.cliff, 1.15)));
    // rubble
    for k in 0..4 {
        let a = k as f32 * 1.6 + f.seed;
        m.box_tapered(f.at(a.cos() * 5.0, 0.3, a.sin() * 4.2) * Mat4::from_rotation_y(a), Vec3::new(0.7, 0.5, 0.6), 0.6, rgb(shade(style.cliff, 0.9)));
    }
}

fn tumbleweed(m: &mut MeshBuilder, f: &Frame) {
    let c = [0.72, 0.58, 0.38];
    m.blob(f.at(0.0, 0.45, 0.0), Vec3::splat(0.42), 2, 6, rgb(shade(c, 0.9)));
    for k in 0..6 {
        let a = k as f32 * 1.05 + f.seed;
        let d = Vec3::new(a.cos(), (k as f32 * 0.9).sin() * 0.8, a.sin()).normalize() * 0.55;
        m.beam(f.p(-d.x, 0.45 - d.y, -d.z), f.p(d.x, 0.45 + d.y, d.z), 0.04, rgb(c));
    }
}

fn adobe(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let wall = [[0.9, 0.72, 0.55], [0.95, 0.8, 0.62], [0.88, 0.65, 0.5]][p.variant as usize % 3];
    plinth(m, f, 2.8, 2.3);
    m.bevel_box(f.at(0.0, 1.3, 0.0), Vec3::new(2.8, 1.4, 2.3), 0.3, rgb(wall));
    m.bevel_box(f.at(1.0, 2.9, 0.4), Vec3::new(1.5, 0.6, 1.4), 0.2, rgb(shade(wall, 1.04)));
    for k in 0..4 {
        let z = -1.6 + k as f32 * 1.05;
        m.beam(f.p(-3.2, 2.45, z), f.p(-2.6, 2.45, z), 0.16, rgb(WOOD_D));
        m.beam(f.p(2.6, 2.45, z), f.p(3.2, 2.45, z), 0.16, rgb(WOOD_D));
    }
    m.cube(f.at(-0.8, 1.0, -2.32), Vec3::new(0.5, 1.0, 0.05), rgb([0.35, 0.6, 0.75]));
    window(m, f, Vec3::new(1.2, 1.5, -2.3), Vec3::NEG_Z, 0.7, 0.6, [0.35, 0.6, 0.75], [0.3, 0.25, 0.25], None);
    // chili string + pots
    for k in 0..5 {
        m.blob(f.at(0.2, 2.0 - k as f32 * 0.17, -2.4), Vec3::new(0.06, 0.09, 0.06), 1, 4, rgb([0.85, 0.2, 0.18]));
    }
    for (k, x) in [-2.0f32, -1.6].iter().enumerate() {
        m.frustum(f.at(*x, 0.25, -2.7), 0.18, 0.26, 0.25, 7, rgb([0.82, 0.45, 0.3]), rgb([0.4, 0.3, 0.25]));
        if k == 0 {
            m.cylinder(f.at(*x, 0.75, -2.7), 0.1, 0.3, 6, rgb([0.36, 0.62, 0.38]), rgb([0.42, 0.7, 0.42]));
        }
    }
}

fn snowman(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    m.blob(f.at(0.0, 0.5, 0.0), Vec3::splat(0.55), 3, 8, rgb(SNOW));
    m.blob(f.at(0.0, 1.25, 0.0), Vec3::splat(0.4), 3, 8, rgb(SNOW));
    m.blob(f.at(0.0, 1.85, 0.0), Vec3::splat(0.28), 3, 8, rgb(SNOW));
    m.frustum(f.at(0.0, 1.85, -0.38), 0.06, 0.0, 0.14, 5, rgb([0.98, 0.55, 0.2]), rgb([0.98, 0.55, 0.2]));
    m.frustum(f.at(0.0, 1.85, -0.38) * Mat4::from_rotation_x(-FRAC_PI_2), 0.06, 0.0, 0.15, 5, rgb([0.98, 0.55, 0.2]), rgb([0.98, 0.55, 0.2]));
    for sx in [-1.0f32, 1.0] {
        m.blob(f.at(sx * 0.1, 1.95, -0.24), Vec3::splat(0.035), 1, 4, rgb(DARK));
        m.beam(f.p(sx * 0.35, 1.3, 0.0), f.p(sx * 0.85, 1.65, 0.05), 0.04, rgb(WOOD_D));
    }
    for k in 0..3 {
        m.blob(f.at(0.0, 1.1 + k as f32 * 0.15, -0.39 + (k as f32 - 1.0).abs() * 0.02), Vec3::splat(0.035), 1, 4, rgb(DARK));
    }
    let sc = PASTELS[p.variant as usize % 6];
    m.cylinder(f.at(0.0, 1.6, 0.0), 0.3, 0.06, 8, rgb(sc), rgb(sc));
    m.beam(f.p(0.18, 1.6, -0.22), f.p(0.25, 1.2, -0.3), 0.08, rgb(sc));
    if p.variant % 2 == 0 {
        m.cylinder(f.at(0.0, 2.1, 0.0), 0.28, 0.02, 8, rgb(DARK), rgb(DARK));
        m.cylinder(f.at(0.0, 2.3, 0.0), 0.18, 0.2, 8, rgb(DARK), rgb(DARK));
        m.cylinder(f.at(0.0, 2.15, 0.0), 0.185, 0.04, 8, rgb([0.85, 0.25, 0.25]), rgb([0.85, 0.25, 0.25]));
    } else {
        m.blob(f.at(0.0, 2.08, 0.0), Vec3::new(0.27, 0.16, 0.27), 2, 7, rgb(sc));
        m.blob(f.at(0.0, 2.27, 0.0), Vec3::splat(0.08), 1, 5, rgb(WHITE));
    }
}

fn ski_cabin(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let (hx, hz, h) = (3.0, 2.4, 2.6);
    plinth(m, f, hx, hz);
    let logs = [[0.55, 0.38, 0.26], [0.5, 0.34, 0.23]];
    for k in 0..8 {
        let y = 0.17 + k as f32 * 0.33;
        let c = rgb(logs[k % 2]);
        m.beam(f.p(-hx - 0.2, y, -hz), f.p(hx + 0.2, y, -hz), 0.34, c);
        m.beam(f.p(-hx - 0.2, y, hz), f.p(hx + 0.2, y, hz), 0.34, c);
        m.beam(f.p(-hx, y + 0.16, -hz - 0.2), f.p(-hx, y + 0.16, hz + 0.2), 0.34, c);
        m.beam(f.p(hx, y + 0.16, -hz - 0.2), f.p(hx, y + 0.16, hz + 0.2), 0.34, c);
    }
    gable(m, f, hx, hz, h, h + 2.6, 0.5, SNOW, logs[0]);
    m.cube(f.at(-1.5, h + 2.0, 0.8), Vec3::new(0.3, 1.0, 0.3), rgb(STONE));
    m.cube(f.at(0.0, 0.95, -hz - 0.15), Vec3::new(0.5, 0.95, 0.04), rgb([0.75, 0.3, 0.28]));
    for sx in [-1.0f32, 1.0] {
        window(m, f, Vec3::new(sx * 1.8, 1.5, -hz - 0.15), Vec3::NEG_Z, 0.8, 0.7, [0.95, 0.9, 0.8], [1.0, 0.85, 0.5], Some([0.3, 0.45, 0.35]));
    }
    // skis leaning on the wall
    for (k, x) in [1.0f32, 1.25].iter().enumerate() {
        let c = PASTELS[(p.variant as usize + k * 3) % 6];
        m.plank(f.p(*x, 0.0, -hz - 0.4), f.p(*x + 0.1, 1.8, -hz - 0.25), 0.1, 0.03, f.base.transform_vector3(Vec3::NEG_Z), rgb(c));
    }
    // snow drifts
    for k in 0..3 {
        m.blob(f.at(-hx + k as f32 * 2.5, 0.0, hz + 0.4), Vec3::new(1.0, 0.4, 0.6), 2, 6, rgb(SNOW));
    }
}

fn frozen_pond(m: &mut MeshBuilder, f: &Frame, p: &Prop) {
    let segs = 14;
    let r = 5.0;
    let pts: Vec<Vec3> = (0..segs)
        .map(|i| {
            let a = i as f32 / segs as f32 * TAU;
            let rr = r * (0.85 + 0.15 * (a * 3.0 + f.seed).sin());
            f.p(a.cos() * rr, 0.06, a.sin() * rr * 0.75)
        })
        .collect();
    let c = f.p(0.0, 0.06, 0.0);
    for i in 0..segs {
        let col = if i % 3 == 0 { [0.78, 0.9, 0.98] } else { [0.7, 0.85, 0.95] };
        m.tri(c, pts[i], pts[(i + 1) % segs], rgb(col));
    }
    for i in 0..segs {
        if i % 2 == 0 {
            let q = pts[i];
            m.blob(Mat4::from_translation(q), Vec3::new(0.7, 0.25, 0.7) * f.s, 2, 6, rgb(SNOW));
        }
    }
    // skating penguin-ish friend
    if p.variant % 2 == 0 {
        m.blob(f.at(1.0, 0.45, 0.5), Vec3::new(0.25, 0.38, 0.22), 2, 7, rgb(DARK));
        m.blob(f.at(1.0, 0.42, 0.33), Vec3::new(0.17, 0.28, 0.08), 2, 6, rgb(WHITE));
        m.frustum(f.at(1.0, 0.65, 0.22) * Mat4::from_rotation_x(-FRAC_PI_2), 0.04, 0.0, 0.06, 4, rgb([0.98, 0.65, 0.2]), rgb([0.98, 0.65, 0.2]));
        m.cylinder(f.at(1.0, 0.82, 0.5), 0.17, 0.05, 7, rgb([0.9, 0.3, 0.3]), rgb([0.9, 0.3, 0.3]));
    }
}

/// Append a prop (world space) to `m`
pub fn add_prop(m: &mut MeshBuilder, p: &Prop, style: &BiomeStyle) {
    let base = Mat4::from_scale_rotation_translation(Vec3::splat(p.scale), Quat::from_rotation_y(p.rot_y), p.pos);
    let f = Frame { base, pos: p.pos, s: p.scale, seed: (p.pos.x * 0.37 + p.pos.z * 0.71).fract() * 10.0 };
    use PropKind::*;
    match p.kind {
        Tree => tree(m, &f, p, style),
        Birch => birch(m, &f, p, style),
        Blossom => blossom(m, &f, p),
        Pine => pine(m, &f, p, style),
        Palm => palm(m, &f, style),
        Bush => bush(m, &f, p, style),
        Rock => rock(m, &f, p, style),
        Cactus => cactus(m, &f, p, style),
        Flowers => flowers(m, &f, style),
        Grass => grass(m, &f, style),
        Mushroom => mushroom(m, &f, p),
        FlowerField => flower_field(m, &f, p, style),
        Spectator => spectator(m, &f, p),
        HayBale => hay_bale(m, &f, p),
        Tent => tent(m, &f, p),
        Cone => cone(m, &f),
        Fence => fence(m, &f, p, style),
        Chevron => chevron(m, &f, p),
        ArrowSign => arrow_sign(m, &f, p),
        Cottage => cottage(m, &f, p, style),
        Barn => barn(m, &f, p),
        Windmill => windmill(m, &f, p),
        Picnic => picnic(m, &f, p),
        Camper => camper(m, &f, p),
        Sheep => sheep(m, &f, p),
        Cow => cow(m, &f, p),
        Deer => deer(m, &f, p),
        Bunting => bunting(m, p),
        FlagPole => flag_pole(m, &f, p),
        Lighthouse => lighthouse(m, &f),
        BeachUmbrella => beach_umbrella(m, &f, p),
        Boat => boat(m, &f, p),
        Mesa => mesa(m, &f, p, style),
        Tumbleweed => tumbleweed(m, &f),
        Adobe => adobe(m, &f, p),
        Snowman => snowman(m, &f, p),
        SkiCabin => ski_cabin(m, &f, p),
        FrozenPond => frozen_pond(m, &f, p),
        Pumpkin => pumpkin(m, &f, p),
        Scarecrow => scarecrow(m, &f, p),
        HayWagon => hay_wagon(m, &f, p),
        Beehive => beehive(m, &f, p),
    }
}

/// Every prop kind/variant worth showing for a biome (dev showroom catalogue)
pub fn catalogue_kinds(b: Biome) -> Vec<(PropKind, u8)> {
    use PropKind::*;
    let mut v: Vec<(PropKind, u8)> = Vec::new();
    let common = [(Tree, 0u8), (Tree, 1), (Tree, 2), (Bush, 0), (Bush, 1), (Rock, 0), (Rock, 1), (Flowers, 0), (Grass, 0), (Fence, 0), (Fence, 1), (Chevron, 0), (Chevron, 1), (ArrowSign, 0), (HayBale, 0), (HayBale, 1)];
    v.extend_from_slice(&common);
    for k in 0..6 {
        v.push((Spectator, k));
    }
    v.extend_from_slice(&[(Tent, 0), (Camper, 1), (Picnic, 2), (FlagPole, 0), (Bunting, 0), (Cone, 0)]);
    let special: &[(PropKind, u8)] = match b {
        Biome::Meadow => &[(Birch, 0), (Blossom, 0), (Pine, 0), (Mushroom, 0), (Cottage, 0), (Barn, 0), (Windmill, 0), (Sheep, 0), (Sheep, 1), (Cow, 0), (Cow, 1), (Beehive, 0), (FlowerField, 0), (FlowerField, 1)],
        Biome::Autumn => &[(Birch, 1), (Pine, 1), (Mushroom, 1), (Barn, 1), (Cottage, 1), (Pumpkin, 2), (Scarecrow, 0), (HayWagon, 0), (Deer, 0), (Deer, 1), (FlowerField, 2)],
        Biome::Snow => &[(Pine, 0), (Pine, 1), (Birch, 0), (SkiCabin, 0), (Snowman, 0), (Snowman, 1), (FrozenPond, 0), (Deer, 0)],
        Biome::Desert => &[(Cactus, 0), (Cactus, 1), (Palm, 0), (Mesa, 0), (Tumbleweed, 0), (Adobe, 0), (Cow, 2)],
        Biome::Coast => &[(Palm, 0), (Lighthouse, 0), (BeachUmbrella, 0), (BeachUmbrella, 1), (Boat, 0), (Boat, 1), (Cottage, 2), (Sheep, 0)],
    };
    v.extend_from_slice(special);
    v
}
