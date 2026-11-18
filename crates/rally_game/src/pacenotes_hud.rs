//! Rally pace-notes strip: the next notes as cozy cards at the top centre of each viewport.
//!
//! Data comes from `rally_core::pacenotes` (UI-agnostic, also usable by a co-driver mode); this
//! file only draws. Toggle with **N** or through `PACE_NOTES_ENABLED` (e.g. a settings menu).
use crate::game::{RaceSession, Viewport};
use crate::ui::*;
use macroquad::prelude::*;
use rally_core::pacenotes::{notes_for, upcoming, Direction, Modifier, NoteKind, PaceNote, PaceNotes};
use rally_core::race::RacePhase;
use rally_core::surface::Surface;
use rally_core::surfacemap::road_color;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

/// Global on/off switch (default on). A settings screen can store into this.
pub static PACE_NOTES_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn enabled() -> bool {
    PACE_NOTES_ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    PACE_NOTES_ENABLED.store(on, Ordering::Relaxed);
}

/// Pick up the persisted setting (called every frame, so a settings screen change applies
/// immediately).
pub fn sync_from_settings() {
    set_enabled(crate::settings::get().pace_notes);
}

#[derive(Default, Clone)]
struct PlayerAnim {
    head: Option<usize>,
    /// Visual offset in card slots (eases to 0 after the strip advances)
    scroll: f32,
    last_frame: i64,
}

struct HudState {
    key: (usize, u64, u32),
    notes: PaceNotes,
    players: Vec<PlayerAnim>,
    toast_until: f64,
}

thread_local! {
    static STATE: RefCell<Option<HudState>> = const { RefCell::new(None) };
}

const PEACH_DEEP: Color = Color::new(0.97, 0.52, 0.45, 1.0);
const ORANGE: Color = Color::new(0.98, 0.72, 0.48, 1.0);
const LIME: Color = Color::new(0.76, 0.87, 0.55, 1.0);

fn severity_color(n: &PaceNote) -> Color {
    match n.kind {
        NoteKind::Hairpin => PEACH_DEEP,
        NoteKind::Kink => SKY,
        NoteKind::Corner | NoteKind::Square => match n.severity {
            1 => PEACH_DEEP,
            2 => ORANGE,
            3 => BUTTER,
            4 => LIME,
            5 => MINT,
            _ => SKY,
        },
        _ => LAVENDER,
    }
}

fn c3(c: [f32; 3]) -> Color {
    Color::new(c[0], c[1], c[2], 1.0)
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, c.a * a)
}

/// Draw the pace notes for one local player. Also handles the N toggle (once per frame).
pub fn draw_pace_notes(session: &RaceSession, player: usize, vp: &Viewport) {
    if player == 0 {
        sync_from_settings();
    }
    if player == 0 && is_key_pressed(KeyCode::N) {
        let on = !enabled();
        set_enabled(on);
        crate::settings::update(|st| st.pace_notes = on);
        crate::settings::save();
        STATE.with(|st| {
            if let Some(s) = st.borrow_mut().as_mut() {
                s.toast_until = get_time() + 1.6;
            }
        });
    }
    let track = session.track();
    let key = (std::sync::Arc::as_ptr(&session.world.track) as usize, track.params.seed, track.length.to_bits());
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        if st.as_ref().is_none_or(|s| s.key != key) {
            let toast = st.as_ref().map(|s| s.toast_until).unwrap_or(0.0);
            *st = Some(HudState { key, notes: notes_for(track), players: Vec::new(), toast_until: toast });
        }
        let state = st.as_mut().unwrap();
        let k = crate::hud::hud_scale(vp);
        if get_time() < state.toast_until && player == 0 {
            let msg = if enabled() { "Pace notes on" } else { "Pace notes off" };
            text_outlined_centered(msg, vp.x + vp.w * 0.5, vp.y + vp.h * 0.62, 30.0 * k, CREAM);
        }
        if !enabled() || state.notes.is_empty() {
            return;
        }
        let Some(lp) = session.players.get(player) else { return };
        let r = &session.world.race.racers[lp.car];
        if r.finished || session.world.race.phase == RacePhase::Finished {
            return;
        }
        if state.players.len() <= player {
            state.players.resize(player + 1, PlayerAnim::default());
        }
        let car = &session.world.cars[lp.car];
        draw_strip(state, player, r.s, car.vel.length(), track.biome, vp, k);
    });
}

fn draw_strip(state: &mut HudState, player: usize, s: f32, speed: f32, biome: rally_core::biome::Biome, vp: &Viewport, k: f32) {
    let notes = &state.notes;
    let up = upcoming(notes, s, 4);
    if up.is_empty() {
        return;
    }
    let anim = &mut state.players[player];
    let frame = (get_time() * 1000.0) as i64;
    let dt = if anim.last_frame == 0 { 0.0 } else { ((frame - anim.last_frame) as f32 / 1000.0).clamp(0.0, 0.1) };
    anim.last_frame = frame;
    let head = up[0].index;
    if let Some(prev) = anim.head {
        if prev != head {
            let steps = (head + notes.len() - prev) % notes.len();
            if steps <= 3 {
                anim.scroll += steps as f32;
            }
        }
    }
    anim.head = Some(head);
    anim.scroll *= (-dt * 7.0).exp();
    if anim.scroll < 0.002 {
        anim.scroll = 0.0;
    }
    let scroll = anim.scroll;

    // ---- layout: top centre between the position and timer panels, or below them when narrow
    let pad = 16.0 * k;
    let big = 112.0 * k;
    let small = 84.0 * k;
    let gap = 18.0 * k;
    let strip_w = big + 2.0 * small + 2.0 * gap;
    // position card (214) on the left, timer card (230) on the right
    let free = vp.w - 2.0 * pad - (214.0 + 230.0) * k - 2.0 * gap;
    let (cs, top) = if free >= strip_w { (1.0, vp.y + pad) } else { (((vp.w - 2.0 * pad) / strip_w).min(1.0), vp.y + pad + 110.0 * k) };
    let big = big * cs;
    let small = small * cs;
    let gap = gap * cs;
    let strip_w = big + 2.0 * small + 2.0 * gap;
    let x0 = vp.x + (vp.w - strip_w) * 0.5;
    // slot p (0 = current/big). Continuous so cards slide.
    let slot_size = |p: f32| if p <= 0.0 { big } else if p < 1.0 { big + (small - big) * p } else { small };
    let slot_x = |p: f32| {
        if p <= 0.0 {
            x0 + p * (big + gap)
        } else if p < 1.0 {
            x0 + p * (big + gap)
        } else {
            x0 + big + gap + (p - 1.0) * (small + gap)
        }
    };

    // draw far-to-near so the current card is on top
    for (i, u) in up.iter().enumerate().rev() {
        let p = i as f32 + scroll;
        if p > 2.95 {
            continue;
        }
        let alpha = if p > 2.0 { (2.95 - p) / 0.95 } else { 1.0 };
        let size = slot_size(p);
        let x = slot_x(p);
        let y = top + (big - size) * 0.3;
        let note = &notes.notes[u.index];
        let current = i == 0;
        draw_card(note, x, y, size, alpha, current && u.distance <= 0.0, biome, cs * k);
        // link to the next card: "into" or the distance call
        if i + 1 < up.len() && p + 1.0 <= 2.95 {
            let nx = slot_x(p + 1.0);
            let cx = (x + size + nx) * 0.5;
            let cy = y + size * 0.5 + 0.0;
            let label = if note.distance_to_next == 0 { "into".to_string() } else { format!("{}", note.distance_to_next) };
            let fs = 17.0 * k * cs;
            let tw = text_size(&label, fs).x + 10.0 * k * cs;
            let a = alpha.min(if p + 1.0 > 2.0 { (2.95 - (p + 1.0)) / 0.95 } else { 1.0 });
            rounded_rect(cx - tw * 0.5, cy - 11.0 * k * cs, tw, 20.0 * k * cs, 9.0 * k * cs, with_alpha(INK, 0.78 * a));
            text_centered(&label, cx, cy + 5.0 * k * cs, fs, with_alpha(CREAM, a));
        }
    }

    // ---- proximity bar under the current card + spoken text
    let u0 = up[0];
    let note0 = &notes.notes[u0.index];
    let bx = slot_x(scroll);
    let bw = slot_size(scroll);
    let by = top + big + 8.0 * k * cs;
    let bh = 9.0 * k * cs;
    let window = (speed * 4.0).clamp(60.0, 220.0);
    let fill = if u0.distance <= 0.0 { 1.0 } else { (1.0 - u0.distance / window).clamp(0.0, 1.0) };
    rounded_rect(bx, by, bw, bh, bh * 0.5, with_alpha(INK, 0.35));
    if fill > 0.02 {
        let col = if u0.distance <= 0.0 { BUTTER } else { severity_color(note0) };
        rounded_rect(bx, by, (bw * fill).max(bh), bh, bh * 0.5, col);
    }
    if u0.distance > 0.0 {
        let d = format!("{:.0}m", (u0.distance / 5.0).round() * 5.0);
        text_outlined_centered(&d, bx + bw + 22.0 * k * cs, by + bh * 0.9, 17.0 * k * cs, CREAM);
    }
    // the call as a co-driver would read it
    let call = note0.text();
    let fs = 22.0 * k * cs;
    let cy = by + bh + 26.0 * k * cs;
    let tw = text_size(&call, fs).x + 22.0 * k * cs;
    let cx = x0 + strip_w * 0.5;
    rounded_rect(cx - tw * 0.5, cy, tw, 28.0 * k * cs, 14.0 * k * cs, Color::new(0.24, 0.20, 0.27, 0.62));
    text_centered(&call, cx, cy + 20.0 * k * cs, fs, CREAM);
}

#[allow(clippy::too_many_arguments)]
fn draw_card(n: &PaceNote, x: f32, y: f32, size: f32, alpha: f32, inside: bool, biome: rally_core::biome::Biome, k: f32) {
    let r = size * 0.18;
    let col = severity_color(n);
    let grow = if inside { size * 0.04 } else { 0.0 };
    let (x, y, size) = (x - grow, y - grow, size + 2.0 * grow);
    rounded_rect(x + 3.0 * k, y + 5.0 * k, size, size, r, with_alpha(SHADOW, alpha));
    if inside {
        rounded_rect(x - 4.0 * k, y - 4.0 * k, size + 8.0 * k, size + 8.0 * k, r + 4.0 * k, with_alpha(BUTTER, alpha));
    }
    rounded_rect(x, y, size, size, r, with_alpha(CREAM, alpha * 0.96));
    // severity band along the top
    rounded_rect(x, y, size, size * 0.12 + r, r, with_alpha(col, alpha));
    draw_rectangle(x, y + size * 0.12, size, r, with_alpha(CREAM, alpha * 0.96));

    let ink = with_alpha(INK, alpha);
    let gx = x + size * 0.08;
    let gy = y + size * 0.16;
    let gs = size * 0.52;
    let number: Option<String> = match n.kind {
        NoteKind::Corner | NoteKind::Square => Some(n.severity.to_string()),
        NoteKind::Hairpin => Some("1".into()),
        NoteKind::Kink => Some("K".into()),
        _ => None,
    };
    match n.kind {
        NoteKind::Corner | NoteKind::Hairpin | NoteKind::Square | NoteKind::Kink => {
            // glyph on the side of the turn, number on the other
            let left = n.direction == Direction::Left;
            let glyph_x = if left { gx } else { x + size - size * 0.08 - gs };
            draw_turn_glyph(n, glyph_x, gy, gs, gs * 1.05, size * 0.075, with_alpha(col_dark(col), alpha));
            if let Some(num) = number {
                let nx = if left { x + size * 0.76 } else { x + size * 0.26 };
                text_centered(&num, nx, gy + gs * 0.72, size * 0.46, ink);
                // exit grade of tightening / opening corners
                if let Some(e) = n.exit_severity.filter(|e| *e != n.severity) {
                    text_centered(&format!("> {e}"), nx, gy + gs * 1.05, size * 0.2, with_alpha(INK_SOFT, alpha));
                }
            }
        }
        NoteKind::Crest => draw_crest(x + size * 0.5, gy + gs * 0.85, gs * 0.9, ink, false),
        NoteKind::Jump => draw_crest(x + size * 0.5, gy + gs * 0.85, gs * 0.9, ink, true),
        NoteKind::Hazard => {
            if let Some(sf) = n.surface_change() {
                draw_surface_badge(sf, biome, x + size * 0.5, gy + gs * 0.5, gs * 0.42, alpha);
                text_centered(&surface_short(sf, n.has(Modifier::Patch)), x + size * 0.5, gy + gs * 1.12, size * 0.13, ink);
            } else if n.has(Modifier::Compression) {
                draw_dip(x + size * 0.5, gy + gs * 0.5, gs * 0.9, ink);
            } else if n.has(Modifier::Narrows) {
                draw_narrows(x + size * 0.5, gy + gs * 0.5, gs * 0.5, ink, k);
            }
        }
    }

    // badges along the bottom
    let mut badges: Vec<Badge> = Vec::new();
    if n.has(Modifier::DoubleCaution) {
        badges.push(Badge::Caution(2));
    } else if n.has(Modifier::Caution) {
        badges.push(Badge::Caution(1));
    }
    if n.kind != NoteKind::Hazard {
        if let Some(sf) = n.surface_change() {
            badges.push(Badge::Surface(sf));
        }
    }
    if n.kind != NoteKind::Crest && (n.has(Modifier::Crest) || n.has(Modifier::OverCrest)) {
        badges.push(Badge::Crest);
    }
    if n.kind != NoteKind::Jump && n.has(Modifier::Jump) {
        badges.push(Badge::Jump);
    }
    if n.has(Modifier::Tightens) {
        badges.push(Badge::Tightens);
    } else if n.has(Modifier::Opens) {
        badges.push(Badge::Opens);
    }
    if n.has(Modifier::Long) {
        badges.push(Badge::Long);
    }
    if n.has(Modifier::DontCut) {
        badges.push(Badge::DontCut);
    }
    if n.kind != NoteKind::Hazard && n.has(Modifier::Narrows) {
        badges.push(Badge::Narrows);
    }
    if n.has(Modifier::OffCamber) {
        badges.push(Badge::OffCamber);
    }
    badges.truncate(4);
    let br = size * 0.1;
    let total = badges.len() as f32 * br * 2.3;
    let mut bx = x + (size - total) * 0.5 + br * 1.15;
    let byc = y + size - br * 1.35;
    for b in badges {
        draw_badge(b, bx, byc, br, biome, alpha, k);
        bx += br * 2.3;
    }
}

fn col_dark(c: Color) -> Color {
    Color::new(c.r * 0.62, c.g * 0.58, c.b * 0.62, 1.0)
}

fn surface_short(s: Surface, patch: bool) -> String {
    let w = match s {
        Surface::Asphalt => "TARMAC",
        Surface::Gravel => "GRAVEL",
        Surface::Dirt => "DIRT",
        Surface::Grass => "GRASS",
        Surface::Sand => "SAND",
        Surface::Snow => "SNOW",
        Surface::Ice => "ICE",
        Surface::Mud => "MUD",
    };
    if patch && s == Surface::Ice { format!("{w}!") } else { w.to_string() }
}

/// Turtle path for a turn: straight lead, one or two arcs, tail, arrowhead.
fn draw_turn_glyph(n: &PaceNote, x: f32, y: f32, w: f32, h: f32, thick: f32, color: Color) {
    let sev = n.severity.clamp(1, 6) as usize;
    let sign = if n.direction == Direction::Left { -1.0 } else { 1.0 };
    // (radius, angle deg) pairs
    let mut arcs: Vec<(f32, f32)> = match n.kind {
        NoteKind::Hairpin => vec![(0.42, 180.0)],
        NoteKind::Square => vec![(0.1, 90.0)],
        NoteKind::Kink => vec![(1.4, 24.0)],
        _ => {
            let r = [0.35, 0.5, 0.7, 0.95, 1.3, 1.8][sev - 1];
            let a: f32 = [140.0f32, 115.0, 90.0, 70.0, 50.0, 32.0][sev - 1] * if n.has(Modifier::Long) { 1.25 } else { 1.0 };
            let a: f32 = a.min(170.0);
            if n.has(Modifier::Tightens) {
                vec![(r * 1.7, a * 0.45), (r * 0.7, a * 0.55)]
            } else if n.has(Modifier::Opens) {
                vec![(r * 0.7, a * 0.55), (r * 1.7, a * 0.45)]
            } else {
                vec![(r, a)]
            }
        }
    };
    let lead = if n.kind == NoteKind::Hairpin { 1.0 } else { 0.9 };
    let tail = match n.kind {
        NoteKind::Hairpin => 0.6,
        NoteKind::Square => 0.85,
        NoteKind::Kink => 0.5,
        _ => 0.25,
    };
    let mut pts: Vec<Vec2> = vec![vec2(0.0, 0.0)];
    let mut p = vec2(0.0, 0.0);
    let mut hd = vec2(0.0, -1.0);
    p += hd * lead;
    pts.push(p);
    for (r, a) in arcs.drain(..) {
        let steps = ((a / 10.0).ceil() as usize).max(2);
        let da = (a / steps as f32).to_radians() * sign;
        for _ in 0..steps {
            let (s, c) = da.sin_cos();
            hd = vec2(hd.x * c - hd.y * s, hd.x * s + hd.y * c);
            p += hd * (r * da.abs());
            pts.push(p);
        }
    }
    p += hd * tail;
    pts.push(p);
    let ah = 0.32;
    let perp = vec2(-hd.y, hd.x);
    let tip = p + hd * ah;
    let b1 = p + perp * ah * 0.75;
    let b2 = p - perp * ah * 0.75;
    // fit into the box
    let mut min = vec2(f32::MAX, f32::MAX);
    let mut max = vec2(f32::MIN, f32::MIN);
    for q in pts.iter().chain([tip, b1, b2].iter()) {
        min = min.min(*q);
        max = max.max(*q);
    }
    let ext = max - min;
    let sc = (w / ext.x.max(0.6)).min(h / ext.y.max(0.6));
    let off = vec2(x + (w - ext.x * sc) * 0.5, y + (h - ext.y * sc) * 0.5) - min * sc;
    let tr = |q: Vec2| off + q * sc;
    for seg in pts.windows(2) {
        let a = tr(seg[0]);
        let b = tr(seg[1]);
        draw_line(a.x, a.y, b.x, b.y, thick, color);
        draw_circle(b.x, b.y, thick * 0.5, color);
    }
    let s0 = tr(pts[0]);
    draw_circle(s0.x, s0.y, thick * 0.5, color);
    let (t, c1, c2) = (tr(tip + hd * 0.05), tr(b1 - hd * 0.05), tr(b2 - hd * 0.05));
    draw_triangle(t, c1, c2, color);
}

fn draw_crest(cx: f32, base_y: f32, w: f32, color: Color, jump: bool) {
    // dome
    let segs = 14;
    let hgt = w * 0.32;
    for i in 0..segs {
        let a0 = i as f32 / segs as f32;
        let a1 = (i + 1) as f32 / segs as f32;
        let p = |t: f32| vec2(cx - w * 0.5 + w * t, base_y - hgt * (std::f32::consts::PI * t).sin());
        draw_triangle(p(a0), p(a1), vec2(cx - w * 0.5 + w * a0, base_y), color);
        draw_triangle(p(a1), vec2(cx - w * 0.5 + w * a1, base_y), vec2(cx - w * 0.5 + w * a0, base_y), color);
    }
    if jump {
        // dotted flight arc above the ramp
        for i in 0..7 {
            let t = i as f32 / 6.0;
            let px = cx - w * 0.45 + w * 0.9 * t;
            let py = base_y - hgt - w * 0.25 * (std::f32::consts::PI * t).sin() - w * 0.08;
            draw_circle(px, py, w * 0.035, color);
        }
    } else {
        // "blind" eye-line over the top
        draw_line(cx - w * 0.5, base_y - hgt - w * 0.12, cx + w * 0.5, base_y - hgt - w * 0.12, w * 0.03, with_alpha(color, 0.5));
    }
}

fn draw_dip(cx: f32, cy: f32, w: f32, color: Color) {
    let segs = 12;
    for i in 0..segs {
        let t0 = i as f32 / segs as f32;
        let t1 = (i + 1) as f32 / segs as f32;
        let p = |t: f32| vec2(cx - w * 0.5 + w * t, cy + w * 0.22 * (std::f32::consts::PI * t).sin());
        let (a, b) = (p(t0), p(t1));
        draw_line(a.x, a.y, b.x, b.y, w * 0.08, color);
    }
}

fn draw_narrows(cx: f32, cy: f32, r: f32, color: Color, k: f32) {
    draw_line(cx - r, cy + r, cx - r * 0.35, cy - r, 4.0 * k, color);
    draw_line(cx + r, cy + r, cx + r * 0.35, cy - r, 4.0 * k, color);
}

fn draw_surface_badge(s: Surface, biome: rally_core::biome::Biome, cx: f32, cy: f32, r: f32, alpha: f32) {
    let base = c3(road_color(biome, s));
    draw_circle(cx, cy, r + r * 0.12, with_alpha(INK, alpha));
    draw_circle(cx, cy, r, with_alpha(base, alpha));
    let w = with_alpha(WHITE, alpha * 0.9);
    let dk = with_alpha(Color::new(base.r * 0.6, base.g * 0.55, base.b * 0.5, 1.0), alpha);
    match s {
        Surface::Ice | Surface::Snow => {
            for i in 0..3 {
                let a = i as f32 * std::f32::consts::PI / 3.0;
                let (sn, cs) = a.sin_cos();
                draw_line(cx - cs * r * 0.65, cy - sn * r * 0.65, cx + cs * r * 0.65, cy + sn * r * 0.65, r * 0.12, if s == Surface::Ice { w } else { with_alpha(SKY, alpha) });
            }
        }
        Surface::Asphalt => {
            for i in 0..3 {
                let yy = cy - r * 0.6 + i as f32 * r * 0.5;
                draw_rectangle(cx - r * 0.07, yy, r * 0.14, r * 0.28, with_alpha(BUTTER, alpha));
            }
        }
        Surface::Sand => {
            for j in 0..2 {
                let yy = cy - r * 0.2 + j as f32 * r * 0.4;
                for i in 0..6 {
                    let t0 = i as f32 / 6.0;
                    let t1 = (i + 1) as f32 / 6.0;
                    let px = |t: f32| cx - r * 0.6 + r * 1.2 * t;
                    let py = |t: f32| yy + (t * 12.0).sin() * r * 0.1;
                    draw_line(px(t0), py(t0), px(t1), py(t1), r * 0.1, dk);
                }
            }
        }
        _ => {
            for (dx, dy) in [(-0.35, -0.25), (0.3, -0.3), (0.0, 0.1), (-0.3, 0.38), (0.35, 0.32)] {
                draw_circle(cx + dx * r, cy + dy * r, r * 0.12, dk);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Badge {
    Caution(u8),
    Surface(Surface),
    Crest,
    Jump,
    Tightens,
    Opens,
    Long,
    DontCut,
    Narrows,
    OffCamber,
}

fn draw_badge(b: Badge, cx: f32, cy: f32, r: f32, biome: rally_core::biome::Biome, alpha: f32, k: f32) {
    let ink = with_alpha(INK, alpha);
    let t = (r * 0.2).max(1.5 * k);
    match b {
        Badge::Caution(level) => {
            let c = with_alpha(if level >= 2 { PEACH_DEEP } else { BUTTER }, alpha);
            draw_triangle(vec2(cx, cy - r * 1.05), vec2(cx - r * 1.05, cy + r * 0.8), vec2(cx + r * 1.05, cy + r * 0.8), c);
            let label = if level >= 2 { "!!" } else { "!" };
            text_centered(label, cx, cy + r * 0.65, r * 1.6, ink);
        }
        Badge::Surface(s) => draw_surface_badge(s, biome, cx, cy, r * 0.9, alpha),
        _ => {
            draw_circle(cx, cy, r, with_alpha(Color::new(0.24, 0.20, 0.27, 0.12), alpha));
            match b {
                Badge::Crest => draw_crest(cx, cy + r * 0.45, r * 1.7, ink, false),
                Badge::Jump => draw_crest(cx, cy + r * 0.55, r * 1.5, ink, true),
                Badge::Tightens => {
                    draw_line(cx - r * 0.7, cy - r * 0.6, cx + r * 0.6, cy - r * 0.1, t, ink);
                    draw_line(cx - r * 0.7, cy + r * 0.6, cx + r * 0.6, cy + r * 0.1, t, ink);
                }
                Badge::Opens => {
                    draw_line(cx - r * 0.6, cy - r * 0.1, cx + r * 0.7, cy - r * 0.6, t, ink);
                    draw_line(cx - r * 0.6, cy + r * 0.1, cx + r * 0.7, cy + r * 0.6, t, ink);
                }
                Badge::Long => {
                    draw_line(cx - r * 0.7, cy, cx + r * 0.7, cy, t, ink);
                    draw_line(cx - r * 0.7, cy - r * 0.4, cx - r * 0.7, cy + r * 0.4, t, ink);
                    draw_line(cx + r * 0.7, cy - r * 0.4, cx + r * 0.7, cy + r * 0.4, t, ink);
                }
                Badge::DontCut => {
                    let c = with_alpha(PEACH_DEEP, alpha);
                    draw_line(cx - r * 0.55, cy - r * 0.55, cx + r * 0.55, cy + r * 0.55, t * 1.2, c);
                    draw_line(cx - r * 0.55, cy + r * 0.55, cx + r * 0.55, cy - r * 0.55, t * 1.2, c);
                }
                Badge::Narrows => draw_narrows(cx, cy, r * 0.65, ink, k * 0.6),
                Badge::OffCamber => {
                    draw_line(cx - r * 0.7, cy - r * 0.35, cx + r * 0.7, cy + r * 0.35, t, ink);
                    draw_circle(cx - r * 0.1, cy - r * 0.45, r * 0.22, ink);
                }
                _ => {}
            }
        }
    }
}
