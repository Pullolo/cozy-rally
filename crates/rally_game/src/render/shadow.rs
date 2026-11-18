//! Single-cascade sun shadow map. Depth is packed into an RGBA8 colour target (portable to
//! GLES2 without depth-texture sampling); a depth attachment is only used for z-testing.
use super::gpu::View;
use macroquad::math::{Mat4, Vec3};
use macroquad::miniquad::*;

pub struct ShadowMap {
    pub size: u32,
    pub color: TextureId,
    pub depth: TextureId,
    pub pass: RenderPass,
}

/// Half-width of the shadowed square around the camera (metres)
pub const SHADOW_RADIUS: f32 = 64.0;

impl ShadowMap {
    pub fn new(ctx: &mut dyn RenderingBackend, size: u32) -> ShadowMap {
        let color = ctx.new_render_texture(TextureParams {
            format: TextureFormat::RGBA8,
            width: size,
            height: size,
            min_filter: FilterMode::Nearest,
            mag_filter: FilterMode::Nearest,
            wrap: TextureWrap::Clamp,
            ..Default::default()
        });
        let depth = ctx.new_render_texture(TextureParams { format: TextureFormat::Depth, width: size, height: size, ..Default::default() });
        let pass = ctx.new_render_pass(color, Some(depth));
        ShadowMap { size, color, depth, pass }
    }

    pub fn delete(self, ctx: &mut dyn RenderingBackend) {
        ctx.delete_render_pass(self.pass);
        ctx.delete_texture(self.color);
        ctx.delete_texture(self.depth);
    }
}

/// Light-space view for the camera `view`: an orthographic box along the sun direction,
/// centred a little ahead of the camera and snapped to whole texels so edges don't shimmer.
pub fn light_view(view: &View, sun_dir: Vec3, size: u32, radius: f32) -> (View, f32) {
    let l = sun_dir.normalize_or_zero();
    let up = if l.y.abs() > 0.98 { Vec3::Z } else { Vec3::Y };
    let lv = Mat4::look_at_rh(Vec3::ZERO, -l, up);
    // camera forward = inward normal of the near plane
    let fwd = view.forward();
    let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
    let center = view.eye + flat * radius * 0.62 + Vec3::new(0.0, -2.0, 0.0);
    let mut c = lv.transform_point3(center);
    let texel = radius * 2.0 / size as f32;
    c.x = (c.x / texel).floor() * texel;
    c.y = (c.y / texel).floor() * texel;
    let depth_range = 260.0;
    let proj = Mat4::orthographic_rh_gl(c.x - radius, c.x + radius, c.y - radius, c.y + radius, -c.z - depth_range, -c.z + depth_range);
    // the "eye" is only used for distance culling in shadow passes: put it far back along the light
    let eye = center + l * depth_range;
    (View::new(lv, proj, eye), texel)
}
