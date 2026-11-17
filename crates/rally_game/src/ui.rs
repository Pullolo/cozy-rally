//! Tiny immediate-mode UI with a cozy look: rounded pastel panels, chunky buttons,
//! mouse / touch / keyboard navigation.
use macroquad::prelude::*;

pub const CREAM: Color = Color::new(1.0, 0.965, 0.90, 1.0);
pub const INK: Color = Color::new(0.24, 0.20, 0.27, 1.0);
pub const INK_SOFT: Color = Color::new(0.45, 0.40, 0.48, 1.0);
pub const PEACH: Color = Color::new(0.98, 0.62, 0.52, 1.0);
pub const MINT: Color = Color::new(0.55, 0.82, 0.68, 1.0);
pub const SKY: Color = Color::new(0.55, 0.75, 0.95, 1.0);
pub const BUTTER: Color = Color::new(0.99, 0.84, 0.45, 1.0);
pub const LAVENDER: Color = Color::new(0.75, 0.66, 0.92, 1.0);
pub const SHADOW: Color = Color::new(0.15, 0.10, 0.20, 0.25);

/// UI scale factor so layouts work from phones to 4K
pub fn scale() -> f32 {
    (screen_height() / 720.0).min(screen_width() / 1100.0).clamp(0.55, 3.0)
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32, color: Color) {
    let r = r.min(w * 0.5).min(h * 0.5);
    draw_rectangle(x + r, y, w - 2.0 * r, h, color);
    draw_rectangle(x, y + r, r, h - 2.0 * r, color);
    draw_rectangle(x + w - r, y + r, r, h - 2.0 * r, color);
    for (cx, cy) in [(x + r, y + r), (x + w - r, y + r), (x + r, y + h - r), (x + w - r, y + h - r)] {
        draw_circle(cx, cy, r, color);
    }
}

pub fn panel(x: f32, y: f32, w: f32, h: f32) {
    let s = scale();
    rounded_rect(x + 4.0 * s, y + 6.0 * s, w, h, 18.0 * s, SHADOW);
    rounded_rect(x, y, w, h, 18.0 * s, CREAM);
}

pub fn text_size(text: &str, size: f32) -> Vec2 {
    let d = measure_text(text, None, size as u16, 1.0);
    vec2(d.width, d.height)
}

pub fn text(text: &str, x: f32, y: f32, size: f32, color: Color) {
    draw_text(text, x, y, size, color);
}

pub fn text_centered(t: &str, cx: f32, y: f32, size: f32, color: Color) {
    let d = text_size(t, size);
    draw_text(t, cx - d.x * 0.5, y, size, color);
}

/// Text with a soft outline/shadow - readable on top of the 3D scene
pub fn text_outlined(t: &str, x: f32, y: f32, size: f32, color: Color) {
    let o = (size / 18.0).max(1.0);
    let sh = Color::new(0.15, 0.1, 0.2, 0.55 * color.a);
    for (dx, dy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o), (o, o)] {
        draw_text(t, x + dx, y + dy, size, sh);
    }
    draw_text(t, x, y, size, color);
}

pub fn text_outlined_centered(t: &str, cx: f32, y: f32, size: f32, color: Color) {
    let d = text_size(t, size);
    text_outlined(t, cx - d.x * 0.5, y, size, color);
}

/// Keyboard/gamepad-style focus for a vertical list of widgets.
#[derive(Default)]
pub struct Focus {
    pub index: usize,
    pub count: usize,
    pub activated: bool,
    pub left: bool,
    pub right: bool,
    counter: usize,
}

impl Focus {
    /// Call at the start of a frame with the number of focusable items
    pub fn begin(&mut self, count: usize) {
        self.count = count.max(1);
        self.counter = 0;
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
    }

    fn next_id(&mut self) -> usize {
        let id = self.counter;
        self.counter += 1;
        id
    }
}

fn pointer_pressed_in(r: Rect) -> (bool, bool) {
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

/// Big chunky button. Returns true when activated.
pub fn button(focus: &mut Focus, label: &str, x: f32, y: f32, w: f32, h: f32, color: Color) -> bool {
    let id = focus.next_id();
    let s = scale();
    let r = Rect::new(x, y, w, h);
    let (hover, clicked) = pointer_pressed_in(r);
    if hover && (mouse_delta_position().length() > 0.0) {
        if focus.index != id {
            focus.index = id;
        }
    }
    let focused = focus.index == id;
    let lift = if focused { 3.0 * s } else { 0.0 };
    rounded_rect(x, y + 5.0 * s, w, h, 14.0 * s, Color::new(color.r * 0.7, color.g * 0.7, color.b * 0.75, 1.0));
    let c = if focused { Color::new((color.r * 1.08).min(1.0), (color.g * 1.08).min(1.0), (color.b * 1.08).min(1.0), 1.0) } else { color };
    rounded_rect(x, y - lift, w, h, 14.0 * s, c);
    if focused {
        let t = get_time() as f32;
        let pulse = 0.5 + 0.5 * (t * 5.0).sin();
        draw_circle(x + 18.0 * s, y - lift + h * 0.5, 5.0 * s + pulse * 2.0 * s, INK);
    }
    let fs = (h * 0.5).min(34.0 * s);
    text_centered(label, x + w * 0.5, y - lift + h * 0.5 + fs * 0.32, fs, INK);
    let activated = clicked || (focused && focus.activated);
    if activated {
        crate::audio::ui_sound(crate::audio::UiSound::Select);
    }
    activated
}

/// "< value >" selector. Returns -1/0/+1 change.
pub fn selector(focus: &mut Focus, label: &str, value: &str, x: f32, y: f32, w: f32, h: f32, color: Color) -> i32 {
    let id = focus.next_id();
    let s = scale();
    let focused = focus.index == id;
    let row = Rect::new(x, y, w, h);
    let (hover, _) = pointer_pressed_in(row);
    if hover && mouse_delta_position().length() > 0.0 {
        focus.index = id;
    }
    let bg = if focused { color } else { Color::new(color.r, color.g, color.b, 0.45) };
    rounded_rect(x, y, w, h, 12.0 * s, bg);
    let fs = (h * 0.45).min(28.0 * s);
    text(label, x + 18.0 * s, y + h * 0.5 + fs * 0.32, fs, INK);
    let aw = h * 0.8;
    let left = Rect::new(x + w * 0.45, y + h * 0.1, aw, h * 0.8);
    let right = Rect::new(x + w - aw - 10.0 * s, y + h * 0.1, aw, h * 0.8);
    let (_, cl) = pointer_pressed_in(left);
    let (_, cr) = pointer_pressed_in(right);
    for (r, t) in [(left, "<"), (right, ">")] {
        rounded_rect(r.x, r.y, r.w, r.h, 10.0 * s, CREAM);
        text_centered(t, r.x + r.w * 0.5, r.y + r.h * 0.5 + fs * 0.32, fs, INK);
    }
    text_centered(value, (left.x + left.w + right.x) * 0.5, y + h * 0.5 + fs * 0.32, fs, INK);
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

/// Soft animated background: pastel gradient with drifting blobs (used behind menus)
pub fn cozy_background(t: f32) {
    let w = screen_width();
    let h = screen_height();
    let top = Color::new(0.62, 0.80, 0.95, 1.0);
    let bot = Color::new(0.99, 0.88, 0.78, 1.0);
    let bands = 24;
    for i in 0..bands {
        let a = i as f32 / bands as f32;
        let c = Color::new(top.r + (bot.r - top.r) * a, top.g + (bot.g - top.g) * a, top.b + (bot.b - top.b) * a, 1.0);
        draw_rectangle(0.0, h * a, w, h / bands as f32 + 1.0, c);
    }
    // rolling hills
    for (k, col) in [(0, Color::new(0.62, 0.82, 0.55, 1.0)), (1, Color::new(0.50, 0.74, 0.46, 1.0))] {
        let base = h * (0.78 + k as f32 * 0.08);
        let mut px = 0.0;
        let step = w / 40.0;
        while px < w + step {
            let y = base + ((px / w * 6.0 + k as f32 * 2.0 + t * 0.05).sin() * 0.03 + (px / w * 13.0).sin() * 0.01) * h;
            draw_rectangle(px, y, step + 1.0, h - y, col);
            px += step;
        }
    }
    // clouds
    for i in 0..5 {
        let x = ((i as f32 * 0.27 + t * 0.012 * (1.0 + i as f32 * 0.2)) % 1.3 - 0.15) * w;
        let y = h * (0.12 + 0.08 * (i % 3) as f32);
        let s = scale() * (1.0 + (i % 2) as f32 * 0.4);
        for (dx, dy, r) in [(0.0, 0.0, 30.0), (28.0, -10.0, 34.0), (60.0, 0.0, 28.0), (30.0, 8.0, 30.0)] {
            draw_circle(x + dx * s, y + dy * s, r * s, Color::new(1.0, 1.0, 1.0, 0.85));
        }
    }
}
