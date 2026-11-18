//! Post-processing: the 3D view is rendered into an offscreen target, then bloomed
//! (emissive mask in alpha + soft luma knee), colour graded, vignetted and anti-aliased
//! (MSAA resolve when available, FXAA otherwise) onto the default framebuffer per viewport.
use super::shaders;
use macroquad::miniquad::*;

/// Per-biome cozy colour grade
#[derive(Clone, Copy, Debug)]
pub struct Grade {
    /// multiplier applied in the shadows (split toning)
    pub lift: [f32; 3],
    /// multiplier applied in the highlights
    pub gain: [f32; 3],
    pub saturation: f32,
    pub contrast: f32,
    pub vignette: f32,
    pub warmth: f32,
    pub bloom: f32,
    /// extra wide-halation amount
    pub softness: f32,
    pub exposure: f32,
    /// luma threshold for the bloom knee (bright sky, snow glints)
    pub threshold: f32,
    pub knee: f32,
}

impl Default for Grade {
    fn default() -> Self {
        Grade {
            lift: [0.97, 0.96, 1.03],
            gain: [1.03, 1.0, 0.96],
            saturation: 1.06,
            contrast: 1.04,
            vignette: 0.55,
            warmth: 0.5,
            bloom: 0.9,
            softness: 0.15,
            exposure: 1.0,
            threshold: 0.86,
            knee: 0.6,
        }
    }
}

#[repr(C)]
struct PassUniforms {
    rect: [f32; 4],
    texel: [f32; 4],
}

#[repr(C)]
struct CompositeUniforms {
    rect: [f32; 4],
    texel: [f32; 4],
    lift: [f32; 3],
    gain: [f32; 3],
    grade: [f32; 4],
    extra: [f32; 4],
}

struct Target {
    tex: TextureId,
    pass: RenderPass,
    w: u32,
    h: u32,
}

impl Target {
    fn new(ctx: &mut dyn RenderingBackend, w: u32, h: u32) -> Target {
        let tex = ctx.new_render_texture(TextureParams { format: TextureFormat::RGBA8, width: w, height: h, min_filter: FilterMode::Linear, mag_filter: FilterMode::Linear, wrap: TextureWrap::Clamp, ..Default::default() });
        let pass = ctx.new_render_pass(tex, None);
        Target { tex, pass, w, h }
    }
    fn delete(self, ctx: &mut dyn RenderingBackend) {
        ctx.delete_render_pass(self.pass);
        ctx.delete_texture(self.tex);
    }
}

/// Size-dependent render targets
pub struct PostTargets {
    pub w: u32,
    pub h: u32,
    pub msaa: bool,
    /// colour attachment (MSAA renderbuffer when `msaa`)
    color: TextureId,
    depth: TextureId,
    /// resolved texture to sample (== color without MSAA)
    pub scene_tex: TextureId,
    resolve: Option<TextureId>,
    pub scene_pass: RenderPass,
    qa: Target,
    qb: Target,
    ea: Target,
    eb: Target,
}

fn depth_format() -> TextureFormat {
    // 16-bit depth is far too coarse for a 0.25..2200 m frustum; desktop GL gets a float/24+ bit
    // buffer. GLES2 (Android later) only guarantees 16-bit depth textures.
    if cfg!(any(target_os = "android", target_arch = "wasm32")) { TextureFormat::Depth } else { TextureFormat::Depth32 }
}

impl PostTargets {
    pub fn new(ctx: &mut dyn RenderingBackend, w: u32, h: u32, want_msaa: bool) -> PostTargets {
        let msaa = want_msaa && ctx.info().features.resolve_attachments;
        let samples = if msaa { 4 } else { 1 };
        let color = ctx.new_render_texture(TextureParams { format: TextureFormat::RGBA8, width: w, height: h, min_filter: FilterMode::Linear, mag_filter: FilterMode::Linear, sample_count: samples, ..Default::default() });
        let depth = ctx.new_render_texture(TextureParams { format: depth_format(), width: w, height: h, min_filter: FilterMode::Nearest, mag_filter: FilterMode::Nearest, sample_count: samples, ..Default::default() });
        let (scene_tex, resolve, scene_pass) = if msaa {
            let r = ctx.new_render_texture(TextureParams { format: TextureFormat::RGBA8, width: w, height: h, min_filter: FilterMode::Linear, mag_filter: FilterMode::Linear, ..Default::default() });
            let pass = ctx.new_render_pass_mrt(&[color], Some(&[r]), Some(depth));
            (r, Some(r), pass)
        } else {
            (color, None, ctx.new_render_pass(color, Some(depth)))
        };
        let q = |d: u32| ((w / d).max(1), (h / d).max(1));
        let (qw, qh) = q(4);
        let (ew, eh) = q(8);
        PostTargets {
            w,
            h,
            msaa,
            color,
            depth,
            scene_tex,
            resolve,
            scene_pass,
            qa: Target::new(ctx, qw, qh),
            qb: Target::new(ctx, qw, qh),
            ea: Target::new(ctx, ew, eh),
            eb: Target::new(ctx, ew, eh),
        }
    }

    pub fn delete(self, ctx: &mut dyn RenderingBackend) {
        ctx.delete_render_pass(self.scene_pass);
        ctx.delete_texture(self.color);
        ctx.delete_texture(self.depth);
        if let Some(r) = self.resolve {
            ctx.delete_texture(r);
        }
        self.qa.delete(ctx);
        self.qb.delete(ctx);
        self.ea.delete(ctx);
        self.eb.delete(ctx);
    }
}

/// Size-independent post pipelines
pub struct PostPipes {
    bright: Pipeline,
    down: Pipeline,
    blur: Pipeline,
    composite: Pipeline,
    quad_vb: BufferId,
    quad_ib: BufferId,
}

fn pass_meta() -> ShaderMeta {
    ShaderMeta {
        images: vec!["tex".to_string()],
        uniforms: UniformBlockLayout { uniforms: vec![UniformDesc::new("rect", UniformType::Float4), UniformDesc::new("texel", UniformType::Float4)] },
    }
}

fn composite_meta() -> ShaderMeta {
    ShaderMeta {
        images: vec!["scene".to_string(), "bloom1".to_string(), "bloom2".to_string()],
        uniforms: UniformBlockLayout {
            uniforms: vec![
                UniformDesc::new("rect", UniformType::Float4),
                UniformDesc::new("texel", UniformType::Float4),
                UniformDesc::new("lift", UniformType::Float3),
                UniformDesc::new("gain", UniformType::Float3),
                UniformDesc::new("grade", UniformType::Float4),
                UniformDesc::new("extra", UniformType::Float4),
            ],
        },
    }
}

/// Screen rectangle in physical pixels, origin top-left
#[derive(Clone, Copy, Debug)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PostPipes {
    pub fn new(ctx: &mut dyn RenderingBackend) -> PostPipes {
        let mk = |ctx: &mut dyn RenderingBackend, fs: &str, meta: ShaderMeta| -> Pipeline {
            let sh = ctx.new_shader(ShaderSource::Glsl { vertex: shaders::POST_VS, fragment: fs }, meta).unwrap_or_else(|e| panic!("post shader compile failed: {e:?}"));
            ctx.new_pipeline(&[BufferLayout::default()], &[VertexAttribute::new("in_pos", VertexFormat::Float2)], sh, PipelineParams { depth_test: Comparison::Always, depth_write: false, ..Default::default() })
        };
        let bright = mk(ctx, shaders::BRIGHT_FS, pass_meta());
        let down = mk(ctx, shaders::DOWN_FS, pass_meta());
        let blur = mk(ctx, shaders::BLUR_FS, pass_meta());
        let composite = mk(ctx, shaders::COMPOSITE_FS, composite_meta());
        let verts: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        let idx: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let quad_vb = ctx.new_buffer(BufferType::VertexBuffer, BufferUsage::Immutable, BufferSource::slice(&verts));
        let quad_ib = ctx.new_buffer(BufferType::IndexBuffer, BufferUsage::Immutable, BufferSource::slice(&idx));
        PostPipes { bright, down, blur, composite, quad_vb, quad_ib }
    }

    fn run(&self, ctx: &mut dyn RenderingBackend, pipe: &Pipeline, src: TextureId, dst: &Target, texel: [f32; 4]) {
        ctx.begin_pass(Some(dst.pass), PassAction::Nothing);
        ctx.apply_viewport(0, 0, dst.w as i32, dst.h as i32);
        ctx.apply_scissor_rect(0, 0, dst.w as i32, dst.h as i32);
        ctx.apply_pipeline(pipe);
        ctx.apply_bindings(&Bindings { vertex_buffers: vec![self.quad_vb], index_buffer: self.quad_ib, images: vec![src] });
        ctx.apply_uniforms(UniformsSource::table(&PassUniforms { rect: [0.0, 0.0, 1.0, 1.0], texel }));
        ctx.draw(0, 6, 1);
        ctx.end_render_pass();
    }

    /// Bloom chain over the whole scene target, then composite each viewport to the screen.
    pub fn finish(&self, ctx: &mut dyn RenderingBackend, t: &PostTargets, viewports: &[PxRect], grade: &Grade, time: f32) {
        let (sw, sh) = (t.w as f32, t.h as f32);
        // bright pass: full -> 1/4 (4 bilinear taps cover a 4x4 block)
        self.run(ctx, &self.bright, t.scene_tex, &t.qa, [1.0 / sw, 1.0 / sh, grade.threshold, grade.knee]);
        let (qw, qh) = (t.qa.w as f32, t.qa.h as f32);
        self.run(ctx, &self.blur, t.qa.tex, &t.qb, [1.0 / qw, 0.0, 0.0, 0.0]);
        self.run(ctx, &self.blur, t.qb.tex, &t.qa, [0.0, 1.0 / qh, 0.0, 0.0]);
        // wide level: 1/4 -> 1/8, blurred twice for a soft halo
        self.run(ctx, &self.down, t.qa.tex, &t.ea, [0.5 / qw, 0.5 / qh, 0.0, 0.0]);
        let (ew, eh) = (t.ea.w as f32, t.ea.h as f32);
        for _ in 0..2 {
            self.run(ctx, &self.blur, t.ea.tex, &t.eb, [1.0 / ew, 0.0, 0.0, 0.0]);
            self.run(ctx, &self.blur, t.eb.tex, &t.ea, [0.0, 1.0 / eh, 0.0, 0.0]);
        }
        // composite
        ctx.begin_default_pass(PassAction::Nothing);
        ctx.apply_pipeline(&self.composite);
        for vp in viewports {
            let gl_y = t.h as i32 - (vp.y + vp.h);
            ctx.apply_viewport(vp.x, gl_y, vp.w, vp.h);
            ctx.apply_scissor_rect(vp.x, gl_y, vp.w, vp.h);
            ctx.apply_bindings(&Bindings { vertex_buffers: vec![self.quad_vb], index_buffer: self.quad_ib, images: vec![t.scene_tex, t.qa.tex, t.ea.tex] });
            let u = CompositeUniforms {
                rect: [vp.x as f32 / sw, gl_y as f32 / sh, vp.w as f32 / sw, vp.h as f32 / sh],
                texel: [1.0 / sw, 1.0 / sh, if t.msaa { 0.0 } else { 1.0 }, grade.bloom],
                lift: grade.lift,
                gain: grade.gain,
                grade: [grade.saturation, grade.contrast, grade.vignette, grade.warmth],
                extra: [vp.w as f32 / vp.h.max(1) as f32, grade.softness, time, grade.exposure],
            };
            ctx.apply_uniforms(UniformsSource::table(&u));
            ctx.draw(0, 6, 1);
        }
        ctx.end_render_pass();
    }
}
