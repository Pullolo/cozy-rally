//! Tiny immediate-mode UI with a cozy look: rounded pastel panels, chunky buttons,
//! mouse / touch / keyboard navigation, and a friendly rounded font (Fredoka, OFL).
use macroquad::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub const CREAM: Color = Color::new(1.0, 0.965, 0.90, 1.0);
/// Slightly deeper cream for insets / tracks
pub const CREAM_DEEP: Color = Color::new(0.95, 0.89, 0.80, 1.0);
pub const INK: Color = Color::new(0.24, 0.20, 0.27, 1.0);
pub const INK_SOFT: Color = Color::new(0.45, 0.40, 0.48, 1.0);
pub const PEACH: Color = Color::new(0.98, 0.62, 0.52, 1.0);
pub const MINT: Color = Color::new(0.55, 0.82, 0.68, 1.0);
pub const SKY: Color = Color::new(0.55, 0.75, 0.95, 1.0);
pub const BUTTER: Color = Color::new(0.99, 0.84, 0.45, 1.0);
pub const LAVENDER: Color = Color::new(0.75, 0.66, 0.92, 1.0);
pub const SHADOW: Color = Color::new(0.15, 0.10, 0.20, 0.25);
/// Warm red-peach for warnings (wrong way, redline, lost grip)
pub const BERRY: Color = Color::new(0.93, 0.42, 0.42, 1.0);

/// The one embedded asset: Fredoka SemiBold (SIL Open Font License, see assets/fonts/OFL.txt)
static FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/Fredoka-SemiBold.ttf");

/// Load the rounded UI font and make it macroquad's default (so every draw_text uses it).
pub fn init_font() {
    match load_ttf_font_from_bytes(FONT_BYTES) {
        Ok(mut f) => {
            f.set_filter(FilterMode::Linear);
            set_default_font(f);
        }
        Err(e) => eprintln!("font load failed, using default: {e:?}"),
    }
}

// ---------------------------------------------------------------- input blocking

static INPUT_BLOCKED: AtomicBool = AtomicBool::new(false);

/// While a screen transition runs, widgets ignore input (no double activations).
pub fn set_input_blocked(b: bool) {
    INPUT_BLOCKED.store(b, Ordering::Relaxed);
}

pub fn input_blocked() -> bool {
    INPUT_BLOCKED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------- colour helpers

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, a)
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::new(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, a.a + (b.a - a.a) * t)
}

pub fn shade(c: Color, f: f32) -> Color {
    Color::new((c.r * f).min(1.0), (c.g * f).min(1.0), (c.b * f).min(1.0), c.a)
}

pub fn lighten(c: Color, t: f32) -> Color {
    mix(c, Color::new(1.0, 1.0, 1.0, c.a), t)
}

pub fn rgb3(c: [f32; 3]) -> Color {
    Color::new(c[0], c[1], c[2], 1.0)
}

/// Smooth exponential approach, frame-rate independent
pub fn approach(v: f32, target: f32, rate: f32, dt: f32) -> f32 {
    v + (target - v) * (1.0 - (-rate * dt).exp())
}

/// 0 -> 1 with a springy overshoot (for pop-in animations)
pub fn ease_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.9;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Damped spring "pop": starts at 0, overshoots, settles at 1
pub fn pop(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    1.0 - (-t * 9.0).exp() * (t * 16.0).cos()
}

// ---------------------------------------------------------------- shapes

/// UI scale factor so layouts work from phones to 4K
pub fn scale() -> f32 {
    (screen_height() / 720.0).min(screen_width() / 1100.0).clamp(0.55, 3.0)
}

fn vtx(x: f32, y: f32, c: Color) -> Vertex {
    Vertex::new(x, y, 0.0, 0.0, 0.0, c)
}

fn corner_segments(r: f32) -> usize {
    ((r * screen_dpi_scale()).sqrt() * 1.6).clamp(3.0, 18.0) as usize
}

/// Outline points of a rounded rectangle (clockwise from the top-left corner)
fn rounded_points(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<Vec2> {
    rounded_points_n(x, y, w, h, r, corner_segments(r))
}

fn rounded_points_n(x: f32, y: f32, w: f32, h: f32, r: f32, n: usize) -> Vec<Vec2> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let mut pts = Vec::with_capacity(4 * (n + 1));
    let corners = [
        (x + r, y + r, std::f32::consts::PI),
        (x + w - r, y + r, std::f32::consts::PI * 1.5),
        (x + w - r, y + h - r, 0.0),
        (x + r, y + h - r, std::f32::consts::PI * 0.5),
    ];
    for (cx, cy, a0) in corners {
        for k in 0..=n {
            let a = a0 + k as f32 / n as f32 * std::f32::consts::FRAC_PI_2;
            pts.push(vec2(cx + a.cos() * r, cy + a.sin() * r));
        }
    }
    pts
}

/// Rounded rectangle with a vertical gradient (no overlapping geometry, so alpha is clean)
pub fn rounded_rect_gradient(x: f32, y: f32, w: f32, h: f32, r: f32, top: Color, bottom: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let pts = rounded_points(x, y, w, h, r);
    let col = |py: f32| mix(top, bottom, (py - y) / h);
    let mut vertices = Vec::with_capacity(pts.len() + 1);
    vertices.push(vtx(x + w * 0.5, y + h * 0.5, col(y + h * 0.5)));
    for p in &pts {
        vertices.push(vtx(p.x, p.y, col(p.y)));
    }
    let n = pts.len() as u16;
    let mut indices = Vec::with_capacity(pts.len() * 3);
    for i in 0..n {
        indices.extend_from_slice(&[0, 1 + i, 1 + (i + 1) % n]);
    }
    draw_mesh(&Mesh { vertices, indices, texture: None });
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32, color: Color) {
    rounded_rect_gradient(x, y, w, h, r, color, color);
}

/// Rounded outline ring of thickness `t` drawn inside the rect
pub fn rounded_rect_outline(x: f32, y: f32, w: f32, h: f32, r: f32, t: f32, color: Color) {
    let seg = corner_segments(r);
    let outer = rounded_points_n(x, y, w, h, r, seg);
    let inner = rounded_points_n(x + t, y + t, w - 2.0 * t, h - 2.0 * t, (r - t).max(0.5), seg);
    let n = outer.len();
    let mut vertices = Vec::with_capacity(n * 2);
    for i in 0..n {
        vertices.push(vtx(outer[i].x, outer[i].y, color));
        vertices.push(vtx(inner[i].x, inner[i].y, color));
    }
    let mut indices = Vec::with_capacity(n * 6);
    for i in 0..n as u16 {
        let a = i * 2;
        let b = ((i + 1) % n as u16) * 2;
        indices.extend_from_slice(&[a, a + 1, b, b, a + 1, b + 1]);
    }
    draw_mesh(&Mesh { vertices, indices, texture: None });
}

/// Soft blurred drop shadow under a rounded rect
pub fn soft_shadow(x: f32, y: f32, w: f32, h: f32, r: f32, strength: f32) {
    let s = scale();
    for (grow, a) in [(10.0, 0.035), (6.0, 0.05), (3.0, 0.07), (0.0, 0.09)] {
        let g = grow * s;
        rounded_rect(x - g, y + 6.0 * s - g * 0.5, w + 2.0 * g, h + 2.0 * g, r + g, Color::new(0.2, 0.12, 0.25, a * strength));
    }
}

pub fn circle(x: f32, y: f32, r: f32, color: Color) {
    let sides = ((r * screen_dpi_scale()).sqrt() * 4.0).clamp(12.0, 64.0) as u8;
    draw_poly(x, y, sides, r, 0.0, color);
}

/// Ring segment between radii r0..r1 from angle a0 to a1 (radians, clockwise on screen)
pub fn arc(cx: f32, cy: f32, r0: f32, r1: f32, a0: f32, a1: f32, color: Color) {
    arc_gradient(cx, cy, r0, r1, a0, a1, color, color);
}

pub fn arc_gradient(cx: f32, cy: f32, r0: f32, r1: f32, a0: f32, a1: f32, c0: Color, c1: Color) {
    let span = (a1 - a0).abs();
    if span < 1e-4 {
        return;
    }
    let n = ((span * r1 * screen_dpi_scale()).sqrt() * 2.0).clamp(4.0, 96.0) as usize;
    let mut vertices = Vec::with_capacity((n + 1) * 2);
    for k in 0..=n {
        let f = k as f32 / n as f32;
        let a = a0 + (a1 - a0) * f;
        let c = mix(c0, c1, f);
        vertices.push(vtx(cx + a.cos() * r0, cy + a.sin() * r0, c));
        vertices.push(vtx(cx + a.cos() * r1, cy + a.sin() * r1, c));
    }
    let mut indices = Vec::with_capacity(n * 6);
    for k in 0..n as u16 {
        let a = k * 2;
        indices.extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
    }
    draw_mesh(&Mesh { vertices, indices, texture: None });
}

/// Cozy panel: soft shadow, cream body with a gentle gradient, light rim.
pub fn panel(x: f32, y: f32, w: f32, h: f32) {
    panel_tinted(x, y, w, h, CREAM, 1.0);
}

pub fn panel_tinted(x: f32, y: f32, w: f32, h: f32, color: Color, alpha: f32) {
    let s = scale();
    let r = (18.0 * s).min(h * 0.5);
    soft_shadow(x, y, w, h, r, alpha);
    rounded_rect_gradient(x, y, w, h, r, with_alpha(lighten(color, 0.35), alpha), with_alpha(color, alpha));
    rounded_rect_outline(x, y, w, h, r, 1.5 * s, Color::new(1.0, 1.0, 1.0, 0.55 * alpha));
}

/// Small rounded pill (badge) with centred text. Returns its width.
pub fn pill(label: &str, cx: f32, cy: f32, size: f32, bg: Color, fg: Color) -> f32 {
    let d = text_size(label, size);
    let w = d.x + size * 1.1;
    let h = size * 1.45;
    rounded_rect(cx - w * 0.5, cy - h * 0.5, w, h, h * 0.5, bg);
    text_mid(label, cx, cy, size, fg);
    w
}

/// Little filled triangle arrow pointing up (dir = 1) or down (dir = -1)
pub fn arrow_tri(cx: f32, cy: f32, size: f32, dir: f32, color: Color) {
    let h = size * 0.5 * dir;
    draw_triangle(vec2(cx, cy - h), vec2(cx - size * 0.55, cy + h), vec2(cx + size * 0.55, cy + h), color);
}

// ---------------------------------------------------------------- text

/// Rasterise at quantised sizes (≈6% steps) and scale the rest, so animated text
/// doesn't fill the glyph atlas with hundreds of sizes.
fn quantize(size: f32) -> (u16, f32) {
    let size = size.max(1.0);
    if size <= 20.0 {
        let b = size.round().max(1.0);
        return (b as u16, size / b);
    }
    let step = 1.06f32.ln();
    let b = (size.ln() / step).round() * step;
    let b = b.exp().round().max(1.0);
    (b as u16, size / b)
}

fn params(size: f32, color: Color) -> TextParams<'static> {
    let (fs, sc) = quantize(size);
    TextParams { font: None, font_size: fs, font_scale: sc, font_scale_aspect: 1.0, rotation: 0.0, color }
}

pub fn text_size(text: &str, size: f32) -> Vec2 {
    let (fs, sc) = quantize(size);
    let d = measure_text(text, None, fs, sc);
    vec2(d.width, d.height)
}

/// Draw text with its baseline at y
pub fn text(text: &str, x: f32, y: f32, size: f32, color: Color) {
    draw_text_ex(text, x, y, params(size, color));
}

pub fn text_centered(t: &str, cx: f32, y: f32, size: f32, color: Color) {
    let d = text_size(t, size);
    text(t, cx - d.x * 0.5, y, size, color);
}

pub fn text_right(t: &str, rx: f32, y: f32, size: f32, color: Color) {
    let d = text_size(t, size);
    text(t, rx - d.x, y, size, color);
}

/// Baseline offset that visually centres cap-height text on a line
pub fn mid_baseline(cy: f32, size: f32) -> f32 {
    cy + size * 0.34
}

/// Text centred horizontally and vertically on (cx, cy)
pub fn text_mid(t: &str, cx: f32, cy: f32, size: f32, color: Color) {
    text_centered(t, cx, mid_baseline(cy, size), size, color);
}

/// Text with a soft ink outline + drop shadow - readable on top of the 3D scene
pub fn text_outlined(t: &str, x: f32, y: f32, size: f32, color: Color) {
    let o = (size / 16.0).max(1.0);
    let sh = Color::new(0.2, 0.13, 0.25, 0.30 * color.a);
    text(t, x, y + o * 1.6, size, sh);
    let ol = Color::new(0.24, 0.18, 0.27, 0.85 * color.a);
    for k in 0..8 {
        let a = k as f32 / 8.0 * std::f32::consts::TAU;
        text(t, x + a.cos() * o, y + a.sin() * o, size, ol);
    }
    text(t, x, y, size, color);
}

pub fn text_outlined_centered(t: &str, cx: f32, y: f32, size: f32, color: Color) {
    let d = text_size(t, size);
    text_outlined(t, cx - d.x * 0.5, y, size, color);
}

// ---------------------------------------------------------------- focus / navigation

/// Keyboard/gamepad-style focus for a vertical list of widgets.
#[derive(Default)]
pub struct Focus {
    pub index: usize,
    pub count: usize,
    pub activated: bool,
    pub left: bool,
    pub right: bool,
    /// Escape / Backspace pressed this frame (screens decide what "back" means)
    pub back: bool,
    counter: usize,
    /// Per-widget smoothed highlight (0..1) for hover/focus animations
    anim: Vec<f32>,
    /// Widget currently being dragged with the mouse/touch (sliders)
    drag: Option<usize>,
    /// Set once the pointer moved (so keyboard users don't lose focus to a resting cursor)
    pointer_active: bool,
    /// Optional fixed label column width for selectors (aligns arrows in narrow columns)
    pub label_w: Option<f32>,
}

impl Focus {
    /// Call at the start of a frame with the number of focusable items
    pub fn begin(&mut self, count: usize) {
        self.count = count.max(1);
        self.counter = 0;
        self.activated = false;
        self.left = false;
        self.right = false;
        self.back = false;
        if self.anim.len() < self.count {
            self.anim.resize(self.count, 0.0);
        }
        if input_blocked() {
            return;
        }
        if is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::S) || is_key_pressed(KeyCode::Tab) {
            self.index = (self.index + 1) % self.count;
            crate::audio::ui_sound(crate::audio::UiSound::Move);
        }
        if is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::W) {
            self.index = (self.index + self.count - 1) % self.count;
            crate::audio::ui_sound(crate::audio::UiSound::Move);
        }
        self.index = self.index.min(self.count - 1);
        self.activated = is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) || is_key_pressed(KeyCode::KpEnter);
        self.left = is_key_pressed(KeyCode::Left) || is_key_pressed(KeyCode::A);
        self.right = is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::D);
        self.back = is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Backspace);
        if mouse_delta_position().length() > 0.0 {
            self.pointer_active = true;
        }
    }

    fn next_id(&mut self) -> usize {
        let id = self.counter;
        self.counter += 1;
        if self.anim.len() <= id {
            self.anim.resize(id + 1, 0.0);
        }
        id
    }

    /// Smoothed 0..1 highlight for widget `id`
    fn highlight(&mut self, id: usize, on: bool) -> f32 {
        let v = approach(self.anim[id], if on { 1.0 } else { 0.0 }, 14.0, get_frame_time().min(0.05));
        self.anim[id] = v;
        v
    }

    /// Mouse moved over widget `id`: give it focus
    fn hover(&mut self, id: usize, hover: bool) {
        if hover && self.pointer_active && mouse_delta_position().length() > 0.0 && self.index != id {
            self.index = id;
            crate::audio::ui_sound(crate::audio::UiSound::Move);
        }
    }
}

fn pointer_pressed_in(r: Rect) -> (bool, bool) {
    if input_blocked() {
        return (false, false);
    }
    let (mx, my) = mouse_position();
    let hover = r.contains(vec2(mx, my));
    let mut clicked = hover && is_mouse_button_pressed(MouseButton::Left);
    for t in touches() {
        if t.phase == TouchPhase::Started && r.contains(t.position) {
            clicked = true;
        }
    }
    (hover, clicked)
}

fn pointer_down_in(r: Rect) -> bool {
    if input_blocked() {
        return false;
    }
    let (mx, my) = mouse_position();
    (r.contains(vec2(mx, my)) && is_mouse_button_down(MouseButton::Left)) || touches().iter().any(|t| t.phase != TouchPhase::Ended && t.phase != TouchPhase::Cancelled && r.contains(t.position))
}

/// Big chunky button. Returns true when activated.
pub fn button(focus: &mut Focus, label: &str, x: f32, y: f32, w: f32, h: f32, color: Color) -> bool {
    let id = focus.next_id();
    let s = scale();
    let r = Rect::new(x, y, w, h);
    let (hover, clicked) = pointer_pressed_in(r);
    focus.hover(id, hover);
    let focused = focus.index == id;
    let hl = focus.highlight(id, focused);
    let pressed = pointer_down_in(r);
    let lip = 5.0 * s;
    let lift = if pressed { -lip * 0.6 } else { hl * 3.0 * s };
    let grow = hl * 4.0 * s;
    let (bx, bw) = (x - grow * 0.5, w + grow);
    let rad = (h * 0.32).min(18.0 * s);
    soft_shadow(bx, y, bw, h, rad, 0.5 + hl * 0.5);
    rounded_rect(bx, y + lip, bw, h, rad, shade(color, 0.78));
    let face = mix(color, lighten(color, 0.18), hl);
    rounded_rect_gradient(bx, y - lift, bw, h, rad, lighten(face, 0.25), face);
    if hl > 0.01 {
        rounded_rect_outline(bx - 3.0 * s, y - lift - 3.0 * s, bw + 6.0 * s, h + 6.0 * s, rad + 3.0 * s, 3.0 * s, Color::new(1.0, 1.0, 1.0, hl * 0.95));
        // little bobbing chevron on the left
        let t = get_time() as f32;
        let cx = bx + 20.0 * s + (t * 5.0).sin() * 2.5 * s;
        let cy = y - lift + h * 0.5;
        let k = 7.0 * s * hl;
        draw_triangle(vec2(cx - k * 0.6, cy - k), vec2(cx - k * 0.6, cy + k), vec2(cx + k * 0.8, cy), with_alpha(INK, hl));
    }
    let fs = (h * 0.46).min(32.0 * s);
    text_mid(label, bx + bw * 0.5, y - lift + h * 0.5, fs, INK);
    let activated = clicked || (focused && focus.activated);
    if activated {
        crate::audio::ui_sound(crate::audio::UiSound::Select);
    }
    activated
}

/// "< value >" selector. Returns -1/0/+1 change.
pub fn selector(focus: &mut Focus, label: &str, value: &str, x: f32, y: f32, w: f32, h: f32, color: Color) -> i32 {
    selector_swatch(focus, label, value, None, x, y, w, h, color)
}

fn row_background(focus: &mut Focus, id: usize, x: f32, y: f32, w: f32, h: f32, color: Color) -> f32 {
    let s = scale();
    let focused = focus.index == id;
    let hl = focus.highlight(id, focused);
    let rad = (h * 0.3).min(14.0 * s);
    let bg = mix(mix(color, CREAM, 0.55), color, hl);
    rounded_rect(x, y, w, h, rad, bg);
    if hl > 0.01 {
        rounded_rect_outline(x - 2.5 * s, y - 2.5 * s, w + 5.0 * s, h + 5.0 * s, rad + 2.5 * s, 2.5 * s, Color::new(1.0, 1.0, 1.0, hl));
    }
    hl
}

fn round_arrow(r: Rect, dir: f32, hl: f32, pressed: bool) {
    let s = scale();
    let c = vec2(r.x + r.w * 0.5, r.y + r.h * 0.5 + if pressed { 1.5 * s } else { 0.0 });
    let rad = r.h.min(r.w) * 0.5;
    circle(c.x, c.y + 2.0 * s, rad, Color::new(0.2, 0.12, 0.25, 0.12));
    circle(c.x, c.y, rad, mix(Color::new(1.0, 0.98, 0.94, 0.85), Color::new(1.0, 1.0, 1.0, 1.0), hl));
    let k = rad * 0.38;
    draw_triangle(vec2(c.x + dir * k * 1.1, c.y), vec2(c.x - dir * k * 0.7, c.y - k * 1.1), vec2(c.x - dir * k * 0.7, c.y + k * 1.1), INK);
}

/// Selector with an optional colour swatch next to the value (e.g. paint colours)
pub fn selector_swatch(focus: &mut Focus, label: &str, value: &str, swatch: Option<Color>, x: f32, y: f32, w: f32, h: f32, color: Color) -> i32 {
    let id = focus.next_id();
    let s = scale();
    let row = Rect::new(x, y, w, h);
    let (hover, _) = pointer_pressed_in(row);
    focus.hover(id, hover);
    let focused = focus.index == id;
    let hl = row_background(focus, id, x, y, w, h, color);
    let fs = (h * 0.44).min(27.0 * s);
    text(label, x + 16.0 * s, mid_baseline(y + h * 0.5, fs), fs, INK);
    let aw = h * 0.72;
    let lx = match focus.label_w {
        Some(lw) => x + 16.0 * s + lw + 6.0 * s,
        None => x + w * 0.42,
    };
    let left = Rect::new(lx, y + (h - aw) * 0.5, aw, aw);
    let right = Rect::new(x + w - aw - 8.0 * s, y + (h - aw) * 0.5, aw, aw);
    let (_, cl) = pointer_pressed_in(left);
    let (_, cr) = pointer_pressed_in(right);
    round_arrow(left, -1.0, hl, pointer_down_in(left));
    round_arrow(right, 1.0, hl, pointer_down_in(right));
    let vcx = (left.x + left.w + right.x) * 0.5;
    let vy = y + h * 0.5;
    if let Some(sw) = swatch {
        let d = text_size(value, fs);
        let r = fs * 0.42;
        let total = r * 2.0 + 8.0 * s + d.x;
        let sx = vcx - total * 0.5 + r;
        circle(sx, vy + 1.5 * s, r + 2.0 * s, with_alpha(INK, 0.25));
        circle(sx, vy, r + 2.0 * s, CREAM);
        circle(sx, vy, r, sw);
        text(value, sx + r + 8.0 * s, mid_baseline(vy, fs), fs, INK);
    } else {
        // shrink long values to fit between the arrows
        let room = right.x - (left.x + left.w) - 8.0 * s;
        let d = text_size(value, fs);
        let fs2 = if d.x > room { fs * room / d.x } else { fs };
        text_mid(value, vcx, vy, fs2, INK);
    }
    let mut d = 0;
    if cl || (focused && focus.left) {
        d = -1;
    }
    if cr || (focused && (focus.right || focus.activated)) {
        d = 1;
    }
    if d != 0 {
        crate::audio::ui_sound(crate::audio::UiSound::Move);
    }
    d
}

/// Horizontal slider for 0..1 values (keyboard steps of 5%, mouse/touch drag).
/// Returns true when the value changed.
pub fn slider(focus: &mut Focus, label: &str, value: &mut f32, x: f32, y: f32, w: f32, h: f32, color: Color) -> bool {
    let id = focus.next_id();
    let s = scale();
    let row = Rect::new(x, y, w, h);
    let (hover, clicked) = pointer_pressed_in(row);
    focus.hover(id, hover);
    let focused = focus.index == id;
    let hl = row_background(focus, id, x, y, w, h, color);
    let fs = (h * 0.44).min(27.0 * s);
    text(label, x + 16.0 * s, mid_baseline(y + h * 0.5, fs), fs, INK);
    let tx = x + w * 0.42;
    let tw = w * 0.58 - 86.0 * s;
    let th = (h * 0.24).max(6.0 * s);
    let cy = y + h * 0.5;
    let old = *value;
    // dragging
    let track = Rect::new(tx - 10.0 * s, y, tw + 20.0 * s, h);
    let (_, tclick) = pointer_pressed_in(track);
    if tclick {
        focus.drag = Some(id);
        focus.index = id;
    }
    if focus.drag == Some(id) {
        let mut px = None;
        if is_mouse_button_down(MouseButton::Left) {
            px = Some(mouse_position().0);
        }
        for t in touches() {
            if t.phase != TouchPhase::Ended && t.phase != TouchPhase::Cancelled {
                px = Some(t.position.x);
            }
        }
        match px {
            Some(px) if !input_blocked() => *value = ((px - tx) / tw).clamp(0.0, 1.0),
            _ => focus.drag = None,
        }
    }
    if clicked {
        focus.index = id;
    }
    if focused && focus.left {
        *value = ((*value * 20.0).round() - 1.0).max(0.0) / 20.0;
    }
    if focused && (focus.right || focus.activated) {
        *value = ((*value * 20.0).round() + 1.0).min(20.0) / 20.0;
    }
    rounded_rect(tx, cy - th * 0.5, tw, th, th * 0.5, Color::new(0.24, 0.2, 0.27, 0.14));
    rounded_rect(tx, cy - th * 0.5, (tw * *value).max(th), th, th * 0.5, mix(shade(color, 0.8), INK_SOFT, 0.2));
    let kx = tx + tw * *value;
    let kr = h * (0.26 + 0.04 * hl);
    circle(kx, cy + 2.0 * s, kr, Color::new(0.2, 0.12, 0.25, 0.18));
    circle(kx, cy, kr, Color::new(1.0, 1.0, 1.0, 1.0));
    circle(kx, cy, kr * 0.55, shade(color, 0.85));
    text_right(&format!("{}%", (*value * 100.0).round() as i32), x + w - 16.0 * s, mid_baseline(cy, fs), fs, INK);
    let changed = (*value - old).abs() > 1e-4;
    if changed && focus.drag != Some(id) {
        crate::audio::ui_sound(crate::audio::UiSound::Move);
    }
    changed
}

/// On/off pill switch. Returns true when toggled.
pub fn toggle(focus: &mut Focus, label: &str, value: &mut bool, x: f32, y: f32, w: f32, h: f32, color: Color) -> bool {
    let id = focus.next_id();
    let s = scale();
    let row = Rect::new(x, y, w, h);
    let (hover, clicked) = pointer_pressed_in(row);
    focus.hover(id, hover);
    let focused = focus.index == id;
    let hl = row_background(focus, id, x, y, w, h, color);
    let fs = (h * 0.44).min(27.0 * s);
    text(label, x + 16.0 * s, mid_baseline(y + h * 0.5, fs), fs, INK);
    let mut changed = false;
    if clicked || (focused && (focus.activated || focus.left || focus.right)) {
        *value = !*value;
        changed = true;
        crate::audio::ui_sound(crate::audio::UiSound::Select);
    }
    let sh = h * 0.58;
    let sw = sh * 1.85;
    let sx = x + w - sw - 14.0 * s;
    let sy = y + (h - sh) * 0.5;
    let on = if *value { 1.0 } else { 0.0 };
    rounded_rect(sx, sy, sw, sh, sh * 0.5, mix(Color::new(0.24, 0.2, 0.27, 0.18), mix(MINT, shade(MINT, 0.8), 0.3), on));
    let kr = sh * 0.5 - 3.0 * s;
    let kx = sx + sh * 0.5 + (sw - sh) * on;
    circle(kx, sy + sh * 0.5 + 1.5 * s, kr, Color::new(0.2, 0.12, 0.25, 0.18));
    circle(kx, sy + sh * 0.5, kr, Color::new(1.0, 1.0, 1.0, 1.0));
    let lbl = if *value { "On" } else { "Off" };
    text_right(lbl, sx - 10.0 * s, mid_baseline(y + h * 0.5, fs * 0.85), fs * 0.85, with_alpha(INK, 0.6 + 0.4 * hl));
    changed
}

// ---------------------------------------------------------------- backgrounds

/// Soft animated background: pastel gradient with drifting clouds and hills (used behind menus)
pub fn cozy_background(t: f32) {
    let w = screen_width();
    let h = screen_height();
    let top = Color::new(0.62, 0.80, 0.95, 1.0);
    let bot = Color::new(0.99, 0.88, 0.78, 1.0);
    rounded_rect_gradient(0.0, 0.0, w, h, 0.0, top, bot);
    // sun glow
    let s = scale();
    for k in 0..5 {
        circle(w * 0.78, h * 0.3, (60.0 + k as f32 * 40.0) * s, Color::new(1.0, 0.95, 0.75, 0.10));
    }
    circle(w * 0.78, h * 0.3, 52.0 * s, Color::new(1.0, 0.94, 0.72, 0.9));
    // rolling hills
    for (k, col) in [(0, Color::new(0.70, 0.86, 0.62, 1.0)), (1, Color::new(0.58, 0.79, 0.52, 1.0)), (2, Color::new(0.48, 0.72, 0.46, 1.0))] {
        let base = h * (0.70 + k as f32 * 0.08);
        let step = w / 64.0;
        let mut px = 0.0;
        while px < w + step {
            let y = base + ((px / w * 5.0 + k as f32 * 2.0 + t * 0.03).sin() * 0.035 + (px / w * 11.0 + k as f32).sin() * 0.012) * h;
            draw_rectangle(px, y, step + 1.0, h - y, col);
            px += step;
        }
    }
    // clouds
    for i in 0..6 {
        let x = ((i as f32 * 0.23 + t * 0.010 * (1.0 + i as f32 * 0.2)) % 1.3 - 0.15) * w;
        let y = h * (0.10 + 0.07 * (i % 3) as f32);
        let cs = s * (0.9 + (i % 2) as f32 * 0.4);
        for (dx, dy, r) in [(0.0, 0.0, 26.0), (28.0, -12.0, 32.0), (60.0, -2.0, 26.0), (32.0, 6.0, 28.0)] {
            circle(x + dx * cs, y + dy * cs, r * cs, Color::new(1.0, 1.0, 1.0, 0.9));
        }
    }
}

/// Full-screen vertical wash (for readability on top of the 3D backdrop)
pub fn screen_wash(top: Color, bottom: Color) {
    rounded_rect_gradient(0.0, 0.0, screen_width(), screen_height(), 0.0, top, bottom);
}

/// Horizontal wash from the left edge (menu column readability)
pub fn left_wash(width: f32, color: Color) {
    let h = screen_height();
    let clear = with_alpha(color, 0.0);
    let vertices = vec![vtx(0.0, 0.0, color), vtx(width, 0.0, clear), vtx(width, h, clear), vtx(0.0, h, color)];
    draw_mesh(&Mesh { vertices, indices: vec![0, 1, 2, 0, 2, 3], texture: None });
}

/// Keycap glyph for control references. Returns its width.
pub fn keycap(label: &str, x: f32, cy: f32, size: f32) -> f32 {
    let s = scale();
    let d = text_size(label, size * 0.62);
    let w = (d.x + size * 0.6).max(size);
    let h = size;
    rounded_rect(x, cy - h * 0.5 + 3.0 * s, w, h, size * 0.22, shade(CREAM_DEEP, 0.85));
    rounded_rect_gradient(x, cy - h * 0.5, w, h, size * 0.22, Color::new(1.0, 1.0, 1.0, 1.0), CREAM);
    rounded_rect_outline(x, cy - h * 0.5, w, h, size * 0.22, 1.2 * s, with_alpha(INK, 0.18));
    text_mid(label, x + w * 0.5, cy - 1.0 * s, size * 0.62, INK);
    w
}
