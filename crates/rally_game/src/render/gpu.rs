//! Thin layer over miniquad: the lit/fogged/shadowed vertex-colour world shader (opaque,
//! alpha-blended and overlay pipelines), sky + water + shadow-depth pipelines, static GPU
//! meshes with bounding boxes for frustum culling, a streaming buffer for dynamic geometry,
//! and the frame structure for shadow map + offscreen scene + post-processing.
//!
//! Frame flow for the race view (see `game::RaceSession::draw_3d`):
//! `begin_frame` -> per viewport { `begin_shadow` .. casters .. `end_shadow`,
//! `begin_scene` .. world .. `end_scene` } -> `finish_frame` (bloom/grade/composite to screen).
//! `begin_3d`/`end_3d` still draw straight to the screen (menus, car preview).
use super::mesh::{MeshBuilder, Vertex};
use super::post::{Grade, PostPipes, PostTargets, PxRect};
use super::shaders;
use super::shadow::{light_view, ShadowMap, SHADOW_RADIUS};
use super::quality::Quality;
use macroquad::math::{Mat4, Vec3, Vec4};
use macroquad::miniquad::*;
use macroquad::window::get_internal_gl;

/// World shader uniforms (order must match `meta()`)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Uniforms {
    pub view_proj: Mat4,
    pub model: Mat4,
    pub shadow_mat: Mat4,
    pub sun_dir: [f32; 3],
    pub sun_color: [f32; 3],
    pub sky_amb: [f32; 3],
    pub ground_amb: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_sun: [f32; 3],
    pub cam_pos: [f32; 3],
    pub tint: [f32; 4],
    pub params: [f32; 4],
    pub params2: [f32; 4],
    pub params3: [f32; 4],
    pub params4: [f32; 4],
}

fn meta() -> ShaderMeta {
    ShaderMeta {
        images: vec!["shadow_map".to_string()],
        uniforms: UniformBlockLayout {
            uniforms: vec![
                UniformDesc::new("view_proj", UniformType::Mat4),
                UniformDesc::new("model", UniformType::Mat4),
                UniformDesc::new("shadow_mat", UniformType::Mat4),
                UniformDesc::new("sun_dir", UniformType::Float3),
                UniformDesc::new("sun_color", UniformType::Float3),
                UniformDesc::new("sky_amb", UniformType::Float3),
                UniformDesc::new("ground_amb", UniformType::Float3),
                UniformDesc::new("fog_color", UniformType::Float3),
                UniformDesc::new("fog_sun", UniformType::Float3),
                UniformDesc::new("cam_pos", UniformType::Float3),
                UniformDesc::new("tint", UniformType::Float4),
                UniformDesc::new("params", UniformType::Float4),
                UniformDesc::new("params2", UniformType::Float4),
                UniformDesc::new("params3", UniformType::Float4),
                UniformDesc::new("params4", UniformType::Float4),
            ],
        },
    }
}

#[repr(C)]
struct ShadowUniforms {
    view_proj: Mat4,
    model: Mat4,
    params: [f32; 4],
}

fn shadow_meta() -> ShaderMeta {
    ShaderMeta {
        images: vec![],
        uniforms: UniformBlockLayout {
            uniforms: vec![UniformDesc::new("view_proj", UniformType::Mat4), UniformDesc::new("model", UniformType::Mat4), UniformDesc::new("params", UniformType::Float4)],
        },
    }
}

#[repr(C)]
struct SkyUniforms {
    view_proj: Mat4,
    model: Mat4,
    sky_top: [f32; 3],
    sky_horizon: [f32; 3],
    fog_color: [f32; 3],
    sun_dir: [f32; 3],
    sun_color: [f32; 3],
    params: [f32; 4],
}

fn sky_meta() -> ShaderMeta {
    ShaderMeta {
        images: vec![],
        uniforms: UniformBlockLayout {
            uniforms: vec![
                UniformDesc::new("view_proj", UniformType::Mat4),
                UniformDesc::new("model", UniformType::Mat4),
                UniformDesc::new("sky_top", UniformType::Float3),
                UniformDesc::new("sky_horizon", UniformType::Float3),
                UniformDesc::new("fog_color", UniformType::Float3),
                UniformDesc::new("sun_dir", UniformType::Float3),
                UniformDesc::new("sun_color", UniformType::Float3),
                UniformDesc::new("params", UniformType::Float4),
            ],
        },
    }
}

#[repr(C)]
struct WaterUniforms {
    view_proj: Mat4,
    sun_dir: [f32; 3],
    sun_color: [f32; 3],
    sky_color: [f32; 3],
    fog_color: [f32; 3],
    fog_sun: [f32; 3],
    cam_pos: [f32; 3],
    shallow: [f32; 3],
    params: [f32; 4],
}

fn water_meta() -> ShaderMeta {
    ShaderMeta {
        images: vec![],
        uniforms: UniformBlockLayout {
            uniforms: vec![
                UniformDesc::new("view_proj", UniformType::Mat4),
                UniformDesc::new("sun_dir", UniformType::Float3),
                UniformDesc::new("sun_color", UniformType::Float3),
                UniformDesc::new("sky_color", UniformType::Float3),
                UniformDesc::new("fog_color", UniformType::Float3),
                UniformDesc::new("fog_sun", UniformType::Float3),
                UniformDesc::new("cam_pos", UniformType::Float3),
                UniformDesc::new("shallow", UniformType::Float3),
                UniformDesc::new("params", UniformType::Float4),
            ],
        },
    }
}

pub struct GpuChunk {
    pub vb: BufferId,
    pub ib: BufferId,
    pub count: i32,
    pub min: Vec3,
    pub max: Vec3,
}

pub struct GpuMesh {
    pub chunks: Vec<GpuChunk>,
}

/// Lighting / fog environment for a frame
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    pub sun_dir: Vec3,
    pub sun_color: [f32; 3],
    /// sky (upper hemisphere) ambient; the ground bounce is derived from it
    pub ambient: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_density: f32,
    pub time: f32,
}

impl Environment {
    /// Warm ground bounce for hemisphere ambient
    pub fn ground_ambient(&self) -> [f32; 3] {
        let a = self.ambient;
        [a[0] * 0.92, a[1] * 0.82, a[2] * 0.66]
    }

    /// Fog colour looking towards the sun (warm forward scattering)
    pub fn fog_sun(&self) -> [f32; 3] {
        let f = self.fog_color;
        let s = self.sun_color;
        [(f[0] * 0.75 + s[0] * 0.3).min(1.0), (f[1] * 0.75 + s[1] * 0.24).min(1.0), (f[2] * 0.8 + s[2] * 0.1).min(1.0)]
    }
}

/// View matrices for one viewport
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub view_proj: Mat4,
    pub eye: Vec3,
    /// Frustum planes (xyz normal, w distance), pointing inward
    pub planes: [Vec4; 6],
}

impl View {
    pub fn new(view: Mat4, proj: Mat4, eye: Vec3) -> View {
        let vp = proj * view;
        let r0 = vp.row(0);
        let r1 = vp.row(1);
        let r2 = vp.row(2);
        let r3 = vp.row(3);
        let mut planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 + r2, r3 - r2];
        for p in planes.iter_mut() {
            let l = p.truncate().length();
            *p /= l;
        }
        View { view_proj: vp, eye, planes }
    }

    pub fn aabb_visible(&self, min: Vec3, max: Vec3) -> bool {
        for p in &self.planes {
            let v = Vec3::new(if p.x > 0.0 { max.x } else { min.x }, if p.y > 0.0 { max.y } else { min.y }, if p.z > 0.0 { max.z } else { min.z });
            if p.truncate().dot(v) + p.w < 0.0 {
                return false;
            }
        }
        true
    }

    pub fn sphere_visible(&self, c: Vec3, r: f32) -> bool {
        self.planes.iter().all(|p| p.truncate().dot(c) + p.w > -r)
    }

    /// Camera forward direction (inward normal of the near plane)
    pub fn forward(&self) -> Vec3 {
        self.planes[4].truncate().normalize_or_zero()
    }
}

/// Draw style flags. Construct with `..Default::default()` — new fields get added over time.
#[derive(Clone, Copy, Debug)]
pub struct DrawParams {
    /// Colour multiplier. With `unlit = 1`, values > 1 are emissive and bloom (brake lights).
    pub tint: [f32; 4],
    pub unlit: f32,
    pub rim: f32,
    pub fog: bool,
    /// Specular glint strength (0 = matte; ~0.6 car paint)
    pub gloss: f32,
    /// Darken towards the bottom of the mesh in model space: (y_bottom, y_top); equal = off
    pub ao_y: [f32; 2],
    /// Pull towards the camera in clip space (road decals): ~1e-5
    pub depth_bias: f32,
    /// Rendered into the sun shadow map (opaque draws only)
    pub cast_shadow: bool,
    /// Samples the sun shadow map
    pub receive_shadow: bool,
    /// Skip chunks farther than this from the eye (0 = no limit)
    pub cull_dist: f32,
    /// Opaque meshes only: vertex alpha encodes gloss (255 = matte) instead of wind sway.
    /// Used by the road (ice sheen, mud puddles).
    pub vertex_gloss: bool,
    /// Upper limit for the fog amount (0 = none); for distant scenery with baked haze
    pub fog_max: f32,
}

impl Default for DrawParams {
    fn default() -> Self {
        DrawParams { tint: [1.0; 4], unlit: 0.0, rim: 0.0, fog: true, gloss: 0.0, ao_y: [0.0, 0.0], depth_bias: 0.0, cast_shadow: true, receive_shadow: true, cull_dist: 0.0, vertex_gloss: false, fog_max: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// default framebuffer (window MSAA)
    Screen,
    /// offscreen scene target (post-processing on)
    Scene,
    /// shadow depth pass
    Shadow,
}

struct ShadowState {
    mat: Mat4,
    texel_uv: f32,
    texel_world: f32,
    depth_bias: f32,
}

pub struct Gpu {
    pub opaque: Pipeline,
    pub alpha: Pipeline,
    /// Alpha-blended, no depth test (overlays)
    pub overlay: Pipeline,
    shadow_pipe: Pipeline,
    sky_pipe: Pipeline,
    water_pipe: Pipeline,
    stream_vb: BufferId,
    stream_ib: BufferId,
    stream_cap: usize,
    white: TextureId,
    target: Target,
    shadow_map: Option<ShadowMap>,
    shadow: Option<ShadowState>,
    light_vp: Mat4,
    post_pipes: Option<PostPipes>,
    post: Option<PostTargets>,
    /// MSAA was requested but the offscreen target failed; stick to FXAA
    msaa_broken: bool,
    frame_viewports: Vec<PxRect>,
    frame_quality: Quality,
    /// when false, no draw samples the shadow map (e.g. volumetric fx particles)
    shadow_receive: bool,
}

pub const STREAM_VERTS: usize = 60_000;

/// miniquad 0.4 only enables GL_DEPTH_TEST for pipelines that also write depth; transparent
/// pipelines still need depth testing (without writing), so re-enable it by hand.
fn force_depth_test() {
    unsafe {
        use macroquad::miniquad::gl;
        gl::glEnable(gl::GL_DEPTH_TEST);
        gl::glDepthFunc(gl::GL_LEQUAL);
        gl::glDepthMask(0);
    }
}

fn depth_mask_on() {
    unsafe {
        macroquad::miniquad::gl::glDepthMask(1);
    }
}

fn ctx() -> &'static mut dyn RenderingBackend {
    unsafe { get_internal_gl().quad_context }
}

fn world_attrs() -> [VertexAttribute; 3] {
    [VertexAttribute::new("in_pos", VertexFormat::Float3), VertexAttribute::new("in_normal", VertexFormat::Float3), VertexAttribute::new("in_color", VertexFormat::Byte4)]
}

impl Gpu {
    pub fn new() -> Gpu {
        let ctx = ctx();
        let shader = ctx
            .new_shader(ShaderSource::Glsl { vertex: shaders::WORLD_VS, fragment: shaders::WORLD_FS }, meta())
            .unwrap_or_else(|e| panic!("shader compile failed: {e:?}"));
        let layout = [BufferLayout::default()];
        let attrs = world_attrs();
        let opaque = ctx.new_pipeline(&layout, &attrs, shader, PipelineParams { depth_test: Comparison::LessOrEqual, depth_write: true, cull_face: CullFace::Nothing, ..Default::default() });
        let blend = Some(BlendState::new(Equation::Add, BlendFactor::Value(BlendValue::SourceAlpha), BlendFactor::OneMinusValue(BlendValue::SourceAlpha)));
        // keep destination alpha (it carries the bloom mask in the scene target)
        let keep_alpha = Some(BlendState::new(Equation::Add, BlendFactor::Zero, BlendFactor::One));
        let alpha = ctx.new_pipeline(
            &layout,
            &attrs,
            shader,
            PipelineParams { depth_test: Comparison::LessOrEqual, depth_write: false, cull_face: CullFace::Nothing, color_blend: blend, alpha_blend: keep_alpha, ..Default::default() },
        );
        let overlay = ctx.new_pipeline(
            &layout,
            &attrs,
            shader,
            PipelineParams { depth_test: Comparison::Always, depth_write: false, cull_face: CullFace::Nothing, color_blend: blend, alpha_blend: keep_alpha, ..Default::default() },
        );
        let sh = ctx.new_shader(ShaderSource::Glsl { vertex: shaders::SHADOW_VS, fragment: shaders::SHADOW_FS }, shadow_meta()).unwrap_or_else(|e| panic!("shadow shader compile failed: {e:?}"));
        let shadow_pipe = ctx.new_pipeline(&layout, &attrs, sh, PipelineParams { depth_test: Comparison::LessOrEqual, depth_write: true, cull_face: CullFace::Nothing, ..Default::default() });
        let sky = ctx.new_shader(ShaderSource::Glsl { vertex: shaders::SKY_VS, fragment: shaders::SKY_FS }, sky_meta()).unwrap_or_else(|e| panic!("sky shader compile failed: {e:?}"));
        let sky_pipe = ctx.new_pipeline(&layout, &attrs, sky, PipelineParams { depth_test: Comparison::LessOrEqual, depth_write: true, cull_face: CullFace::Nothing, ..Default::default() });
        let water = ctx.new_shader(ShaderSource::Glsl { vertex: shaders::WATER_VS, fragment: shaders::WATER_FS }, water_meta()).unwrap_or_else(|e| panic!("water shader compile failed: {e:?}"));
        let water_pipe = ctx.new_pipeline(&layout, &attrs, water, PipelineParams { depth_test: Comparison::LessOrEqual, depth_write: true, cull_face: CullFace::Nothing, ..Default::default() });
        let stream_vb = ctx.new_buffer(BufferType::VertexBuffer, BufferUsage::Stream, BufferSource::empty::<Vertex>(STREAM_VERTS));
        let stream_ib = ctx.new_buffer(BufferType::IndexBuffer, BufferUsage::Stream, BufferSource::empty::<u16>(STREAM_VERTS * 2));
        // packed depth 1.0 everywhere = never in shadow
        let white = ctx.new_texture_from_rgba8(1, 1, &[255, 255, 255, 255]);
        ctx.texture_set_filter(white, FilterMode::Nearest, MipmapFilterMode::None);
        Gpu {
            opaque,
            alpha,
            overlay,
            shadow_pipe,
            sky_pipe,
            water_pipe,
            stream_vb,
            stream_ib,
            stream_cap: STREAM_VERTS,
            white,
            target: Target::Screen,
            shadow_map: None,
            shadow: None,
            light_vp: Mat4::IDENTITY,
            post_pipes: None,
            post: None,
            msaa_broken: false,
            frame_viewports: Vec::new(),
            frame_quality: Quality::High,
            shadow_receive: true,
        }
    }

    pub fn upload(&mut self, mesh: &MeshBuilder) -> GpuMesh {
        let ctx = ctx();
        let chunks = mesh
            .chunks
            .iter()
            .filter(|c| !c.indices.is_empty())
            .map(|c| GpuChunk {
                vb: ctx.new_buffer(BufferType::VertexBuffer, BufferUsage::Immutable, BufferSource::slice(&c.vertices)),
                ib: ctx.new_buffer(BufferType::IndexBuffer, BufferUsage::Immutable, BufferSource::slice(&c.indices)),
                count: c.indices.len() as i32,
                min: c.min,
                max: c.max,
            })
            .collect();
        GpuMesh { chunks }
    }

    pub fn free(&mut self, mesh: GpuMesh) {
        let ctx = ctx();
        for c in mesh.chunks {
            ctx.delete_buffer(c.vb);
            ctx.delete_buffer(c.ib);
        }
    }

    /// Flush macroquad's 2D batch and start a raw 3D pass straight to the screen in the given
    /// viewport (physical pixels, origin top-left). No shadows / post (menus, car preview).
    pub fn begin_3d(&mut self, x: i32, y: i32, w: i32, h: i32, clear_depth: bool) {
        unsafe {
            get_internal_gl().flush();
        }
        self.target = Target::Screen;
        self.shadow = None;
        let ctx = ctx();
        let (_, sh) = macroquad::miniquad::window::screen_size();
        ctx.begin_default_pass(PassAction::Nothing);
        let gl_y = sh as i32 - (y + h);
        ctx.apply_viewport(x, gl_y, w, h);
        ctx.apply_scissor_rect(x, gl_y, w, h);
        if clear_depth {
            // glClear ignores the depth buffer while depth writes are masked, so bind a
            // depth-writing pipeline first.
            ctx.apply_pipeline(&self.opaque);
            depth_mask_on();
            ctx.clear(None, Some(1.0), None);
        }
    }

    pub fn end_3d(&mut self) {
        depth_mask_on();
        let ctx = ctx();
        ctx.end_render_pass();
    }

    // ------------------------------------------------------------------ frame structure

    /// Start a 3D frame for the race view at quality `q` (usually `render::quality()`).
    /// Creates / resizes offscreen targets as needed. Returns true when shadows are on.
    pub fn begin_frame(&mut self, q: Quality) -> bool {
        unsafe {
            get_internal_gl().flush();
        }
        self.frame_quality = q;
        self.frame_viewports.clear();
        self.shadow = None;
        let ctx = ctx();
        // shadow map
        let want = q.shadow_size();
        if self.shadow_map.as_ref().map(|s| s.size) != Some(want).filter(|&s| s > 0) {
            if let Some(s) = self.shadow_map.take() {
                s.delete(ctx);
            }
            if want > 0 {
                self.shadow_map = Some(ShadowMap::new(ctx, want));
            }
        }
        // post targets
        if q.post() {
            if self.post_pipes.is_none() {
                self.post_pipes = Some(PostPipes::new(ctx));
            }
            let (w, h) = macroquad::miniquad::window::screen_size();
            let (w, h) = ((w as u32).max(1), (h as u32).max(1));
            let want_msaa = q == Quality::High && !self.msaa_broken;
            let ok = self.post.as_ref().map_or(false, |p| p.w == w && p.h == h && (p.msaa == want_msaa || (want_msaa && !ctx.info().features.resolve_attachments)));
            if !ok {
                if let Some(p) = self.post.take() {
                    p.delete(ctx);
                }
                let mut t = PostTargets::new(ctx, w, h, want_msaa);
                if t.msaa && !framebuffer_complete(ctx, t.scene_pass) {
                    // some drivers refuse MSAA float depth: fall back to FXAA
                    t.delete(ctx);
                    self.msaa_broken = true;
                    t = PostTargets::new(ctx, w, h, false);
                }
                self.post = Some(t);
            }
        } else if let Some(p) = self.post.take() {
            p.delete(ctx);
        }
        self.shadow_map.is_some() && std::env::var("COZY_NOSHADOW").is_err()
    }

    /// Start rendering the sun shadow map for `view`. Returns the light view to cull casters
    /// with (pass it to the usual `draw` calls; only opaque `cast_shadow` draws are rendered).
    pub fn begin_shadow(&mut self, view: &View, env: &Environment) -> Option<View> {
        let sm = self.shadow_map.as_ref()?;
        let (lv, texel) = light_view(view, env.sun_dir, sm.size, SHADOW_RADIUS);
        let ctx = ctx();
        ctx.begin_pass(Some(sm.pass), PassAction::Nothing);
        ctx.apply_viewport(0, 0, sm.size as i32, sm.size as i32);
        ctx.apply_scissor_rect(0, 0, sm.size as i32, sm.size as i32);
        ctx.apply_pipeline(&self.shadow_pipe);
        depth_mask_on();
        ctx.clear(Some((1.0, 1.0, 1.0, 1.0)), Some(1.0), None);
        self.target = Target::Shadow;
        self.light_vp = lv.view_proj;
        // depth units: ortho depth range is 2 * 260 m
        self.shadow = Some(ShadowState { mat: lv.view_proj, texel_uv: 1.0 / sm.size as f32, texel_world: texel, depth_bias: (texel * 1.2 + 0.04) / 520.0 });
        Some(lv)
    }

    pub fn end_shadow(&mut self) {
        let ctx = ctx();
        ctx.end_render_pass();
        self.target = Target::Screen;
    }

    /// Start drawing the 3D world for one viewport (physical pixels, origin top-left).
    /// Draws into the offscreen scene target when post-processing is on.
    pub fn begin_scene(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let ctx = ctx();
        let (_, sh) = macroquad::miniquad::window::screen_size();
        let gl_y = sh as i32 - (y + h);
        if let Some(p) = &self.post {
            ctx.begin_pass(Some(p.scene_pass), PassAction::Nothing);
            self.target = Target::Scene;
            self.frame_viewports.push(PxRect { x, y, w, h });
        } else {
            ctx.begin_default_pass(PassAction::Nothing);
            self.target = Target::Screen;
        }
        ctx.apply_viewport(x, gl_y, w, h);
        ctx.apply_scissor_rect(x, gl_y, w, h);
        ctx.apply_pipeline(&self.opaque);
        depth_mask_on();
        ctx.clear(Some((0.0, 0.0, 0.0, 0.0)), Some(1.0), None);
    }

    pub fn end_scene(&mut self) {
        depth_mask_on();
        let ctx = ctx();
        ctx.end_render_pass();
    }

    /// Bloom, grade and composite all viewports drawn this frame onto the screen.
    pub fn finish_frame(&mut self, grade: &Grade, time: f32) {
        self.shadow = None;
        self.target = Target::Screen;
        let (Some(pipes), Some(t)) = (&self.post_pipes, &self.post) else {
            return;
        };
        pipes.finish(ctx(), t, &self.frame_viewports, grade, time);
    }

    /// Globally allow/suppress shadow-map sampling for subsequent draws (default: allowed).
    /// Used around fx so soft dust/smoke stays light instead of catching car shadows.
    pub fn set_shadow_receive(&mut self, on: bool) {
        self.shadow_receive = on;
    }

    /// True when the current pass renders the shadow map
    pub fn in_shadow_pass(&self) -> bool {
        self.target == Target::Shadow
    }

    // ------------------------------------------------------------------ drawing

    fn uniforms(&self, view: &View, env: &Environment, model: Mat4, p: &DrawParams, blended: bool) -> Uniforms {
        let (shadow_mat, shadow_on, texel_uv, texel_world, sbias) = match (&self.shadow, p.receive_shadow && self.shadow_receive) {
            (Some(s), true) => (s.mat, 1.0, s.texel_uv, s.texel_world, s.depth_bias),
            _ => (Mat4::IDENTITY, 0.0, 0.0, 0.0, 0.0),
        };
        let out_mode = if blended {
            2.0
        } else if self.target == Target::Scene {
            1.0
        } else {
            0.0
        };
        let ao = p.ao_y[1] != p.ao_y[0];
        Uniforms {
            view_proj: view.view_proj,
            model,
            shadow_mat,
            sun_dir: env.sun_dir.to_array(),
            sun_color: env.sun_color,
            sky_amb: env.ambient,
            ground_amb: env.ground_ambient(),
            fog_color: env.fog_color,
            fog_sun: env.fog_sun(),
            cam_pos: view.eye.to_array(),
            tint: p.tint,
            params: [if p.fog { env.fog_density } else { 0.0 }, p.unlit, env.time, p.rim],
            params2: [p.gloss, if blended || p.vertex_gloss { 0.0 } else { 1.0 }, shadow_on, if ao { p.ao_y[1] } else { 0.0 }],
            params3: [p.depth_bias, out_mode, texel_uv, p.ao_y[0]],
            params4: [texel_world, sbias, if p.vertex_gloss && !blended { 1.0 } else { 0.0 }, p.fog_max],
        }
    }

    fn shadow_image(&self) -> TextureId {
        match (&self.shadow, &self.shadow_map) {
            (Some(_), Some(m)) => m.color,
            _ => self.white,
        }
    }

    fn chunk_visible(view: &View, c: &GpuChunk, model: &Mat4, identity_rot: bool, cull_dist: f32) -> bool {
        let t = model.w_axis.truncate();
        if identity_rot {
            let (min, max) = (c.min + t, c.max + t);
            if cull_dist > 0.0 {
                let closest = view.eye.clamp(min, max);
                if closest.distance_squared(view.eye) > cull_dist * cull_dist {
                    return false;
                }
            }
            view.aabb_visible(min, max)
        } else {
            let center = model.transform_point3((c.min + c.max) * 0.5);
            let scale = model.x_axis.truncate().length().max(model.y_axis.truncate().length()).max(model.z_axis.truncate().length());
            let r = (c.max - c.min).length() * 0.5 * scale;
            if cull_dist > 0.0 && center.distance(view.eye) - r > cull_dist {
                return false;
            }
            view.sphere_visible(center, r)
        }
    }

    /// Draw a static mesh. Culls chunks by their (transformed) bounds when model is a
    /// pure translation/identity; otherwise tests a bounding sphere.
    /// During a shadow pass only opaque, shadow-casting draws are rendered (as depth).
    pub fn draw(&mut self, mesh: &GpuMesh, view: &View, env: &Environment, model: Mat4, p: &DrawParams, alpha: bool) {
        let ctx = ctx();
        let shadow_pass = self.target == Target::Shadow;
        if shadow_pass {
            if alpha || !p.cast_shadow || p.tint[3] < 0.99 {
                return;
            }
            ctx.apply_pipeline(&self.shadow_pipe);
            ctx.apply_uniforms(UniformsSource::table(&ShadowUniforms { view_proj: view.view_proj, model, params: [0.0, 1.0, env.time, 0.0] }));
        } else {
            ctx.apply_pipeline(if alpha { &self.alpha } else { &self.opaque });
            if alpha {
                force_depth_test();
            } else {
                depth_mask_on();
            }
            let u = self.uniforms(view, env, model, p, alpha);
            ctx.apply_uniforms(UniformsSource::table(&u));
        }
        let images = if shadow_pass { vec![] } else { vec![self.shadow_image()] };
        let identity_rot = model.x_axis.truncate() == Vec3::X && model.y_axis.truncate() == Vec3::Y && model.z_axis.truncate() == Vec3::Z;
        for c in &mesh.chunks {
            if !Self::chunk_visible(view, c, &model, identity_rot, p.cull_dist) {
                continue;
            }
            ctx.apply_bindings(&Bindings { vertex_buffers: vec![c.vb], index_buffer: c.ib, images: images.clone() });
            ctx.draw(0, c.count, 1);
        }
        if alpha {
            depth_mask_on();
        }
    }

    /// Draw immediate geometry (re-uploaded every call). Pipeline: 0 opaque, 1 alpha, 2 overlay.
    pub fn draw_dynamic(&mut self, verts: &[Vertex], indices: &[u16], view: &View, env: &Environment, model: Mat4, p: &DrawParams, pipeline: u8) {
        if indices.is_empty() {
            return;
        }
        let shadow_pass = self.target == Target::Shadow;
        if shadow_pass && (pipeline != 0 || !p.cast_shadow) {
            return;
        }
        let n = verts.len().min(self.stream_cap);
        let ctx = ctx();
        ctx.buffer_update(self.stream_vb, BufferSource::slice(&verts[..n]));
        let ni = indices.len().min(self.stream_cap * 2);
        ctx.buffer_update(self.stream_ib, BufferSource::slice(&indices[..ni]));
        if shadow_pass {
            ctx.apply_pipeline(&self.shadow_pipe);
            ctx.apply_uniforms(UniformsSource::table(&ShadowUniforms { view_proj: view.view_proj, model, params: [0.0, 1.0, env.time, 0.0] }));
            ctx.apply_bindings(&Bindings { vertex_buffers: vec![self.stream_vb], index_buffer: self.stream_ib, images: vec![] });
            ctx.draw(0, ni as i32, 1);
            return;
        }
        let pl = match pipeline {
            0 => &self.opaque,
            1 => &self.alpha,
            _ => &self.overlay,
        };
        ctx.apply_pipeline(pl);
        if pipeline == 1 {
            force_depth_test();
        } else {
            depth_mask_on();
        }
        let u = self.uniforms(view, env, model, p, pipeline != 0);
        ctx.apply_uniforms(UniformsSource::table(&u));
        ctx.apply_bindings(&Bindings { vertex_buffers: vec![self.stream_vb], index_buffer: self.stream_ib, images: vec![self.shadow_image()] });
        ctx.draw(0, ni as i32, 1);
        depth_mask_on();
    }

    /// Draw a MeshBuilder's chunks through the stream buffer (for per-frame geometry).
    pub fn draw_builder(&mut self, mesh: &MeshBuilder, view: &View, env: &Environment, model: Mat4, p: &DrawParams, pipeline: u8) {
        for ch in &mesh.chunks {
            self.draw_dynamic(&ch.vertices, &ch.indices, view, env, model, p, pipeline);
        }
    }

    /// Sky dome (per-pixel gradient + sun). `mesh` is a unit-ish dome centred on the eye.
    pub fn draw_sky(&mut self, mesh: &GpuMesh, view: &View, env: &Environment, sky_top: [f32; 3], sky_horizon: [f32; 3]) {
        if self.target == Target::Shadow {
            return;
        }
        let ctx = ctx();
        ctx.apply_pipeline(&self.sky_pipe);
        depth_mask_on();
        let u = SkyUniforms {
            view_proj: view.view_proj,
            model: Mat4::from_translation(view.eye),
            sky_top,
            sky_horizon,
            fog_color: env.fog_color,
            sun_dir: env.sun_dir.to_array(),
            sun_color: env.sun_color,
            params: [if self.target == Target::Scene { 1.0 } else { 0.0 }, 0.0011, 1.0, env.time],
        };
        ctx.apply_uniforms(UniformsSource::table(&u));
        for c in &mesh.chunks {
            ctx.apply_bindings(&Bindings { vertex_buffers: vec![c.vb], index_buffer: c.ib, images: vec![] });
            ctx.draw(0, c.count, 1);
        }
    }

    /// Stylised water. Vertex colour = deep colour, alpha = depth (0..4 m).
    pub fn draw_water(&mut self, mesh: &GpuMesh, view: &View, env: &Environment, sky_color: [f32; 3], shallow: [f32; 3]) {
        if self.target == Target::Shadow {
            return;
        }
        let ctx = ctx();
        ctx.apply_pipeline(&self.water_pipe);
        depth_mask_on();
        let u = WaterUniforms {
            view_proj: view.view_proj,
            sun_dir: env.sun_dir.to_array(),
            sun_color: env.sun_color,
            sky_color,
            fog_color: env.fog_color,
            fog_sun: env.fog_sun(),
            cam_pos: view.eye.to_array(),
            shallow,
            params: [env.fog_density, if self.target == Target::Scene { 1.0 } else { 0.0 }, env.time, 0.09],
        };
        ctx.apply_uniforms(UniformsSource::table(&u));
        for c in &mesh.chunks {
            if !view.aabb_visible(c.min - Vec3::Y, c.max + Vec3::Y) {
                continue;
            }
            ctx.apply_bindings(&Bindings { vertex_buffers: vec![c.vb], index_buffer: c.ib, images: vec![] });
            ctx.draw(0, c.count, 1);
        }
    }
}

/// Check the offscreen framebuffer is usable (binds it, queries status, unbinds).
fn framebuffer_complete(ctx: &mut dyn RenderingBackend, pass: RenderPass) -> bool {
    ctx.begin_pass(Some(pass), PassAction::Nothing);
    let status = unsafe { macroquad::miniquad::gl::glCheckFramebufferStatus(macroquad::miniquad::gl::GL_FRAMEBUFFER) };
    ctx.end_render_pass();
    status == macroquad::miniquad::gl::GL_FRAMEBUFFER_COMPLETE
}
