//! The four cars, built in car-local space (origin = CG, -Z forward, +Y up, +X right).
//!
//! Shared construction: a lofted, chamfered lower body (rings along Z), an open-sided cabin
//! (pillars + roof + windshield, so the little helmeted driver is visible), wheel-arch flares,
//! bumpers, lights and per-car accessories. Light lenses go into separate meshes so the game
//! can make them glow (brake / reverse / head); the body keeps the unlit lens housings.
use super::hull::{key, Arch, Cabin, Hull, Livery};
use super::parts::{roundel, MeshExt};
use crate::render::mesh::{mix, rgb, rgba, shade, trs, MeshBuilder};
use macroquad::math::{Mat4, Quat, Vec3};
use rally_core::vehicle::CarSpec;
use std::f32::consts::FRAC_PI_2;

#[allow(dead_code)] // light/glass meshes are wired up by the renderer at merge time
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
    /// Translucent window glass (vertex alpha ~0.4): draw with the ALPHA pipeline after the
    /// body so the little drivers show through. Without it the windows are simply open.
    pub glass: MeshBuilder,
}

const DARK: [f32; 3] = [0.21, 0.20, 0.23];
const CHROME: [f32; 3] = [0.86, 0.87, 0.89];
const CREAM: [f32; 3] = [0.98, 0.94, 0.84];
const SEAT: [f32; 3] = [0.26, 0.23, 0.28];
const GLASS: [f32; 3] = [0.58, 0.77, 0.88];
const GLINT: [f32; 3] = [0.86, 0.95, 0.99];
const TAIL_OFF: [f32; 3] = [0.66, 0.14, 0.16];
const REV_OFF: [f32; 3] = [0.88, 0.88, 0.86];
const HEAD_OFF: [f32; 3] = [0.95, 0.93, 0.82];
const LENS: [f32; 3] = [1.0, 1.0, 1.0];
const HEAD_LENS: [f32; 3] = [1.0, 0.98, 0.92];

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


/// Round lamp facing `n`: bezel+housing in body, bright lens in `lens_m`.
fn round_lamp(body: &mut MeshBuilder, lens_m: &mut MeshBuilder, c: Vec3, n: Vec3, r: f32, bezel: [f32; 3], off: [f32; 3]) {
    body.cylinder(Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::Y, n), c), r + 0.025, 0.02, 10, rgb(bezel), rgb(bezel));
    body.disc(c + n * 0.035, n, r, 10, rgb(off));
    lens_m.disc(c + n * 0.05, n, r * 0.92, 10, rgb(LENS));
}

/// Rectangular lamp on a face with normal n (right = in-plane x axis)
fn rect_lamp(body: &mut MeshBuilder, lens_m: &mut MeshBuilder, c: Vec3, n: Vec3, right: Vec3, hw: f32, hh: f32, off: [f32; 3], lens: [f32; 3]) {
    let up = n.cross(right).normalize();
    let q = |m: &mut MeshBuilder, e: f32, lift: f32, col: [u8; 4]| {
        let (a, b) = (right * (hw + e), up * (hh + e));
        let o = c + n * lift;
        m.quad(o - a - b, o + a - b, o + a + b, o - a + b, col);
    };
    q(body, 0.025, 0.01, rgb(DARK));
    q(body, 0.0, 0.025, rgb(off));
    q(lens_m, -0.008, 0.04, rgb(lens));
}

fn exhaust(m: &mut MeshBuilder, p: Vec3, r: f32, len: f32) {
    let rot = Quat::from_rotation_x(FRAC_PI_2);
    m.cylinder(trs(p, rot, Vec3::ONE), r, len * 0.5, 8, rgb([0.55, 0.55, 0.58]), rgb([0.12, 0.12, 0.14]));
    m.cylinder(trs(p + Vec3::Z * (len * 0.5 + 0.005), rot, Vec3::ONE), r * 1.45, 0.025, 8, rgb(CHROME), rgb([0.1, 0.1, 0.12]));
}

fn mirror(m: &mut MeshBuilder, sx: f32, base: Vec3, paint: [f32; 3]) {
    let tip = base + Vec3::new(sx * 0.14, 0.06, 0.0);
    m.beam(base, tip, 0.035, rgb(DARK));
    m.bevel_box(trs(tip + Vec3::new(sx * 0.04, 0.03, 0.0), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.07, 0.05, 0.04), 0.02, rgb(paint));
    m.quad(
        tip + Vec3::new(sx * -0.02, -0.01, -0.062),
        tip + Vec3::new(sx * 0.10, -0.01, -0.062),
        tip + Vec3::new(sx * 0.10, 0.07, -0.062),
        tip + Vec3::new(sx * -0.02, 0.07, -0.062),
        rgb(GLINT),
    );
}

fn plate(m: &mut MeshBuilder, c: Vec3, n: Vec3) {
    let right = Vec3::Y.cross(n).normalize();
    let (a, b) = (right * 0.17, Vec3::Y * 0.06);
    let o = c + n * 0.02;
    m.quad(o - a - b, o + a - b, o + a + b, o - a + b, rgb([0.98, 0.95, 0.80]));
    let o = c + n * 0.035;
    for k in 0..4 {
        let x = -0.11 + k as f32 * 0.075;
        let (p0, p1) = (o + right * x, o + right * (x + 0.045));
        m.quad(p0 - b * 0.5, p1 - b * 0.5, p1 + b * 0.5, p0 + b * 0.5, rgb([0.3, 0.3, 0.4]));
    }
}

fn mud_flap(m: &mut MeshBuilder, x: f32, z: f32, top: f32, bottom: f32, w: f32, color: [f32; 3], logo: [f32; 3]) {
    m.cube(trs(Vec3::new(x, (top + bottom) * 0.5, z), Quat::IDENTITY, Vec3::ONE), Vec3::new(w * 0.5, (top - bottom) * 0.5, 0.012), rgb(color));
    m.disc(Vec3::new(x, bottom + (top - bottom) * 0.35, z + 0.028), Vec3::Z, w * 0.25, 6, rgb(logo));
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
        (-hw, 0.64 * r),
        (-hw, 0.84 * r),
        (-hw * 0.84, 0.96 * r),
        (-hw * 0.55, r),
        (hw * 0.55, r),
        (hw * 0.84, 0.96 * r),
        (hw, 0.84 * r),
        (hw, 0.64 * r),
    ];
    let segs = 14;
    w.lathe_x(Mat4::IDENTITY, &prof, segs, |s, i| {
        rgb(match s {
            3 => {
                if i % 2 == 0 {
                    tread_l
                } else {
                    tire
                }
            }
            0 | 6 => wall,
            _ => tire,
        })
    });
    if knobby {
        for i in (0..segs).step_by(2) {
            let a = (i as f32 + 0.5) / segs as f32 * std::f32::consts::TAU;
            let off = if (i / 2) % 2 == 0 { hw * 0.3 } else { -hw * 0.3 };
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
        w.lathe_x(Mat4::IDENTITY, &[(face, 0.50 * r), (face + sx * 0.014, 0.56 * r), (face, 0.64 * r)], segs, |s, _| rgb(shade(rim, 1.0 - s as f32 * 0.12)));
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

// ---------------------------------------------------------------- flank / surface details




fn heart(m: &mut MeshBuilder, c: Vec3, n: Vec3, up: Vec3, s: f32, col: [f32; 3]) {
    let right = up.cross(n).normalize();
    m.disc(c + (right * -0.5 + up * 0.35) * s, n, s * 0.55, 8, rgb(col));
    m.disc(c + (right * 0.5 + up * 0.35) * s, n, s * 0.55, 8, rgb(col));
    m.tri(c + (right * -1.02 + up * 0.25) * s, c + (right * 1.02 + up * 0.25) * s, c - up * 1.1 * s, rgb(col));
}

fn leaf(m: &mut MeshBuilder, c: Vec3, n: Vec3, dir: Vec3, s: f32, col: [f32; 3]) {
    let side = n.cross(dir).normalize();
    let tip = c + dir * s;
    let tail = c - dir * s;
    let pts = [tail, c - dir * 0.4 * s + side * 0.42 * s, c + dir * 0.3 * s + side * 0.38 * s, tip, c + dir * 0.3 * s - side * 0.38 * s, c - dir * 0.4 * s - side * 0.42 * s];
    m.poly(&pts, rgb(col));
    let o = n * 0.015;
    m.quad(tail + o - side * 0.02 * s, tail + o + side * 0.02 * s, tip + o + side * 0.02 * s, tip + o - side * 0.02 * s, rgb(shade(col, 0.7)));
    m.beam(tail, tail - dir * 0.25 * s - side * 0.1 * s, 0.025 * s, rgb(shade(col, 0.7)));
}

fn sun(m: &mut MeshBuilder, c: Vec3, n: Vec3, up: Vec3, s: f32, col: [f32; 3]) {
    let right = up.cross(n).normalize();
    m.disc(c, n, s * 0.55, 10, rgb(col));
    for k in 0..8 {
        let a = k as f32 / 8.0 * std::f32::consts::TAU;
        let d = right * a.cos() + up * a.sin();
        let t = right * -a.sin() + up * a.cos();
        m.tri(c + d * s * 0.62 + t * s * 0.13, c + d * s * 0.62 - t * s * 0.13, c + d * s, rgb(col));
    }
}


/// Five-petal cherry blossom decal facing `n`
fn flower(m: &mut MeshBuilder, c: Vec3, n: Vec3, up: Vec3, s: f32, col: [f32; 3]) {
    let right = up.cross(n).normalize();
    let up = n.cross(right).normalize();
    for k in 0..5 {
        let a = k as f32 / 5.0 * std::f32::consts::TAU + 0.3;
        m.disc(c + (right * a.cos() + up * a.sin()) * s * 0.55, n, s * 0.42, 7, rgb(col));
    }
    m.disc(c + n * 0.012, n, s * 0.3, 7, rgb([0.98, 0.85, 0.4]));
}

fn tow_hook(m: &mut MeshBuilder, p: Vec3, dir: f32) {
    let c = rgb([0.92, 0.3, 0.3]);
    m.beam(p, p + Vec3::Z * dir * 0.08, 0.025, c);
    m.beam(p + Vec3::new(-0.04, 0.0, dir * 0.08), p + Vec3::new(0.04, 0.0, dir * 0.08), 0.025, c);
}

/// Frog-eye headlight pod sitting on the bonnet (Pip)
fn eye_pod(m: &mut MeshBuilder, head: &mut MeshBuilder, c: Vec3, r: f32, paint: [f32; 3]) {
    // pod: a short tapered barrel pointing forward, lamp in a chrome ring on its face
    let rot = Quat::from_rotation_x(-FRAC_PI_2);
    m.frustum(trs(c + Vec3::Z * 0.1, rot, Vec3::ONE), r * 1.05, r * 0.95, 0.13, 12, rgb(paint), rgb(paint));
    round_lamp(m, head, c - Vec3::Z * 0.03, Vec3::NEG_Z, r * 0.78, CHROME, HEAD_OFF);
}

/// Racing slick: smooth low-profile tyre (rim radius = `rim_frac` of the wheel radius),
/// multi-spoke rim with a centre-lock nut. Same origin/axis convention as `wheel_mesh`.
pub fn slick_wheel(spec: &CarSpec, rim: [f32; 3], accent: [f32; 3], spokes: usize, rim_frac: f32) -> MeshBuilder {
    let mut w = MeshBuilder::new();
    let r = spec.wheel_radius;
    let hw = spec.wheel_width * 0.5;
    let rf = rim_frac;
    let tire = [0.15, 0.14, 0.17];
    let wall = [0.2, 0.19, 0.22];
    let prof = [(-hw, rf * r), (-hw, (rf + (1.0 - rf) * 0.55) * r), (-hw * 0.9, 0.975 * r), (-hw * 0.7, r), (hw * 0.7, r), (hw * 0.9, 0.975 * r), (hw, (rf + (1.0 - rf) * 0.55) * r), (hw, rf * r)];
    let segs = 16;
    w.lathe_x(Mat4::IDENTITY, &prof, segs, |s, _| rgb(match s {
        0 | 6 => wall,
        _ => tire,
    }));
    // a coloured sidewall stripe (cozy compound marking)
    for sx in [-1.0f32, 1.0] {
        let x = sx * (hw + 0.014);
        w.lathe_x(Mat4::IDENTITY, &[(x, (rf + (1.0 - rf) * 0.25) * r), (x, (rf + (1.0 - rf) * 0.4) * r)], segs, |_, _| rgb(accent));
    }
    let rot = Quat::from_rotation_z(FRAC_PI_2);
    w.cylinder(trs(Vec3::ZERO, rot, Vec3::ONE), rf * r, hw * 0.85, segs, rgb(shade(rim, 0.55)), rgb(shade(rim, 0.45)));
    for sx in [-1.0f32, 1.0] {
        let face = sx * hw * 0.85;
        w.lathe_x(Mat4::IDENTITY, &[(face, (rf - 0.1) * r), (face + sx * 0.014, (rf - 0.05) * r), (face, rf * r)], segs, |_, _| rgb(rim));
        for k in 0..spokes {
            let a = k as f32 / spokes as f32 * std::f32::consts::TAU;
            let d = Vec3::new(0.0, a.cos(), a.sin());
            let t = Vec3::new(0.0, -a.sin(), a.cos());
            w.beam(Vec3::new(face, 0.0, 0.0) + d * 0.14 * r, Vec3::new(face, 0.0, 0.0) + d * (rf - 0.06) * r + t * 0.04 * r, 0.07 * r, rgb(rim));
        }
        w.cylinder(trs(Vec3::new(face + sx * 0.015, 0.0, 0.0), rot, Vec3::ONE), 0.16 * r, 0.025, 6, rgb(accent), rgb(accent));
    }
    w
}


// ---------------------------------------------------------------- hull-anchored details

fn h_mirror(m: &mut MeshBuilder, hull: &Hull, z: f32, sx: f32, paint: [f32; 3]) {
    let (p, _) = hull.point(z, sx, 5.0);
    mirror(m, sx, p - Vec3::Y * 0.01, paint);
}

fn h_roundel(m: &mut MeshBuilder, hull: &Hull, z: f32, sx: f32, u: f32, r: f32, num: u8) {
    let (c, n) = hull.point3(z, sx, u);
    let up = (Vec3::Y - n * n.y).normalize();
    roundel(m, c + n * 0.028, n, up, r, num, WHITE, INK);
}

fn h_handle(m: &mut MeshBuilder, hull: &Hull, z: f32, sx: f32, u: f32) {
    let (p, n) = hull.point3(z, sx, u);
    let rot = Quat::from_rotation_arc(Vec3::X, n);
    m.bevel_box(trs(p + n * 0.01, rot, Vec3::ONE), Vec3::new(0.018, 0.018, 0.07), 0.01, rgb(CHROME));
}

fn h_seams(m: &mut MeshBuilder, hull: &Hull, sx: f32, zs: &[f32]) {
    for &z in zs {
        hull.strip(m, z, sx, 2.2, 4.95, 0.008, 0.012, rgb(shade(DARK, 1.2)));
    }
}

/// Round lamp on the hull surface (bezel half-sunk into the skin)
fn h_lamp(m: &mut MeshBuilder, lens: &mut MeshBuilder, hull: &Hull, z: f32, sx: f32, u: f32, r: f32, bezel: [f32; 3], off: [f32; 3]) {
    let (p, n) = hull.point3(z, sx, u);
    round_lamp(m, lens, p + n * 0.004, n, r, bezel, off);
}

/// Bumper bar wrapping the nose (dir = -1) or tail (dir = +1), sunk into the skin
fn h_bumper(m: &mut MeshBuilder, hull: &Hull, dir: f32, y_up: f32, hh: f32, col: [f32; 3]) {
    let z = if dir < 0.0 { hull.front() + 0.1 } else { hull.rear() - 0.1 };
    let s = hull.sec(z);
    let y = s.ybot + y_up;
    let w = hull.side_x(z, y) + 0.015;
    m.bevel_box(trs(Vec3::new(0.0, y, z + dir * 0.04), Quat::IDENTITY, Vec3::ONE), Vec3::new(w, hh, 0.12), hh.min(0.04), rgb(col));
}

/// Mud flap hanging from the underside just behind a wheel, kept inside the body width
fn h_flap(m: &mut MeshBuilder, hull: &Hull, sx: f32, x: f32, z: f32, ground: f32, w: f32, col: [f32; 3], logo: [f32; 3]) {
    let top = hull.sec(z).ybot + 0.02;
    let bottom = ground + 0.1;
    if top - bottom < 0.05 {
        return;
    }
    let side = hull.side_x(z, top - 0.02) - 0.03;
    let w = w.min(side * 0.9);
    let x = x.min(side - w * 0.5);
    mud_flap(m, sx * x, z, top, bottom, w, col, logo);
}

/// Flat decal on the flank at (z, u)
fn h_frame(hull: &Hull, z: f32, sx: f32, u: f32, lift: f32) -> (Vec3, Vec3, Vec3) {
    let (c, n) = hull.point3(z, sx, u);
    let up = (Vec3::Y - n * n.y).normalize();
    (c + n * lift, n, up)
}

struct Look {
    number: u8,
    helmet: [f32; 3],
    suit: [f32; 3],
}

const INK: [f32; 3] = [0.25, 0.22, 0.3];
const WHITE: [f32; 3] = [0.98, 0.97, 0.94];

pub fn car_model(spec: &CarSpec, paint: [f32; 3]) -> CarModel {
    let mut m = MeshBuilder::new();
    let mut head = MeshBuilder::new();
    let mut brake = MeshBuilder::new();
    let mut rev = MeshBuilder::new();
    let mut glass = MeshBuilder::new();
    let _he = spec.half_extents;
    let r = spec.wheel_radius;
    // wheel centre height at rest (matches the setup preview: 80% of rest suspension length)
    let wy = spec.mount_height - spec.susp_rest * 0.8;
    let (fz, rz) = (-spec.cg_to_front, spec.cg_to_rear);
    let wheel_out = spec.half_track + spec.wheel_width * 0.5 + 0.02;
    let lower = shade(paint, 0.8);
    let bright = paint[0] + paint[1] + paint[2] > 2.6;
    let accent = if bright { [0.95, 0.45, 0.40] } else { WHITE };
    let trim = shade(paint, 0.62);

    let wheel;
    match spec.id {
        0 => {
            // ---------------- Pip: tiny round FWD hatch, frog-eye lamps on the bonnet, cream roof
            let look = Look { number: 7, helmet: WHITE, suit: [0.35, 0.55, 0.85] };
            let hull = Hull {
                keys: vec![
                    key(-1.7, -0.4, 0.03, 0.74, 0.14, 0.05),
                    key(-1.3, -0.42, 0.09, 0.76, 0.15, 0.04),
                    key(-0.5, -0.42, 0.13, 0.77, 0.16, 0.02),
                    key(0.6, -0.42, 0.15, 0.77, 0.16, 0.0),
                    key(1.45, -0.42, 0.15, 0.76, 0.15, 0.0),
                    key(1.66, -0.41, 0.13, 0.74, 0.13, 0.0),
                ],
                cabin: Some(Cabin { z0: -0.5, z1: -0.02, z2: 1.22, z3: 1.62, yr: 0.68, wr: 0.6, rail: 0.1, crown: 0.03, win: (-0.36, 1.32), b_pillar: Some(0.45), rear_solid: false, open: false }),
                arches: vec![Arch { z: fz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.03 - 0.76).max(0.05) }, Arch { z: rz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.03 - 0.77).max(0.05) }],
                nose_r: 0.12,
                tail_r: 0.1,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower, side_band: Some(accent), top_band: None, roof: CREAM, pillar: paint });
            let (front, rear) = (hull.front(), hull.rear());
            roundel(&mut m, Vec3::new(0.0, hull.top_y(0.62, 0.0) + 0.03, 0.62), Vec3::Y, Vec3::X, 0.25, look.number, paint, INK);
            antenna(&mut m, Vec3::new(-0.42, hull.top_y(1.0, 0.42) - 0.01, 1.0), 0.45, accent);
            // lip spoiler continuing the roof line over the hatch
            let zl = 1.24;
            let yl = hull.top_y(zl, 0.0);
            m.plank(Vec3::new(-0.5, yl, zl), Vec3::new(0.5, yl, zl), 0.2, 0.035, Vec3::new(0.0, 1.0, 0.25), rgb(CREAM));
            driver(&mut m, Vec3::new(-0.3, -0.06, 0.45), look.suit, look.helmet, paint, 1.0);
            m.bevel_box(trs(Vec3::new(0.3, 0.07, 0.62), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.2, 0.2, 0.05), 0.04, rgb(SEAT));
            heart(&mut m, Vec3::new(0.0, hull.top_y(-1.05, 0.0) + 0.015, -1.05), Vec3::new(0.0, 1.0, -0.12).normalize(), Vec3::NEG_Z, 0.13, accent);
            for sx in [-1.0f32, 1.0] {
                // frog-eye pods sitting on the bonnet shoulders
                let (pz, px) = (-1.5f32, 0.45f32);
                let py = hull.top_y(pz, px) + 0.08;
                eye_pod(&mut m, &mut head, Vec3::new(sx * px, py, pz), 0.13, paint);
                h_lamp(&mut m, &mut brake, &hull, rear - 0.04, sx, 3.9, 0.085, CHROME, TAIL_OFF);
                h_lamp(&mut m, &mut rev, &hull, rear - 0.04, sx, 3.1, 0.045, CHROME, REV_OFF);
                h_mirror(&mut m, &hull, -0.38, sx, paint);
                h_roundel(&mut m, &hull, 0.08, sx, 3.0, 0.19, look.number);
                h_seams(&mut m, &hull, sx, &[-0.5, 0.72]);
                h_handle(&mut m, &hull, 0.6, sx, 4.3);
                let (p, n) = hull.point3(-1.62, sx, 3.6);
                m.disc(p + n * 0.03, n, 0.03, 6, rgb([0.98, 0.7, 0.25]));
            }
            // smiley grille + cream bumpers
            let gy = hull.sec(front + 0.1).ybot + 0.16;
            m.bevel_box(trs(Vec3::new(0.0, gy, front + 0.06), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.2, 0.035, 0.06), 0.02, rgb(DARK));
            h_bumper(&mut m, &hull, -1.0, 0.06, 0.06, CREAM);
            h_bumper(&mut m, &hull, 1.0, 0.06, 0.06, CREAM);
            plate(&mut m, Vec3::new(0.0, hull.sec(rear).ybot + 0.2, rear), Vec3::Z);
            exhaust(&mut m, Vec3::new(0.42, hull.sec(rear - 0.1).ybot + 0.02, rear - 0.1), 0.04, 0.24);
            wheel = wheel_mesh(spec, WHITE, paint, 4, false);
        }
        1 => {
            // ---------------- Bramble: long-nose RWD coupe, wide centre stripe, fastback, ducktail
            let look = Look { number: 3, helmet: [0.98, 0.85, 0.35], suit: [0.92, 0.92, 0.88] };
            let stripe = accent;
            let hull = Hull {
                keys: vec![
                    key(-1.96, -0.4, -0.02, 0.8, 0.12, 0.06),
                    key(-1.3, -0.42, 0.05, 0.84, 0.14, 0.09),
                    key(-0.6, -0.42, 0.08, 0.85, 0.15, 0.06),
                    key(-0.12, -0.42, 0.09, 0.85, 0.15, 0.02),
                    key(1.25, -0.42, 0.12, 0.85, 0.15, 0.0),
                    key(1.93, -0.4, 0.18, 0.82, 0.12, 0.0),
                ],
                cabin: Some(Cabin { z0: -0.12, z1: 0.4, z2: 0.92, z3: 1.8, yr: 0.55, wr: 0.6, rail: 0.08, crown: 0.03, win: (0.0, 1.05), b_pillar: None, rear_solid: false, open: false }),
                arches: vec![Arch { z: fz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.03 - 0.84).max(0.05) }, Arch { z: rz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.03 - 0.85).max(0.05) }],
                nose_r: 0.12,
                tail_r: 0.07,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower, side_band: Some(stripe), top_band: Some(stripe), roof: paint, pillar: paint });
            let (front, rear) = (hull.front(), hull.rear());
            driver(&mut m, Vec3::new(-0.3, -0.17, 0.58), look.suit, look.helmet, paint, 0.95);
            m.bevel_box(trs(Vec3::new(0.3, -0.04, 0.74), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.19, 0.19, 0.05), 0.04, rgb(SEAT));
            for sx in [-1.0f32, 1.0] {
                // twin round headlights on the rounded nose, quad round tail lights
                h_lamp(&mut m, &mut head, &hull, front + 0.07, sx, 4.4, 0.095, CHROME, HEAD_OFF);
                h_lamp(&mut m, &mut head, &hull, front + 0.03, sx, 3.5, 0.065, CHROME, HEAD_OFF);
                for x in [0.55f32, 0.33] {
                    round_lamp(&mut m, &mut brake, Vec3::new(sx * x, 0.0, rear + 0.03), Vec3::Z, 0.075, CHROME, TAIL_OFF);
                }
                round_lamp(&mut m, &mut rev, Vec3::new(sx * 0.15, -0.01, rear + 0.03), Vec3::Z, 0.035, CHROME, REV_OFF);
                h_mirror(&mut m, &hull, 0.02, sx, paint);
                h_roundel(&mut m, &hull, 0.42, sx, 3.05, 0.2, look.number);
                h_seams(&mut m, &hull, sx, &[-0.1, 1.0]);
                h_handle(&mut m, &hull, 0.86, sx, 4.3);
                // gills behind the front wheels following the flank
                for k in 0..3 {
                    hull.strip(&mut m, -0.66 + k as f32 * 0.07, sx, 3.0, 4.0, 0.012, 0.012, rgb(DARK));
                }
                let (c, n, _) = h_frame(&hull, -0.95, sx, 3.5, 0.03);
                leaf(&mut m, c, n, Vec3::new(0.0, 0.25, -sx).normalize() * sx * sx, 0.14, [0.5, 0.75, 0.4]);
                exhaust(&mut m, Vec3::new(sx * 0.42, hull.sec(rear - 0.1).ybot + 0.02, rear - 0.1), 0.045, 0.24);
                // bonnet louvres lying on the bonnet
                for k in 0..4 {
                    let z = -1.05 - k as f32 * 0.1;
                    let y = hull.top_y(z, 0.45);
                    m.plank(Vec3::new(sx * 0.36, y + 0.01, z), Vec3::new(sx * 0.54, y + 0.01, z), 0.035, 0.01, Vec3::Y, rgb(DARK));
                }
            }
            // chrome bumpers + grille mouth set into the nose
            h_bumper(&mut m, &hull, -1.0, 0.05, 0.045, CHROME);
            h_bumper(&mut m, &hull, 1.0, 0.05, 0.045, CHROME);
            let gy = hull.sec(front + 0.1).ybot + 0.17;
            m.bevel_box(trs(Vec3::new(0.0, gy, front + 0.05), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.2, 0.06, 0.06), 0.03, rgb(DARK));
            for k in 0..3 {
                m.cube(trs(Vec3::new(0.0, gy - 0.04 + k as f32 * 0.04, front - 0.015), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.17, 0.007, 0.006), rgb(CHROME));
            }
            plate(&mut m, Vec3::new(0.0, hull.sec(rear).ybot + 0.2, rear), Vec3::Z);
            wheel = wheel_mesh(spec, CHROME, [0.95, 0.75, 0.3], 6, false);
        }
        2 => {
            // ---------------- Thistle: AWD three-box rally sedan, big wing, light pod, mud flaps
            let look = Look { number: 21, helmet: WHITE, suit: [0.95, 0.55, 0.35] };
            let hull = Hull {
                keys: vec![
                    key(-1.92, -0.42, 0.07, 0.81, 0.12, 0.03),
                    key(-1.2, -0.44, 0.12, 0.84, 0.15, 0.04),
                    key(-0.45, -0.44, 0.15, 0.85, 0.15, 0.02),
                    key(1.2, -0.44, 0.17, 0.85, 0.15, 0.0),
                    key(1.93, -0.42, 0.17, 0.83, 0.13, 0.0),
                ],
                cabin: Some(Cabin { z0: -0.43, z1: 0.1, z2: 0.88, z3: 1.25, yr: 0.74, wr: 0.62, rail: 0.09, crown: 0.03, win: (-0.3, 1.1), b_pillar: Some(0.47), rear_solid: false, open: false }),
                arches: vec![Arch { z: fz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.03 - 0.84).max(0.05) }, Arch { z: rz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.03 - 0.85).max(0.05) }],
                nose_r: 0.11,
                tail_r: 0.08,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower: trim, side_band: Some(accent), top_band: None, roof: paint, pillar: paint });
            let (front, rear) = (hull.front(), hull.rear());
            let ground = wy - r;
            // roof: number roundel + air scoop sitting on the roof
            let rt = hull.top_y(0.55, 0.0);
            roundel(&mut m, Vec3::new(0.0, rt + 0.012, 0.55), Vec3::Y, Vec3::X, 0.22, look.number, WHITE, INK);
            let st = hull.top_y(0.2, 0.0);
            m.bevel_box(trs(Vec3::new(0.0, st + 0.03, 0.2), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.15, 0.05, 0.11), 0.035, rgb(trim));
            m.quad(Vec3::new(-0.11, st + 0.02, 0.085), Vec3::new(0.11, st + 0.02, 0.085), Vec3::new(0.11, st + 0.07, 0.085), Vec3::new(-0.11, st + 0.07, 0.085), rgb(DARK));
            // roll cage meeting floor and roof
            let cage = rgb([0.85, 0.85, 0.85]);
            let cy = hull.top_y(0.8, 0.5) - 0.03;
            for sx in [-0.52f32, 0.52] {
                m.beam(Vec3::new(sx, 0.0, 0.85), Vec3::new(sx, cy, 0.8), 0.045, cage);
            }
            m.beam(Vec3::new(-0.52, cy, 0.8), Vec3::new(0.52, cy, 0.8), 0.045, cage);
            driver(&mut m, Vec3::new(-0.32, -0.04, 0.4), look.suit, look.helmet, paint, 1.0);
            driver(&mut m, Vec3::new(0.32, -0.04, 0.46), [0.4, 0.55, 0.85], shade(WHITE, 0.95), accent, 1.0);
            m.plank(Vec3::new(0.24, 0.2, 0.2), Vec3::new(0.42, 0.22, 0.2), 0.14, 0.01, Vec3::new(0.0, 0.5, -1.0), rgb([0.98, 0.95, 0.8]));
            // big rear wing on uprights standing on the boot lid
            let wz = rear - 0.22;
            let wing_y = hull.top_y(wz, 0.0) + 0.34;
            for sx in [-1.0f32, 1.0] {
                let base = Vec3::new(sx * 0.55, hull.top_y(wz - 0.05, 0.55) - 0.02, wz - 0.05);
                m.plank(base, Vec3::new(sx * 0.55, wing_y, wz), 0.14, 0.035, Vec3::X, rgb(trim));
                m.bevel_box(trs(Vec3::new(sx * 0.86, wing_y + 0.02, wz), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.018, 0.1, 0.19), 0.012, rgb(trim));
            }
            m.bevel_box(trs(Vec3::new(0.0, wing_y + 0.02, wz), Quat::from_rotation_x(0.12), Vec3::ONE), Vec3::new(0.84, 0.03, 0.17), 0.02, rgb(paint));
            m.cube(trs(Vec3::new(0.0, wing_y + 0.065, wz), Quat::from_rotation_x(0.12), Vec3::ONE), Vec3::new(0.8, 0.006, 0.05), rgb(accent));
            // light pod: a bar sitting on the bonnet nose carrying four spots
            let pz = front + 0.3;
            let py = hull.top_y(pz, 0.0);
            m.bevel_box(trs(Vec3::new(0.0, py + 0.02, pz), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.6, 0.03, 0.05), 0.02, rgb(DARK));
            for k in 0..4 {
                let x = -0.45 + k as f32 * 0.3;
                m.cylinder(trs(Vec3::new(x, py + 0.13, pz - 0.02), Quat::from_rotation_x(FRAC_PI_2), Vec3::ONE), 0.118, 0.06, 10, rgb(DARK), rgb(DARK));
                m.beam(Vec3::new(x, py + 0.02, pz), Vec3::new(x, py + 0.06, pz - 0.02), 0.04, rgb(DARK));
                round_lamp(&mut m, &mut head, Vec3::new(x, py + 0.13, pz - 0.085), Vec3::NEG_Z, 0.075, [0.3, 0.3, 0.33], HEAD_OFF);
            }
            for sx in [-1.0f32, 1.0] {
                // bonnet vents lying on the bonnet
                for k in 0..3 {
                    let z = -0.95 - k as f32 * 0.09;
                    let y = hull.top_y(z, 0.32);
                    m.plank(Vec3::new(sx * 0.22, y + 0.008, z), Vec3::new(sx * 0.42, y + 0.008, z), 0.035, 0.01, Vec3::Y, rgb(DARK));
                }
                h_lamp(&mut m, &mut head, &hull, front + 0.06, sx, 4.0, 0.075, CHROME, HEAD_OFF);
                // wide tail lights on the flat tail face, reverse lamps inboard
                rect_lamp(&mut m, &mut brake, Vec3::new(sx * 0.46, 0.0, rear + 0.004), Vec3::Z, Vec3::X * sx, 0.15, 0.05, TAIL_OFF, LENS);
                rect_lamp(&mut m, &mut rev, Vec3::new(sx * 0.24, 0.0, rear + 0.004), Vec3::Z, Vec3::X * sx, 0.045, 0.035, REV_OFF, LENS);
                h_mirror(&mut m, &hull, -0.3, sx, paint);
                h_roundel(&mut m, &hull, 0.0, sx, 3.0, 0.2, look.number);
                h_seams(&mut m, &hull, sx, &[-0.43, 0.47, 1.22]);
                h_handle(&mut m, &hull, 0.36, sx, 4.3);
                h_handle(&mut m, &hull, 1.1, sx, 4.3);
                let (c, n, up) = h_frame(&hull, 0.62, sx, 3.4, 0.03);
                sun(&mut m, c, n, up, 0.15, [0.98, 0.78, 0.3]);
                h_flap(&mut m, &hull, sx, spec.half_track, rz + r + 0.12, ground, spec.wheel_width + 0.06, [0.22, 0.22, 0.26], accent);
            }
            h_bumper(&mut m, &hull, -1.0, 0.08, 0.08, DARK);
            h_bumper(&mut m, &hull, 1.0, 0.08, 0.08, DARK);
            let yn = hull.sec(front + 0.25).ybot;
            m.plank(Vec3::new(0.0, yn - 0.012, front + 0.06), Vec3::new(0.0, yn - 0.012, front + 0.45), 1.1, 0.024, Vec3::Y, rgb([0.7, 0.7, 0.72]));
            plate(&mut m, Vec3::new(0.0, hull.sec(rear).ybot + 0.22, rear), Vec3::Z);
            exhaust(&mut m, Vec3::new(0.5, hull.sec(rear - 0.1).ybot + 0.02, rear - 0.1), 0.06, 0.26);
            antenna(&mut m, Vec3::new(0.0, hull.top_y(0.85, 0.0) - 0.01, 0.85), 0.35, accent);
            wheel = wheel_mesh(spec, [0.98, 0.80, 0.30], DARK, 5, false);
        }
        4 => {
            // ---------------- Blossom: pink turbo AWD hot hatch, blister arches, bonnet scoop,
            // roof spoiler with a high brake light, cherry-blossom livery
            let look = Look { number: 8, helmet: WHITE, suit: [0.98, 0.62, 0.78] };
            let white = WHITE;
            let deep = shade(paint, 0.72);
            let hull = Hull {
                keys: vec![
                    key(-1.72, -0.41, 0.04, 0.78, 0.13, 0.05),
                    key(-1.3, -0.43, 0.1, 0.8, 0.15, 0.05),
                    key(-0.5, -0.43, 0.13, 0.81, 0.16, 0.02),
                    key(1.42, -0.43, 0.15, 0.81, 0.16, 0.0),
                    key(1.72, -0.42, 0.15, 0.8, 0.14, 0.0),
                ],
                cabin: Some(Cabin { z0: -0.5, z1: 0.0, z2: 1.12, z3: 1.46, yr: 0.68, wr: 0.62, rail: 0.1, crown: 0.03, win: (-0.35, 1.25), b_pillar: Some(0.45), rear_solid: false, open: false }),
                arches: vec![Arch { z: fz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.06 - 0.8).max(0.08) }, Arch { z: rz, yc: wy, r: r + 0.05, flare: (wheel_out + 0.06 - 0.81).max(0.08) }],
                nose_r: 0.12,
                tail_r: 0.08,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower: deep, side_band: Some(white), top_band: Some(white), roof: paint, pillar: paint });
            let (front, rear) = (hull.front(), hull.rear());
            let ground = wy - r;
            // roof spoiler: continues from the roof's rear edge, end plates meet the roof
            let sz0 = 1.1;
            let ys = hull.top_y(sz0, 0.0);
            let sp_c = Vec3::new(0.0, ys + 0.03, sz0 + 0.16);
            m.bevel_box(trs(sp_c, Quat::from_rotation_x(-0.15), Vec3::ONE), Vec3::new(0.58, 0.025, 0.2), 0.02, rgb(white));
            for sx in [-1.0f32, 1.0] {
                let yb2 = hull.top_y(sz0 + 0.18, 0.56) - 0.01;
                m.bevel_box(trs(Vec3::new(sx * 0.58, (yb2 + ys + 0.06) * 0.5, sz0 + 0.12), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.02, ((ys + 0.06 - yb2) * 0.5).max(0.03), 0.2), 0.01, rgb(paint));
            }
            rect_lamp(&mut m, &mut brake, sp_c + Vec3::new(0.0, -0.005, 0.2), Vec3::Z, Vec3::X, 0.14, 0.016, TAIL_OFF, LENS);
            // bonnet scoop half-sunk into the bonnet
            let bz = -1.0;
            let by = hull.top_y(bz, 0.0);
            m.bevel_box(trs(Vec3::new(0.0, by + 0.02, bz), Quat::from_rotation_x(0.06), Vec3::ONE), Vec3::new(0.22, 0.06, 0.26), 0.05, rgb(paint));
            m.quad(Vec3::new(-0.16, by + 0.0, bz - 0.27), Vec3::new(0.16, by + 0.0, bz - 0.27), Vec3::new(0.16, by + 0.06, bz - 0.27), Vec3::new(-0.16, by + 0.06, bz - 0.27), rgb(DARK));
            flower(&mut m, Vec3::new(-0.42, hull.top_y(-0.8, 0.42) + 0.015, -0.8), Vec3::new(0.0, 1.0, 0.06).normalize(), Vec3::NEG_Z, 0.1, white);
            flower(&mut m, Vec3::new(-0.52, hull.top_y(-0.6, 0.52) + 0.015, -0.6), Vec3::new(0.0, 1.0, 0.04).normalize(), Vec3::NEG_Z, 0.065, white);
            let cage = rgb([0.92, 0.92, 0.92]);
            let cy = hull.top_y(0.85, 0.5) - 0.03;
            for sx in [-0.52f32, 0.52] {
                m.beam(Vec3::new(sx, -0.05, 0.9), Vec3::new(sx, cy, 0.85), 0.045, cage);
            }
            m.beam(Vec3::new(-0.52, cy, 0.85), Vec3::new(0.52, cy, 0.85), 0.045, cage);
            driver(&mut m, Vec3::new(-0.31, -0.07, 0.42), look.suit, look.helmet, paint, 1.0);
            driver(&mut m, Vec3::new(0.31, -0.07, 0.48), [0.95, 0.95, 0.92], look.helmet, deep, 1.0);
            for sx in [-1.0f32, 1.0] {
                h_lamp(&mut m, &mut head, &hull, front + 0.08, sx, 4.2, 0.12, CHROME, HEAD_OFF);
                h_lamp(&mut m, &mut head, &hull, front + 0.04, sx, 2.6, 0.05, DARK, [0.98, 0.9, 0.6]);
                h_lamp(&mut m, &mut brake, &hull, rear - 0.04, sx, 4.0, 0.08, CHROME, TAIL_OFF);
                h_lamp(&mut m, &mut rev, &hull, rear - 0.04, sx, 3.2, 0.04, CHROME, REV_OFF);
                h_mirror(&mut m, &hull, -0.38, sx, paint);
                h_roundel(&mut m, &hull, -0.13, sx, 3.0, 0.2, look.number);
                h_seams(&mut m, &hull, sx, &[-0.5, 0.62]);
                h_handle(&mut m, &hull, 0.53, sx, 4.3);
                let (c, n, up) = h_frame(&hull, 0.27, sx, 3.0, 0.03);
                flower(&mut m, c, n, up, 0.11, white);
                let (c, n, up) = h_frame(&hull, 0.42, sx, 3.9, 0.03);
                flower(&mut m, c, n, up, 0.06, white);
                hull.long_strip(&mut m, sx, 1.4, -0.6, 0.6, 0.03, 0.012, rgb(DARK));
                h_flap(&mut m, &hull, sx, spec.half_track, rz + r + 0.12, ground, spec.wheel_width + 0.06, deep, white);
            }
            h_bumper(&mut m, &hull, -1.0, 0.06, 0.06, paint);
            h_bumper(&mut m, &hull, 1.0, 0.07, 0.07, DARK);
            let gy = hull.sec(front + 0.1).ybot + 0.18;
            m.bevel_box(trs(Vec3::new(0.0, gy, front + 0.05), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.28, 0.065, 0.06), 0.03, rgb(DARK));
            let yn = hull.sec(front + 0.2).ybot;
            m.plank(Vec3::new(-0.7, yn - 0.012, front + 0.12), Vec3::new(0.7, yn - 0.012, front + 0.12), 0.24, 0.024, Vec3::Y, rgb(white));
            plate(&mut m, Vec3::new(0.0, hull.sec(rear).ybot + 0.22, rear), Vec3::Z);
            for sx in [-0.09f32, 0.09] {
                exhaust(&mut m, Vec3::new(sx, hull.sec(rear - 0.1).ybot + 0.03, rear - 0.1), 0.045, 0.22);
            }
            antenna(&mut m, Vec3::new(-0.42, hull.top_y(0.8, 0.42) - 0.01, 0.8), 0.4, [0.98, 0.75, 0.85]);
            wheel = wheel_mesh(spec, WHITE, paint, 6, false);
        }
        5 => {
            // ---------------- Comet (GT): low, wide, smooth fastback; rear-engine; swan-neck wing
            let look = Look { number: 24, helmet: WHITE, suit: [0.3, 0.45, 0.8] };
            let stripe = accent;
            let hull = Hull {
                keys: vec![
                    key(-2.12, -0.34, -0.06, 0.8, 0.1, 0.05),
                    key(-1.75, -0.36, 0.06, 0.86, 0.14, 0.07),
                    key(-1.1, -0.37, 0.13, 0.88, 0.16, 0.06),
                    key(-0.45, -0.37, 0.16, 0.9, 0.17, 0.03),
                    key(0.45, -0.37, 0.18, 0.92, 0.18, 0.02),
                    key(1.25, -0.36, 0.2, 0.94, 0.18, 0.03),
                    key(1.9, -0.33, 0.17, 0.9, 0.15, 0.03),
                    key(2.14, -0.3, 0.12, 0.84, 0.1, 0.02),
                ],
                cabin: Some(Cabin { z0: -0.55, z1: -0.02, z2: 0.5, z3: 1.55, yr: 0.58, wr: 0.6, rail: 0.09, crown: 0.04, win: (-0.42, 0.95), b_pillar: None, rear_solid: false, open: false }),
                arches: vec![Arch { z: fz, yc: wy, r: r + 0.045, flare: (wheel_out + 0.03 - 0.88).max(0.06) }, Arch { z: rz, yc: wy, r: r + 0.045, flare: (wheel_out + 0.04 - 0.94).max(0.06) }],
                nose_r: 0.12,
                tail_r: 0.09,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower: DARK, side_band: Some(stripe), top_band: Some(stripe), roof: paint, pillar: paint });
            let (front, rear) = (hull.front(), hull.rear());
            // twin faired headlights on the nose shoulders, slim tail-light bars
            for sx in [-1.0f32, 1.0] {
                for (k, (z, u)) in [(-1.98f32, 4.4f32), (-1.88, 5.4)].iter().enumerate() {
                    let (p, n) = hull.point3(*z, sx, *u);
                    round_lamp(&mut m, &mut head, p + n * 0.035, n, 0.07 - k as f32 * 0.01, DARK, HEAD_OFF);
                }
                // slim tail-light bar flat on the rear panel, kept inside the body outline
                // (the rear-most ring's real width at that height, minus the trim and a margin)
                let (p, _) = hull.point(rear, sx, 3.9);
                let hw = 0.17;
                let cx = (p.x.abs() - 0.07 - 0.025 - hw).max(hw + 0.3);
                rect_lamp(&mut m, &mut brake, Vec3::new(sx * cx, p.y - 0.02, rear + 0.004), Vec3::Z, Vec3::X * sx, hw, 0.03, TAIL_OFF, LENS);
                rect_lamp(&mut m, &mut rev, Vec3::new(sx * 0.22, hull.sec(rear).ybot + 0.14, rear + 0.004), Vec3::Z, Vec3::X * sx, 0.06, 0.025, REV_OFF, LENS);
                // mirror on a stalk from the door top, just behind the A pillar
                let (p, _) = hull.point(-0.38, sx, 5.0);
                mirror(&mut m, sx, p, paint);
                // door number board following the flank, door seams, side intake
                let (c, n) = hull.point3(0.15, sx, 3.0);
                let up = Vec3::Y - n * n.y;
                let up = up.normalize();
                let right = up.cross(n).normalize();
                let q = |m: &mut MeshBuilder, hw: f32, hh: f32, lift: f32, col: [f32; 3]| {
                    let o = c + n * lift;
                    m.quad(o - right * hw - up * hh, o + right * hw - up * hh, o + right * hw + up * hh, o - right * hw + up * hh, rgb(col));
                };
                q(&mut m, 0.3, 0.14, 0.015, WHITE);
                super::parts::digit(&mut m, c + n * 0.03 - right * 0.1, right, up, 0.22, look.number / 10, rgb(INK));
                super::parts::digit(&mut m, c + n * 0.03 + right * 0.1, right, up, 0.22, look.number % 10, rgb(INK));
                for z in [-0.5f32, 0.62] {
                    hull.strip(&mut m, z, sx, 2.15, 4.9, 0.008, 0.012, rgb(shade(DARK, 1.2)));
                }
                hull.long_strip(&mut m, sx, 3.45, 0.68, 0.98, 0.07, 0.015, rgb(DARK));
                // canard on the front bumper corner
                let (p, n) = hull.point3(front + 0.2, sx, 2.6);
                m.plank(p - n * 0.02, p + n * 0.1 + Vec3::Z * 0.06, 0.12, 0.018, Vec3::Y, rgb(DARK));
            }
            // splitter flush under the nose, diffuser fins under the tail
            let yn = hull.sec(front + 0.25).ybot;
            m.plank(Vec3::new(-0.78, yn - 0.012, front + 0.14), Vec3::new(0.78, yn - 0.012, front + 0.14), 0.3, 0.024, Vec3::Y, rgb(DARK));
            let yr0 = hull.sec(rear - 0.3).ybot;
            for k in 0..5 {
                let x = (k as f32 - 2.0) * 0.16;
                m.plank(Vec3::new(x, yr0 - 0.015, rear - 0.42), Vec3::new(x, yr0 - 0.015, rear - 0.08), 0.035, 0.015, Vec3::X, rgb([0.3, 0.3, 0.34]));
            }
            // engine-deck louvres following the deck
            for k in 0..5 {
                let z = 1.62 + k as f32 * 0.1;
                let y = hull.top_y(z, 0.0);
                m.plank(Vec3::new(-0.3, y + 0.012, z), Vec3::new(0.3, y + 0.012, z), 0.035, 0.012, Vec3::Y, rgb(DARK));
            }
            // swan-neck wing: stalks rise from the deck and hook onto the wing's top surface
            let (wz, wyy) = (rear - 0.2, hull.top_y(rear - 0.2, 0.0) + 0.34);
            let chord = 0.36;
            for sx in [-0.42f32, 0.42] {
                let base = Vec3::new(sx, hull.top_y(rear - 0.62, sx), rear - 0.62);
                let mid = Vec3::new(sx, wyy + 0.1, rear - 0.42);
                let top = Vec3::new(sx, wyy + 0.03, wz - 0.02);
                m.plank(base - Vec3::Y * 0.02, mid, 0.1, 0.035, Vec3::X, rgb(DARK));
                m.plank(mid, top, 0.1, 0.035, Vec3::X, rgb(DARK));
            }
            let wing_rot = Quat::from_rotation_x(0.12);
            m.bevel_box(trs(Vec3::new(0.0, wyy, wz), wing_rot, Vec3::ONE), Vec3::new(0.86, 0.022, chord * 0.5), 0.018, rgb(paint));
            m.cube(trs(Vec3::new(0.0, wyy + 0.03, wz + 0.03), wing_rot, Vec3::ONE), Vec3::new(0.8, 0.006, 0.05), rgb(stripe));
            for sx in [-1.0f32, 1.0] {
                m.bevel_box(trs(Vec3::new(sx * 0.875, wyy - 0.02, wz), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.015, 0.1, chord * 0.55), 0.01, rgb(DARK));
            }
            // roof roundel, driver, roll cage meeting the roof
            roundel(&mut m, Vec3::new(0.0, hull.top_y(0.25, 0.0) + 0.012, 0.25), Vec3::Y, Vec3::X, 0.17, look.number, WHITE, INK);
            let floor = hull.sec(0.1).ybot;
            driver(&mut m, Vec3::new(-0.3, floor + 0.16, 0.12), look.suit, look.helmet, paint, 0.95);
            let cage = rgb([0.92, 0.92, 0.92]);
            let ry = hull.top_y(0.45, 0.4) - 0.04;
            for sx in [-0.4f32, 0.4] {
                m.beam(Vec3::new(sx, floor + 0.05, 0.5), Vec3::new(sx, ry, 0.45), 0.04, cage);
            }
            m.beam(Vec3::new(-0.4, ry, 0.45), Vec3::new(0.4, ry, 0.45), 0.04, cage);
            exhaust(&mut m, Vec3::new(0.0, hull.sec(rear).ybot + 0.08, rear - 0.1), 0.055, 0.22);
            plate(&mut m, hull.point3(rear - 0.06, 1.0, 1.9).0 * Vec3::new(0.0, 1.0, 1.0), Vec3::Z);
            wheel = slick_wheel(spec, [0.85, 0.86, 0.9], accent, 10, 0.72);
        }
        6 => {
            // ---------------- Zephyr (hypercar): teardrop canopy, long tapering tail with a fin,
            // active wing, aero channels, LED light bars
            let look = Look { number: 77, helmet: WHITE, suit: [0.25, 0.25, 0.3] };
            let dark2 = [0.18, 0.18, 0.22];
            let hull = Hull {
                keys: vec![
                    key(-2.18, -0.33, -0.12, 0.82, 0.1, 0.05),
                    key(-1.8, -0.35, -0.03, 0.9, 0.14, 0.08),
                    key(-1.2, -0.35, 0.05, 0.92, 0.16, 0.05),
                    key(-0.45, -0.35, 0.07, 0.84, 0.2, 0.0),
                    key(0.55, -0.35, 0.1, 0.88, 0.2, 0.0),
                    key(1.35, -0.34, 0.13, 0.96, 0.17, 0.02),
                    key(1.95, -0.31, 0.11, 0.9, 0.14, 0.02),
                    key(2.24, -0.27, 0.06, 0.8, 0.1, 0.0),
                ],
                cabin: Some(Cabin { z0: -1.2, z1: -0.6, z2: -0.1, z3: 1.75, yr: 0.43, wr: 0.48, rail: 0.13, crown: 0.05, win: (-1.0, 0.25), b_pillar: None, rear_solid: true, open: false }),
                arches: vec![Arch { z: fz, yc: wy, r: r + 0.045, flare: (wheel_out + 0.03 - 0.92).max(0.06) }, Arch { z: rz, yc: wy, r: r + 0.045, flare: (wheel_out + 0.04 - 0.96).max(0.06) }],
                nose_r: 0.13,
                tail_r: 0.07,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower: dark2, side_band: Some(dark2), top_band: Some(accent), roof: [0.22, 0.24, 0.32], pillar: paint });
            let (front, rear) = (hull.front(), hull.rear());
            for sx in [-1.0f32, 1.0] {
                // LED bars: a slim wraparound nose bar and a full-width tail bar
                hull.long_strip(&mut m, sx, 4.75, front + 0.06, front + 0.5, 0.035, 0.012, rgb(DARK));
                hull.long_strip(&mut head, sx, 4.75, front + 0.08, front + 0.48, 0.018, 0.05, rgb(HEAD_LENS));
                hull.strip(&mut m, rear - 0.035, sx, 2.6, 5.3, 0.03, 0.012, rgb(TAIL_OFF));
                hull.strip(&mut brake, rear - 0.035, sx, 2.7, 5.2, 0.018, 0.05, rgb(LENS));
                // aero channels along the flanks, side intake, mirrors on stalks
                hull.long_strip(&mut m, sx, 3.7, -0.85, 0.85, 0.035, 0.04, rgb(accent));
                let (p, _) = hull.point(-0.95, sx, 5.0);
                mirror(&mut m, sx, p, paint);
                // door roundel on the flank
                let (c, n) = hull.point3(-0.35, sx, 2.9);
                let up = (Vec3::Y - n * n.y).normalize();
                roundel(&mut m, c + n * 0.015, n, up, 0.17, look.number, WHITE, INK);
            }
            let (p, n) = hull.point3(rear - 0.04, 1.0, 2.6);
            rect_lamp(&mut m, &mut rev, Vec3::new(0.0, p.y, p.z) - n * 0.01, Vec3::Z, Vec3::X, 0.08, 0.02, REV_OFF, LENS);
            // dorsal fin rising from the canopy spine down the tail
            let fin: Vec<Vec3> = (0..=6).map(|k| {
                let z = 0.2 + k as f32 * 0.3;
                Vec3::new(0.0, hull.top_y(z, 0.0) - 0.01, z)
            }).collect();
            for w2 in fin.windows(2) {
                let (a, b) = (w2[0], w2[1]);
                let t = (a.z - 0.2) / 1.8;
                let h = 0.05 + 0.2 * (t * std::f32::consts::PI).sin().max(0.0) * (1.0 - t * 0.3);
                let tb = (b.z - 0.2) / 1.8;
                let hb = 0.05 + 0.2 * (tb * std::f32::consts::PI).sin().max(0.0) * (1.0 - tb * 0.3);
                m.quad(a, b, b + Vec3::Y * hb, a + Vec3::Y * h, rgb(paint));
            }
            // active wing on slim struts standing on the tail deck
            let wz = rear - 0.22;
            let wyy = hull.top_y(wz, 0.0) + 0.22;
            for sx in [-0.5f32, 0.5] {
                let base = Vec3::new(sx, hull.top_y(wz - 0.05, sx), wz - 0.05);
                m.plank(base - Vec3::Y * 0.02, Vec3::new(sx, wyy, wz), 0.07, 0.02, Vec3::X, rgb(DARK));
            }
            m.bevel_box(trs(Vec3::new(0.0, wyy + 0.02, wz), Quat::from_rotation_x(0.2), Vec3::ONE), Vec3::new(0.82, 0.018, 0.15), 0.014, rgb(paint));
            m.cube(trs(Vec3::new(0.0, wyy + 0.045, wz), Quat::from_rotation_x(0.2), Vec3::ONE), Vec3::new(0.78, 0.005, 0.03), rgb(accent));
            // splitter + diffuser flush with the underside
            let yn = hull.sec(front + 0.3).ybot;
            m.plank(Vec3::new(-0.8, yn - 0.012, front + 0.16), Vec3::new(0.8, yn - 0.012, front + 0.16), 0.32, 0.024, Vec3::Y, rgb(DARK));
            let yr0 = hull.sec(rear - 0.3).ybot;
            for k in 0..7 {
                let x = (k as f32 - 3.0) * 0.13;
                m.plank(Vec3::new(x, yr0 - 0.015, rear - 0.45), Vec3::new(x, yr0 - 0.015, rear - 0.08), 0.035, 0.015, Vec3::X, rgb([0.3, 0.3, 0.34]));
            }
            let floor = hull.sec(-0.5).ybot;
            driver(&mut m, Vec3::new(0.0, floor + 0.14, -0.45), look.suit, look.helmet, accent, 0.95);
            roundel(&mut m, Vec3::new(0.0, hull.top_y(-0.35, 0.0) + 0.012, -0.35), Vec3::Y, Vec3::X, 0.12, look.number, WHITE, INK);
            for sx in [-0.12f32, 0.12] {
                exhaust(&mut m, Vec3::new(sx, hull.top_y(rear - 0.12, sx) - 0.05, rear - 0.12), 0.04, 0.14);
            }
            wheel = slick_wheel(spec, [0.25, 0.26, 0.3], accent, 12, 0.74);
        }
        7 => {
            // ---------------- Dragonfly (formula): needle nose, sculpted sidepods narrowing to a
            // coke-bottle rear, airbox behind the driver, halo, wings on pylons/endplates
            let look = Look { number: 9, helmet: [0.98, 0.85, 0.35], suit: [0.95, 0.95, 0.92] };
            let yb = wy - 0.17;
            let hull = Hull {
                keys: vec![
                    key(-2.38, yb + 0.06, wy - 0.06, 0.07, 0.04, 0.0),
                    key(-2.0, yb + 0.04, wy - 0.01, 0.1, 0.05, 0.02),
                    key(-1.45, yb + 0.02, wy + 0.07, 0.16, 0.07, 0.03),
                    key(-0.85, yb, wy + 0.14, 0.24, 0.09, 0.03),
                    key(-0.5, yb, wy + 0.1, 0.5, 0.12, 0.1),
                    key(0.15, yb, wy + 0.1, 0.56, 0.14, 0.11),
                    key(0.75, yb, wy + 0.06, 0.38, 0.12, 0.13),
                    key(1.3, yb + 0.01, wy + 0.02, 0.2, 0.08, 0.1),
                    key(1.72, yb + 0.02, wy - 0.02, 0.12, 0.05, 0.04),
                ],
                cabin: Some(Cabin { z0: -0.95, z1: -0.9, z2: -0.3, z3: -0.25, yr: wy + 0.2, wr: 0.19, rail: 0.03, crown: 0.0, win: (0.0, 0.0), b_pillar: None, rear_solid: true, open: true }),
                arches: vec![],
                nose_r: 0.06,
                tail_r: 0.05,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower: DARK, side_band: Some(accent), top_band: Some(accent), roof: paint, pillar: paint });
            let (front, rear) = (hull.front(), hull.rear());
            // airbox + engine-cover spine: a second slim hull rising behind the driver
            let spine = Hull {
                keys: vec![
                    key(-0.22, wy + 0.02, wy + 0.36, 0.13, 0.06, 0.04),
                    key(0.15, wy + 0.02, wy + 0.32, 0.12, 0.06, 0.03),
                    key(0.8, wy + 0.0, wy + 0.18, 0.09, 0.05, 0.02),
                    key(1.35, wy - 0.02, wy + 0.07, 0.06, 0.03, 0.01),
                ],
                cabin: None,
                arches: vec![],
                nose_r: 0.04,
                tail_r: 0.05,
            };
            spine.build(&mut m, &mut glass, &Livery { paint, lower: paint, side_band: None, top_band: Some(accent), roof: paint, pillar: paint });
            let (ai, _) = spine.point3(spine.front() + 0.01, 1.0, 6.5);
            m.disc(Vec3::new(0.0, ai.y - 0.04, spine.front() - 0.004), Vec3::NEG_Z, 0.065, 8, rgb(DARK));
            // cockpit rim + driver
            let floor = hull.sec(-0.6).ybot;
            driver(&mut m, Vec3::new(0.0, floor + 0.05, -0.55), look.suit, look.helmet, paint, 0.92);
            // halo: front anchor on the chassis spine, two rear anchors behind the cockpit
            let halo = rgb([0.2, 0.2, 0.24]);
            let fa = Vec3::new(0.0, hull.top_y(-1.0, 0.0), -1.0);
            let hy = wy + 0.42;
            let apex = Vec3::new(0.0, hy, -0.92);
            m.beam(fa, apex, 0.045, halo);
            let pts: Vec<Vec3> = (0..=6).map(|k| {
                let t = k as f32 / 6.0;
                let ang = t * std::f32::consts::FRAC_PI_2;
                Vec3::new(ang.sin() * 0.25, hy - t * 0.03, -0.92 + (1.0 - ang.cos()) * 0.5)
            }).collect();
            for sx in [-1.0f32, 1.0] {
                for w2 in pts.windows(2) {
                    m.beam(w2[0] * Vec3::new(sx, 1.0, 1.0), w2[1] * Vec3::new(sx, 1.0, 1.0), 0.045, halo);
                }
                let end = *pts.last().unwrap() * Vec3::new(sx, 1.0, 1.0);
                let anchor = Vec3::new(sx * 0.25, hull.top_y(end.z, 0.25), end.z);
                m.beam(end, anchor - Vec3::Y * 0.01, 0.045, halo);
            }
            glass.quad(fa + Vec3::new(-0.12, 0.0, 0.03), fa + Vec3::new(0.12, 0.0, 0.03), apex + Vec3::new(0.09, -0.06, 0.0), apex + Vec3::new(-0.09, -0.06, 0.0), rgba(GLASS, 0.45));
            // front wing hung under the nose on two pylons, endplates at the tips
            let wing_y = wy - 0.28;
            let wz = front + 0.2;
            for sx in [-0.06f32, 0.06] {
                let top = Vec3::new(sx, hull.sec(wz + 0.05).ybot + 0.01, wz + 0.05);
                m.plank(top, Vec3::new(sx, wing_y + 0.02, wz - 0.02), 0.16, 0.022, Vec3::X, rgb(DARK));
            }
            m.plank(Vec3::new(-0.92, wing_y, wz), Vec3::new(0.92, wing_y, wz), 0.34, 0.026, Vec3::new(0.0, 1.0, 0.06), rgb(paint));
            m.plank(Vec3::new(-0.9, wing_y + 0.06, wz + 0.14), Vec3::new(-0.2, wing_y + 0.06, wz + 0.14), 0.12, 0.02, Vec3::new(0.0, 1.0, 0.5), rgb(accent));
            m.plank(Vec3::new(0.2, wing_y + 0.06, wz + 0.14), Vec3::new(0.9, wing_y + 0.06, wz + 0.14), 0.12, 0.02, Vec3::new(0.0, 1.0, 0.5), rgb(accent));
            for sx in [-1.0f32, 1.0] {
                m.bevel_box(trs(Vec3::new(sx * 0.93, wing_y + 0.07, wz + 0.04), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.012, 0.09, 0.22), 0.008, rgb(DARK));
            }
            // rear wing: central pylon from the crash structure, endplates carry both elements
            let rwz = rear - 0.05;
            let rwy = wy + 0.55;
            let pylon_base = Vec3::new(0.0, hull.top_y(rear - 0.25, 0.0), rear - 0.25);
            m.plank(pylon_base - Vec3::Y * 0.02, Vec3::new(0.0, rwy - 0.01, rwz), 0.14, 0.03, Vec3::X, rgb(DARK));
            m.plank(Vec3::new(-0.6, rwy, rwz), Vec3::new(0.6, rwy, rwz), 0.3, 0.026, Vec3::new(0.0, 1.0, 0.15), rgb(paint));
            m.plank(Vec3::new(-0.6, rwy + 0.12, rwz + 0.13), Vec3::new(0.6, rwy + 0.12, rwz + 0.13), 0.14, 0.022, Vec3::new(0.0, 1.0, 0.6), rgb(accent));
            for sx in [-1.0f32, 1.0] {
                m.bevel_box(trs(Vec3::new(sx * 0.615, rwy + 0.02, rwz + 0.05), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.012, 0.17, 0.25), 0.008, rgb(DARK));
            }
            // rain light on the crash structure (glows with the brake lights)
            rect_lamp(&mut m, &mut brake, Vec3::new(0.0, hull.sec(rear).ybot + 0.06, rear - 0.01), Vec3::Z, Vec3::X, 0.045, 0.035, TAIL_OFF, LENS);
            // suspension: wishbones from the chassis side to an upright at the wheel centre
            for sx in [-1.0f32, 1.0] {
                for z in [fz, rz] {
                    let ux = sx * (spec.half_track - spec.wheel_width * 0.5 - 0.03);
                    m.bevel_box(trs(Vec3::new(ux, wy, z), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.03, 0.09, 0.05), 0.012, rgb(DARK));
                    for dy in [-0.06f32, 0.06] {
                        let y = wy + dy;
                        for dz in [-0.17f32, 0.17] {
                            let cz = z + dz;
                            let cx = sx * (hull.side_x(cz, y) - 0.01);
                            m.beam(Vec3::new(cx, y, cz), Vec3::new(ux, y, z), 0.026, rgb(DARK));
                        }
                    }
                }
                // sidepod mirrors on short stalks
                let base = Vec3::new(sx * 0.4, hull.top_y(-0.4, 0.4), -0.4);
                mirror(&mut m, sx, base, paint);
                // numbers on the sidepods
                let (c, n) = hull.point3(0.1, sx, 3.2);
                let up = (Vec3::Y - n * n.y).normalize();
                roundel(&mut m, c + n * 0.015, n, up, 0.12, look.number, WHITE, INK);
            }
            roundel(&mut m, Vec3::new(0.0, hull.top_y(-1.65, 0.0) + 0.012, -1.65), Vec3::Y, Vec3::X, 0.08, look.number, WHITE, INK);
            // floor plank flush under the chassis
            m.plank(Vec3::new(0.0, yb - 0.012, -0.7), Vec3::new(0.0, yb - 0.012, 1.4), 1.0, 0.022, Vec3::Y, rgb(DARK));
            wheel = slick_wheel(spec, [0.92, 0.92, 0.95], accent, 6, 0.6);
        }
        _ => {
            // ---------------- Burrow: chunky 4x4 camper, two-tone, rack, spare wheel, dog
            let look = Look { number: 9, helmet: WHITE, suit: [0.45, 0.7, 0.45] };
            let cream = CREAM;
            let hull = Hull {
                keys: vec![
                    key(-2.02, -0.47, 0.18, 0.86, 0.13, 0.03),
                    key(-1.55, -0.49, 0.23, 0.88, 0.14, 0.03),
                    key(-1.1, -0.49, 0.25, 0.88, 0.15, 0.0),
                    key(2.0, -0.49, 0.27, 0.88, 0.15, 0.0),
                ],
                cabin: Some(Cabin { z0: -1.1, z1: -0.62, z2: 1.98, z3: 2.04, yr: 1.04, wr: 0.78, rail: 0.13, crown: 0.02, win: (-0.98, -0.36), b_pillar: None, rear_solid: true, open: false }),
                arches: vec![Arch { z: fz, yc: wy, r: r + 0.06, flare: (wheel_out + 0.04 - 0.88).max(0.08) }, Arch { z: rz, yc: wy, r: r + 0.06, flare: (wheel_out + 0.04 - 0.88).max(0.08) }],
                nose_r: 0.1,
                tail_r: 0.06,
            };
            hull.build(&mut m, &mut glass, &Livery { paint, lower, side_band: Some(cream), top_band: None, roof: cream, pillar: cream });
            let (front, rear) = (hull.front(), hull.rear());
            let ground = wy - r;
            // camper windows with curtains on the box sides (frame / glass / curtains)
            let curtain = [[0.95, 0.6, 0.6], [0.6, 0.75, 0.95], [0.98, 0.85, 0.45]];
            for sx in [-1.0f32, 1.0] {
                for (k, (z0, z1)) in [(0.05f32, 0.75f32), (1.05, 1.75)].iter().enumerate() {
                    let pt = |z: f32, u: f32, lift: f32| {
                        let (p, n) = hull.point3(z, sx, u);
                        p + n * lift
                    };
                    let (ua, ub) = (5.25f32, 5.8f32);
                    m.quad(pt(*z0 - 0.04, ua - 0.06, 0.02), pt(*z1 + 0.04, ua - 0.06, 0.02), pt(*z1 + 0.04, ub + 0.06, 0.02), pt(*z0 - 0.04, ub + 0.06, 0.02), rgb(shade(paint, 0.85)));
                    m.quad(pt(*z0, ua, 0.045), pt(*z1, ua, 0.045), pt(*z1, ub, 0.045), pt(*z0, ub, 0.045), rgb(GLASS));
                    let cc = rgb(curtain[(k + if sx > 0.0 { 0 } else { 1 }) % 3]);
                    m.quad(pt(*z0 + 0.02, ua + 0.02, 0.07), pt(*z0 + 0.15, ua + 0.02, 0.07), pt(*z0 + 0.11, ub - 0.02, 0.07), pt(*z0 + 0.02, ub - 0.02, 0.07), cc);
                    m.quad(pt(*z1 - 0.15, ua + 0.02, 0.07), pt(*z1 - 0.02, ua + 0.02, 0.07), pt(*z1 - 0.02, ub - 0.02, 0.07), pt(*z1 - 0.11, ub - 0.02, 0.07), cc);
                }
                // flower box under the rear window
                let (p, n) = hull.point3(1.4, sx, 5.15);
                m.bevel_box(trs(p + n * 0.05, Quat::IDENTITY, Vec3::ONE), Vec3::new(0.05, 0.04, 0.3), 0.015, rgb([0.72, 0.5, 0.32]));
                for k in 0..5 {
                    let c = [[0.95, 0.5, 0.6], [0.98, 0.9, 0.4], [0.95, 0.95, 1.0]][k % 3];
                    m.blob(trs(p + n * 0.06 + Vec3::new(0.0, 0.07, -0.24 + k as f32 * 0.12), Quat::IDENTITY, Vec3::ONE), Vec3::splat(0.04), 1, 5, rgb(c));
                }
                h_mirror(&mut m, &hull, -0.95, sx, paint);
                round_lamp(&mut m, &mut head, Vec3::new(sx * 0.56, -0.02, front + 0.005), Vec3::NEG_Z, 0.12, CHROME, HEAD_OFF);
                round_lamp(&mut m, &mut brake, Vec3::new(sx * 0.62, 0.05, rear - 0.005), Vec3::Z, 0.085, CHROME, TAIL_OFF);
                round_lamp(&mut m, &mut rev, Vec3::new(sx * 0.62, -0.16, rear - 0.005), Vec3::Z, 0.05, CHROME, REV_OFF);
                h_roundel(&mut m, &hull, -0.7, sx, 3.0, 0.19, look.number);
                h_seams(&mut m, &hull, sx, &[-1.08, -0.31]);
                h_handle(&mut m, &hull, -0.45, sx, 4.4);
                let (c, n, up) = h_frame(&hull, 0.35, sx, 2.9, 0.03);
                sun(&mut m, c, n, up, 0.2, [0.98, 0.78, 0.3]);
                h_flap(&mut m, &hull, sx, spec.half_track, rz + r + 0.12, ground, spec.wheel_width + 0.08, DARK, cream);
                // side step bolted under the sill
                let sy = hull.sec(0.0).ybot;
                let sxp = hull.side_x(0.0, sy + 0.02);
                m.bevel_box(trs(Vec3::new(sx * (sxp + 0.04), sy - 0.02, 0.0), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.07, 0.022, 0.6), 0.012, rgb([0.6, 0.6, 0.62]));
            }
            // back door: window with curtain, ladder, spare wheel
            let bw = [Vec3::new(-0.62, 0.5, rear + 0.012), Vec3::new(-0.12, 0.5, rear + 0.012), Vec3::new(-0.12, 0.84, rear + 0.012), Vec3::new(-0.62, 0.84, rear + 0.012)];
            m.panel(&bw, -0.03, 0.0, Vec3::Z, rgb(shade(paint, 0.85)));
            m.panel(&bw, 0.0, 0.015, Vec3::Z, rgb(GLASS));
            m.quad(Vec3::new(-0.6, 0.52, rear + 0.042), Vec3::new(-0.44, 0.52, rear + 0.042), Vec3::new(-0.48, 0.82, rear + 0.042), Vec3::new(-0.6, 0.82, rear + 0.042), rgb([0.95, 0.6, 0.6]));
            for x in [0.5f32, 0.74] {
                m.beam(Vec3::new(x, -0.1, rear + 0.08), Vec3::new(x, 1.08, rear + 0.08), 0.035, rgb(DARK));
                for y in [0.0f32, 0.95] {
                    m.beam(Vec3::new(x, y, rear), Vec3::new(x, y, rear + 0.08), 0.03, rgb(DARK));
                }
            }
            for k in 0..6 {
                let y = -0.02 + k as f32 * 0.21;
                m.beam(Vec3::new(0.5, y, rear + 0.08), Vec3::new(0.74, y, rear + 0.08), 0.03, rgb(DARK));
            }
            let spare = wheel_mesh(spec, WHITE, [0.9, 0.3, 0.3], 5, false);
            let sz = rear + spec.wheel_width * 0.5 + 0.015;
            m.append_transformed(&spare, trs(Vec3::new(-0.12, -0.02, sz), Quat::from_rotation_y(FRAC_PI_2), Vec3::ONE));
            m.disc(Vec3::new(-0.12, -0.02, sz + spec.wheel_width * 0.5 + 0.012), Vec3::Z, r * 0.66, 12, rgb(paint));
            sun(&mut m, Vec3::new(-0.12, -0.02, sz + spec.wheel_width * 0.5 + 0.027), Vec3::Z, Vec3::Y, 0.14, CREAM);
            // bumpers, bull bar tied into the front bumper, grille on the nose
            h_bumper(&mut m, &hull, -1.0, 0.1, 0.09, DARK);
            h_bumper(&mut m, &hull, 1.0, 0.1, 0.08, DARK);
            let bby = hull.sec(front + 0.1).ybot + 0.1;
            for sx in [-0.35f32, 0.35] {
                m.beam(Vec3::new(sx, bby, front - 0.12), Vec3::new(sx, 0.1, front - 0.12), 0.05, rgb(DARK));
                m.beam(Vec3::new(sx, 0.1, front - 0.12), Vec3::new(sx, 0.1, front + 0.02), 0.04, rgb(DARK));
            }
            m.beam(Vec3::new(-0.38, 0.1, front - 0.12), Vec3::new(0.38, 0.1, front - 0.12), 0.05, rgb(DARK));
            m.bevel_box(trs(Vec3::new(0.0, -0.08, front + 0.01), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.28, 0.13, 0.03), 0.03, rgb(CREAM));
            for k in 0..4 {
                m.cube(trs(Vec3::new(0.0, -0.17 + k as f32 * 0.06, front - 0.035), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.24, 0.012, 0.01), rgb(DARK));
            }
            tow_hook(&mut m, Vec3::new(0.0, bby - 0.02, front - 0.1), -1.0);
            plate(&mut m, Vec3::new(0.32, hull.sec(rear).ybot + 0.2, rear), Vec3::Z);
            // snorkel strapped beside the A pillar
            // snorkel running up alongside the A pillar, strapped to the wing and the pillar
            let (pa, na) = hull.point3(-1.08, 1.0, 5.0);
            let (pb, nb) = hull.point3(-0.7, 1.0, 5.9);
            let s_base = pa + na * 0.05 + Vec3::new(0.0, -0.05, 0.0);
            let s_top = pb + nb * 0.05;
            m.beam(s_base, s_top, 0.07, rgb(DARK));
            m.beam(s_top, s_top + Vec3::new(0.0, 0.08, -0.08), 0.08, rgb(DARK));
            for t in [0.2f32, 0.75] {
                let q = s_base.lerp(s_top, t);
                m.beam(q, q - na * 0.09, 0.03, rgb(DARK));
            }
            // roof rack on legs standing on the roof, luggage, surfboard, light bar
            let rack_y = hull.top_y(0.8, 0.0) + 0.12;
            for z in [-0.4f32, 0.4, 1.2, 1.9] {
                m.beam(Vec3::new(-0.66, rack_y, z), Vec3::new(0.66, rack_y, z), 0.04, rgb(DARK));
            }
            for sx in [-0.66f32, 0.66] {
                m.beam(Vec3::new(sx, rack_y, -0.5), Vec3::new(sx, rack_y, 2.0), 0.04, rgb(DARK));
                for z in [-0.4f32, 1.9] {
                    m.beam(Vec3::new(sx, hull.top_y(z, sx) - 0.01, z), Vec3::new(sx, rack_y, z), 0.04, rgb(DARK));
                }
            }
            m.bevel_box(trs(Vec3::new(0.25, rack_y + 0.17, 0.2), Quat::from_rotation_y(0.15), Vec3::ONE), Vec3::new(0.3, 0.15, 0.4), 0.05, rgb([0.78, 0.52, 0.32]));
            m.cube(trs(Vec3::new(0.25, rack_y + 0.17, 0.2), Quat::from_rotation_y(0.15), Vec3::ONE), Vec3::new(0.31, 0.03, 0.41), rgb([0.5, 0.32, 0.2]));
            m.blob(trs(Vec3::new(-0.35, rack_y + 0.13, 0.25), Quat::from_rotation_y(0.4), Vec3::ONE), Vec3::new(0.24, 0.13, 0.32), 3, 7, rgb([0.45, 0.65, 0.88]));
            m.bevel_box(trs(Vec3::new(0.0, rack_y + 0.045, 1.35), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.24, 0.025, 0.85), 0.02, rgb(accent));
            m.cube(trs(Vec3::new(0.0, rack_y + 0.085, 1.35), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.03, 0.006, 0.8), rgb(paint));
            m.bevel_box(trs(Vec3::new(0.0, rack_y + 0.04, -0.48), Quat::IDENTITY, Vec3::ONE), Vec3::new(0.5, 0.04, 0.04), 0.015, rgb(DARK));
            for k in 0..4 {
                round_lamp(&mut m, &mut head, Vec3::new(-0.36 + k as f32 * 0.24, rack_y + 0.04, -0.53), Vec3::NEG_Z, 0.045, DARK, HEAD_OFF);
            }
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
            wheel = wheel_mesh(spec, WHITE, [0.9, 0.3, 0.3], 5, true);
        }
    }

    let _ = mix;
    CarModel { body: m, wheel, head_lights: head, brake_lights: brake, reverse_lights: rev, glass }
}
