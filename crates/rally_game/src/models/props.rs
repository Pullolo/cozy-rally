//! Props: trees, rocks, buildings, critters, signs and festive bits, per biome.
use crate::render::mesh::{mix, rgb, shade, trs, MeshBuilder};
use macroquad::math::{Mat4, Quat, Vec3};
use rally_core::biome::BiomeStyle;
use rally_core::track::{Prop, PropKind};

fn leaf_color(style: &BiomeStyle, variant: u8, x: f32) -> [f32; 3] {
    let c = style.leaves[(variant as usize) % 2];
    shade(c, 0.9 + 0.2 * ((x * 12.9898).sin() * 0.5 + 0.5))
}

/// Append a prop (world space) to `m`
pub fn add_prop(m: &mut MeshBuilder, p: &Prop, style: &BiomeStyle) {
    let base = Mat4::from_scale_rotation_translation(Vec3::splat(p.scale), Quat::from_rotation_y(p.rot_y), p.pos);
    let at = |x: f32, y: f32, z: f32| base * Mat4::from_translation(Vec3::new(x, y, z));
    let trunk = rgb(style.trunk);
    match p.kind {
        PropKind::Tree => {
            m.frustum(at(0.0, 1.0, 0.0), 0.28, 0.18, 1.0, 6, trunk, trunk);
            let c = leaf_color(style, p.variant, p.pos.x);
            m.blob(at(0.0, 2.9, 0.0), Vec3::new(1.6, 1.4, 1.6), 3, 7, rgb(c));
            m.blob(at(0.5, 3.8, 0.2), Vec3::new(1.0, 0.9, 1.0), 3, 6, rgb(shade(c, 1.08)));
            if p.variant == 2 {
                m.blob(at(-0.6, 2.4, -0.4), Vec3::new(0.9, 0.8, 0.9), 2, 6, rgb(style.leaves[2]));
            }
        }
        PropKind::Pine => {
            m.frustum(at(0.0, 0.6, 0.0), 0.22, 0.16, 0.6, 5, trunk, trunk);
            let c = shade(style.leaves[0], 0.85 + 0.1 * p.variant as f32);
            for (i, (y, r)) in [(1.6, 1.5), (2.7, 1.15), (3.7, 0.8)].iter().enumerate() {
                let col = if style.name == "Frosty Peaks" && i == 2 { [0.95, 0.97, 1.0] } else { shade(c, 1.0 + i as f32 * 0.06) };
                m.frustum(at(0.0, *y, 0.0), *r, 0.0, 0.75, 7, rgb(col), rgb(shade(col, 0.7)));
            }
            if style.name == "Frosty Peaks" {
                m.frustum(at(0.0, 1.95, 0.0), 1.25, 0.0, 0.32, 7, rgb([0.95, 0.97, 1.0]), rgb([0.9, 0.92, 0.95]));
            }
        }
        PropKind::Palm => {
            let mut prev = Vec3::ZERO;
            for i in 0..5 {
                let y = i as f32 * 0.9;
                let p2 = Vec3::new((y * 0.15).powi(2), y + 0.9, 0.0);
                let mid = (prev + p2) * 0.5;
                m.frustum(base * Mat4::from_translation(mid), 0.22 - i as f32 * 0.02, 0.2 - i as f32 * 0.02, 0.45, 5, trunk, trunk);
                prev = p2;
            }
            let leaf = rgb(style.leaves[1]);
            for k in 0..6 {
                let a = k as f32 / 6.0 * std::f32::consts::TAU;
                let rot = Quat::from_rotation_y(a) * Quat::from_rotation_z(-0.5);
                m.box_tapered(base * Mat4::from_translation(prev + Vec3::new(a.cos() * 1.0, 0.1, a.sin() * 1.0)) * Mat4::from_quat(rot), Vec3::new(1.2, 0.04, 0.3), 0.4, leaf);
            }
        }
        PropKind::Bush => {
            let c = rgb(shade(style.leaves[p.variant as usize % 2], 0.85));
            m.blob(at(0.0, 0.45, 0.0), Vec3::new(0.9, 0.6, 0.9), 2, 6, c);
            m.blob(at(0.5, 0.35, 0.3), Vec3::new(0.6, 0.45, 0.6), 2, 5, c);
        }
        PropKind::Rock => {
            let c = rgb(shade(style.cliff, 0.85 + 0.1 * p.variant as f32));
            m.box_tapered(base * Mat4::from_translation(Vec3::new(0.0, 0.35, 0.0)) * Mat4::from_rotation_x(0.2), Vec3::new(0.9, 0.6, 0.75), 0.6, c);
            m.box_tapered(base * Mat4::from_translation(Vec3::new(0.5, 0.25, 0.4)) * Mat4::from_rotation_z(0.3), Vec3::new(0.45, 0.35, 0.4), 0.5, rgb(shade(style.cliff, 1.05)));
        }
        PropKind::Cactus => {
            let c = rgb([0.35, 0.6, 0.35]);
            m.frustum(at(0.0, 1.2, 0.0), 0.28, 0.24, 1.2, 7, c, c);
            m.frustum(at(0.45, 1.3, 0.0) * Mat4::from_rotation_z(-1.2), 0.15, 0.15, 0.3, 6, c, c);
            m.frustum(at(0.62, 1.75, 0.0), 0.15, 0.12, 0.45, 6, c, c);
            if p.variant == 1 {
                m.blob(at(0.0, 2.45, 0.0), Vec3::splat(0.2), 2, 5, rgb(style.leaves[2]));
            }
        }
        PropKind::Flowers => {
            for k in 0..6 {
                let a = k as f32 * 1.7 + p.rot_y;
                let r = 0.3 + (k as f32 * 0.37) % 0.7;
                let pos = Vec3::new(a.cos() * r, 0.2, a.sin() * r);
                let c = [style.leaves[2], [1.0, 0.95, 0.6], [0.95, 0.95, 1.0]][k % 3];
                m.frustum(base * Mat4::from_translation(pos), 0.02, 0.02, 0.2, 3, rgb([0.3, 0.55, 0.25]), rgb([0.3, 0.55, 0.25]));
                m.blob(base * Mat4::from_translation(pos + Vec3::Y * 0.22), Vec3::splat(0.09), 2, 4, rgb(c));
            }
        }
        PropKind::Spectator => {
            let shirts = [[0.9, 0.35, 0.35], [0.35, 0.55, 0.9], [0.95, 0.8, 0.3], [0.45, 0.75, 0.45], [0.8, 0.5, 0.85], [0.95, 0.6, 0.3]];
            let skins = [[0.98, 0.82, 0.68], [0.85, 0.65, 0.48], [0.6, 0.42, 0.3]];
            let shirt = rgb(shirts[p.variant as usize % 6]);
            let skin = rgb(skins[(p.variant as usize / 2) % 3]);
            m.frustum(at(0.0, 0.4, 0.0), 0.16, 0.14, 0.4, 5, rgb([0.3, 0.32, 0.45]), rgb([0.3, 0.32, 0.45]));
            m.frustum(at(0.0, 1.05, 0.0), 0.22, 0.17, 0.27, 6, shirt, shirt);
            m.blob(at(0.0, 1.5, 0.0), Vec3::splat(0.17), 3, 6, skin);
            // arms up (cheering!)
            m.frustum(at(0.25, 1.45, 0.0) * Mat4::from_rotation_z(-0.4), 0.05, 0.05, 0.25, 4, shirt, shirt);
            m.frustum(at(-0.25, 1.45, 0.0) * Mat4::from_rotation_z(0.4), 0.05, 0.05, 0.25, 4, shirt, shirt);
            if p.variant % 3 == 0 {
                m.frustum(at(0.0, 1.68, 0.0), 0.18, 0.12, 0.06, 6, shirt, shirt);
            }
        }
        PropKind::HayBale => {
            let c = rgb([0.92, 0.80, 0.45]);
            m.cylinder(at(0.0, 0.5, 0.0) * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2), 0.5, 1.0, 8, c, rgb([0.85, 0.72, 0.38]));
        }
        PropKind::Tent => {
            let cols = [[0.95, 0.5, 0.4], [0.4, 0.65, 0.9], [0.95, 0.85, 0.4], [0.55, 0.8, 0.5]];
            let c = cols[p.variant as usize % 4];
            m.frustum(at(0.0, 1.2, 0.0), 2.0, 0.0, 1.2, 4, rgb(c), rgb(c));
            m.cube(at(0.0, 0.02, 0.0), Vec3::new(1.4, 0.02, 1.4), rgb(shade(c, 0.5)));
        }
        PropKind::Cone => {
            m.frustum(at(0.0, 0.35, 0.0), 0.22, 0.03, 0.35, 6, rgb([1.0, 0.5, 0.15]), rgb([1.0, 0.5, 0.15]));
            m.frustum(at(0.0, 0.38, 0.0), 0.13, 0.11, 0.05, 6, rgb([1.0, 1.0, 1.0]), rgb([1.0, 1.0, 1.0]));
        }
    }
    let _ = (mix, trs, Quat::IDENTITY);
}

/// Every prop kind/variant worth showing for a biome (dev showroom catalogue)
pub fn catalogue_kinds(_b: rally_core::biome::Biome) -> Vec<(PropKind, u8)> {
    use PropKind::*;
    let mut v = Vec::new();
    for k in [Tree, Pine, Bush, Rock, Cactus, Palm, Flowers, Spectator, HayBale, Tent, Cone] {
        for var in 0..3 {
            v.push((k, var));
        }
    }
    v
}
