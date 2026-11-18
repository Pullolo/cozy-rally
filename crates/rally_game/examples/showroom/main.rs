//! Dev showroom for the procedural models (not part of the game build).
//!
//!   SHOW=cars  OUT=/tmp/cars.png  xvfb-run -a -s "-screen 0 1600x900x24" cargo run --release -p rally_game --example showroom
//!   SHOW=props BIOME=1 OUT=/tmp/props.png ...      (catalogue of every prop kind for a biome)
//!   SHOW=stats ...                                  (vertex counts for cars + props on several tracks)
//! Optional: LIGHTS=1 renders the brake/head/reverse lenses lit.
#![allow(dead_code)]
mod render;
#[path = "../../src/models/mod.rs"]
mod models;

use macroquad::prelude::*;
use rally_core::biome::Biome;
use rally_core::track::{LayoutKind, Prop, PropKind, Track, TrackParams};
use rally_core::vehicle::car_specs;
use render::gpu::{DrawParams, Environment, Gpu, View};
use render::mesh::MeshBuilder;

fn window_conf() -> Conf {
    Conf { window_title: "showroom".into(), window_width: 1600, window_height: 900, sample_count: 4, ..Default::default() }
}

fn env() -> Environment {
    Environment { sun_dir: vec3(-0.45, 0.75, -0.35).normalize(), sun_color: [1.05, 0.97, 0.85], ambient: [0.45, 0.48, 0.56], fog_color: [0.8, 0.89, 0.95], fog_density: 0.0, time: 0.0 }
}

/// Car spec by id; id 4 (Blossom) falls back to a provisional hot-hatch spec until the
/// gameplay branch adds the real one to `car_specs()`.
fn spec_of(car: usize) -> rally_core::vehicle::CarSpec {
    let specs = car_specs();
    if let Some(s) = specs.iter().find(|s| s.id as usize == car) {
        return s.clone();
    }
    let mut s = specs[2].clone();
    s.id = 4;
    s.name = "Blossom";
    s.half_extents = Vec3::new(0.88, 0.58, 1.9);
    s.cg_to_front = 1.18;
    s.cg_to_rear = 1.18;
    s.half_track = 0.80;
    s.mount_height = -0.03;
    s.wheel_radius = 0.32;
    s.wheel_width = 0.24;
    s.susp_rest = 0.38;
    s.susp_travel = 0.26;
    s
}

fn paint_of(car: usize) -> usize {
    if car == 4 { models::PAINT_PINK } else { car % 4 }
}

fn draw_car(gpu: &mut Gpu, v: &View, car: usize, paint: usize, model: Mat4, lights: bool) {
    let spec = spec_of(car);
    let m = models::car_model(&spec, models::PAINTS[paint]);
    let e = env();
    gpu.draw_builder(&m.body, v, &e, model, &DrawParams { rim: 0.5, ..Default::default() }, 0);
    let lit = |c: [f32; 4]| DrawParams { unlit: 1.0, tint: c, ..Default::default() };
    if lights {
        gpu.draw_builder(&m.brake_lights, v, &e, model, &lit([1.0, 0.25, 0.2, 1.0]), 0);
        gpu.draw_builder(&m.head_lights, v, &e, model, &lit([1.0, 0.97, 0.85, 1.0]), 0);
        gpu.draw_builder(&m.reverse_lights, v, &e, model, &lit([1.0, 1.0, 1.0, 1.0]), 0);
    }
    for (x, z) in [(-1.0, -spec.cg_to_front), (1.0, -spec.cg_to_front), (-1.0, spec.cg_to_rear), (1.0, spec.cg_to_rear)] {
        let p = vec3(x * (spec.half_track + 0.02), spec.mount_height - spec.susp_rest * 0.8, z);
        let steer = if z < 0.0 { 0.25 } else { 0.0 };
        gpu.draw_builder(&m.wheel, v, &e, model * Mat4::from_translation(p) * Mat4::from_rotation_y(steer) * Mat4::from_rotation_x(0.3), &DrawParams::default(), 0);
    }
    if std::env::var("NOGLASS").is_err() {
        gpu.draw_builder(&m.glass, v, &e, model, &DrawParams { rim: 0.6, ..Default::default() }, 1);
    }
}

fn ground(gpu: &mut Gpu, v: &View, y: f32, r: f32, c: [f32; 3]) {
    let mut disc = MeshBuilder::new();
    disc.cylinder(Mat4::from_translation(vec3(0.0, y - 0.03, 0.0)), r, 0.03, 32, render::mesh::rgb(c), render::mesh::rgb(c));
    gpu.draw_builder(&disc, v, &env(), Mat4::IDENTITY, &DrawParams::default(), 0);
}

fn viewport(gpu: &mut Gpu, x: f32, y: f32, w: f32, h: f32) {
    gpu.begin_3d(x as i32, y as i32, w as i32, h as i32, true);
}

fn shot(path: &str) {
    unsafe { get_internal_gl().flush() };
    get_screen_data().export_png(path);
    println!("saved {path}");
}

/// Count overlapping, differently-coloured triangle pairs whose planes are closer than `eps`
/// (these z-fight on real GPUs at distance). Returns (pairs, examples).
fn coplanar_fights(m: &MeshBuilder, eps: f32) -> (usize, Vec<String>) {
    let mut tris: Vec<([Vec3; 3], Vec3, f32, [u8; 3])> = Vec::new();
    for ch in &m.chunks {
        for t in ch.indices.chunks(3) {
            let p: [Vec3; 3] = [0, 1, 2].map(|k| Vec3::from(ch.vertices[t[k] as usize].pos));
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            if n.length() < 1e-7 {
                continue;
            }
            let n = n.normalize();
            let c = ch.vertices[t[0] as usize].color;
            tris.push((p, n, n.dot(p[0]), [c[0], c[1], c[2]]));
        }
    }
    let mut count = 0;
    let mut ex = Vec::new();
    for i in 0..tris.len() {
        for j in i + 1..tris.len() {
            let (a, na, da, ca) = &tris[i];
            let (b, nb, db, cb) = &tris[j];
            if ca == cb {
                continue;
            }
            let dot = na.dot(*nb);
            // only same-facing faces can show flicker (opposite = two solids touching, hidden)
            if dot < 0.9995 {
                continue;
            }
            // faces lying on the ground plane facing down are never seen
            if na.y < -0.99 && a[0].y.abs() < 0.03 {
                continue;
            }
            let dbb = if dot < 0.0 { -db } else { *db };
            if (da - dbb).abs() >= eps {
                continue;
            }
            // project to the plane and test 2D overlap (SAT), shrinking slightly to ignore shared edges
            let u = na.any_orthonormal_vector();
            let v = na.cross(u);
            let pr = |t: &[Vec3; 3]| {
                let c = (t[0] + t[1] + t[2]) / 3.0;
                t.map(|p| {
                    let q = c + (p - c) * 0.97;
                    vec2(q.dot(u), q.dot(v))
                })
            };
            let (ta, tb) = (pr(a), pr(b));
            let mut sep = false;
            for t in [&ta, &tb] {
                for k in 0..3 {
                    let e = t[(k + 1) % 3] - t[k];
                    let ax = vec2(-e.y, e.x);
                    let (mut a0, mut a1, mut b0, mut b1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
                    for p in ta {
                        let d = p.dot(ax);
                        a0 = a0.min(d);
                        a1 = a1.max(d);
                    }
                    for p in tb {
                        let d = p.dot(ax);
                        b0 = b0.min(d);
                        b1 = b1.max(d);
                    }
                    if a1 <= b0 || b1 <= a0 {
                        sep = true;
                        break;
                    }
                }
                if sep {
                    break;
                }
            }
            if !sep {
                count += 1;
                if ex.len() < std::env::var("EX").ok().and_then(|s| s.parse().ok()).unwrap_or(3) {
                    let (cA, cB) = ((a[0] + a[1] + a[2]) / 3.0, (b[0] + b[1] + b[2]) / 3.0);
                    ex.push(format!("A({:.2},{:.2},{:.2}) B({:.2},{:.2},{:.2}) n({:.1},{:.1},{:.1}) gap {:.4} {:?}/{:?}", cA.x, cA.y, cA.z, cB.x, cB.y, cB.z, na.x, na.y, na.z, (da - dbb).abs(), ca, cb));
                }
            }
        }
    }
    (count, ex)
}

pub fn catalogue(biome: Biome) -> Vec<Prop> {
    use PropKind::*;
    let filt = std::env::var("KINDS").ok();
    let kinds: Vec<(PropKind, u8)> = models::catalogue_kinds(biome)
        .into_iter()
        .filter(|(k, _)| filt.as_ref().map_or(true, |f| f.split(',').any(|n| n == format!("{:?}", k))))
        .collect();
    let cols = 7;
    let mut v = Vec::new();
    for (i, (k, var)) in kinds.iter().enumerate() {
        let (cx, cz) = ((i % cols) as f32, (i / cols) as f32);
        let _ = Tree;
        v.push(Prop { kind: *k, pos: vec3(-(cx - (cols - 1) as f32 * 0.5) * 11.0, 0.0, cz * 11.0), rot_y: 0.6, scale: 1.0, radius: 0.0, variant: *var });
    }
    v
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut gpu = Gpu::new();
    let show = std::env::var("SHOW").unwrap_or_else(|_| "cars".into());
    let out = std::env::var("OUT").unwrap_or_else(|_| "showroom.png".into());
    let lights = std::env::var("LIGHTS").is_ok();
    for frame in 0..3 {
        clear_background(Color::new(0.80, 0.89, 0.95, 1.0));
        let _ = vec2(0.0, 0.0);
        let (sw, sh) = (screen_width(), screen_height());
        match show.as_str() {
            "cars" | "closeup" => {
                let close = show == "closeup";
                let only: Option<usize> = std::env::var("CAR").ok().and_then(|s| s.parse().ok());
                let cars: Vec<usize> = match only {
                    Some(c) => vec![c],
                    None => (0..5).collect(),
                };
                let n = cars.len() as f32;

                for (ci, &car) in cars.iter().enumerate() {
                    for row in 0..2 {
                        let (w, h) = if n == 1.0 { (sw / 2.0, sh) } else { (sw / n, sh / 2.0) };
                        let (vx, vy) = if n == 1.0 { (row as f32 * w, 0.0) } else { (ci as f32 * w, row as f32 * h) };
                        viewport(&mut gpu, vx, vy, w, h);
                        let sides = std::env::var("SIDES").is_ok();
                        let ang: f32 = if sides { if row == 0 { -1.5708 } else { 1.5708 } } else if row == 0 { -0.75 } else { 2.55 };
                        let dist: f32 = std::env::var("DIST").ok().and_then(|s| s.parse().ok()).unwrap_or(if close { 5.6 } else { 6.6 });
                        let eye = vec3(ang.sin() * dist, if row == 0 { 1.6 } else { 2.3 }, -ang.cos() * dist);
                        let eye = if sides { eye } else if row == 0 { vec3(eye.x, eye.y, -eye.z.abs()) } else { vec3(eye.x, eye.y, eye.z.abs()) };
                        let view = Mat4::look_at_rh(eye, vec3(0.0, 0.0, 0.0), Vec3::Y);
                        let proj = Mat4::perspective_rh_gl(38f32.to_radians(), w / h, 0.1, 100.0);
                        let v = View::new(view, proj, eye);
                        let spec = spec_of(car);
                        ground(&mut gpu, &v, spec.mount_height - spec.susp_rest * 0.8 - spec.wheel_radius, 3.4, [0.98, 0.9, 0.78]);
                        draw_car(&mut gpu, &v, car, paint_of(car), Mat4::IDENTITY, lights);
                        gpu.end_3d();
                    }
                }
            }
            "corner" => {
                // driver's-eye view of a real track corner with chevrons (terrain + road strip + props)
                let b: usize = std::env::var("BIOME").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
                let k: usize = std::env::var("K").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
                let what = std::env::var("KIND").unwrap_or_else(|_| "ArrowSign".into());
                let layout = if std::env::var("LAYOUT").map_or(false, |v| v == "p2p") { LayoutKind::PointToPoint { length_m: 1500.0 } } else { LayoutKind::Loop };
                let len: u8 = std::env::var("LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
                let t = Track::generate(&TrackParams { seed: 3, biome: Some(Biome::from_index(b.max(1) - 1)), length_class: len, layout, ..Default::default() });
                let st = &t.style;
                let target = t.props.iter().filter(|p| format!("{:?}", p.kind) == what).nth(k).cloned();
                let Some(sign) = target else {
                    println!("no {what} #{k}");
                    return;
                };
                let q = t.query(sign.pos, None);
                let s = t.samples[q.index];
                let dist: f32 = std::env::var("BACK").ok().and_then(|v| v.parse().ok()).unwrap_or(18.0);
                let eye = s.pos - s.tangent * dist + Vec3::Y * 2.6;
                let look = if std::env::var("AIM").is_ok() { sign.pos + Vec3::Y * 1.5 } else { s.pos + s.tangent * 25.0 };
                let mut m = MeshBuilder::new();
                let hf = &t.terrain;
                let c = vec2(eye.x, eye.z);
                for iz in 0..hf.nz - 1 {
                    for ix in 0..hf.nx - 1 {
                        let p00 = hf.vertex_pos(ix, iz);
                        if vec2(p00.x, p00.z).distance(c) > 220.0 {
                            continue;
                        }
                        let p10 = hf.vertex_pos(ix + 1, iz);
                        let p01 = hf.vertex_pos(ix, iz + 1);
                        let p11 = hf.vertex_pos(ix + 1, iz + 1);
                        let col = render::mesh::rgb(st.ground_a);
                        m.tri(p00, p11, p10, col);
                        m.tri(p00, p01, p11, col);
                    }
                }
                let n = t.samples.len();
                for i in 0..n {
                    let a = t.samples[i];
                    let bb = t.samples[(i + 1) % n];
                    if vec2(a.pos.x, a.pos.z).distance(c) > 220.0 {
                        continue;
                    }
                    let g = |p: Vec3| vec3(p.x, hf.height(p.x, p.z) + 0.06, p.z);
                    let (ra, rb) = (vec3(a.right.x, 0.0, a.right.z).normalize(), vec3(bb.right.x, 0.0, bb.right.z).normalize());
                    m.quad(g(a.pos - ra * a.half_width), g(a.pos + ra * a.half_width), g(bb.pos + rb * bb.half_width), g(bb.pos - rb * bb.half_width), render::mesh::rgb(st.road));
                }
                for p in &t.props {
                    if vec2(p.pos.x, p.pos.z).distance(c) < 260.0 {
                        models::add_prop(&mut m, p, st);
                    }
                }
                viewport(&mut gpu, 0.0, 0.0, sw, sh);
                let view = Mat4::look_at_rh(eye, look, Vec3::Y);
                let proj = Mat4::perspective_rh_gl(55f32.to_radians(), sw / sh, 0.3, 600.0);
                let v = View::new(view, proj, eye);
                let e = Environment { sun_dir: Vec3::from(st.sun_dir).normalize(), sun_color: st.sun_color, ambient: st.ambient, fog_color: st.fog, fog_density: st.fog_density, time: 0.0 };
                gpu.draw_builder(&m, &v, &e, Mat4::IDENTITY, &DrawParams { rim: 0.4, fog: true, ..Default::default() }, 0);
                gpu.end_3d();
            }
            "coplanar" => {
                if frame == 0 {
                    let mut total = 0;
                    for b in 0..5 {
                        let biome = Biome::from_index(b);
                        let style = biome.style();
                        for (k, v) in models::catalogue_kinds(biome) {
                            let mut m = MeshBuilder::new();
                            models::add_prop(&mut m, &Prop { kind: k, pos: Vec3::ZERO, rot_y: 0.3, scale: 1.0, radius: 0.0, variant: v }, &style);
                            let (n, ex) = coplanar_fights(&m, 0.01);
                            if n > 0 {
                                println!("{:?} {:?}#{}: {} pairs {:?}", biome, k, v, n, ex);
                                total += n;
                            }
                        }
                    }
                    for i in 0..5 {
                        let s = &spec_of(i);
                        let cm = models::car_model(s, models::PAINTS[paint_of(i)]);
                        for (name, mesh) in [("body", &cm.body), ("wheel", &cm.wheel)] {
                            let (n, ex) = coplanar_fights(mesh, 0.008);
                            if n > 0 {
                                println!("car {} {} {}: {} pairs {:?}", i, s.name, name, n, ex);
                                total += n;
                            }
                        }
                    }
                    println!("TOTAL coplanar fighting pairs: {total}");
                }
                if frame == 2 {
                    return;
                }
            }
            "zfight" => {
                // signs, decals, buildings and cars at distance D with the in-game near/far planes
                let d: f32 = std::env::var("D").ok().and_then(|s| s.parse().ok()).unwrap_or(80.0);
                let jit: f32 = if std::env::var("JITTER").is_ok() { 0.02 } else { 0.0 };
                let style = Biome::Meadow.style();
                use PropKind::*;
                let line = [(Chevron, 0u8), (Chevron, 1), (ArrowSign, 0), (Cottage, 0), (Barn, 0), (Camper, 1), (HayBale, 1), (SkiCabin, 0), (Adobe, 0), (FlagPole, 0), (Picnic, 0), (Cow, 0)];
                let mut m = MeshBuilder::new();
                let mut g = MeshBuilder::new();
                g.cube(Mat4::from_translation(vec3(0.0, -0.05, 0.0)), vec3(120.0, 0.05, 40.0), render::mesh::rgb(style.ground_a));
                let mut x = -30.0;
                for (k, v) in line {
                    let w = match k { Cottage | Barn | SkiCabin | Adobe => 8.5, Camper => 6.0, _ => 3.5 };
                    x += w * 0.5;
                    models::add_prop(&mut m, &Prop { kind: k, pos: vec3(x, 0.0, 0.0), rot_y: 0.25, scale: if k == Chevron { 1.3 } else { 1.0 }, radius: 0.0, variant: v }, &style);
                    x += w * 0.5;
                }
                let eye = vec3(jit, 2.0 + jit * 0.5, -d);
                let target = vec3(0.0, 2.0, 0.0);
                let fov = 2.0 * (36.0 / d).atan();
                viewport(&mut gpu, 0.0, 0.0, sw, sh);
                let view = Mat4::look_at_rh(eye, target, Vec3::Y);
                let proj = Mat4::perspective_rh_gl(fov, sw / sh, 0.25, 2200.0);
                let v = View::new(view, proj, eye);
                let e = env();
                gpu.draw_builder(&g, &v, &e, Mat4::IDENTITY, &DrawParams::default(), 0);
                gpu.draw_builder(&m, &v, &e, Mat4::IDENTITY, &DrawParams { rim: 0.4, ..Default::default() }, 0);
                for (i, car) in [2usize, 3, 0].iter().enumerate() {
                    let spec = car_specs()[*car].clone();
                    let y = -(spec.mount_height - spec.susp_rest * 0.8 - spec.wheel_radius);
                    draw_car(&mut gpu, &v, *car, *car % 4, Mat4::from_rotation_translation(Quat::from_rotation_y(2.6), vec3(-24.0 + i as f32 * 6.0, y, -9.0)), true);
                }
                gpu.end_3d();
            }
            "props" => {
                let b: usize = std::env::var("BIOME").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
                let biome = Biome::from_index(b.max(1) - 1);
                let style = biome.style();
                let props = catalogue(biome);
                let mut m = MeshBuilder::new();
                for p in &props {
                    models::add_prop(&mut m, p, &style);
                }
                viewport(&mut gpu, 0.0, 0.0, sw, sh);
                let rows = (props.len() as f32 / 7.0).ceil();
                let half: usize = std::env::var("HALF").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
                let (z0, z1) = match half {
                    0 => (0.0, (rows / 2.0).ceil() * 11.0 - 11.0),
                    1 => ((rows / 2.0).ceil() * 11.0, rows * 11.0 - 11.0),
                    _ => (0.0, rows * 11.0 - 11.0),
                };
                let center = vec3(0.0, 1.5, (z0 + z1) * 0.5);
                let span = z1 - z0 + 11.0;
                let eye = center + vec3(0.0, 14.0 + span * 0.55, -22.0 - span * 0.6);
                let view = Mat4::look_at_rh(eye, center, Vec3::Y);
                let proj = Mat4::perspective_rh_gl(45f32.to_radians(), sw / sh, 0.5, 400.0);
                let v = View::new(view, proj, eye);
                let mut g = MeshBuilder::new();
                g.cube(Mat4::from_translation(vec3(-4.5, -0.05, rows * 5.0)), vec3(80.0, 0.05, rows * 6.0 + 10.0), render::mesh::rgb(style.ground_a));
                let e = Environment { sun_dir: Vec3::from(style.sun_dir).normalize(), sun_color: style.sun_color, ambient: style.ambient, fog_color: style.fog, fog_density: 0.0, time: 0.0 };
                gpu.draw_builder(&g, &v, &e, Mat4::IDENTITY, &DrawParams::default(), 0);
                gpu.draw_builder(&m, &v, &e, Mat4::IDENTITY, &DrawParams { rim: 0.4, ..Default::default() }, 0);
                gpu.end_3d();
            }
            _ => {
                if frame == 0 {
                    for i in 0..5 {
                        let s = &spec_of(i);
                        let m = models::car_model(s, models::PAINTS[paint_of(i)]);
                        println!(
                            "car {} {}: body {} wheel {} (x4 drawn) head {} brake {} rev {} glass {} => total drawn {}",
                            i,
                            s.name,
                            m.body.vertex_count(),
                            m.wheel.vertex_count(),
                            m.head_lights.vertex_count(),
                            m.brake_lights.vertex_count(),
                            m.reverse_lights.vertex_count(),
                            m.glass.vertex_count(),
                            m.glass.vertex_count() + m.body.vertex_count() + 4 * m.wheel.vertex_count() + m.head_lights.vertex_count() + m.brake_lights.vertex_count() + m.reverse_lights.vertex_count()
                        );
                    }
                    for b in 0..5 {
                        for len in [1u8, 2] {
                            let layout = if std::env::var("LAYOUT").map_or(false, |v| v == "p2p") { LayoutKind::PointToPoint { length_m: 2000.0 } } else { LayoutKind::Loop };
                            let t = Track::generate(&TrackParams { seed: 3, biome: Some(Biome::from_index(b)), length_class: len, layout, ..Default::default() });
                            let mut m = MeshBuilder::new();
                            for p in &t.props {
                                models::add_prop(&mut m, p, &t.style);
                            }
                            println!("{:?} len{}: {:.0} m, {} props, {} verts ({:.0}k/km)", t.biome, len, t.length, t.props.len(), m.vertex_count(), m.vertex_count() as f32 / t.length);
                            let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
                            for p in &t.props {
                                *counts.entry(format!("{:?}", p.kind)).or_default() += 1;
                            }
                            if std::env::var("BYKIND").is_ok() {
                                let mut vk: std::collections::BTreeMap<String, usize> = Default::default();
                                for p in &t.props {
                                    let mut mm = MeshBuilder::new();
                                    models::add_prop(&mut mm, p, &t.style);
                                    *vk.entry(format!("{:?}", p.kind)).or_default() += mm.vertex_count();
                                }
                                let mut v: Vec<_> = vk.into_iter().collect();
                                v.sort_by(|a, b| b.1.cmp(&a.1));
                                println!("    verts by kind: {:?}", &v[..v.len().min(12)]);
                            } else {
                                println!("    {:?}", counts);
                            }
                        }
                    }
                }
                if frame == 2 {
                    return;
                }
            }
        }
        if frame == 2 {
            shot(&out);
            return;
        }
        next_frame().await;
    }
}
