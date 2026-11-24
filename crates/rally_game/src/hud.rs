//! In-race HUD drawn with macroquad 2D on top of each viewport.
//!
//! `HudState` follows the race (position changes, lap traces for the live delta vs best,
//! checkpoint splits, gaps to the leader); `draw_hud` renders one player's viewport.
//! Everything is sized from the viewport so split-screen and 720p..4K read well.
//! The top-centre strip of each viewport is left free for the pace-notes HUD.
use crate::game::{RaceSession, Viewport};
use crate::ui::*;
use macroquad::prelude::*;
use rally_core::race::{format_time, RaceEvent, RacePhase};
use rally_core::vehicle::{Car, WheelState};
use rally_core::world::WorldEvent;

pub fn ordinal_suffix(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

pub fn ordinal(n: usize) -> String {
    format!("{n}{}", ordinal_suffix(n))
}

/// Per-tyre loss of grip, 0 (gripping) .. 1 (sliding), from the sim's tyre model.
pub fn tyre_slip(w: &WheelState) -> f32 {
    if w.contact { w.traction_loss.clamp(0.0, 1.0) } else { 0.0 }
}

/// Driver-assist intervention this frame as (traction control, ABS), when the sim exposes it.
/// Returns None while that data isn't available (the pips are hidden then).
pub fn assist_activity(car: &Car) -> Option<(bool, bool)> {
    if car.assist_level <= 0.2 {
        return None; // Sim: no TC / ABS fitted
    }
    let tc = car.wheels.iter().any(|w| w.tc_cut > 0.12);
    let abs = car.wheels.iter().any(|w| w.abs_cut > 0.12);
    Some((tc, abs))
}

/// Paint colour of a car for UI colour coding
pub fn car_color(s: &RaceSession, car: usize) -> Color {
    let p = s.paints.get(car).copied().unwrap_or(car);
    rgb3(crate::models::PAINTS[p % crate::models::PAINTS.len()])
}

fn short_time(t: f32) -> String {
    if t >= 60.0 {
        format_time(t)
    } else {
        format!("{t:.1}")
    }
}

#[derive(Default)]
struct PlayerHud {
    shown_place: usize,
    pending_place: usize,
    pending_time: f32,
    place_change: i32,
    place_age: f32,
    /// (lap progress m, lap time s) samples of the current lap
    trace: Vec<(f32, f32)>,
    best_trace: Vec<(f32, f32)>,
    splits: Vec<f32>,
    best_splits: Vec<f32>,
    /// (split time, delta vs best split)
    split_flash: Option<(f32, Option<f32>)>,
    split_age: f32,
    /// (lap number, lap time, new best)
    lap_flash: Option<(u32, f32, bool)>,
    lap_age: f32,
    final_lap_age: Option<f32>,
    finish_age: Option<f32>,
    delta: Option<f32>,
    traction: [f32; 4],
    shift_flash: f32,
    last_gear: i32,
}

pub struct HudState {
    /// Per car (race distance, clock) samples, for gaps to the leader
    history: Vec<Vec<(f32, f32)>>,
    last_sample: f32,
    players: Vec<PlayerHud>,
    time: f32,
    /// COZY_HUD_DEMO=1..4 forces HUD messages on screen for visual testing
    demo: u32,
    /// Campaign stage: best time to show / compare against (from the campaign store)
    pub ref_best: Option<f32>,
    /// Campaign stage medal targets (for the "next medal" pill)
    pub medals: Option<rally_core::campaign::MedalTimes>,
}

impl HudState {
    pub fn new(s: &RaceSession) -> HudState {
        HudState { history: vec![Vec::new(); s.world.cars.len()], last_sample: -1.0, players: (0..s.players.len()).map(|_| PlayerHud::default()).collect(), time: 0.0, demo: std::env::var("COZY_HUD_DEMO").ok().and_then(|v| v.parse().ok()).unwrap_or(0), ref_best: None, medals: None }
    }

    /// Campaign stages: compare splits against the stored best run
    pub fn set_reference(&mut self, best: Option<f32>, best_splits: Vec<f32>) {
        self.ref_best = best;
        for p in self.players.iter_mut() {
            p.best_splits = best_splits.clone();
        }
    }

    pub fn update(&mut self, s: &RaceSession, dt: f32) {
        self.time += dt;
        let race = &s.world.race;
        if self.history.len() != s.world.cars.len() {
            *self = HudState::new(s);
        }
        if race.is_racing() && race.clock - self.last_sample >= 0.2 {
            self.last_sample = race.clock;
            for (i, r) in race.racers.iter().enumerate() {
                self.history[i].push((r.total, race.clock));
            }
        }
        let len = s.track().length;
        for (pi, p) in s.players.iter().enumerate() {
            let Some(h) = self.players.get_mut(pi) else { continue };
            let r = &race.racers[p.car];
            let car = &s.world.cars[p.car];
            // position changes (debounced: side-by-side cars swap places a lot)
            if h.shown_place == 0 {
                h.shown_place = r.place;
            }
            if r.place != h.shown_place {
                if r.place != h.pending_place {
                    h.pending_place = r.place;
                    h.pending_time = 0.0;
                }
                h.pending_time += dt;
                if h.pending_time > 0.35 || !race.is_racing() {
                    if race.is_racing() {
                        h.place_change = h.shown_place as i32 - r.place as i32;
                        h.place_age = 0.0;
                    }
                    h.shown_place = r.place;
                }
            } else {
                h.pending_place = r.place;
                h.pending_time = 0.0;
            }
            h.place_age += dt;
            h.split_age += dt;
            h.lap_age += dt;
            h.shift_flash = (h.shift_flash - dt * 3.0).max(0.0);
            if car.gear != h.last_gear {
                h.shift_flash = 1.0;
                h.last_gear = car.gear;
            }
            if let Some(a) = h.final_lap_age.as_mut() {
                *a += dt;
            }
            if let Some(a) = h.finish_age.as_mut() {
                *a += dt;
            }
            // lap trace for the live delta
            let progress = r.total - (r.lap as f32 - 1.0) * len;
            let lap_time = race.clock - r.lap_start;
            if race.is_racing() && !r.finished {
                if h.trace.last().is_none_or(|l| progress > l.0 + 4.0) && progress > 0.0 {
                    h.trace.push((progress, lap_time));
                }
                h.delta = None;
                if h.best_trace.len() > 4 && progress > 15.0 {
                    let i = h.best_trace.partition_point(|e| e.0 < progress);
                    if i > 0 && i < h.best_trace.len() {
                        let (p0, t0) = h.best_trace[i - 1];
                        let (p1, t1) = h.best_trace[i];
                        let f = ((progress - p0) / (p1 - p0).max(0.01)).clamp(0.0, 1.0);
                        h.delta = Some(lap_time - (t0 + (t1 - t0) * f));
                    }
                }
            } else {
                h.delta = None;
            }
            for e in &s.frame_events {
                let WorldEvent::Race(re) = e else { continue };
                match *re {
                    RaceEvent::Checkpoint { car } if car == p.car => {
                        let idx = h.splits.len();
                        let delta = h.best_splits.get(idx).map(|b| lap_time - b);
                        h.splits.push(lap_time);
                        h.split_flash = Some((lap_time, delta));
                        h.split_age = 0.0;
                    }
                    RaceEvent::Lap { car, lap, time } if car == p.car => {
                        let is_best = r.best_lap == Some(time);
                        let new_best = is_best && r.lap_times.len() > 1;
                        if is_best {
                            h.best_trace = std::mem::take(&mut h.trace);
                            h.best_splits = std::mem::take(&mut h.splits);
                        }
                        h.trace.clear();
                        h.splits.clear();
                        h.lap_flash = Some((lap, time, new_best));
                        h.lap_age = 0.0;
                        h.split_flash = None;
                        if race.laps > 1 && lap + 1 == race.laps {
                            h.final_lap_age = Some(0.0);
                        }
                    }
                    RaceEvent::Finish { car, .. } if car == p.car => {
                        h.finish_age = Some(0.0);
                        h.lap_flash = None;
                    }
                    _ => {}
                }
            }
            if self.demo > 0 && race.is_racing() {
                let d = 0.6 + (self.time * 0.7).sin().abs() * 0.4;
                match self.demo {
                    1 => {
                        h.final_lap_age = Some(1.2);
                        h.place_change = 1;
                        h.place_age = 1.0;
                        h.split_flash = Some((23.456, Some(-0.42)));
                        h.split_age = 1.0;
                        h.delta = Some(-0.31);
                    }
                    2 => {
                        h.lap_flash = Some((2, 47.123, true));
                        h.lap_age = 1.0;
                        h.place_change = -1;
                        h.place_age = 0.6;
                        h.delta = Some(0.57);
                    }
                    4 => h.finish_age = Some(2.0),
                    _ => {}
                }
                h.traction = [0.1, 0.35 * d, 0.6 * d, 1.0 * d];
                continue;
            }
            for (k, w) in car.wheels.iter().enumerate() {
                let target = if w.contact { tyre_slip(w) } else { 0.0 };
                h.traction[k] = approach(h.traction[k], target, if target > h.traction[k] { 30.0 } else { 6.0 }, dt);
            }
        }
    }

    /// Time gap (s) of `car` behind the race leader, None for the leader / unknown
    pub fn gap_to_leader(&self, s: &RaceSession, car: usize) -> Option<f32> {
        let race = &s.world.race;
        let order = race.standings();
        let leader = *order.first()?;
        if leader == car {
            return None;
        }
        let r = &race.racers[car];
        let lr = &race.racers[leader];
        if let (Some(a), Some(b)) = (r.finish_time, lr.finish_time) {
            return Some(a - b);
        }
        let hist = self.history.get(leader)?;
        let i = hist.partition_point(|e| e.0 < r.total);
        let (_, t) = *hist.get(i)?;
        let t = if i > 0 {
            let (p0, t0) = hist[i - 1];
            let (p1, t1) = hist[i];
            t0 + (t1 - t0) * ((r.total - p0) / (p1 - p0).max(0.01)).clamp(0.0, 1.0)
        } else {
            t
        };
        Some((race.clock - t).max(0.0))
    }
}

/// HUD scale for a viewport: proportional to its size, but never tiny in split-screen
pub fn hud_scale(vp: &Viewport) -> f32 {
    let full = (screen_height() / 720.0).min(screen_width() / 1280.0);
    let own = (vp.h / 720.0).min(vp.w / 1280.0);
    own.max(full * 0.74).max(0.62).clamp(0.4, 4.0)
}

/// Touch HUD top-right row, right to left: timer card, speedo, pause button
pub struct TouchTopRow {
    pub speedo: Vec2,
    pub speedo_r: f32,
    pub pause: Vec2,
    pub pause_r: f32,
}

pub fn touch_top_row(vp: &Viewport) -> TouchTopRow {
    let k = hud_scale(vp);
    let pad = 16.0 * k;
    let timer_x = vp.x + vp.w - 230.0 * k - pad; // timer card width, see draw_hud
    let speedo_r = 62.0 * k;
    let speedo = vec2(timer_x - 14.0 * k - speedo_r, vp.y + pad + speedo_r - 4.0 * k);
    let pause_r = 26.0 * k;
    let pause = vec2(speedo.x - speedo_r - 18.0 * k - pause_r, vp.y + pad + pause_r + 4.0 * k);
    TouchTopRow { speedo, speedo_r, pause, pause_r }
}

pub fn draw_hud(s: &RaceSession, hud: &HudState, player: usize, vp: &Viewport) {
    let p = &s.players[player];
    let car = &s.world.cars[p.car];
    let r = &s.world.race.racers[p.car];
    let race = &s.world.race;
    let Some(h) = hud.players.get(player) else { return };
    let k = hud_scale(vp);
    let pad = 16.0 * k;
    let t = hud.time;

    // ---------------------------------------------------------------- position card (top left)
    let bx = vp.x + pad;
    let by = vp.y + pad;
    let cw = 214.0 * k;
    let ch = 84.0 * k;
    panel_hud(bx, by, cw, ch, k);
    let stage = !s.track().is_loop();
    if stage {
        draw_stage_card(s, r, bx, by, cw, ch, k);
    } else {
    let bounce = if h.place_age < 0.8 && h.place_change != 0 { 1.0 + 0.35 * (1.0 - pop(h.place_age * 1.4)).abs().min(1.0) * (1.0 - h.place_age / 0.8) } else { 1.0 };
    let place = h.shown_place.max(1);
    let num = format!("{place}");
    let ns = 62.0 * k * bounce;
    let nd = text_size(&num, ns);
    let nx = bx + 18.0 * k;
    let base = by + 64.0 * k;
    text(&num, nx, base + (bounce - 1.0) * 12.0 * k, ns, INK);
    text(ordinal_suffix(place), nx + nd.x + 3.0 * k, by + 38.0 * k, 24.0 * k, INK);
    text(&format!("of {}", s.world.cars.len()), nx + nd.x + 3.0 * k, base, 18.0 * k, INK_SOFT);
    // lap block on the right of the card
    let lx = bx + cw - 18.0 * k;
    text_right("LAP", lx, by + 30.0 * k, 16.0 * k, INK_SOFT);
    let lap = r.lap.min(race.laps);
    text_right(&format!("{}/{}", lap, race.laps), lx, by + 64.0 * k, 34.0 * k, INK);
    rounded_rect(bx + cw * 0.56, by + 16.0 * k, 2.0 * k, ch - 32.0 * k, 1.0 * k, with_alpha(INK, 0.1));
    }
    // position change chip
    if h.place_change != 0 && h.place_age < 2.6 {
        let a = (1.0 - (h.place_age - 2.0) / 0.6).clamp(0.0, 1.0);
        let slide = ease_back((h.place_age / 0.35).min(1.0));
        let up = h.place_change > 0;
        let col = if up { MINT } else { BERRY };
        let label = format!("{}{}", if up { "+" } else { "-" }, h.place_change.abs());
        let px = bx + cw + 10.0 * k * slide - 10.0 * k + 8.0 * k;
        let py = by + ch * 0.5 - 18.0 * k;
        let pw = 70.0 * k;
        rounded_rect(px, py + 3.0 * k, pw, 36.0 * k, 18.0 * k, with_alpha(shade(col, 0.7), a * slide.min(1.0)));
        rounded_rect(px, py, pw, 36.0 * k, 18.0 * k, with_alpha(col, a * slide.min(1.0)));
        arrow_tri(px + 22.0 * k, py + 18.0 * k, 16.0 * k, if up { 1.0 } else { -1.0 }, with_alpha(INK, a));
        text(&label, px + 34.0 * k, mid_baseline(py + 18.0 * k, 22.0 * k), 22.0 * k, with_alpha(INK, a));
    }

    // ---------------------------------------------------------------- standings (left, under the card)
    if stage {
        draw_weather_chip(s, car, r, bx, by + ch + 12.0 * k, k);
        draw_stage_progress(s, player, vp, k, t);
    } else {
        draw_standings(s, hud, player, bx, by + ch + 12.0 * k, 196.0 * k, k, vp);
    }

    // ---------------------------------------------------------------- timer card (top right)
    let tw = 230.0 * k;
    let th = 104.0 * k;
    let tx = vp.x + vp.w - tw - pad;
    panel_hud(tx, by, tw, th, k);
    let total = r.finish_time.unwrap_or(race.clock);
    text("TIME", tx + 18.0 * k, by + 26.0 * k, 15.0 * k, INK_SOFT);
    text(&format_time(total), tx + 18.0 * k, by + 58.0 * k, 34.0 * k, INK);
    let cur_lap = if stage {
        r.finish_time.unwrap_or(race.clock) - r.split_times.last().copied().unwrap_or(0.0)
    } else if r.finished {
        r.lap_times.last().copied().unwrap_or(0.0)
    } else {
        race.clock - r.lap_start
    };
    text(if stage { "Split" } else { "Lap" }, tx + 18.0 * k, by + 82.0 * k, 17.0 * k, INK_SOFT);
    text(&format_time(cur_lap), tx + 62.0 * k, by + 82.0 * k, 17.0 * k, INK);
    text("Best", tx + 18.0 * k, by + 98.0 * k, 15.0 * k, INK_SOFT);
    let best = if stage { hud.ref_best } else { r.best_lap };
    text(&best.map(format_time).unwrap_or_else(|| "--:--.---".into()), tx + 62.0 * k, by + 98.0 * k, 15.0 * k, INK_SOFT);
    // live delta chip
    let mut chip_y = by + th + 10.0 * k;
    if let Some(d) = h.delta {
        let col = if d <= 0.0 { MINT } else { PEACH };
        let label = format!("{:+.2}", d);
        let hh = 34.0 * k;
        rounded_rect(tx, chip_y + 2.0 * k, tw, hh, hh * 0.5, Color::new(0.2, 0.12, 0.25, 0.14));
        rounded_rect(tx, chip_y, tw, hh, hh * 0.5, with_alpha(CREAM, 0.94));
        text("vs best lap", tx + 14.0 * k, mid_baseline(chip_y + hh * 0.5, 16.0 * k), 16.0 * k, INK_SOFT);
        let w = 86.0 * k;
        rounded_rect(tx + tw - w - 4.0 * k, chip_y + 4.0 * k, w, hh - 8.0 * k, (hh - 8.0 * k) * 0.5, col);
        text_mid(&label, tx + tw - w * 0.5 - 4.0 * k, chip_y + hh * 0.5, 18.0 * k, INK);
        chip_y += hh + 8.0 * k;
    }
    // campaign: next medal still reachable (one column with the split popup below it)
    if let (true, Some(m)) = (stage, hud.medals) {
        use rally_core::campaign::Medal;
        let clock = r.finish_time.unwrap_or(race.clock);
        let target = [(Medal::Platinum, m.platinum), (Medal::Gold, m.gold), (Medal::Silver, m.silver), (Medal::Bronze, m.bronze)].into_iter().find(|(_, tm)| *tm >= clock);
        if let Some((md, tm)) = target {
            let hh = 32.0 * k;
            rounded_rect(tx, chip_y + 2.0 * k, tw, hh, hh * 0.5, Color::new(0.2, 0.12, 0.25, 0.14));
            rounded_rect(tx, chip_y, tw, hh, hh * 0.5, with_alpha(mix(CREAM, crate::campaign_screens::medal_color(md), 0.35), 0.95));
            crate::campaign_screens::draw_medal(tx + 20.0 * k, chip_y + hh * 0.5 + 2.0 * k, 8.0 * k, md, 1.0);
            text(crate::campaign_screens::medal_name(md), tx + 36.0 * k, mid_baseline(chip_y + hh * 0.5, 16.0 * k), 16.0 * k, INK);
            text_right(&format_time(tm), tx + tw - 14.0 * k, mid_baseline(chip_y + hh * 0.5, 16.0 * k), 16.0 * k, INK);
            chip_y += hh + 8.0 * k;
        }
    }
    // checkpoint split flash (under the timer, keeping top-centre free for pace notes)
    if let Some((split, delta)) = h.split_flash {
        if h.split_age < 3.0 {
            let a = (1.0 - (h.split_age - 2.4) / 0.6).clamp(0.0, 1.0);
            let sc = ease_back((h.split_age / 0.3).min(1.0));
            let w = tw * sc;
            let x0 = tx + tw - w;
            let hh = 34.0 * k;
            rounded_rect(x0, chip_y + 2.0 * k, w, hh, hh * 0.5, with_alpha(shade(CREAM_DEEP, 0.8), a));
            rounded_rect(x0, chip_y, w, hh, hh * 0.5, with_alpha(CREAM, a));
            if sc > 0.6 {
                text(&format!("Split {}", format_time(split)), x0 + 14.0 * k, mid_baseline(chip_y + hh * 0.5, 17.0 * k), 17.0 * k, with_alpha(INK, a));
                if let Some(d) = delta {
                    let col = if d <= 0.0 { shade(MINT, 0.75) } else { shade(BERRY, 0.9) };
                    text_right(&format!("{:+.2}", d), x0 + w - 14.0 * k, mid_baseline(chip_y + hh * 0.5, 18.0 * k), 18.0 * k, with_alpha(col, a));
                }
            }
        }
    }

    // ---------------------------------------------------------------- speedo + traction (bottom right)
    // touch controls own the bottom corners: the speedo moves to the top row, left of the timer
    // card (the column under the card holds the delta / medal / split chips), then the pause
    // button; the minimap goes up the left side (no grip widget)
    let touch = crate::touch::visible();
    let (cx, cy, sr) = if touch {
        let l = touch_top_row(vp);
        (l.speedo.x, l.speedo.y, l.speedo_r)
    } else {
        let sr = 78.0 * k;
        (vp.x + vp.w - sr - pad - 6.0 * k, vp.y + vp.h - sr - pad, sr)
    };
    draw_speedo(car, h, cx, cy, sr, k, t);
    // circuits: gentle "track limits" reminder when all four wheels are beyond the white lines
    if s.track().params.discipline == rally_core::track::Discipline::Circuit && race.is_racing() && !r.finished {
        let track = s.track();
        let out = car.wheels.iter().all(|w| w.contact && track.ground_at(w.contact_point, Some(r.track_index)).beyond_limits);
        if out {
            // sits above the on-screen pause button (bottom centre) so they never overlap
            let ty = vp.y + vp.h - 40.0 * k - if crate::pause_button_visible() { 70.0 * k } else { 0.0 };
            let w = pill("Track limits", vp.x + vp.w * 0.5, ty, 18.0 * k, BUTTER, INK);
            let _ = w;
        }
    }
    if crate::settings::get().show_traction && !touch {
        draw_traction(car, h, cx - sr - 64.0 * k, cy - 4.0 * k, k);
    }

    // ---------------------------------------------------------------- minimap (bottom left)
    let ms = if touch { (150.0 * k).min(vp.h * 0.26) } else { (190.0 * k).min(vp.h * 0.36) };
    let my = if touch { vp.y + 238.0 * k } else { vp.y + vp.h - pad - ms };
    draw_minimap(s, player, vp.x + pad, my, ms);

    // ---------------------------------------------------------------- pace notes (top centre; N / settings)
    crate::pacenotes_hud::draw_pace_notes(s, player, vp);

    // ---------------------------------------------------------------- centre messages
    let mx = vp.x + vp.w * 0.5;
    let my = vp.y + vp.h * 0.40;
    match race.phase {
        RacePhase::Countdown(ct) => draw_countdown(s, ct, mx, my, k, stage),
        RacePhase::Racing | RacePhase::Finished => {
            if race.clock < 1.3 && race.is_racing() {
                let age = race.clock;
                let a = (1.0 - (age - 0.8) / 0.5).clamp(0.0, 1.0);
                let sc = pop(age * 1.2) * (1.0 + age * 0.15);
                draw_lights(mx, my + if stage { 104.0 } else { -112.0 } * k, k, 3, true, a);
                text_sticker_centered("GO!", mx, my + 50.0 * k * sc, 150.0 * k * sc.max(0.01), MINT, a);
            }
            if let Some(fa) = h.finish_age {
                let sc = pop(fa * 1.1);
                let msg = if r.place == 1 { "You win!".to_string() } else { format!("Finished {}!", ordinal(r.place)) };
                text_sticker_centered(&msg, mx, my + 26.0 * k, (80.0 * k * sc).max(1.0), BUTTER, 1.0);
                if fa > 0.4 {
                    let a = ((fa - 0.4) / 0.4).min(1.0);
                    text_outlined_centered(&format!("Time {}", format_time(r.finish_time.unwrap_or(0.0))), mx, my + 66.0 * k, 30.0 * k, with_alpha(CREAM, a));
                }
            } else if r.wrong_way || hud.demo == 3 {
                let pulse = 1.0 + 0.05 * (t * 8.0).sin();
                let w = 300.0 * k * pulse;
                let hh = 64.0 * k * pulse;
                soft_shadow(mx - w * 0.5, my - hh * 0.5, w, hh, hh * 0.5, 1.0);
                rounded_rect(mx - w * 0.5, my - hh * 0.5, w, hh, hh * 0.5, BERRY);
                rounded_rect_outline(mx - w * 0.5, my - hh * 0.5, w, hh, hh * 0.5, 3.0 * k, with_alpha(CREAM, 0.8));
                uturn_icon(mx - w * 0.5 + 40.0 * k, my, 18.0 * k, CREAM);
                text_mid("Wrong way!", mx + 22.0 * k, my, 34.0 * k * pulse, CREAM);
            }
            let reset_timer = if hud.demo == 3 { 1.6 } else { r.reset_timer };
            if reset_timer > 0.6 && !r.finished {
                let f = ((reset_timer - 0.6) / 1.9).clamp(0.0, 1.0);
                let yy = my + 74.0 * k;
                let w = 240.0 * k;
                let hh = 46.0 * k;
                rounded_rect(mx - w * 0.5, yy - hh * 0.5, w, hh, hh * 0.5, with_alpha(CREAM, 0.94));
                arc(mx - w * 0.5 + 26.0 * k, yy, 9.0 * k, 14.0 * k, -1.57, -1.57 + f * 6.283, SKY);
                arc(mx - w * 0.5 + 26.0 * k, yy, 9.0 * k, 14.0 * k, -1.57 + f * 6.283, 4.713, with_alpha(INK, 0.1));
                text_mid("Respawning...", mx + 16.0 * k, yy, 24.0 * k, INK);
            }
            // final lap ribbon
            if let Some(fa) = h.final_lap_age {
                if fa < 3.2 && h.finish_age.is_none() {
                    draw_ribbon("Final lap!", vp, my - 70.0 * k, fa, 3.2, BUTTER, k);
                }
            }
            // lap complete toast
            if let Some((lap, lt, best)) = h.lap_flash {
                if h.lap_age < 3.0 && h.finish_age.is_none() && !(h.final_lap_age.is_some_and(|a| a < 3.2)) {
                    let a = (1.0 - (h.lap_age - 2.4) / 0.6).clamp(0.0, 1.0);
                    let sc = pop(h.lap_age * 1.3);
                    text_outlined_centered(&format!("Lap {}  {}", lap, format_time(lt)), mx, my - 40.0 * k, (40.0 * k * sc).max(1.0), with_alpha(CREAM, a));
                    if best && h.lap_age > 0.25 {
                        pill("New best lap!", mx, my + 2.0 * k, 20.0 * k * ease_back(((h.lap_age - 0.25) / 0.3).min(1.0)).max(0.01), with_alpha(MINT, a), with_alpha(INK, a));
                    }
                }
            }
            let air = car.telemetry.airtime;
            if air > 0.6 {
                let sc = pop((air - 0.6) * 2.0);
                text_outlined_centered(&format!("Big air! {:.1}s", air), mx, vp.y + vp.h * 0.62, (32.0 * k * sc).max(1.0), SKY);
            }
            if car.telemetry.drift_angle.abs() > 0.35 && car.telemetry.speed > 10.0 && race.is_racing() {
                let wob = (t * 6.0).sin() * 0.04;
                text_outlined_centered("Drift!", mx, vp.y + vp.h * 0.68, 30.0 * k * (1.0 + wob), LAVENDER);
            }
        }
    }
}

/// HUD card: like a panel but slightly translucent and scaled by the HUD factor
fn panel_hud(x: f32, y: f32, w: f32, h: f32, k: f32) {
    let r = 18.0 * k;
    rounded_rect(x, y + 4.0 * k, w, h, r, Color::new(0.2, 0.12, 0.25, 0.16));
    rounded_rect_gradient(x, y, w, h, r, Color::new(1.0, 0.99, 0.96, 0.95), with_alpha(CREAM, 0.92));
    rounded_rect_outline(x, y, w, h, r, 1.5 * k, Color::new(1.0, 1.0, 1.0, 0.6));
}

fn draw_standings(s: &RaceSession, hud: &HudState, player: usize, x: f32, y: f32, w: f32, k: f32, vp: &Viewport) {
    let race = &s.world.race;
    let order = race.standings();
    let n = order.len();
    if n < 2 {
        return;
    }
    let me = s.players[player].car;
    let rh = 26.0 * k;
    let room = ((vp.h * if vp.h < screen_height() * 0.75 { 0.30 } else { 0.42 }) / (rh + 3.0 * k)).floor() as usize;
    let max_rows = room.clamp(3, 8);
    // which rows to show: everyone if they fit, else the leader + a window around me
    let rows: Vec<usize> = if n <= max_rows {
        (0..n).collect()
    } else {
        let my_row = order.iter().position(|&c| c == me).unwrap_or(0);
        let win = max_rows - 1;
        let start = my_row.saturating_sub(win / 2).max(1).min(n - win);
        std::iter::once(0).chain(start..start + win).collect()
    };
    let locals: Vec<usize> = s.players.iter().map(|p| p.car).collect();
    let mut yy = y;
    let mut prev = None;
    for &row in &rows {
        if let Some(pr) = prev {
            if row > pr + 1 {
                text_mid("...", x + w * 0.5, yy - 2.0 * k, 14.0 * k, CREAM);
                yy += 10.0 * k;
            }
        }
        prev = Some(row);
        let c = order[row];
        let is_me = c == me;
        let is_local = locals.contains(&c);
        let bg = if is_me { Color::new(1.0, 0.98, 0.94, 0.95) } else { Color::new(1.0, 0.97, 0.92, 0.80) };
        rounded_rect(x, yy, w, rh, rh * 0.5, bg);
        if is_me {
            rounded_rect_outline(x, yy, w, rh, rh * 0.5, 2.0 * k, BUTTER);
        }
        let col = car_color(s, c);
        circle(x + rh * 0.5, yy + rh * 0.5, rh * 0.42, col);
        text_mid(&format!("{}", row + 1), x + rh * 0.5, yy + rh * 0.5, 14.0 * k, INK);
        let name = &s.names[c];
        let fs = 16.0 * k;
        text(name, x + rh + 6.0 * k, mid_baseline(yy + rh * 0.5, fs), fs, if is_local { INK } else { INK_SOFT });
        let gap = if race.clock < 3.0 && !race.racers[c].finished {
            None
        } else if race.racers[c].finished && row == 0 {
            Some("Done".to_string())
        } else {
            hud.gap_to_leader(s, c).map(|g| format!("+{}", short_time(g)))
        };
        if let Some(g) = gap {
            text_right(&g, x + w - 10.0 * k, mid_baseline(yy + rh * 0.5, 14.0 * k), 14.0 * k, INK_SOFT);
        }
        yy += rh + 3.0 * k;
    }
}

fn draw_speedo(car: &Car, h: &PlayerHud, cx: f32, cy: f32, r: f32, k: f32, t: f32) {
    use std::f32::consts::PI;
    circle(cx, cy + 5.0 * k, r + 2.0 * k, Color::new(0.2, 0.12, 0.25, 0.18));
    circle(cx, cy, r, Color::new(1.0, 0.99, 0.96, 0.95));
    circle(cx, cy, r * 0.97, with_alpha(CREAM, 0.95));
    let rpm_frac = (car.rpm / car.spec.redline).clamp(0.0, 1.05);
    let a0 = PI * 0.75;
    let sweep = PI * 1.5;
    // thin outer speed ring: 0..220 km/h for rally cars, 0..360 for circuit cars
    let vmax = if car.spec.class == rally_core::vehicle::CarClass::Circuit { 360.0 } else { 220.0 };
    let vf = (car.vel.length() * 3.6 / vmax).clamp(0.0, 1.0);
    arc(cx, cy, r * 0.93, r * 0.975, a0, a0 + sweep, with_alpha(INK, 0.06));
    arc(cx, cy, r * 0.93, r * 0.975, a0, a0 + sweep * vf, shade(SKY, 0.85));
    let r0 = r * 0.74;
    let r1 = r * 0.91;
    let red = 0.86;
    arc(cx, cy, r0, r1, a0, a0 + sweep * red, with_alpha(INK, 0.08));
    arc(cx, cy, r0, r1, a0 + sweep * red, a0 + sweep, with_alpha(BERRY, 0.25));
    let f = rpm_frac.min(1.0);
    let at_red = rpm_frac > red;
    let flash = at_red && (t * 14.0).sin() > 0.0;
    let fill_end = if at_red { BERRY } else { BUTTER };
    arc_gradient(cx, cy, r0, r1, a0, a0 + sweep * f, shade(MINT, 0.9), mix(shade(MINT, 0.9), fill_end, (f / red).min(1.0)));
    // ticks
    for i in 0..=10 {
        let ff = i as f32 / 10.0;
        let a = a0 + sweep * ff;
        let (ri, ro) = (r * 0.64, r * 0.70);
        draw_line(cx + a.cos() * ri, cy + a.sin() * ri, cx + a.cos() * ro, cy + a.sin() * ro, 2.0 * k, with_alpha(INK, if ff > red { 0.5 } else { 0.25 }));
    }
    // needle dot
    let a = a0 + sweep * f;
    circle(cx + a.cos() * (r0 + r1) * 0.5, cy + a.sin() * (r0 + r1) * 0.5, (r1 - r0) * 0.75, if flash { BERRY } else { INK });
    circle(cx + a.cos() * (r0 + r1) * 0.5, cy + a.sin() * (r0 + r1) * 0.5, (r1 - r0) * 0.35, CREAM);
    let kmh = (car.vel.length() * 3.6).round() as i32;
    text_mid(&format!("{kmh}"), cx, cy - 2.0 * k, if kmh >= 100 { 41.0 } else { 46.0 } * k, INK);
    text_mid("km/h", cx, cy + 24.0 * k, 14.0 * k, INK_SOFT);
    // gear chip at the bottom of the dial
    let gear = match car.gear {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        g => g.to_string(),
    };
    let gy = cy + r * 0.72;
    let gs = 1.0 + h.shift_flash * 0.25;
    let gr = 17.0 * k * gs;
    let gc = if car.telemetry.shifting { BUTTER } else if flash { BERRY } else { SKY };
    circle(cx, gy + 2.0 * k, gr, shade(gc, 0.75));
    circle(cx, gy, gr, gc);
    text_mid(&gear, cx, gy, 22.0 * k * gs, INK);
}

/// Top-down car with four tyres tinting neutral -> butter -> berry as each loses grip
fn draw_traction(car: &Car, h: &PlayerHud, cx: f32, cy: f32, k: f32) {
    let k = k * 1.38;
    let w = 64.0 * k;
    let hh = 104.0 * k;
    rounded_rect(cx - w * 0.5, cy - hh * 0.5 + 4.0 * k, w, hh, 18.0 * k, Color::new(0.2, 0.12, 0.25, 0.14));
    rounded_rect(cx - w * 0.5, cy - hh * 0.5, w, hh, 18.0 * k, with_alpha(CREAM, 0.9));
    // body
    let bw = 26.0 * k;
    let bh = 54.0 * k;
    let top = cy - bh * 0.5 - 8.0 * k;
    rounded_rect(cx - bw * 0.5, top, bw, bh, 9.0 * k, with_alpha(INK, 0.16));
    rounded_rect(cx - bw * 0.5 + 4.0 * k, top + 12.0 * k, bw - 8.0 * k, 11.0 * k, 4.0 * k, with_alpha(SKY, 0.8));
    rounded_rect(cx - bw * 0.5 + 5.0 * k, top + bh - 14.0 * k, bw - 10.0 * k, 7.0 * k, 3.0 * k, with_alpha(SKY, 0.55));
    // tyres: wheels 0,1 = front left/right; 2,3 = rear
    let tw = 8.0 * k;
    let th = 15.0 * k;
    for (i, w) in car.wheels.iter().enumerate() {
        let side = if w.local_mount.x < 0.0 { -1.0 } else { 1.0 };
        let front = i < 2;
        let x = cx + side * (bw * 0.5 + tw * 0.5 + 1.5 * k) - tw * 0.5;
        let y = if front { top + 6.0 * k } else { top + bh - th - 6.0 * k };
        let slip = h.traction[i];
        let col = if slip < 0.45 { mix(with_alpha(INK, 0.55), BUTTER, slip / 0.45) } else { mix(BUTTER, BERRY, (slip - 0.45) / 0.55) };
        if slip > 0.5 {
            // soft glow so a sliding tyre catches the eye
            rounded_rect(x - 3.0 * k, y - 3.0 * k, tw + 6.0 * k, th + 6.0 * k, 5.0 * k, with_alpha(BERRY, (slip - 0.5) * 0.6));
        }
        let grow = slip * 2.0 * k;
        rounded_rect(x - grow * 0.5, y - grow * 0.5, tw + grow, th + grow, 3.0 * k, col);
    }
    // assist pips
    if let Some((tcs, abs)) = assist_activity(car) {
        let py = cy + hh * 0.5 - 13.0 * k;
        for (j, (label, on)) in [("TCS", tcs), ("ABS", abs)].iter().enumerate() {
            let px = cx - 14.0 * k + j as f32 * 28.0 * k;
            let c = if *on { BUTTER } else { with_alpha(INK, 0.1) };
            rounded_rect(px - 13.0 * k, py - 7.0 * k, 26.0 * k, 14.0 * k, 7.0 * k, c);
            text_mid(label, px, py, 9.0 * k, with_alpha(INK, if *on { 1.0 } else { 0.4 }));
        }
    } else {
        text_mid("GRIP", cx, cy + hh * 0.5 - 12.0 * k, 11.0 * k, INK_SOFT);
    }
}

/// Start lights: three dots that light up with the countdown, all mint at GO
fn draw_lights(cx: f32, cy: f32, k: f32, lit: usize, go: bool, alpha: f32) {
    let w = 150.0 * k;
    let hh = 50.0 * k;
    rounded_rect(cx - w * 0.5, cy - hh * 0.5 + 4.0 * k, w, hh, hh * 0.5, Color::new(0.2, 0.12, 0.25, 0.2 * alpha));
    rounded_rect(cx - w * 0.5, cy - hh * 0.5, w, hh, hh * 0.5, with_alpha(INK, 0.85 * alpha));
    for i in 0..3 {
        let x = cx + (i as f32 - 1.0) * 42.0 * k;
        let on = go || i < lit;
        let col = if go { MINT } else if on { [PEACH, BUTTER, BUTTER][i] } else { Color::new(1.0, 1.0, 1.0, 0.15) };
        if on {
            circle(x, cy, 19.0 * k, with_alpha(col, 0.3 * alpha));
        }
        circle(x, cy, 14.0 * k, with_alpha(col, alpha));
    }
}

/// On stages the start lights sit *below* the countdown so the pace-notes strip stays clear
fn draw_countdown(s: &RaceSession, ct: f32, mx: f32, my: f32, k: f32, stage: bool) {
    if ct > 3.0 {
        let age = 4.0 - ct;
        let sc = ease_back((age / 0.5).min(1.0));
        let name = race_title(s.track());
        let w = (text_size(&name, 44.0 * k).x + 80.0 * k) * sc;
        let hh = 110.0 * k * sc.min(1.0);
        if sc > 0.05 {
            soft_shadow(mx - w * 0.5, my - hh * 0.5 - 20.0 * k, w, hh, 30.0 * k, 1.0);
            rounded_rect_gradient(mx - w * 0.5, my - hh * 0.5 - 20.0 * k, w, hh, 30.0 * k, Color::new(1.0, 0.99, 0.96, 0.96), with_alpha(CREAM, 0.96));
            if sc > 0.7 {
                text_mid(&name, mx, my - 36.0 * k, 44.0 * k, INK);
                text_mid("Get ready!", mx, my + 8.0 * k, 24.0 * k, shade(PEACH, 0.85));
            }
        }
        draw_lights(mx, my + if stage { 70.0 } else { -132.0 } * k, k, 0, false, sc.min(1.0));
        return;
    }
    let n = ct.ceil().max(1.0) as i32;
    let age = (n as f32 - ct).clamp(0.0, 1.0);
    let lit = (4 - n).clamp(0, 3) as usize;
    draw_lights(mx, my + if stage { 104.0 } else { -112.0 } * k, k, lit, false, 1.0);
    let sc = pop(age * 1.15).max(0.0);
    let col = [MINT, BUTTER, PEACH][((n - 1).clamp(0, 2)) as usize];
    // soft burst ring behind the number
    let ring = ease_out(age) * 110.0 * k;
    circle(mx, my, ring + 4.0 * k, with_alpha(col, 0.22 * (1.0 - age)));
    // solid sticker digit; only the last ~0.1 s of each second fades
    let a = (1.0 - (age - 0.9) / 0.1).clamp(0.0, 1.0);
    text_sticker_centered(&format!("{n}"), mx, my + 58.0 * k * sc, (170.0 * k * sc).max(1.0), col, a);
}

/// A banner ribbon that sweeps across the viewport
fn draw_ribbon(label: &str, vp: &Viewport, y: f32, age: f32, dur: f32, color: Color, k: f32) {
    let enter = ease_out((age / 0.45).min(1.0));
    let exit = ease_out(((age - (dur - 0.45)) / 0.45).clamp(0.0, 1.0));
    let off = (1.0 - enter) * -vp.w + exit * vp.w;
    let hh = 64.0 * k;
    let w = vp.w * 0.6;
    let x = vp.x + (vp.w - w) * 0.5 + off;
    rounded_rect(x, y + 5.0 * k, w, hh, 14.0 * k, Color::new(0.2, 0.12, 0.25, 0.2));
    rounded_rect_gradient(x, y, w, hh, 14.0 * k, lighten(color, 0.3), color);
    // little notches like a ribbon
    for (ex, dir) in [(x, -1.0), (x + w, 1.0)] {
        draw_triangle(vec2(ex, y + 8.0 * k), vec2(ex + dir * 26.0 * k, y + hh * 0.5 + 4.0 * k), vec2(ex, y + hh), shade(color, 0.85));
    }
    text_mid(label, x + w * 0.5, y + hh * 0.5, 38.0 * k, INK);
}

fn uturn_icon(cx: f32, cy: f32, r: f32, color: Color) {
    arc(cx, cy, r * 0.55, r * 0.9, std::f32::consts::PI, std::f32::consts::TAU, color);
    draw_rectangle(cx + r * 0.55, cy, r * 0.35, r * 0.7, color);
    draw_triangle(vec2(cx - r * 1.15, cy - r * 0.1), vec2(cx - r * 0.3, cy - r * 0.1), vec2(cx - r * 0.72, cy + r * 0.7), color);
}

fn draw_minimap(s: &RaceSession, player: usize, x: f32, y: f32, size: f32) {
    let t = s.track();
    let mut min = vec2(f32::MAX, f32::MAX);
    let mut max = vec2(f32::MIN, f32::MIN);
    for smp in &t.samples {
        min = min.min(vec2(smp.pos.x, smp.pos.z));
        max = max.max(vec2(smp.pos.x, smp.pos.z));
    }
    let ext = (max - min).max_element().max(1.0);
    let k = size / 190.0;
    let c = vec2(x + size * 0.5, y + size * 0.5);
    circle(c.x, c.y + 5.0 * k, size * 0.5, Color::new(0.2, 0.12, 0.25, 0.16));
    circle(c.x, c.y, size * 0.5, Color::new(1.0, 0.99, 0.96, 0.9));
    circle(c.x, c.y, size * 0.47, with_alpha(lighten(MINT, 0.55), 0.85));
    let inner = size * 0.70;
    let off = vec2(x + (size - inner) * 0.5, y + (size - inner) * 0.5);
    let to = |p: Vec2| off + (p - min) / ext * inner + ((vec2(ext, ext) - (max - min)) / ext * inner) * 0.5;
    let n = t.samples.len();
    let step = 2;
    let pts: Vec<Vec2> = (0..n).step_by(step).map(|i| to(vec2(t.samples[i].pos.x, t.samples[i].pos.z))).collect();
    for (wdt, col) in [(9.0 * k, with_alpha(INK, 0.55)), (5.0 * k, Color::new(1.0, 0.95, 0.85, 1.0))] {
        for i in 0..pts.len() {
            if !t.is_loop() && i + 1 >= pts.len() {
                break; // open stage
            }
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            draw_line(a.x, a.y, b.x, b.y, wdt, col);
            circle(a.x, a.y, wdt * 0.5, col);
        }
    }
    // start/finish flag marker
    let s0 = t.samples[0].pos;
    let p0 = to(vec2(s0.x, s0.z));
    rounded_rect(p0.x - 5.0 * k, p0.y - 5.0 * k, 10.0 * k, 10.0 * k, 2.0 * k, INK);
    rounded_rect(p0.x - 3.0 * k, p0.y - 3.0 * k, 6.0 * k, 6.0 * k, 1.0 * k, CREAM);
    let me = s.players[player].car;
    let locals: Vec<usize> = s.players.iter().map(|p| p.car).collect();
    for (i, cr) in s.world.cars.iter().enumerate() {
        if i == me {
            continue;
        }
        let p = to(vec2(cr.pos.x, cr.pos.z));
        let r = if locals.contains(&i) { 6.0 } else { 5.0 } * k;
        circle(p.x, p.y, r + 1.5 * k, with_alpha(INK, 0.7));
        circle(p.x, p.y, r, car_color(s, i));
    }
    let cr = &s.world.cars[me];
    let p = to(vec2(cr.pos.x, cr.pos.z));
    let pulse = (get_time() as f32 * 4.0).sin() * 0.5 + 0.5;
    circle(p.x, p.y, (11.0 + pulse * 4.0) * k, with_alpha(Color::new(1.0, 1.0, 1.0, 1.0), 0.35 * (1.0 - pulse) + 0.1));
    // heading arrow
    let f = cr.forward();
    let d = vec2(f.x, f.z).normalize_or_zero();
    let n2 = vec2(-d.y, d.x);
    circle(p.x, p.y, 9.0 * k, INK);
    circle(p.x, p.y, 7.0 * k, car_color(s, me));
    let tip = p + d * 17.0 * k;
    draw_triangle(tip, p + d * 8.0 * k + n2 * 6.0 * k, p + d * 8.0 * k - n2 * 6.0 * k, INK);
}

// ---------------------------------------------------------------- campaign stages

fn distance_label(m: f32) -> String {
    if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{:.0} m", (m / 10.0).round() * 10.0)
    }
}

/// Stage card (replaces position/lap): distance to the finish + split counter
fn draw_stage_card(s: &RaceSession, r: &rally_core::race::RacerProgress, bx: f32, by: f32, cw: f32, _ch: f32, k: f32) {
    let track = s.track();
    let len = track.stage_length();
    let to_go = (len * (1.0 - r.progress)).max(0.0);
    text("TO FINISH", bx + 18.0 * k, by + 30.0 * k, 15.0 * k, INK_SOFT);
    text(&distance_label(to_go), bx + 18.0 * k, by + 66.0 * k, 34.0 * k, INK);
    let lx = bx + cw - 18.0 * k;
    text_right("SPLIT", lx, by + 30.0 * k, 15.0 * k, INK_SOFT);
    let n = track.splits.len();
    text_right(&format!("{}/{}", r.next_split.min(n), n), lx, by + 66.0 * k, 30.0 * k, INK);
    rounded_rect(bx + cw * 0.64, by + 16.0 * k, 2.0 * k, 52.0 * k, 1.0 * k, with_alpha(INK, 0.1));
}

/// Weather colour for timelines / chips
pub fn weather_color(kind: crate::campaign_data::WxKind) -> Color {
    use crate::campaign_data::WxKind::*;
    match kind {
        Clear => BUTTER,
        Cloudy => Color::new(0.82, 0.84, 0.9, 1.0),
        Wet => lighten(SKY, 0.3),
        Showers => SKY,
        Rain => shade(SKY, 0.85),
        Storm => Color::new(0.52, 0.55, 0.72, 1.0),
        Fog => LAVENDER,
        Snow => Color::new(0.95, 0.97, 1.0, 1.0),
        Sand => PEACH,
    }
}

/// Small drawn weather icon (no font glyphs needed)
pub fn weather_icon(kind: crate::campaign_data::WxKind, cx: f32, cy: f32, r: f32) {
    use crate::campaign_data::WxKind::*;
    let cloud = |x: f32, y: f32, col: Color| {
        circle(x - r * 0.42, y + r * 0.12, r * 0.42, col);
        circle(x + r * 0.05, y - r * 0.12, r * 0.55, col);
        circle(x + r * 0.5, y + r * 0.15, r * 0.38, col);
        rounded_rect(x - r * 0.82, y + r * 0.1, r * 1.7, r * 0.45, r * 0.22, col);
    };
    let drops = |n: usize, col: Color| {
        for i in 0..n {
            let x = cx - r * 0.45 + i as f32 * r * 0.45;
            let y = cy + r * 0.78;
            draw_triangle(vec2(x, y - r * 0.2), vec2(x - r * 0.1, y + r * 0.02), vec2(x + r * 0.1, y + r * 0.02), col);
            circle(x, y + r * 0.04, r * 0.1, col);
        }
    };
    match kind {
        Clear => {
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                draw_line(cx + a.cos() * r * 0.62, cy + a.sin() * r * 0.62, cx + a.cos() * r * 0.92, cy + a.sin() * r * 0.92, r * 0.14, shade(BUTTER, 0.9));
            }
            circle(cx, cy, r * 0.48, BUTTER);
        }
        Cloudy => cloud(cx, cy - r * 0.1, Color::new(0.78, 0.8, 0.88, 1.0)),
        Wet => {
            draw_triangle(vec2(cx, cy - r * 0.8), vec2(cx - r * 0.45, cy + r * 0.1), vec2(cx + r * 0.45, cy + r * 0.1), SKY);
            circle(cx, cy + r * 0.2, r * 0.47, SKY);
        }
        Showers => {
            circle(cx + r * 0.35, cy - r * 0.35, r * 0.4, BUTTER);
            cloud(cx - r * 0.05, cy - r * 0.15, Color::new(0.85, 0.87, 0.93, 1.0));
            drops(2, SKY);
        }
        Rain => {
            cloud(cx, cy - r * 0.25, Color::new(0.74, 0.77, 0.86, 1.0));
            drops(3, shade(SKY, 0.85));
        }
        Storm => {
            cloud(cx, cy - r * 0.25, Color::new(0.55, 0.57, 0.7, 1.0));
            let x = cx + r * 0.05;
            let y = cy + r * 0.25;
            draw_triangle(vec2(x + r * 0.15, y - r * 0.1), vec2(x - r * 0.25, y + r * 0.35), vec2(x + r * 0.02, y + r * 0.35), BUTTER);
            draw_triangle(vec2(x - r * 0.05, y + r * 0.3), vec2(x + r * 0.2, y + r * 0.3), vec2(x - r * 0.18, y + r * 0.85), BUTTER);
        }
        Fog => {
            for i in 0..3 {
                let y = cy - r * 0.45 + i as f32 * r * 0.45;
                let off = if i == 1 { r * 0.15 } else { 0.0 };
                rounded_rect(cx - r * 0.85 + off, y - r * 0.1, r * 1.6, r * 0.2, r * 0.1, shade(LAVENDER, 0.85));
            }
        }
        Snow => {
            cloud(cx, cy - r * 0.25, Color::new(0.8, 0.83, 0.92, 1.0));
            for i in 0..3 {
                let x = cx - r * 0.45 + i as f32 * r * 0.45;
                circle(x, cy + r * 0.72, r * 0.13, Color::new(1.0, 1.0, 1.0, 1.0));
                circle(x, cy + r * 0.72, r * 0.07, shade(SKY, 0.8));
            }
        }
        Sand => {
            for i in 0..3 {
                let y = cy - r * 0.4 + i as f32 * r * 0.4;
                arc(cx - r * 0.3 + i as f32 * r * 0.15, y, r * 0.3, r * 0.42, std::f32::consts::PI, std::f32::consts::TAU, shade(PEACH, 0.9));
            }
        }
    }
}

/// Weather chip: icon, condition name, grip hint
fn draw_weather_chip(s: &RaceSession, car: &rally_core::vehicle::Car, r: &rally_core::race::RacerProgress, x: f32, y: f32, k: f32) {
    let track = s.track();
    let w = track.weather_at(r.s);
    let kind = crate::campaign_data::classify(&w);
    let (surface, _) = track.surface_at(car.pos, Some(r.track_index));
    let grip = (w.grip_factor(surface) * 100.0).round();
    let hh = 40.0 * k;
    let ww = 196.0 * k;
    rounded_rect(x, y + 3.0 * k, ww, hh, hh * 0.5, Color::new(0.2, 0.12, 0.25, 0.14));
    rounded_rect(x, y, ww, hh, hh * 0.5, with_alpha(CREAM, 0.94));
    circle(x + hh * 0.5, y + hh * 0.5, hh * 0.42, with_alpha(weather_color(kind), 0.35));
    weather_icon(kind, x + hh * 0.5, y + hh * 0.5, hh * 0.34);
    text(kind.label(), x + hh + 4.0 * k, mid_baseline(y + hh * 0.5, 17.0 * k), 17.0 * k, INK);
    let gcol = if grip >= 95.0 { shade(MINT, 0.7) } else if grip >= 85.0 { shade(BUTTER, 0.75) } else { BERRY };
    text_right(&format!("grip {grip:.0}%"), x + ww - 12.0 * k, mid_baseline(y + hh * 0.5, 14.0 * k), 14.0 * k, gcol);
}

/// Start -> finish progress bar: weather-tinted track, split markers, ghost and you
fn draw_stage_progress(s: &RaceSession, player: usize, vp: &Viewport, k: f32, t: f32) {
    let track = s.track();
    let me = s.players[player].car;
    let r = &s.world.race.racers[me];
    let len = track.stage_length().max(1.0);
    let w = (vp.w * 0.40).min(620.0 * k);
    let x = vp.x + (vp.w - w) * 0.5;
    let y = vp.y + vp.h - 44.0 * k;
    let bh = 12.0 * k;
    rounded_rect(x - 6.0 * k, y - bh * 0.5 - 6.0 * k + 3.0 * k, w + 12.0 * k, bh + 12.0 * k, (bh + 12.0 * k) * 0.5, Color::new(0.2, 0.12, 0.25, 0.16));
    rounded_rect(x - 6.0 * k, y - bh * 0.5 - 6.0 * k, w + 12.0 * k, bh + 12.0 * k, (bh + 12.0 * k) * 0.5, with_alpha(CREAM, 0.92));
    // weather timeline under the bar
    let n = 48;
    for i in 0..n {
        let f0 = i as f32 / n as f32;
        let wx = track.weather_at(track.start_s + (f0 + 0.5 / n as f32) * len);
        let kind = crate::campaign_data::classify(&wx);
        let c = if kind == crate::campaign_data::WxKind::Clear { with_alpha(INK, 0.08) } else { with_alpha(weather_color(kind), 0.75) };
        let (sx, sw) = (x + f0 * w, w / n as f32 + 0.5);
        draw_rectangle(sx.max(x + bh * 0.3), y - bh * 0.5, sw.min(x + w - bh * 0.3 - sx).max(0.0), bh, c);
    }
    // driven part
    let p = r.progress.clamp(0.0, 1.0);
    rounded_rect(x, y - bh * 0.25, (w * p).max(bh * 0.5), bh * 0.5, bh * 0.25, shade(MINT, 0.85));
    // split markers
    for (i, sp) in track.splits.iter().enumerate() {
        let f = ((sp - track.start_s) / len).clamp(0.0, 1.0);
        let passed = i < r.next_split;
        let mx = x + f * w;
        draw_triangle(vec2(mx, y - bh * 0.9), vec2(mx - 5.0 * k, y - bh * 1.5), vec2(mx + 5.0 * k, y - bh * 1.5), if passed { BUTTER } else { with_alpha(INK, 0.45) });
        rounded_rect(mx - 1.5 * k, y - bh * 0.6, 3.0 * k, bh * 1.2, 1.5 * k, if passed { shade(BUTTER, 0.8) } else { with_alpha(INK, 0.35) });
    }
    // start dot + finish chequer
    circle(x, y, bh * 0.55, INK);
    circle(x, y, bh * 0.32, MINT);
    let fx = x + w;
    for i in 0..2 {
        for j in 0..2 {
            let c = if (i + j) % 2 == 0 { INK } else { Color::new(1.0, 1.0, 1.0, 1.0) };
            draw_rectangle(fx - bh * 0.5 + i as f32 * bh * 0.5, y - bh * 0.5 + j as f32 * bh * 0.5, bh * 0.5, bh * 0.5, c);
        }
    }
    // you
    let mx = x + p * w;
    let bob = (t * 4.0).sin() * 1.5 * k;
    draw_triangle(vec2(mx, y - 11.0 * k + bob), vec2(mx - 6.0 * k, y - 19.0 * k + bob), vec2(mx + 6.0 * k, y - 19.0 * k + bob), INK);
    circle(mx, y, 9.5 * k, INK);
    circle(mx, y, 7.0 * k, car_color(s, me));
    // ghost (your best run): lavender pin under the bar, so it stays visible when close
    if let Some(tt) = s.tt.as_ref() {
        if let Some((gpos, _)) = tt.tt.ghost_pose(&s.world, player) {
            let q = track.query(gpos, Some(r.track_index));
            let gx = x + track.stage_progress(q.s) * w;
            draw_triangle(vec2(gx, y + 8.0 * k), vec2(gx - 6.0 * k, y + 16.0 * k), vec2(gx + 6.0 * k, y + 16.0 * k), shade(LAVENDER, 0.8));
            circle(gx, y + 19.0 * k, 6.5 * k, shade(LAVENDER, 0.8));
            circle(gx, y + 19.0 * k, 4.0 * k, Color::new(1.0, 1.0, 1.0, 0.9));
            circle(gx, y, 5.0 * k, with_alpha(LAVENDER, 0.9));
        }
    }
}

// ---------------------------------------------------------------- names, car classes

/// Display name for a race: circuit / campaign stage name, else the scenery name
pub fn race_title(track: &rally_core::track::Track) -> String {
    let p = &track.params;
    if p.discipline == rally_core::track::Discipline::Circuit {
        if let Some(c) = rally_core::circuits::circuits().into_iter().find(|c| c.params.seed == p.seed && c.params.length_class == p.length_class) {
            return c.name;
        }
        return format!("{} Raceway", track.style.name);
    }
    if !track.is_loop() {
        if let Some(st) = rally_core::campaign::campaign().into_iter().flat_map(|c| c.stages).find(|st| st.params.seed == p.seed) {
            return st.name;
        }
    }
    track.style.name.to_string()
}

/// Class badge: RALLY / GT / HYPER / FORMULA
pub fn class_label(spec: &rally_core::vehicle::CarSpec) -> &'static str {
    use rally_core::vehicle::CarClass;
    match spec.class {
        CarClass::Rally => "RALLY",
        CarClass::Circuit => {
            if spec.downforce > 3.0 || spec.mass < 1000.0 {
                "FORMULA"
            } else if power_kw(spec) > 500.0 {
                "HYPER"
            } else {
                "GT"
            }
        }
    }
}

pub fn class_color(spec: &rally_core::vehicle::CarSpec) -> Color {
    match class_label(spec) {
        "RALLY" => MINT,
        "GT" => SKY,
        "HYPER" => LAVENDER,
        _ => PEACH,
    }
}

pub fn tyre_label(spec: &rally_core::vehicle::CarSpec) -> &'static str {
    match spec.tyre {
        rally_core::vehicle::TyreType::Rally => "Rally tyres",
        rally_core::vehicle::TyreType::Slick => "Slicks",
    }
}

fn power_kw(spec: &rally_core::vehicle::CarSpec) -> f32 {
    spec.peak_torque * spec.peak_rpm * std::f32::consts::TAU / 60.0 / 1000.0
}

/// Gearing-limited top speed (km/h)
fn top_speed(spec: &rally_core::vehicle::CarSpec) -> f32 {
    let top = spec.gears.last().copied().unwrap_or(1.0) * spec.final_drive;
    spec.redline * std::f32::consts::TAU / 60.0 / top.max(0.1) * spec.wheel_radius * 3.6
}

/// Stat bars for the car picker, scaled so all classes stay distinguishable:
/// (label, 0..1, value text). Top speed and grip are measured with the real sim
/// (`car_perf`); until that finishes, rough estimates are shown.
pub fn car_stats(spec: &rally_core::vehicle::CarSpec) -> [(&'static str, f32, String); 4] {
    let kw = power_kw(spec);
    let pw = kw / (spec.mass / 1000.0);
    let slick = spec.tyre == rally_core::vehicle::TyreType::Slick;
    let perf = crate::car_perf::get(spec.id as usize);
    let v = perf.map(|p| p.vmax_kmh).unwrap_or_else(|| top_speed(spec).min(330.0));
    let g = perf.map(|p| p.lat_g).unwrap_or(spec.tire_grip * (1.0 + spec.downforce * 0.06) * 0.85);
    let surf = if slick { "tarmac" } else { "all roads" };
    [
        ("Power", (pw / 900.0).sqrt().clamp(0.05, 1.0), format!("{:.0} hp", kw * 1.341)),
        ("Top speed", ((v - 120.0) / 240.0).clamp(0.05, 1.0), format!("{:.0} km/h", v)),
        ("Grip", ((g - 0.6) / 1.9).clamp(0.05, 1.0), format!("{g:.1} g {surf}")),
        ("Weight", (spec.mass / 1700.0).clamp(0.05, 1.0), format!("{:.0} kg", spec.mass)),
    ]
}
