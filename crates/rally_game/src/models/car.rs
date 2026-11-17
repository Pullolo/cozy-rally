//! The four cars, built in car-local space (origin = CG, -Z forward, +Y up, +X right).
//!
//! Shared construction: a lofted, chamfered lower body (rings along Z), an open-sided cabin
//! (pillars + roof + windshield, so the little helmeted driver is visible), wheel-arch flares,
//! bumpers, lights and per-car accessories. Light lenses go into separate meshes so the game
//! can make them glow (brake / reverse / head); the body keeps the unlit lens housings.
use super::parts::{roundel, MeshExt};
use crate::render::mesh::{mix, rgb, rgba, shade, trs, MeshBuilder};
use macroquad::math::{Mat4, Quat, Vec3};
use rally_core::vehicle::CarSpec;
use std::f32::consts::{FRAC_PI_2, PI};

pub struct CarModel {
    pub body: MeshBuilder,
    /// Wheel centred at the origin, rolling around local X; symmetric so it serves both sides.
    pub wheel: MeshBuilder,
    /// Headlight lenses (neutral bright colour, ~1.5 cm proud of the body). Draw emissive.
    pub head_lights: MeshBuilder,
    /// Brake light lenses (neutral bright colour; tint red when braking).
    pub brake_lights: MeshBuilder,
    /// Reverse light lenses (neutral bright colour; draw when in reverse).
    pub reverse_lights: MeshBuilder,
}

const DARK: [f32; 3] = [0.21, 0.20, 0.23];
const CHROME: [f32; 3] = [0.86, 0.87, 0.89];
const CREAM: [f32; 3] = [0.98, 0.94, 0.84];
const INTERIOR: [f32; 3] = [0.30, 0.25, 0.25];
const SEAT: [f32; 3] = [0.26, 0.23, 0.28];
const GLASS: [f32; 3] = [0.58, 0.77, 0.88];
const GLINT: [f32; 3] = [0.86, 0.95, 0.99];
const TAIL_OFF: [f32; 3] = [0.66, 0.14, 0.16];
const REV_OFF: [f32; 3] = [0.88, 0.88, 0.86];
const HEAD_OFF: [f32; 3] = [0.95, 0.93, 0.82];
const LENS: [f32; 3] = [1.0, 1.0, 1.0];
const HEAD_LENS: [f32; 3] = [1.0, 0.98, 0.92];

/// One lofted cross-section of the lower body.
#[derive(Clone, Copy)]
struct Sec {
    z: f32,
    yb: f32,
    yt: f32,
    w: f32,
    wt: f32,
    cb: f32,
    ct: f32,
    /// extra height of the top centre (sculpted bonnet / crowned roof)
    cr: f32,
}

const fn sec(z: f32, yb: f32, yt: f32, w: f32, wt: f32, cb: f32, ct: f32) -> Sec {
    Sec { z, yb, yt, w, wt, cb, ct, cr: 0.015 }
}

impl Sec {
    fn crown(mut self, cr: f32) -> Sec {
        self.cr = cr;
        self
    }
    fn shrink(self, z: f32, d: f32) -> Sec {
        Sec { z, yb: self.yb + d * 0.6, yt: self.yt - d, w: self.w - d, wt: self.wt - d, cb: (self.cb - d * 0.5).max(0.02), ct: (self.ct - d * 0.5).max(0.02), cr: self.cr * 0.6 }
    }
}

/// Add quarter-round rings to both ends of a section list (radius rf at the nose, rr at the tail).
fn round_ends(secs: &[Sec], rf: f32, rr: f32) -> Vec<Sec> {
    let f = secs[0];
    let l = *secs.last().unwrap();
    let mut v = vec![f.shrink(f.z - rf, rf), f.shrink(f.z - rf * 0.75, rf * 0.34)];
    v.extend_from_slice(secs);
    v.push(l.shrink(l.z + rr * 0.75, rr * 0.34));
    v.push(l.shrink(l.z + rr, rr));
    v
}

/// Where the livery bands sit: side band as fractions up the flank, top band as |x| range.
struct Bands {
    side: (f32, f32),
    top: (f32, f32),
}

const RING_HALF: usize = 12;
const RING: usize = 2 * RING_HALF - 2;

/// 22-point ring: right half from bottom centre to top centre, then the mirrored left half.
fn ring(s: &Sec, b: &Bands) -> Vec<Vec3> {
    let side = |t: f32| -> (f32, f32) { (s.w + (s.wt - s.w) * t, (s.yb + s.cb) + ((s.yt - s.ct) - (s.yb + s.cb)) * t) };
    let (x4, y4) = side(b.side.0);
    let (x5, y5) = side(b.side.1);
    let x9 = b.top.1.min(s.wt - s.ct - 0.02);
    let x10 = b.top.0.min(x9 - 0.02).max(0.01);
    let top = |x: f32| s.yt + s.cr * (1.0 - (x / (s.wt - s.ct)).clamp(0.0, 1.0).powi(2));
    let k = 0.293;
    let half = [
        (0.0, s.yb),
        (s.w - s.cb, s.yb),
        (s.w - s.cb * k, s.yb + s.cb * k),
        (s.w, s.yb + s.cb),
        (x4, y4),
        (x5, y5),
        (s.wt, s.yt - s.ct),
        (s.wt - s.ct * k, s.yt - s.ct * k),
        (s.wt - s.ct, s.yt),
        (x9, top(x9)),
        (x10, top(x10)),
        (0.0, top(0.0)),
    ];
    let mut pts: Vec<Vec3> = half.iter().map(|&(x, y)| Vec3::new(x, y, s.z)).collect();
    for k in (1..RING_HALF - 1).rev() {
        let p = pts[k];
        pts.push(Vec3::new(-p.x, p.y, p.z));
    }
    debug_assert_eq!(pts.len(), RING);
    pts
}

/// Quad kind around the ring: 0 underside, 1-2 lower round, 3 lower flank, 4 side band,
/// 5 upper flank, 6-7 shoulder round, 8 top outer, 9 top band, 10 top centre.
fn kind(k: usize) -> usize {
    if k < RING_HALF - 1 {
        k
    } else {
        RING - 1 - k
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Interpolate a body section at z
fn sec_at(secs: &[Sec], z: f32) -> Sec {
    for w in secs.windows(2) {
        if z >= w[0].z && z <= w[1].z {
            let t = (z - w[0].z) / (w[1].z - w[0].z).max(1e-5);
            let (a, b) = (w[0], w[1]);
            return Sec {
                z,
                yb: lerp(a.yb, b.yb, t),
                yt: lerp(a.yt, b.yt, t),
                w: lerp(a.w, b.w, t),
                wt: lerp(a.wt, b.wt, t),
                cb: lerp(a.cb, b.cb, t),
                ct: lerp(a.ct, b.ct, t),
                cr: lerp(a.cr, b.cr, t),
            };
        }
    }
    if z < secs[0].z {
        secs[0]
    } else {
        *secs.last().unwrap()
    }
}

/// Point on the right/left flank at height fraction t (0 = above the sill, 1 = shoulder)
/// plus the outward flank normal.
fn flank(secs: &[Sec], sx: f32, z: f32, t: f32) -> (Vec3, Vec3) {
    let s = sec_at(secs, z);
    let (x0, y0) = (s.w, s.yb + s.cb);
    let (x1, y1) = (s.wt, s.yt - s.ct);
    let p = Vec3::new(sx * lerp(x0, x1, t), lerp(y0, y1, t), z);
    let n = Vec3::new(sx * (y1 - y0), -(x1 - x0), 0.0).normalize();
    (p, n)
}

struct BodyPaint {
    paint: [f32; 3],
    lower: [f32; 3],
    side_band: [f32; 3],
    top_band: [f32; 3],
    top: [f32; 3],
    /// z range of the open cabin (top surface there is the interior floor)
    cabin: (f32, f32),
}

fn body_loft(m: &mut MeshBuilder, secs: &[Sec], bands: &Bands, p: &BodyPaint) {
    let rings: Vec<Vec<Vec3>> = secs.iter().map(|s| ring(s, bands)).collect();
    let zs: Vec<f32> = secs.iter().map(|s| s.z).collect();
    m.loft(
        &rings,
        |s, k| {
            let kd = kind(k);
            let inside = zs[s] >= p.cabin.0 - 1e-3 && zs[s + 1] <= p.cabin.1 + 1e-3;
            rgb(match kd {
                0 => DARK,
                1 | 2 => p.lower,
                4 => p.side_band,
                8..=10 if inside => INTERIOR,
                9 => p.top_band,
                8 | 10 => p.top,
                _ => p.paint,
            })
        },
        Some(rgb(p.paint)),
        Some(rgb(p.paint)),
    );
}

/// Glass/greenhouse description: windshield base z0 rises to the roof at zr0, roof to zr1,
/// rear glass falls to the deck at z1.
struct Green {
    z0: f32,
    zr0: f32,
    zr1: f32,
    z1: f32,
    wr: f32,
    yr: f32,
    roof: [f32; 3],
    pillar: [f32; 3],
    b_pillar: Option<f32>,
    /// rear side windows are solid (wide C pillar) from this roof fraction on
    c_from: f32,
}

fn glass_c(c: [f32; 3], a: f32) -> [u8; 4] {
    rgba(c, a)
}

/// Curved greenhouse shell: pillars/roof into `m`, translucent glass into `g`.
fn greenhouse(m: &mut MeshBuilder, g: &mut MeshBuilder, secs: &[Sec], gr: &Green) {
    let ease = |t: f32| (t * FRAC_PI_2).sin().powf(0.85);
    let mut zs: Vec<(f32, u8)> = Vec::new(); // (z, section class) 0 windshield,1 roof,2 rear
    for i in 0..=3 {
        zs.push((lerp(gr.z0, gr.zr0, i as f32 / 3.0), 0));
    }
    let roof_n = 5;
    for i in 1..roof_n {
        zs.push((lerp(gr.zr0, gr.zr1, i as f32 / roof_n as f32), 1));
    }
    for i in 0..=3 {
        zs.push((lerp(gr.zr1, gr.z1, i as f32 / 3.0), 2));
    }
    if let Some(f) = gr.b_pillar {
        let zb = lerp(gr.zr0, gr.zr1, f);
        zs.push((zb - 0.05, 1));
        zs.push((zb + 0.05, 1));
    }
    zs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    zs.dedup_by(|a, b| (a.0 - b.0).abs() < 0.01);
    let height = |z: f32| -> f32 {
        if z <= gr.zr0 {
            ease(((z - gr.z0) / (gr.zr0 - gr.z0)).clamp(0.0, 1.0))
        } else if z >= gr.zr1 {
            ease(((gr.z1 - z) / (gr.z1 - gr.zr1)).clamp(0.0, 1.0))
        } else {
            1.0
        }
    };
    let rings: Vec<Vec<Vec3>> = zs
        .iter()
        .map(|&(z, _)| {
            let s = sec_at(secs, z);
            let belt = s.yt;
            let wb = s.wt - s.ct * 0.75;
            let h = height(z);
            let mid = if z > gr.zr0 && z < gr.zr1 { ((z - gr.zr0) / (gr.zr1 - gr.zr0) * PI).sin() * 0.025 } else { 0.0 };
            let top = belt + (gr.yr - belt) * h + mid;
            let wt = lerp(wb, gr.wr, h.max(0.3));
            let c = 0.09 * h.max(0.15);
            let half = [
                (wb, belt - 0.004),
                (lerp(wb, wt, 0.06), belt + 0.035 * h.min(1.0)),
                (wt, top - c),
                (wt - c * 0.293, top - c * 0.293),
                (wt - c, top),
                (0.0, top + 0.02 * h),
            ];
            let mut pts: Vec<Vec3> = half.iter().map(|&(x, y)| Vec3::new(x, y, z)).collect();
            for k in (0..5).rev() {
                let p = pts[k];
                pts.push(Vec3::new(-p.x, p.y, p.z));
            }
            pts
        })
        .collect();
    let bz = gr.b_pillar.map(|f| lerp(gr.zr0, gr.zr1, f));
    for si in 0..rings.len() - 1 {
        let (a, b) = (&rings[si], &rings[si + 1]);
        let zc = (zs[si].0 + zs[si + 1].0) * 0.5;
        let class = if zc < gr.zr0 { 0 } else if zc > gr.zr1 { 2 } else { 1 };
        let in_b = bz.map_or(false, |z| (zc - z).abs() < 0.05);
        let roof_t = ((zc - gr.zr0) / (gr.zr1 - gr.zr0)).clamp(0.0, 1.0);
        for k in 0..rings[si].len() - 1 {
            let kd = if k < 5 { k } else { 9 - k };
            // (is_glass, colour)
            let (is_glass, col) = match (class, kd) {
                (_, 0) => (false, gr.pillar),
                (0, 2) | (2, 2) => (false, gr.pillar),
                (0, _) => (true, GLASS),
                (2, 1) => (false, gr.pillar),
                (2, _) => (true, GLASS),
                (1, 1) if in_b || roof_t >= gr.c_from => (false, gr.pillar),
                (1, 1) => (true, GLASS),
                (1, 2) => (false, gr.pillar),
                _ => (false, gr.roof),
            };
            let (p0, p1, p2, p3) = (a[k], a[k + 1], b[k + 1], b[k]);
            if is_glass {
                // slightly lighter towards the top: cheap sky reflection
                let c = if kd >= 3 { mix(col, GLINT, 0.25) } else { col };
                let gc = glass_c(c, 0.42);
                if p0.distance_squared(p1) < 1e-8 {
                    g.tri(p0, p2, p3, gc);
                } else {
                    g.quad(p0, p1, p2, p3, gc);
                }
            } else if p0.distance_squared(p1) < 1e-8 {
                m.tri(p0, p2, p3, rgb(col));
            } else {
                m.quad(p0, p1, p2, p3, rgb(col));
            }
        }
    }
    // glints on windshield + rear glass
    let w0 = &rings[0];
    let wi = zs.iter().position(|z| z.1 == 0 && (z.0 - gr.zr0).abs() < 0.01).unwrap_or(3);
    let w1 = &rings[wi];
    let on = |r: &Vec<Vec3>, u: f32| -> Vec3 {
        let x = lerp(-r[4].x.abs(), r[4].x.abs(), u);
        Vec3::new(x, r[4].y + 0.02 - 0.03 * (x / r[4].x.abs().max(0.1)).powi(2), r[4].z)
    };
    let lift = |p: Vec3| p + Vec3::new(0.0, 0.006, -0.006);
    for (u0, u1, d) in [(0.18f32, 0.30, 0.2), (0.34, 0.38, 0.2)] {
        let a0 = Vec3::new(lerp(-w0[0].x, w0[0].x, u0), w0[0].y + 0.02, w0[0].z + 0.02);
        let a1 = Vec3::new(lerp(-w0[0].x, w0[0].x, u1), w0[0].y + 0.02, w0[0].z + 0.02);
        g.quad(lift(a0), lift(a1), lift(on(w1, u1 + d)), lift(on(w1, u0 + d)), glass_c(GLINT, 0.6));
    }
}

/// Seated driver at `p` (seat base), facing -Z, plus seat and steering wheel.
fn driver(m: &mut MeshBuilder, p: Vec3, suit: [f32; 3], helmet: [f32; 3], band: [f32; 3], scale: f32) {
    let at = |x: f32, y: f32, z: f32| trs(p + Vec3::new(x, y, z) * scale, Quat::IDENTITY, Vec3::splat(scale));
    // bucket seat
    m.bevel_box(at(0.0, 0.12, 0.17), Vec3::new(0.2, 0.2, 0.05), 0.04, rgb(SEAT));
    m.bevel_box(at(0.0, -0.02, 0.0), Vec3::new(0.2, 0.06, 0.18), 0.03, rgb(SEAT));
    // torso + arms
    m.frustum(at(0.0, 0.12, 0.02), 0.17, 0.13, 0.13, 7, rgb(suit), rgb(suit));
    for sx in [-1.0f32, 1.0] {
        m.beam(p + Vec3::new(sx * 0.15, 0.22, 0.02) * scale, p + Vec3::new(sx * 0.1, 0.16, -0.3) * scale, 0.07 * scale, rgb(shade(suit, 0.9)));
    }
    // steering wheel
    m.cylinder(at(0.0, 0.17, -0.33) * Mat4::from_rotation_x(1.2), 0.12, 0.015, 8, rgb(DARK), rgb(DARK));
    // helmet with a band and a dark visor
    m.blob(at(0.0, 0.39, 0.0), Vec3::new(0.15, 0.15, 0.16), 4, 8, rgb(helmet));
    m.cylinder(at(0.0, 0.43, 0.0), 0.152, 0.025, 8, rgb(band), rgb(band));
    m.blob(at(0.0, 0.38, -0.105), Vec3::new(0.115, 0.065, 0.07), 2, 8, rgb([0.16, 0.18, 0.28]));
    m.blob(at(0.04, 0.40, -0.16), Vec3::new(0.03, 0.015, 0.012), 1, 4, rgb([0.7, 0.8, 0.95]));
}

fn arch(m: &mut MeshBuilder, x_in: f32, x_out: f32, zc: f32, yc: f32, r_in: f32, r_out: f32, color: [f32; 3]) {
    let segs = 7;
    let rings: Vec<Vec<Vec3>> = (0..=segs)
        .map(|i| {
            let a = -0.12 + (PI + 0.24) * i as f32 / segs as f32;
            let (s, c) = a.sin_cos();
            let at = |x: f32, r: f32| Vec3::new(x, yc + s * r, zc - c * r);
            vec![at(x_in, r_in), at(x_out, r_in), at(x_out, r_out), at(x_in, r_out)]
        })
        .collect();
    m.loft(&rings, |_, k| rgb(if k == 2 { shade(color, 1.05) } else { color }), Some(rgb(color)), Some(rgb(color)));
}

/// Round lamp facing `n`: bezel+housing in body, bright lens in `lens_m`.
fn round_lamp(body: &mut MeshBuilder, lens_m: &mut MeshBuilder, c: Vec3, n: Vec3, r: f32, bezel: [f32; 3], off: [f32; 3]) {
    body.cylinder(Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::Y, n), c), r + 0.025, 0.02, 10, rgb(bezel), rgb(bezel));
    body.disc(c + n * 0.022, n, r, 10, rgb(off));
    lens_m.disc(c + n * 0.035, n, r * 0.92, 10, rgb(LENS));
}

/// Rectangular lamp on a face with normal n (right = in-plane x axis)
fn rect_lamp(body: &mut MeshBuilder, lens_m: &mut MeshBuilder, c: Vec3, n: Vec3, right: Vec3, hw: f32, hh: f32, off: [f32; 3], lens: [f32; 3]) {
    let up = n.cross(right).normalize();
    let q = |m: &mut MeshBuilder, e: f32, lift: f32, col: [u8; 4]| {
        let (a, b) = (right * (hw + e), up * (hh + e));
        let o = c + n * lift;
        m.quad(o - a - b, o + a - b, o + a + b, o - a + b, col);
    };
    q(body, 0.025, 0.006, rgb(DARK));
    q(body, 0.0, 0.012, rgb(off));
    q(lens_m, -0.008, 0.025, rgb(lens));
}

fn exhaust(m: &mut MeshBuilder, p: Vec3, r: f32, len: f32) {
    let rot = Quat::from_rotation_x(FRAC_PI_2);
    m.cylinder(trs(p, rot, Vec3::ONE), r, len * 0.5, 8, rgb([0.55, 0.55, 0.58]), rgb([0.12, 0.12, 0.14]));
    m.cylinder(trs(p + Vec3::Z * (len * 0.5 - 0.02), rot, Vec3::ONE), r * 1.2, 0.025, 8, rgb(CHROME), rgb([0.1, 0.1, 0.12]));
}

fn mirror(m: &mut MeshBuilder, sx: f32, base: Vec3, paint: [f32; 3]) {
    let tip = base + Vec3::new(sx * 0.14, 0.06, 0.0);
    m.beam(base, tip, 0.035, rgb(DARK));
    m.bevel_box(trs(tip + Vec3::new(sx * 0.04, 0.03, 0.0), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.07, 0.05, 0.04), 0.02, rgb(paint));
    m.quad(
        tip + Vec3::new(sx * -0.02, -0.01, -0.045),
        tip + Vec3::new(sx * 0.10, -0.01, -0.045),
        tip + Vec3::new(sx * 0.10, 0.07, -0.045),
        tip + Vec3::new(sx * -0.02, 0.07, -0.045),
        rgb(GLINT),
    );
}

fn plate(m: &mut MeshBuilder, c: Vec3, n: Vec3) {
    let right = Vec3::Y.cross(n).normalize();
    let (a, b) = (right * 0.17, Vec3::Y * 0.06);
    let o = c + n * 0.01;
    m.quad(o - a - b, o + a - b, o + a + b, o - a + b, rgb([0.98, 0.95, 0.80]));
    let o = c + n * 0.016;
    for k in 0..4 {
        let x = -0.11 + k as f32 * 0.075;
        let (p0, p1) = (o + right * x, o + right * (x + 0.045));
        m.quad(p0 - b * 0.5, p1 - b * 0.5, p1 + b * 0.5, p0 + b * 0.5, rgb([0.3, 0.3, 0.4]));
    }
}

fn mud_flap(m: &mut MeshBuilder, x: f32, z: f32, top: f32, bottom: f32, w: f32, color: [f32; 3], logo: [f32; 3]) {
    m.cube(trs(Vec3::new(x, (top + bottom) * 0.5, z), Quat::IDENTITY, Vec3::ONE), Vec3::new(w * 0.5, (top - bottom) * 0.5, 0.012), rgb(color));
    m.disc(Vec3::new(x, bottom + (top - bottom) * 0.35, z + 0.014), Vec3::Z, w * 0.25, 6, rgb(logo));
}

/// Little triangle pennant on a whip antenna (sways in the wind)
fn antenna(m: &mut MeshBuilder, base: Vec3, h: f32, flag: [f32; 3]) {
    m.beam(base, base + Vec3::new(0.0, h, 0.04), 0.018, rgb(DARK));
    let top = base + Vec3::new(0.0, h, 0.04);
    let mk = m.mark();
    m.tri(top, top - Vec3::Y * 0.16, top + Vec3::new(0.0, -0.08, 0.26), rgb(flag));
    m.sway_since(mk, |p| ((p.z - top.z) / 0.26).clamp(0.0, 1.0) * 0.8);
}

/// Wheel mesh: rounded tyre with tread blocks, dished rim with spokes, hub cap.
pub fn wheel_mesh(spec: &CarSpec, rim: [f32; 3], accent: [f32; 3], spokes: usize, knobby: bool) -> MeshBuilder {
    let mut w = MeshBuilder::new();
    let r = spec.wheel_radius;
    let hw = spec.wheel_width * 0.5;
    let tire = [0.17, 0.16, 0.19];
    let tread_l = [0.24, 0.23, 0.26];
    let wall = [0.22, 0.21, 0.24];
    let prof = [
        (-hw * 0.78, 0.60 * r),
        (-hw, 0.66 * r),
        (-hw, 0.84 * r),
        (-hw * 0.84, 0.96 * r),
        (-hw * 0.55, r),
        (hw * 0.55, r),
        (hw * 0.84, 0.96 * r),
        (hw, 0.84 * r),
        (hw, 0.66 * r),
        (hw * 0.78, 0.60 * r),
    ];
    let segs = 16;
    w.lathe_x(Mat4::IDENTITY, &prof, segs, |s, i| {
        rgb(match s {
            4 => {
                if i % 2 == 0 {
                    tread_l
                } else {
                    tire
                }
            }
            1 | 7 => wall,
            _ => tire,
        })
    });
    if knobby {
        for i in 0..segs {
            let a = (i as f32 + 0.5) / segs as f32 * std::f32::consts::TAU;
            let off = if i % 2 == 0 { hw * 0.3 } else { -hw * 0.3 };
            let rot = Quat::from_rotation_x(a);
            w.cube(trs(rot * Vec3::new(off, 0.0, r), rot, Vec3::ONE), Vec3::new(hw * 0.38, r * 0.12, 0.025), rgb(tire));
        }
    }
    let rot = Quat::from_rotation_z(FRAC_PI_2);
    // rim barrel with dark dish faces
    w.cylinder(trs(Vec3::ZERO, rot, Vec3::ONE), 0.62 * r, hw * 0.8, segs, rgb(shade(rim, 0.6)), rgb(shade(rim, 0.55)));
    for sx in [-1.0f32, 1.0] {
        let face = sx * hw * 0.8;
        // bright lip
        w.lathe_x(Mat4::IDENTITY, &[(face, 0.50 * r), (face + sx * 0.012, 0.55 * r), (face + sx * 0.012, 0.61 * r), (face, 0.63 * r)], segs, |_, _| rgb(rim));
        // spokes
        for k in 0..spokes {
            let a = k as f32 / spokes as f32 * std::f32::consts::TAU;
            let d = Vec3::new(0.0, a.cos(), a.sin());
            w.beam(Vec3::new(face, 0.0, 0.0) + d * 0.12 * r, Vec3::new(face, 0.0, 0.0) + d * 0.52 * r, 0.11 * r, rgb(rim));
        }
        // hub + cap
        w.cylinder(trs(Vec3::new(face + sx * 0.012, 0.0, 0.0), rot, Vec3::ONE), 0.2 * r, 0.02, 8, rgb(rim), rgb(accent));
    }
    w
}

struct Look {
    number: u8,
    helmet: [f32; 3],
    suit: [f32; 3],
}

pub fn car_model(spec: &CarSpec, paint: [f32; 3]) -> CarModel {
    let mut m = MeshBuilder::new();
    let mut head = MeshBuilder::new();
    let mut brake = MeshBuilder::new();
    let mut rev = MeshBuilder::new();
    let he = spec.half_extents;
    let r = spec.wheel_radius;
    // wheel centre height at rest (matches the setup preview: 80% of rest suspension length)
    let wy = spec.mount_height - spec.susp_rest * 0.8;
    let (fz, rz) = (-spec.cg_to_front, spec.cg_to_rear);
    let wheel_out = spec.half_track + spec.wheel_width * 0.5 + 0.02;
    let lower = shade(paint, 0.78);
    let accent = if paint[0] + paint[1] + paint[2] > 2.6 { [0.95, 0.45, 0.40] } else { [0.98, 0.97, 0.94] };
    let trim = shade(paint, 0.62);

    let wheel;
    match spec.id {
        0 => {
            // ---------------- Pip: tiny round FWD hatch, cream roof, big round eyes
            let look = Look { number: 7, helmet: [0.98, 0.97, 0.94], suit: [0.35, 0.55, 0.85] };
            let secs = [
                sec(-1.80, -0.34, -0.02, 0.62, 0.56, 0.07, 0.10),
                sec(-1.72, -0.40, 0.04, 0.74, 0.67, 0.10, 0.13),
                sec(-1.35, -0.42, 0.09, 0.77, 0.70, 0.11, 0.14),
                sec(-0.55, -0.42, 0.13, 0.78, 0.71, 0.11, 0.14),
                sec(1.45, -0.42, 0.14, 0.78, 0.71, 0.11, 0.14),
                sec(1.70, -0.41, 0.13, 0.76, 0.69, 0.10, 0.13),
                sec(1.78, -0.35, 0.08, 0.66, 0.60, 0.07, 0.10),
            ];
            let bands = Bands { side: (0.38, 0.62), top: (0.12, 0.2) };
            body_loft(&mut m, &secs, &bands, &BodyPaint { paint, lower, side_band: accent, top_band: paint, top: paint, cabin: (-0.55, 1.45) });
            cabin(
                &mut m,
                &secs,
                &Cabin { z0: -0.55, z1: 1.45, zr0: -0.02, zr1: 1.40, wr: 0.6, yr: 0.66, roof: CREAM, pillar: paint, b_pillar: Some(0.42), c_width: 0.3, rear_glass: false },
            );
            roundel(&mut m, Vec3::new(0.0, 0.712, 0.72), Vec3::Y, Vec3::NEG_Z, 0.26, look.number, paint, [0.25, 0.22, 0.3]);
            antenna(&mut m, Vec3::new(-0.42, 0.7, 1.2), 0.45, accent);
            // little roof lip spoiler
            m.plank(Vec3::new(-0.55, 0.69, 1.47), Vec3::new(0.55, 0.69, 1.47), 0.16, 0.03, Vec3::new(0.0, 1.0, 0.35), rgb(CREAM));
            driver(&mut m, Vec3::new(-0.3, -0.08, 0.5), look.suit, look.helmet, paint, 1.0);
            m.bevel_box(trs(Vec3::new(0.3, 0.07, 0.68), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.2, 0.2, 0.05), 0.04, rgb(SEAT));
            // face: big round headlights + smiley grille
            for sx in [-1.0f32, 1.0] {
                round_lamp(&mut m, &mut head, Vec3::new(sx * 0.42, -0.12, -1.76), Vec3::NEG_Z, 0.13, CHROME, HEAD_OFF);
                // round tail lights with a small reverse lamp
                round_lamp(&mut m, &mut brake, Vec3::new(sx * 0.48, -0.05, 1.76), Vec3::Z, 0.09, CHROME, TAIL_OFF);
                round_lamp(&mut m, &mut rev, Vec3::new(sx * 0.3, -0.08, 1.765), Vec3::Z, 0.045, CHROME, REV_OFF);
                mirror(&mut m, sx, Vec3::new(sx * 0.66, 0.2, -0.4), paint);
                roundel(&mut m, Vec3::new(sx * 0.785, -0.1, 0.05), Vec3::X * sx, Vec3::Y, 0.2, look.number, [0.98, 0.97, 0.94], [0.25, 0.22, 0.3]);
            }
            m.bevel_box(trs(Vec3::new(0.0, -0.27, -1.77), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.22, 0.06, 0.03), 0.025, rgb(DARK));
            m.bevel_box(trs(Vec3::new(0.0, -0.34, -1.82), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.7, 0.07, 0.05), 0.04, rgb(CREAM));
            m.bevel_box(trs(Vec3::new(0.0, -0.34, 1.82), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.7, 0.07, 0.05), 0.04, rgb(CREAM));
            plate(&mut m, Vec3::new(0.0, -0.2, 1.78), Vec3::Z);
            exhaust(&mut m, Vec3::new(0.42, -0.42, 1.75), 0.04, 0.22);
            wheel = wheel_mesh(spec, [0.97, 0.96, 0.93], paint, 4, false);
        }
        1 => {
            // ---------------- Bramble: long-nose RWD coupe, twin stripes, ducktail, chrome
            let look = Look { number: 3, helmet: [0.98, 0.85, 0.35], suit: [0.92, 0.92, 0.88] };
            let secs = [
                sec(-2.08, -0.33, -0.08, 0.70, 0.64, 0.07, 0.08),
                sec(-2.00, -0.40, -0.03, 0.80, 0.74, 0.10, 0.10),
                sec(-1.30, -0.42, 0.03, 0.83, 0.77, 0.11, 0.12),
                sec(-0.15, -0.42, 0.08, 0.84, 0.78, 0.11, 0.12),
                sec(1.20, -0.42, 0.10, 0.84, 0.78, 0.11, 0.12),
                sec(1.95, -0.40, 0.17, 0.81, 0.75, 0.10, 0.10),
                sec(2.06, -0.33, 0.12, 0.71, 0.65, 0.07, 0.08),
            ];
            let stripe = accent;
            let bands = Bands { side: (0.2, 0.32), top: (0.10, 0.28) };
            body_loft(&mut m, &secs, &bands, &BodyPaint { paint, lower, side_band: stripe, top_band: stripe, top: paint, cabin: (-0.15, 1.20) });
            cabin(
                &mut m,
                &secs,
                &Cabin { z0: -0.15, z1: 1.20, zr0: 0.42, zr1: 0.98, wr: 0.6, yr: 0.52, roof: paint, pillar: paint, b_pillar: None, c_width: 0.42, rear_glass: false },
            );
            // stripes over the roof
            for sx in [-1.0f32, 1.0] {
                m.cube(trs(Vec3::new(sx * 0.19, 0.568, 0.7), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.09, 0.004, 0.33), rgb(stripe));
            }
            driver(&mut m, Vec3::new(-0.3, -0.18, 0.62), look.suit, look.helmet, paint, 0.95);
            m.bevel_box(trs(Vec3::new(0.3, -0.05, 0.8), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.19, 0.19, 0.05), 0.04, rgb(SEAT));
            // ducktail
            m.plank(Vec3::new(-0.7, 0.2, 1.98), Vec3::new(0.7, 0.2, 1.98), 0.2, 0.04, Vec3::new(0.0, 1.0, 0.6), rgb(paint));
            // long bonnet bulge + vents
            m.bevel_box(trs(Vec3::new(0.0, 0.06, -1.05), Quat::from_rotation_x(-0.03), Vec3::ONE), Vec3::new(0.32, 0.05, 0.7), 0.04, rgb(paint));
            for k in 0..3 {
                m.cube(trs(Vec3::new(0.0, 0.115, -0.7 - k as f32 * 0.14), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.2, 0.004, 0.025), rgb(DARK));
            }
            for sx in [-1.0f32, 1.0] {
                // twin round headlights each side
                round_lamp(&mut m, &mut head, Vec3::new(sx * 0.55, -0.12, -2.05), Vec3::NEG_Z, 0.1, CHROME, HEAD_OFF);
                round_lamp(&mut m, &mut head, Vec3::new(sx * 0.32, -0.14, -2.06), Vec3::NEG_Z, 0.07, CHROME, HEAD_OFF);
                // quad round tail lights
                round_lamp(&mut m, &mut brake, Vec3::new(sx * 0.55, 0.0, 2.04), Vec3::Z, 0.085, CHROME, TAIL_OFF);
                round_lamp(&mut m, &mut brake, Vec3::new(sx * 0.33, 0.0, 2.05), Vec3::Z, 0.085, CHROME, TAIL_OFF);
                round_lamp(&mut m, &mut rev, Vec3::new(sx * 0.16, -0.08, 2.055), Vec3::Z, 0.04, CHROME, REV_OFF);
                mirror(&mut m, sx, Vec3::new(sx * 0.72, 0.13, -0.05), paint);
                roundel(&mut m, Vec3::new(sx * 0.845, -0.08, 0.3), Vec3::X * sx, Vec3::Y, 0.21, look.number, [0.98, 0.97, 0.94], [0.25, 0.22, 0.3]);
                exhaust(&mut m, Vec3::new(sx * 0.45, -0.43, 2.0), 0.045, 0.26);
                // side vents behind the front wheels
                for k in 0..3 {
                    m.cube(trs(Vec3::new(sx * 0.835, -0.06 + k as f32 * 0.06, -0.62), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.01, 0.015, 0.12), rgb(DARK));
                }
            }
            // chrome bumpers + grille
            m.bevel_box(trs(Vec3::new(0.0, -0.33, -2.1), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.78, 0.05, 0.05), 0.03, rgb(CHROME));
            m.bevel_box(trs(Vec3::new(0.0, -0.33, 2.1), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.78, 0.05, 0.05), 0.03, rgb(CHROME));
            m.bevel_box(trs(Vec3::new(0.0, -0.2, -2.06), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.18, 0.08, 0.03), 0.02, rgb(DARK));
            for k in 0..3 {
                m.cube(trs(Vec3::new(0.0, -0.25 + k as f32 * 0.05, -2.09), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.16, 0.008, 0.005), rgb(CHROME));
            }
            plate(&mut m, Vec3::new(0.0, -0.18, 2.06), Vec3::Z);
            wheel = wheel_mesh(spec, CHROME, [0.95, 0.75, 0.3], 6, false);
        }
        2 => {
            // ---------------- Thistle: AWD rally sedan, big wing, mud flaps, spot lights
            let look = Look { number: 21, helmet: [0.98, 0.97, 0.94], suit: [0.95, 0.55, 0.35] };
            let secs = [
                sec(-2.03, -0.35, 0.0, 0.70, 0.64, 0.07, 0.09),
                sec(-1.95, -0.42, 0.06, 0.81, 0.75, 0.10, 0.12),
                sec(-1.20, -0.44, 0.11, 0.83, 0.77, 0.11, 0.13),
                sec(-0.45, -0.44, 0.15, 0.84, 0.78, 0.11, 0.13),
                sec(1.20, -0.44, 0.17, 0.84, 0.78, 0.11, 0.13),
                sec(1.95, -0.42, 0.17, 0.82, 0.76, 0.10, 0.12),
                sec(2.03, -0.35, 0.12, 0.71, 0.65, 0.07, 0.09),
            ];
            let bands = Bands { side: (0.25, 0.55), top: (0.10, 0.18) };
            body_loft(&mut m, &secs, &bands, &BodyPaint { paint, lower: trim, side_band: accent, top_band: paint, top: paint, cabin: (-0.45, 1.20) });
            cabin(
                &mut m,
                &secs,
                &Cabin { z0: -0.45, z1: 1.20, zr0: 0.08, zr1: 0.92, wr: 0.62, yr: 0.72, roof: paint, pillar: paint, b_pillar: Some(0.5), c_width: 0.34, rear_glass: false },
            );
            // roof: number roundel + air scoop
            roundel(&mut m, Vec3::new(0.0, 0.768, 0.62), Vec3::Y, Vec3::NEG_Z, 0.22, look.number, [0.98, 0.97, 0.94], [0.25, 0.22, 0.3]);
            m.bevel_box(trs(Vec3::new(0.0, 0.8, 0.18), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.16, 0.05, 0.12), 0.03, rgb(trim));
            m.quad(Vec3::new(-0.12, 0.77, 0.055), Vec3::new(0.12, 0.77, 0.055), Vec3::new(0.12, 0.84, 0.055), Vec3::new(-0.12, 0.84, 0.055), rgb(DARK));
            // roll cage hoop
            m.beam(Vec3::new(-0.55, 0.17, 0.95), Vec3::new(-0.55, 0.66, 0.85), 0.05, rgb([0.85, 0.85, 0.85]));
            m.beam(Vec3::new(0.55, 0.17, 0.95), Vec3::new(0.55, 0.66, 0.85), 0.05, rgb([0.85, 0.85, 0.85]));
            m.beam(Vec3::new(-0.55, 0.17, 0.95), Vec3::new(0.55, 0.66, 0.85), 0.05, rgb([0.85, 0.85, 0.85]));
            // driver + co-driver
            driver(&mut m, Vec3::new(-0.32, -0.04, 0.42), look.suit, look.helmet, paint, 1.0);
            driver(&mut m, Vec3::new(0.32, -0.04, 0.48), look.suit, shade(look.helmet, 0.94), accent, 1.0);
            // big rear wing
            let wing_y = 0.5;
            for sx in [-1.0f32, 1.0] {
                m.plank(Vec3::new(sx * 0.55, 0.17, 1.80), Vec3::new(sx * 0.55, wing_y, 1.9), 0.18, 0.04, Vec3::X, rgb(trim));
                m.cube(trs(Vec3::new(sx * 0.86, wing_y + 0.02, 1.92), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.02, 0.1, 0.17), rgb(trim));
            }
            m.bevel_box(trs(Vec3::new(0.0, wing_y + 0.03, 1.92), Quat::from_rotation_x(0.12), Vec3::ONE), Vec3::new(0.85, 0.03, 0.17), 0.02, rgb(paint));
            m.cube(trs(Vec3::new(0.0, wing_y + 0.065, 1.92), Quat::from_rotation_x(0.12), Vec3::ONE), Vec3::new(0.85, 0.004, 0.05), rgb(accent));
            for sx in [-1.0f32, 1.0] {
                // rectangular headlights + a bar of spot lights
                rect_lamp(&mut m, &mut head, Vec3::new(sx * 0.5, -0.06, -2.03), Vec3::NEG_Z, Vec3::X * sx, 0.14, 0.06, HEAD_OFF, HEAD_LENS);
                for k in 0..2 {
                    round_lamp(&mut m, &mut head, Vec3::new(sx * (0.16 + k as f32 * 0.3), -0.28, -2.12), Vec3::NEG_Z, 0.075, DARK, HEAD_OFF);
                }
                // wide tail lights with reverse inset
                rect_lamp(&mut m, &mut brake, Vec3::new(sx * 0.5, 0.0, 2.03), Vec3::Z, Vec3::X * sx, 0.16, 0.055, TAIL_OFF, LENS);
                rect_lamp(&mut m, &mut rev, Vec3::new(sx * 0.27, 0.0, 2.03), Vec3::Z, Vec3::X * sx, 0.05, 0.045, REV_OFF, LENS);
                mirror(&mut m, sx, Vec3::new(sx * 0.72, 0.2, -0.32), paint);
                roundel(&mut m, Vec3::new(sx * 0.845, -0.1, 0.35), Vec3::X * sx, Vec3::Y, 0.2, look.number, [0.98, 0.97, 0.94], [0.25, 0.22, 0.3]);
                // mud flaps behind every wheel
                for z in [fz, rz] {
                    mud_flap(&mut m, sx * spec.half_track, z + r + 0.1, wy + 0.05, wy - r + 0.07, spec.wheel_width + 0.08, [0.22, 0.22, 0.26], accent);
                }
            }
            m.bevel_box(trs(Vec3::new(0.0, -0.28, -2.12), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.62, 0.025, 0.04), 0.015, rgb(DARK));
            m.bevel_box(trs(Vec3::new(0.0, -0.36, -2.05), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.8, 0.08, 0.06), 0.04, rgb(DARK));
            m.bevel_box(trs(Vec3::new(0.0, -0.36, 2.05), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.8, 0.08, 0.06), 0.04, rgb(DARK));
            m.bevel_box(trs(Vec3::new(0.0, -0.16, -2.04), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.24, 0.06, 0.02), 0.02, rgb(DARK));
            plate(&mut m, Vec3::new(0.0, -0.17, 2.03), Vec3::Z);
            exhaust(&mut m, Vec3::new(0.5, -0.44, 2.0), 0.06, 0.25);
            antenna(&mut m, Vec3::new(0.0, 0.76, 0.9), 0.35, accent);
            wheel = wheel_mesh(spec, [0.98, 0.80, 0.30], DARK, 5, false);
        }
        _ => {
            // ---------------- Burrow: chunky 4x4 camper van, two-tone, rack, spare wheel
            let look = Look { number: 9, helmet: [0.98, 0.97, 0.94], suit: [0.45, 0.7, 0.45] };
            let secs = [
                sec(-2.12, -0.40, 0.12, 0.76, 0.72, 0.08, 0.10),
                sec(-2.04, -0.47, 0.18, 0.86, 0.82, 0.11, 0.13),
                sec(-1.55, -0.49, 0.23, 0.88, 0.84, 0.12, 0.14),
                sec(-1.10, -0.49, 0.25, 0.88, 0.84, 0.12, 0.14),
                sec(-0.30, -0.49, 0.26, 0.88, 0.84, 0.12, 0.14),
                sec(2.02, -0.49, 0.27, 0.88, 0.84, 0.12, 0.14),
                sec(2.10, -0.42, 0.22, 0.79, 0.75, 0.08, 0.10),
            ];
            let bands = Bands { side: (0.55, 0.75), top: (0.1, 0.2) };
            body_loft(&mut m, &secs, &bands, &BodyPaint { paint, lower, side_band: CREAM, top_band: paint, top: paint, cabin: (-1.10, -0.30) });
            // upper camper box (cream); its front underside forms the cab roof overhang
            let cream = CREAM;
            let ub = |z: f32, yb: f32, yt: f32, w: f32| sec(z, yb, yt, w, w - 0.05, 0.03, 0.13);
            let usecs = [
                ub(-0.66, 0.60, 0.96, 0.78),
                ub(-0.60, 0.58, 1.02, 0.84),
                ub(-0.32, 0.58, 1.04, 0.84),
                ub(-0.30, 0.24, 1.04, 0.84),
                ub(1.98, 0.24, 1.04, 0.84),
                ub(2.06, 0.26, 0.98, 0.78),
            ];
            let ubands = Bands { side: (0.0, 0.12), top: (0.1, 0.2) };
            let rings: Vec<Vec<Vec3>> = usecs.iter().map(|s| ring(s, &ubands)).collect();
            m.loft(&rings, |_, k| rgb(if kind(k) == 3 { paint } else if kind(k) == 0 { shade(cream, 0.85) } else { cream }), Some(rgb(cream)), Some(rgb(cream)));
            // A pillars + windshield
            let s0 = sec_at(&secs, -1.10);
            for sx in [-1.0f32, 1.0] {
                m.beam(Vec3::new(sx * 0.8, s0.yt, -1.08), Vec3::new(sx * 0.8, 0.6, -0.64), 0.08, rgb(cream));
            }
            let q = [Vec3::new(-0.74, s0.yt + 0.01, -1.06), Vec3::new(0.74, s0.yt + 0.01, -1.06), Vec3::new(0.74, 0.58, -0.63), Vec3::new(-0.74, 0.58, -0.63)];
            let out = (q[1] - q[0]).cross(q[3] - q[0]).normalize();
            glass_pane(&mut m, q, if out.z > 0.0 { -out } else { out });
            // camper windows with curtains, both sides
            let curtain = [[0.95, 0.6, 0.6], [0.6, 0.75, 0.95], [0.98, 0.85, 0.45]];
            for sx in [-1.0f32, 1.0] {
                for (k, (z0, z1)) in [(0.05f32, 0.75f32), (1.05, 1.75)].iter().enumerate() {
                    let x = sx * 0.835;
                    let n = Vec3::X * sx;
                    let pts = [Vec3::new(x, 0.45, *z0), Vec3::new(x, 0.45, *z1), Vec3::new(x, 0.86, *z1), Vec3::new(x, 0.86, *z0)];
                    m.panel(&pts, -0.035, 0.008, n, rgb(shade(paint, 0.85)));
                    m.panel(&pts, 0.0, 0.014, n, rgb(GLASS));
                    let cc = rgb(curtain[(k + if sx > 0.0 { 0 } else { 1 }) % 3]);
                    let cx = x + sx * 0.02;
                    m.quad(Vec3::new(cx, 0.47, *z0 + 0.02), Vec3::new(cx, 0.47, *z0 + 0.16), Vec3::new(cx, 0.84, *z0 + 0.12), Vec3::new(cx, 0.84, *z0 + 0.02), cc);
                    m.quad(Vec3::new(cx, 0.47, *z1 - 0.16), Vec3::new(cx, 0.47, *z1 - 0.02), Vec3::new(cx, 0.84, *z1 - 0.02), Vec3::new(cx, 0.84, *z1 - 0.12), cc);
                }
                mirror(&mut m, sx, Vec3::new(sx * 0.84, 0.36, -1.0), paint);
                // round headlights in chrome rings, round tail lights high on the body
                round_lamp(&mut m, &mut head, Vec3::new(sx * 0.56, -0.02, -2.10), Vec3::NEG_Z, 0.12, CHROME, HEAD_OFF);
                round_lamp(&mut m, &mut brake, Vec3::new(sx * 0.62, 0.05, 2.09), Vec3::Z, 0.085, CHROME, TAIL_OFF);
                round_lamp(&mut m, &mut rev, Vec3::new(sx * 0.62, -0.16, 2.09), Vec3::Z, 0.05, CHROME, REV_OFF);
                roundel(&mut m, Vec3::new(sx * 0.885, -0.14, -0.7), Vec3::X * sx, Vec3::Y, 0.2, look.number, [0.98, 0.97, 0.94], [0.25, 0.22, 0.3]);
                mud_flap(&mut m, sx * spec.half_track, rz + r + 0.1, wy + 0.05, wy - r + 0.1, spec.wheel_width + 0.08, DARK, cream);
            }
            // rear window with curtain + ladder + spare wheel on the back door
            let bw = [Vec3::new(-0.62, 0.5, 2.065), Vec3::new(-0.12, 0.5, 2.065), Vec3::new(-0.12, 0.86, 2.065), Vec3::new(-0.62, 0.86, 2.065)];
            m.panel(&bw, -0.03, 0.004, Vec3::Z, rgb(shade(paint, 0.85)));
            m.panel(&bw, 0.0, 0.01, Vec3::Z, rgb(GLASS));
            m.quad(Vec3::new(-0.6, 0.52, 2.08), Vec3::new(-0.44, 0.52, 2.08), Vec3::new(-0.48, 0.84, 2.08), Vec3::new(-0.6, 0.84, 2.08), rgb([0.95, 0.6, 0.6]));
            for sx in [0.5f32, 0.74] {
                m.beam(Vec3::new(sx, -0.1, 2.14), Vec3::new(sx, 1.1, 2.14), 0.035, rgb(DARK));
            }
            for k in 0..6 {
                let y = -0.02 + k as f32 * 0.21;
                m.beam(Vec3::new(0.5, y, 2.14), Vec3::new(0.74, y, 2.14), 0.03, rgb(DARK));
            }
            let spare = wheel_mesh(spec, [0.97, 0.96, 0.93], [0.9, 0.3, 0.3], 5, true);
            m.append_transformed(&spare, trs(Vec3::new(-0.12, -0.02, 2.12 + spec.wheel_width * 0.5), Quat::from_rotation_y(FRAC_PI_2), Vec3::ONE));
            m.disc(Vec3::new(-0.12, -0.02, 2.135 + spec.wheel_width), Vec3::Z, r * 0.62, 12, rgb(paint));
            m.disc(Vec3::new(-0.12, -0.02, 2.14 + spec.wheel_width), Vec3::Z, r * 0.3, 10, rgb(CREAM));
            // bull bar, bumpers, grille
            m.bevel_box(trs(Vec3::new(0.0, -0.36, -2.14), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.85, 0.09, 0.07), 0.04, rgb(DARK));
            m.bevel_box(trs(Vec3::new(0.0, -0.38, 2.12), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.85, 0.08, 0.07), 0.04, rgb(DARK));
            for sx in [-0.35f32, 0.35] {
                m.beam(Vec3::new(sx, -0.3, -2.2), Vec3::new(sx, 0.08, -2.2), 0.05, rgb(DARK));
            }
            m.beam(Vec3::new(-0.38, 0.08, -2.2), Vec3::new(0.38, 0.08, -2.2), 0.05, rgb(DARK));
            for k in 0..4 {
                m.cube(trs(Vec3::new(0.0, -0.18 + k as f32 * 0.06, -2.12), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.24, 0.012, 0.01), rgb(CHROME));
            }
            plate(&mut m, Vec3::new(0.32, -0.3, 2.19), Vec3::Z);
            // snorkel
            m.beam(Vec3::new(0.9, -0.1, -1.2), Vec3::new(0.9, 0.62, -1.12), 0.07, rgb(DARK));
            m.beam(Vec3::new(0.9, 0.62, -1.12), Vec3::new(0.9, 0.72, -1.22), 0.08, rgb(DARK));
            // roof rack + luggage + surfboard
            let rack_y = 1.1;
            for z in [-0.4f32, 0.4, 1.2, 1.9] {
                m.beam(Vec3::new(-0.72, rack_y, z), Vec3::new(0.72, rack_y, z), 0.04, rgb(DARK));
            }
            for sx in [-0.72f32, 0.72] {
                m.beam(Vec3::new(sx, rack_y, -0.5), Vec3::new(sx, rack_y, 2.0), 0.04, rgb(DARK));
                for z in [-0.4f32, 1.9] {
                    m.beam(Vec3::new(sx, 1.0, z), Vec3::new(sx, rack_y, z), 0.04, rgb(DARK));
                }
            }
            m.bevel_box(trs(Vec3::new(0.25, rack_y + 0.17, 0.2), Quat::from_rotation_y(0.15), Vec3::ONE), Vec3::new(0.3, 0.15, 0.4), 0.05, rgb([0.78, 0.52, 0.32]));
            m.cube(trs(Vec3::new(0.25, rack_y + 0.17, 0.2), Quat::from_rotation_y(0.15), Vec3::ONE), Vec3::new(0.31, 0.03, 0.41), rgb([0.5, 0.32, 0.2]));
            m.blob(trs(Vec3::new(-0.35, rack_y + 0.13, 0.25), Quat::from_rotation_y(0.4), Vec3::ONE), Vec3::new(0.24, 0.13, 0.32), 3, 7, rgb([0.45, 0.65, 0.88]));
            m.bevel_box(trs(Vec3::new(0.0, rack_y + 0.06, 1.35), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.24, 0.025, 0.85), 0.02, rgb(accent));
            m.cube(trs(Vec3::new(0.0, rack_y + 0.088, 1.35), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.03, 0.004, 0.8), rgb(paint));
            // cab: driver + a happy dog passenger
            driver(&mut m, Vec3::new(-0.36, 0.08, -0.62), look.suit, look.helmet, paint, 1.0);
            m.bevel_box(trs(Vec3::new(0.36, 0.22, -0.42), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.2, 0.2, 0.05), 0.04, rgb(SEAT));
            let dog = [0.85, 0.65, 0.42];
            m.blob(trs(Vec3::new(0.36, 0.28, -0.6), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.14, 0.17, 0.14), 3, 7, rgb(dog));
            m.blob(trs(Vec3::new(0.36, 0.52, -0.64), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.12, 0.11, 0.12), 3, 7, rgb(dog));
            m.blob(trs(Vec3::new(0.36, 0.49, -0.77), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.06, 0.05, 0.07), 2, 6, rgb(shade(dog, 1.1)));
            m.blob(trs(Vec3::new(0.36, 0.51, -0.83), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.025, 0.02, 0.02), 1, 4, rgb(DARK));
            for sx in [-1.0f32, 1.0] {
                m.blob(trs(Vec3::new(0.36 + sx * 0.1, 0.5, -0.62), Quat::from_rotation_z(sx * 0.5), Vec3::ONE), Vec3::new(0.035, 0.09, 0.05), 2, 5, rgb(shade(dog, 0.7)));
                m.blob(trs(Vec3::new(0.36 + sx * 0.05, 0.55, -0.745), Quat::IDENTITY, Vec3::ONE), Vec3::splat(0.018), 1, 4, rgb(DARK));
            }
            wheel = wheel_mesh(spec, [0.97, 0.96, 0.93], [0.9, 0.3, 0.3], 5, true);
        }
    }

    // wheel-arch flares over every wheel
    let flare = if spec.id == 2 || spec.id == 3 { [0.25, 0.24, 0.27] } else { paint };
    for sx in [-1.0f32, 1.0] {
        for z in [fz, rz] {
            let body_w = he.x.min(spec.half_track) - 0.12;
            arch(&mut m, sx * body_w, sx * (wheel_out + 0.03), z, wy, r + 0.04, r + 0.13, flare);
        }
    }
    // underbody tray so the road never shows through
    m.cube(trs(Vec3::new(0.0, wy + 0.02, 0.0), Quat::IDENTITY, Vec3::ONE), Vec3::new(spec.half_track - spec.wheel_width * 0.5 - 0.02, 0.04, he.z * 0.8), rgb(DARK));
    let _ = mix;
    CarModel { body: m, wheel, head_lights: head, brake_lights: brake, reverse_lights: rev }
}
