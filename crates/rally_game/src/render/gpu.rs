//! Thin layer over miniquad: one lit/fogged vertex-colour shader, an opaque and an
//! alpha-blended pipeline, static GPU meshes with bounding boxes for frustum culling,
//! and a streaming buffer for dynamic geometry (particles, skid marks, shadows).
use super::mesh::{MeshBuilder, Vertex};
use macroquad::math::{Mat4, Vec3, Vec4};
use macroquad::miniquad::*;
use macroquad::window::get_internal_gl;

const VERTEX: &str = r#"#version 100
attribute vec3 in_pos;
attribute vec3 in_normal;
attribute vec4 in_color;
uniform mat4 mvp;
uniform mat4 model;
varying lowp vec4 v_color;
varying vec3 v_normal;
varying vec3 v_world;
void main() {
    vec4 w = model * vec4(in_pos, 1.0);
    v_world = w.xyz;
    v_normal = (model * vec4(in_normal, 0.0)).xyz;
    v_color = in_color / 255.0;
    gl_Position = mvp * vec4(in_pos, 1.0);
}
"#;

const FRAGMENT: &str = r#"#version 100
precision mediump float;
varying lowp vec4 v_color;
varying vec3 v_normal;
varying vec3 v_world;
uniform vec3 sun_dir;
uniform vec3 sun_color;
uniform vec3 ambient;
uniform vec3 fog_color;
uniform vec3 cam_pos;
uniform vec4 tint;
uniform vec4 params; // x = fog density, y = unlit (0..1), z = time, w = rim strength
void main() {
    vec3 n = normalize(v_normal);
    if (!gl_FrontFacing) { n = -n; }
    float d = dot(n, sun_dir);
    float wrap = clamp((d + 0.25) / 1.25, 0.0, 1.0);
    // soft toon step for a cozy painted look
    wrap = mix(wrap, smoothstep(0.15, 0.6, wrap), 0.35);
    float hemi = n.y * 0.5 + 0.5;
    vec3 amb = ambient * mix(0.55, 1.15, hemi);
    vec3 base = v_color.rgb * tint.rgb;
    vec3 lit = base * (amb + sun_color * wrap * 0.85);
    vec3 view = normalize(cam_pos - v_world);
    float rim = pow(1.0 - clamp(dot(n, view), 0.0, 1.0), 3.0) * params.w;
    lit += rim * sun_color * 0.25;
    vec3 col = mix(lit, base, params.y);
    float dist = length(v_world - cam_pos);
    float f = 1.0 - exp(-pow(dist * params.x, 1.5));
    col = mix(col, fog_color, clamp(f, 0.0, 1.0));
    gl_FragColor = vec4(col, v_color.a * tint.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Uniforms {
    pub mvp: Mat4,
    pub model: Mat4,
    pub sun_dir: [f32; 3],
    pub sun_color: [f32; 3],
    pub ambient: [f32; 3],
    pub fog_color: [f32; 3],
    pub cam_pos: [f32; 3],
    pub tint: [f32; 4],
    pub params: [f32; 4],
}

fn meta() -> ShaderMeta {
    ShaderMeta {
        images: vec![],
        uniforms: UniformBlockLayout {
            uniforms: vec![
                UniformDesc::new("mvp", UniformType::Mat4),
                UniformDesc::new("model", UniformType::Mat4),
                UniformDesc::new("sun_dir", UniformType::Float3),
                UniformDesc::new("sun_color", UniformType::Float3),
                UniformDesc::new("ambient", UniformType::Float3),
                UniformDesc::new("fog_color", UniformType::Float3),
                UniformDesc::new("cam_pos", UniformType::Float3),
                UniformDesc::new("tint", UniformType::Float4),
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
    pub ambient: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_density: f32,
    pub time: f32,
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
}

/// Draw style flags
#[derive(Clone, Copy, Debug)]
pub struct DrawParams {
    pub tint: [f32; 4],
    pub unlit: f32,
    pub rim: f32,
    pub fog: bool,
}

impl Default for DrawParams {
    fn default() -> Self {
        DrawParams { tint: [1.0; 4], unlit: 0.0, rim: 0.0, fog: true }
    }
}

pub struct Gpu {
    pub opaque: Pipeline,
    pub alpha: Pipeline,
    /// Alpha-blended, no depth test (overlays)
    pub overlay: Pipeline,
    stream_vb: BufferId,
    stream_ib: BufferId,
    stream_cap: usize,
}

pub const STREAM_VERTS: usize = 60_000;

/// miniquad 0.4 only enables GL_DEPTH_TEST for pipelines that also write depth; transparent
/// pipelines still need depth testing, so re-enable it by hand after applying them.
fn force_depth_test() {
    unsafe {
        macroquad::miniquad::gl::glEnable(macroquad::miniquad::gl::GL_DEPTH_TEST);
        macroquad::miniquad::gl::glDepthFunc(macroquad::miniquad::gl::GL_LEQUAL);
    }
}

fn ctx() -> &'static mut dyn RenderingBackend {
    unsafe { get_internal_gl().quad_context }
}

impl Gpu {
    pub fn new() -> Gpu {
        let ctx = ctx();
        let shader = ctx
            .new_shader(ShaderSource::Glsl { vertex: VERTEX, fragment: FRAGMENT }, meta())
            .unwrap_or_else(|e| panic!("shader compile failed: {e:?}"));
        let layout = [BufferLayout::default()];
        let attrs = [
            VertexAttribute::new("in_pos", VertexFormat::Float3),
            VertexAttribute::new("in_normal", VertexFormat::Float3),
            VertexAttribute::new("in_color", VertexFormat::Byte4),
        ];
        let opaque = ctx.new_pipeline(
            &layout,
            &attrs,
            shader,
            PipelineParams { depth_test: Comparison::LessOrEqual, depth_write: true, cull_face: CullFace::Nothing, ..Default::default() },
        );
        let blend = Some(BlendState::new(Equation::Add, BlendFactor::Value(BlendValue::SourceAlpha), BlendFactor::OneMinusValue(BlendValue::SourceAlpha)));
        let alpha = ctx.new_pipeline(
            &layout,
            &attrs,
            shader,
            PipelineParams {
                depth_test: Comparison::LessOrEqual,
                depth_write: false,
                cull_face: CullFace::Nothing,
                color_blend: blend,
                alpha_blend: Some(BlendState::new(Equation::Add, BlendFactor::Zero, BlendFactor::One)),
                ..Default::default()
            },
        );
        let overlay = ctx.new_pipeline(
            &layout,
            &attrs,
            shader,
            PipelineParams {
                depth_test: Comparison::Always,
                depth_write: false,
                cull_face: CullFace::Nothing,
                color_blend: blend,
                alpha_blend: Some(BlendState::new(Equation::Add, BlendFactor::Zero, BlendFactor::One)),
                ..Default::default()
            },
        );
        let stream_vb = ctx.new_buffer(BufferType::VertexBuffer, BufferUsage::Stream, BufferSource::empty::<Vertex>(STREAM_VERTS));
        let stream_ib = ctx.new_buffer(BufferType::IndexBuffer, BufferUsage::Stream, BufferSource::empty::<u16>(STREAM_VERTS * 2));
        Gpu { opaque, alpha, overlay, stream_vb, stream_ib, stream_cap: STREAM_VERTS }
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

    /// Flush macroquad's 2D batch and start a raw 3D pass in the given viewport
    /// (physical pixels, origin top-left).
    pub fn begin_3d(&mut self, x: i32, y: i32, w: i32, h: i32, clear_depth: bool) {
        unsafe {
            get_internal_gl().flush();
        }
        let ctx = ctx();
        let (_, sh) = macroquad::miniquad::window::screen_size();
        ctx.begin_default_pass(PassAction::Nothing);
        let gl_y = sh as i32 - (y + h);
        ctx.apply_viewport(x, gl_y, w, h);
        ctx.apply_scissor_rect(x, gl_y, w, h);
        if clear_depth {
            // glClear ignores the depth buffer while depth writes are masked (macroquad's 2D
            // pipeline leaves them off), so bind a depth-writing pipeline first.
            ctx.apply_pipeline(&self.opaque);
            ctx.clear(None, Some(1.0), None);
        }
    }

    pub fn end_3d(&mut self) {
        let ctx = ctx();
        ctx.end_render_pass();
    }

    fn uniforms(&self, view: &View, env: &Environment, model: Mat4, p: &DrawParams) -> Uniforms {
        Uniforms {
            mvp: view.view_proj * model,
            model,
            sun_dir: env.sun_dir.to_array(),
            sun_color: env.sun_color,
            ambient: env.ambient,
            fog_color: env.fog_color,
            cam_pos: view.eye.to_array(),
            tint: p.tint,
            params: [if p.fog { env.fog_density } else { 0.0 }, p.unlit, env.time, p.rim],
        }
    }

    /// Draw a static mesh. Culls chunks by their (transformed) bounds when model is a
    /// pure translation/identity; otherwise tests a bounding sphere.
    pub fn draw(&mut self, mesh: &GpuMesh, view: &View, env: &Environment, model: Mat4, p: &DrawParams, alpha: bool) {
        let ctx = ctx();
        ctx.apply_pipeline(if alpha { &self.alpha } else { &self.opaque });
        if alpha {
            force_depth_test();
        }
        let u = self.uniforms(view, env, model, p);
        ctx.apply_uniforms(UniformsSource::table(&u));
        let identity_rot = model.x_axis.truncate() == Vec3::X && model.y_axis.truncate() == Vec3::Y && model.z_axis.truncate() == Vec3::Z;
        let t = model.w_axis.truncate();
        for c in &mesh.chunks {
            let visible = if identity_rot {
                view.aabb_visible(c.min + t, c.max + t)
            } else {
                let center = model.transform_point3((c.min + c.max) * 0.5);
                let scale = model.x_axis.truncate().length().max(model.y_axis.truncate().length()).max(model.z_axis.truncate().length());
                view.sphere_visible(center, (c.max - c.min).length() * 0.5 * scale)
            };
            if !visible {
                continue;
            }
            ctx.apply_bindings(&Bindings { vertex_buffers: vec![c.vb], index_buffer: c.ib, images: vec![] });
            ctx.draw(0, c.count, 1);
        }
    }

    /// Draw immediate geometry (re-uploaded every call). Pipeline: 0 opaque, 1 alpha, 2 overlay.
    pub fn draw_dynamic(&mut self, verts: &[Vertex], indices: &[u16], view: &View, env: &Environment, model: Mat4, p: &DrawParams, pipeline: u8) {
        if indices.is_empty() {
            return;
        }
        let n = verts.len().min(self.stream_cap);
        let ctx = ctx();
        ctx.buffer_update(self.stream_vb, BufferSource::slice(&verts[..n]));
        let ni = indices.len().min(self.stream_cap * 2);
        ctx.buffer_update(self.stream_ib, BufferSource::slice(&indices[..ni]));
        let pl = match pipeline {
            0 => &self.opaque,
            1 => &self.alpha,
            _ => &self.overlay,
        };
        ctx.apply_pipeline(pl);
        if pipeline == 1 {
            force_depth_test();
        }
        let u = self.uniforms(view, env, model, p);
        ctx.apply_uniforms(UniformsSource::table(&u));
        ctx.apply_bindings(&Bindings { vertex_buffers: vec![self.stream_vb], index_buffer: self.stream_ib, images: vec![] });
        ctx.draw(0, ni as i32, 1);
    }

    /// Draw a MeshBuilder's chunks through the stream buffer (for per-frame geometry).
    pub fn draw_builder(&mut self, mesh: &MeshBuilder, view: &View, env: &Environment, model: Mat4, p: &DrawParams, pipeline: u8) {
        for ch in &mesh.chunks {
            self.draw_dynamic(&ch.vertices, &ch.indices, view, env, model, p, pipeline);
        }
    }
}
