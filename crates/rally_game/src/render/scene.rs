//! Static world geometry built from a Track: terrain, road, props, water, sky, far hills.
use super::gpu::{DrawParams, Environment, Gpu, GpuMesh, View};
use super::mesh::{mix, rgb, rgba, shade, MeshBuilder};
use crate::models;
use macroquad::math::{Mat4, Vec3};
use rally_core::biome::{Biome, BiomeStyle};
use rally_core::noise;
use rally_core::surface::Surface;
use rally_core::track::{Track, SHOULDER};

pub struct Scene {
    pub terrain: GpuMesh,
    pub road: GpuMesh,
    pub props: GpuMesh,
    pub water: Option<GpuMesh>,
    pub sky: GpuMesh,
    pub hills: GpuMesh,
    pub env: Environment,
    pub sky_horizon: [f32; 3],
    pub water_level: Option<f32>,
}

fn n3(v: [f32; 3]) -> Vec3 {
    Vec3::from(v).normalize()
}

pub fn terrain_mesh(track: &Track) -> MeshBuilder {
    let hf = &track.terrain;
    let st = &track.style;
    let mut m = MeshBuilder::new();
    let tile = 24usize;
    let seed = track.params.seed as u32;
    let snow_line = 18.0;
    for tz in (0..hf.nz - 1).step_by(tile) {
        for tx in (0..hf.nx - 1).step_by(tile) {
            m.begin_chunk();
            for iz in tz..(tz + tile).min(hf.nz - 1) {
                for ix in tx..(tx + tile).min(hf.nx - 1) {
                    let p00 = hf.vertex_pos(ix, iz);
                    let p10 = hf.vertex_pos(ix + 1, iz);
                    let p01 = hf.vertex_pos(ix, iz + 1);
                    let p11 = hf.vertex_pos(ix + 1, iz + 1);
                    for (a, b, c) in [(p00, p11, p10), (p00, p01, p11)] {
                        let n = (b - a).cross(c - a).normalize();
                        let cen = (a + b + c) / 3.0;
                        let v = noise::fbm(cen.x * 0.03, cen.z * 0.03, 2, seed + 3) * 0.5 + 0.5;
                        let speck = rally_core::rng::hash2((cen.x * 3.0) as i32, (cen.z * 3.0) as i32, seed) * 0.08;
                        let mut col = mix(st.ground_a, st.ground_b, v);
                        if let Some(w) = track.water_level {
                            let beach = ((w + 1.2 - cen.y) / 1.2).clamp(0.0, 1.0);
                            col = mix(col, st.shoulder, beach);
                        }
                        let steep = ((0.86 - n.y) / 0.14).clamp(0.0, 1.0);
                        col = mix(col, st.cliff, steep);
                        if track.biome == Biome::Snow && cen.y > snow_line {
                            col = mix(col, [0.97, 0.98, 1.0], ((cen.y - snow_line) / 6.0).clamp(0.0, 1.0) * (1.0 - steep * 0.5));
                        }
                        col = shade(col, 0.96 + speck);
                        m.tri_n(a, b, c, n, rgb(col));
                    }
                }
            }
        }
    }
    m
}

struct Strip {
    from: f32,
    to: f32,
    color: [f32; 3],
    lift: f32,
}

fn road_strips(st: &BiomeStyle, hw: f32) -> Vec<Strip> {
    let road = st.road;
    let rut = shade(road, 0.86);
    let edge = st.road_edge;
    let sh = st.shoulder;
    let mut v = vec![
        Strip { from: -(hw + SHOULDER), to: -hw, color: sh, lift: 0.03 },
        Strip { from: -hw, to: -hw + 0.6, color: edge, lift: 0.05 },
    ];
    if st.road_surface == Surface::Asphalt {
        v.push(Strip { from: -hw + 0.6, to: -hw + 0.85, color: [0.95, 0.95, 0.92], lift: 0.055 });
        v.push(Strip { from: -hw + 0.85, to: hw - 0.85, color: road, lift: 0.05 });
        v.push(Strip { from: hw - 0.85, to: hw - 0.6, color: [0.95, 0.95, 0.92], lift: 0.055 });
    } else {
        let r1 = -hw * 0.42;
        let r2 = hw * 0.42;
        v.push(Strip { from: -hw + 0.6, to: r1 - 0.45, color: road, lift: 0.05 });
        v.push(Strip { from: r1 - 0.45, to: r1 + 0.45, color: rut, lift: 0.05 });
        v.push(Strip { from: r1 + 0.45, to: r2 - 0.45, color: shade(road, 1.04), lift: 0.05 });
        v.push(Strip { from: r2 - 0.45, to: r2 + 0.45, color: rut, lift: 0.05 });
        v.push(Strip { from: r2 + 0.45, to: hw - 0.6, color: road, lift: 0.05 });
    }
    v.push(Strip { from: hw - 0.6, to: hw, color: edge, lift: 0.05 });
    v.push(Strip { from: hw, to: hw + SHOULDER, color: sh, lift: 0.03 });
    v
}

pub fn road_mesh(track: &Track) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let st = &track.style;
    let n = track.samples.len();
    let hf = &track.terrain;
    let seed = track.params.seed as u32;
    let ground = |p: Vec3, lift: f32| Vec3::new(p.x, hf.height(p.x, p.z) + lift, p.z);
    for i in 0..n {
        if i % 16 == 0 {
            m.begin_chunk();
        }
        let a = &track.samples[i];
        let b = &track.samples[(i + 1) % n];
        let fa = Vec3::new(a.right.x, 0.0, a.right.z).normalize();
        let fb = Vec3::new(b.right.x, 0.0, b.right.z).normalize();
        let var = 0.96 + rally_core::rng::hash2(i as i32, 7, seed) * 0.06;
        for s in road_strips(st, a.half_width) {
            let sb_scale = b.half_width / a.half_width;
            let a0 = ground(a.pos + fa * s.from, s.lift);
            let a1 = ground(a.pos + fa * s.to, s.lift);
            let b0 = ground(b.pos + fb * s.from * sb_scale, s.lift);
            let b1 = ground(b.pos + fb * s.to * sb_scale, s.lift);
            m.quad(a0, a1, b1, b0, rgb(shade(s.color, var)));
        }
        // dashed centre line on asphalt
        if st.road_surface == Surface::Asphalt && (i / 3) % 2 == 0 {
            let a0 = ground(a.pos - fa * 0.12, 0.058);
            let a1 = ground(a.pos + fa * 0.12, 0.058);
            let b0 = ground(b.pos - fb * 0.12, 0.058);
            let b1 = ground(b.pos + fb * 0.12, 0.058);
            m.quad(a0, a1, b1, b0, rgb([0.97, 0.9, 0.5]));
        }
    }
    // chequered start line
    m.begin_chunk();
    let s0 = &track.samples[0];
    let fr = Vec3::new(s0.right.x, 0.0, s0.right.z).normalize();
    let fwd = Vec3::new(s0.tangent.x, 0.0, s0.tangent.z).normalize();
    let cells = 12;
    for row in 0..2 {
        for k in 0..cells {
            let l0 = -s0.half_width + k as f32 * (2.0 * s0.half_width / cells as f32);
            let l1 = l0 + 2.0 * s0.half_width / cells as f32;
            let z0 = row as f32 * 0.8 - 0.8;
            let z1 = z0 + 0.8;
            let c = if (k + row) % 2 == 0 { [0.97, 0.97, 0.97] } else { [0.12, 0.12, 0.14] };
            let p = |l: f32, z: f32| ground(s0.pos + fr * l + fwd * z, 0.07);
            m.quad(p(l0, z0), p(l1, z0), p(l1, z1), p(l0, z1), rgb(c));
        }
    }
    // start arch
    let hw = s0.half_width + 1.2;
    let post = rgb([0.95, 0.95, 0.92]);
    for side in [-1.0f32, 1.0] {
        let base = ground(s0.pos + fr * side * hw, 0.0);
        m.cube(Mat4::from_translation(base + Vec3::Y * 2.6), Vec3::new(0.25, 2.6, 0.25), post);
        // balloons
        for (k, c) in [[0.95, 0.4, 0.4], [0.4, 0.7, 0.95], [0.98, 0.85, 0.35]].iter().enumerate() {
            let off = Vec3::new(side * (0.3 + k as f32 * 0.2), 5.6 + k as f32 * 0.35, (k as f32 - 1.0) * 0.35);
            m.blob(Mat4::from_translation(base + off), Vec3::new(0.32, 0.4, 0.32), 3, 6, rgb(*c));
        }
    }
    let rot = Mat4::from_cols(fr.extend(0.0), Vec3::Y.extend(0.0), (-fwd).extend(0.0), Vec3::ZERO.extend(1.0));
    let top = ground(s0.pos, 0.0).y.max(ground(s0.pos + fr * hw, 0.0).y).max(ground(s0.pos - fr * hw, 0.0).y);
    let center = Vec3::new(s0.pos.x, top + 5.0, s0.pos.z);
    m.cube(Mat4::from_translation(center) * rot, Vec3::new(hw + 0.3, 0.55, 0.2), rgb([0.98, 0.55, 0.45]));
    m.cube(Mat4::from_translation(center + Vec3::Y * 0.0) * rot * Mat4::from_translation(Vec3::new(0.0, 0.0, -0.21)), Vec3::new(hw * 0.7, 0.32, 0.02), rgb([0.98, 0.95, 0.85]));
    m.cube(Mat4::from_translation(center + Vec3::Y * 0.0) * rot * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.21)), Vec3::new(hw * 0.7, 0.32, 0.02), rgb([0.98, 0.95, 0.85]));
    // bunting flags along the arch
    for k in 0..10 {
        let l = -hw + (k as f32 + 0.5) * (2.0 * hw / 10.0);
        let c = [[0.95, 0.4, 0.4], [0.4, 0.7, 0.95], [0.98, 0.85, 0.35], [0.5, 0.8, 0.5]][k % 4];
        let p = center + fr * l - Vec3::Y * 0.6;
        m.tri(p - fr * 0.3, p + fr * 0.3, p - Vec3::Y * 0.5, rgb(c));
    }
    // checkpoint flags
    for (ci, &idx) in track.checkpoints.iter().enumerate().skip(1) {
        let smp = &track.samples[idx];
        let fr = Vec3::new(smp.right.x, 0.0, smp.right.z).normalize();
        let c = if ci % 2 == 0 { [0.98, 0.75, 0.3] } else { [0.4, 0.75, 0.95] };
        for side in [-1.0f32, 1.0] {
            let base = ground(smp.pos + fr * side * (smp.half_width + SHOULDER + 0.6), 0.0);
            m.cylinder(Mat4::from_translation(base + Vec3::Y * 1.6), 0.06, 1.6, 5, rgb([0.95, 0.95, 0.95]), rgb([0.95, 0.95, 0.95]));
            let top = base + Vec3::Y * 3.1;
            let dir = -smp.tangent;
            m.tri(top, top - Vec3::Y * 0.8, top + dir * 1.1 - Vec3::Y * 0.4, rgb(c));
        }
    }
    m
}

pub fn props_mesh(track: &Track) -> MeshBuilder {
    // tile props spatially for culling
    let mut tiles: std::collections::HashMap<(i32, i32), MeshBuilder> = std::collections::HashMap::new();
    for p in &track.props {
        let key = ((p.pos.x / 64.0).floor() as i32, (p.pos.z / 64.0).floor() as i32);
        models::add_prop(tiles.entry(key).or_default(), p, &track.style);
    }
    let mut m = MeshBuilder::new();
    let mut keys: Vec<_> = tiles.keys().cloned().collect();
    keys.sort();
    for k in keys {
        m.begin_chunk();
        m.append(&tiles[&k]);
    }
    m
}

fn sky_mesh(st: &BiomeStyle) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let rings = 10;
    let segs = 24;
    let r = 1300.0;
    let color_at = |y: f32| -> [f32; 3] {
        if y < 0.0 {
            st.fog
        } else {
            let t = y.powf(0.6);
            mix(st.sky_horizon, st.sky_top, t)
        }
    };
    for i in 0..rings {
        let p0 = -0.25 + i as f32 / rings as f32 * (std::f32::consts::FRAC_PI_2 + 0.25);
        let p1 = -0.25 + (i + 1) as f32 / rings as f32 * (std::f32::consts::FRAC_PI_2 + 0.25);
        for j in 0..segs {
            let t0 = j as f32 / segs as f32 * std::f32::consts::TAU;
            let t1 = (j + 1) as f32 / segs as f32 * std::f32::consts::TAU;
            let pt = |p: f32, t: f32| Vec3::new(p.cos() * t.cos(), p.sin(), p.cos() * t.sin()) * r;
            let v = [pt(p0, t0), pt(p0, t1), pt(p1, t1), pt(p1, t0)];
            let c: Vec<[u8; 4]> = v.iter().map(|p| rgb(color_at(p.y / r))).collect();
            let nn = Vec3::Y;
            m.tri_smooth([(v[0], nn, c[0]), (v[1], nn, c[1]), (v[2], nn, c[2])]);
            m.tri_smooth([(v[0], nn, c[0]), (v[2], nn, c[2]), (v[3], nn, c[3])]);
        }
    }
    // a few fluffy clouds
    let mut rng = rally_core::rng::Rng::new(42);
    for _ in 0..18 {
        let a = rng.range(0.0, 6.28);
        let el = rng.range(0.15, 0.45);
        let c = Vec3::new(el.cos() * a.cos(), el.sin(), el.cos() * a.sin()) * (r * 0.9);
        for k in 0..4 {
            let off = Vec3::new(rng.range(-60.0, 60.0), rng.range(-10.0, 10.0), rng.range(-60.0, 60.0));
            m.blob(Mat4::from_translation(c + off), Vec3::new(rng.range(40.0, 80.0), rng.range(14.0, 24.0), rng.range(40.0, 80.0)), 3, 7, rgba([1.0, 1.0, 1.0], 1.0));
            let _ = k;
        }
    }
    m
}

fn hills_mesh(track: &Track) -> MeshBuilder {
    let st = &track.style;
    let hf = &track.terrain;
    let size = hf.size();
    let center = Vec3::new(hf.origin.x + size.x * 0.5, 0.0, hf.origin.y + size.y * 0.5);
    let r_in = size.x.max(size.y) * 0.5 * 1.05;
    let r_out = r_in + 450.0;
    let segs = 72;
    let mut m = MeshBuilder::new();
    let seed = track.params.seed as u32 + 11;
    let h_at = |a: f32, outer: bool| -> f32 {
        let n = noise::fbm(a.cos() * 3.0 + 10.0, a.sin() * 3.0 + 10.0, 3, seed) * 0.5 + 0.5;
        if outer {
            40.0 + n * st.hill_amp * 4.0
        } else {
            -15.0
        }
    };
    let base_c = mix(st.ground_a, st.fog, 0.45);
    let peak_c = if track.biome == Biome::Snow { [0.95, 0.96, 1.0] } else { mix(st.cliff, st.fog, 0.35) };
    for i in 0..segs {
        let a0 = i as f32 / segs as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segs as f32 * std::f32::consts::TAU;
        let d0 = Vec3::new(a0.cos(), 0.0, a0.sin());
        let d1 = Vec3::new(a1.cos(), 0.0, a1.sin());
        let i0 = center + d0 * r_in + Vec3::Y * h_at(a0, false);
        let i1 = center + d1 * r_in + Vec3::Y * h_at(a1, false);
        let o0 = center + d0 * r_out + Vec3::Y * h_at(a0, true);
        let o1 = center + d1 * r_out + Vec3::Y * h_at(a1, true);
        let mid0 = center + d0 * (r_in + 200.0) + Vec3::Y * (h_at(a0, true) * 0.35);
        let mid1 = center + d1 * (r_in + 200.0) + Vec3::Y * (h_at(a1, true) * 0.35);
        m.quad(i0, i1, mid1, mid0, rgb(base_c));
        m.quad(mid0, mid1, o1, o0, rgb(peak_c));
    }
    m
}

fn water_mesh(track: &Track, level: f32) -> MeshBuilder {
    let hf = &track.terrain;
    let size = hf.size();
    let mut m = MeshBuilder::new();
    let o = hf.origin;
    let n = 16;
    let col = rgba(track.style.water, 0.82);
    for iz in 0..n {
        for ix in 0..n {
            let x0 = o.x - 300.0 + (size.x + 600.0) * ix as f32 / n as f32;
            let x1 = o.x - 300.0 + (size.x + 600.0) * (ix + 1) as f32 / n as f32;
            let z0 = o.y - 300.0 + (size.y + 600.0) * iz as f32 / n as f32;
            let z1 = o.y - 300.0 + (size.y + 600.0) * (iz + 1) as f32 / n as f32;
            m.begin_chunk();
            m.tri_n(Vec3::new(x0, level, z0), Vec3::new(x1, level, z1), Vec3::new(x1, level, z0), Vec3::Y, col);
            m.tri_n(Vec3::new(x0, level, z0), Vec3::new(x0, level, z1), Vec3::new(x1, level, z1), Vec3::Y, col);
        }
    }
    m
}

impl Scene {
    pub fn build(track: &Track, gpu: &mut Gpu) -> Scene {
        let st = &track.style;
        let terrain = gpu.upload(&terrain_mesh(track));
        let road = gpu.upload(&road_mesh(track));
        let props = gpu.upload(&props_mesh(track));
        let sky = gpu.upload(&sky_mesh(st));
        let hills = gpu.upload(&hills_mesh(track));
        let water = track.water_level.map(|w| gpu.upload(&water_mesh(track, w)));
        Scene {
            terrain,
            road,
            props,
            water,
            sky,
            hills,
            env: Environment {
                sun_dir: n3(st.sun_dir),
                sun_color: st.sun_color,
                ambient: st.ambient,
                fog_color: st.fog,
                fog_density: st.fog_density,
                time: 0.0,
            },
            sky_horizon: st.sky_horizon,
            water_level: track.water_level,
        }
    }

    pub fn draw_opaque(&self, gpu: &mut Gpu, view: &View) {
        let env = self.env;
        let sky_p = DrawParams { unlit: 1.0, fog: false, ..Default::default() };
        gpu.draw(&self.sky, view, &env, Mat4::from_translation(view.eye), &sky_p, false);
        let p = DrawParams::default();
        gpu.draw(&self.hills, view, &env, Mat4::IDENTITY, &DrawParams { fog: true, ..Default::default() }, false);
        gpu.draw(&self.terrain, view, &env, Mat4::IDENTITY, &p, false);
        gpu.draw(&self.road, view, &env, Mat4::IDENTITY, &p, false);
        gpu.draw(&self.props, view, &env, Mat4::IDENTITY, &DrawParams { rim: 0.4, ..Default::default() }, false);
    }

    pub fn draw_transparent(&self, gpu: &mut Gpu, view: &View) {
        if std::env::var("COZY_NOWATER").is_ok() {
            return;
        }
        if let Some(w) = &self.water {
            let t = self.env.time;
            let shimmer = 0.97 + 0.03 * (t * 0.8).sin();
            gpu.draw(w, view, &self.env, Mat4::IDENTITY, &DrawParams { tint: [shimmer, shimmer, 1.0, 1.0], rim: 0.6, ..Default::default() }, true);
        }
    }

    pub fn free(self, gpu: &mut Gpu) {
        gpu.free(self.terrain);
        gpu.free(self.road);
        gpu.free(self.props);
        gpu.free(self.sky);
        gpu.free(self.hills);
        if let Some(w) = self.water {
            gpu.free(w);
        }
    }
}
