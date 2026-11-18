//! In-race HUD drawn with macroquad 2D on top of each viewport.
use crate::game::{RaceSession, Viewport};
use crate::ui::*;
use macroquad::prelude::*;
use rally_core::race::{format_time, RacePhase};

fn ordinal(n: usize) -> &'static str {
    match n {
        1 => "1st",
        2 => "2nd",
        3 => "3rd",
        4 => "4th",
        5 => "5th",
        6 => "6th",
        7 => "7th",
        _ => "8th",
    }
}

pub fn draw_hud(s: &RaceSession, player: usize, vp: &Viewport) {
    let p = &s.players[player];
    let car = &s.world.cars[p.car];
    let r = &s.world.race.racers[p.car];
    let race = &s.world.race;
    let k = scale() * (vp.h / screen_height()).max(0.6);
    let pad = 16.0 * k;

    // --- speedo (bottom right)
    let cx = vp.x + vp.w - 110.0 * k;
    let cy = vp.y + vp.h - 100.0 * k;
    draw_circle(cx, cy + 4.0 * k, 82.0 * k, SHADOW);
    draw_circle(cx, cy, 82.0 * k, Color::new(1.0, 0.965, 0.90, 0.92));
    let rpm_frac = (car.rpm / car.spec.redline).clamp(0.0, 1.05);
    let segs = 28;
    for i in 0..segs {
        let f = i as f32 / segs as f32;
        let a0 = std::f32::consts::PI * (0.75 + f * 1.5);
        let lit = f <= rpm_frac;
        let col = if f > 0.86 { if lit { PEACH } else { Color::new(0.98, 0.62, 0.52, 0.25) } } else if lit { MINT } else { Color::new(0.55, 0.82, 0.68, 0.25) };
        let r0 = 62.0 * k;
        let r1 = 74.0 * k;
        draw_line(cx + a0.cos() * r0, cy + a0.sin() * r0, cx + a0.cos() * r1, cy + a0.sin() * r1, 5.0 * k, col);
    }
    let kmh = (car.vel.length() * 3.6).round() as i32;
    text_centered(&format!("{kmh}"), cx, cy + 12.0 * k, 52.0 * k, INK);
    text_centered("km/h", cx, cy + 34.0 * k, 18.0 * k, INK_SOFT);
    let gear = match car.gear {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        g => g.to_string(),
    };
    rounded_rect(cx - 18.0 * k, cy - 52.0 * k, 36.0 * k, 30.0 * k, 8.0 * k, if car.telemetry.shifting { BUTTER } else { SKY });
    text_centered(&gear, cx, cy - 29.0 * k, 28.0 * k, INK);

    // --- position + lap (top left)
    let bx = vp.x + pad;
    let by = vp.y + pad;
    panel(bx, by, 190.0 * k, 96.0 * k);
    text(ordinal(r.place), bx + 14.0 * k, by + 46.0 * k, 50.0 * k, INK);
    text(&format!("/ {}", s.world.cars.len()), bx + 100.0 * k, by + 44.0 * k, 26.0 * k, INK_SOFT);
    let lap = r.lap.min(race.laps);
    text(&format!("Lap {}/{}", lap, race.laps), bx + 14.0 * k, by + 80.0 * k, 26.0 * k, INK);

    // --- timers (top right)
    let tw = 210.0 * k;
    let tx = vp.x + vp.w - tw - pad;
    panel(tx, by, tw, 96.0 * k);
    let total = r.finish_time.unwrap_or(race.clock);
    text(&format_time(total), tx + 16.0 * k, by + 40.0 * k, 34.0 * k, INK);
    let cur_lap = if r.finished { r.lap_times.last().copied().unwrap_or(0.0) } else { race.clock - r.lap_start };
    text(&format!("lap  {}", format_time(cur_lap)), tx + 16.0 * k, by + 64.0 * k, 20.0 * k, INK_SOFT);
    if let Some(b) = r.best_lap {
        text(&format!("best {}", format_time(b)), tx + 16.0 * k, by + 86.0 * k, 20.0 * k, INK_SOFT);
    }

    // --- minimap (bottom left)
    draw_minimap(s, player, vp.x + pad, vp.y + vp.h - pad - 170.0 * k, 170.0 * k);

    // --- pace notes strip (top centre; toggled with N / PACE_NOTES_ENABLED)
    crate::pacenotes_hud::draw_pace_notes(s, player, vp);

    // --- centre messages
    let mx = vp.x + vp.w * 0.5;
    let my = vp.y + vp.h * 0.36;
    match race.phase {
        RacePhase::Countdown(t) => {
            if t <= 3.0 {
                let n = t.ceil() as i32;
                let frac = t - t.floor();
                let size = (90.0 + frac * 60.0) * k;
                let col = [PEACH, BUTTER, MINT][((n - 1).clamp(0, 2)) as usize];
                text_outlined_centered(&format!("{n}"), mx, my, size, col);
            } else {
                text_outlined_centered(&s.track().style.name.to_string(), mx, my, 54.0 * k, CREAM);
                text_outlined_centered("Get ready!", mx, my + 46.0 * k, 32.0 * k, BUTTER);
            }
        }
        RacePhase::Racing => {
            if race.clock < 1.2 {
                let a = 1.0 - race.clock / 1.2;
                text_outlined_centered("GO!", mx, my, (110.0 + race.clock * 80.0) * k, Color::new(0.55, 0.82, 0.68, a));
            }
            if r.wrong_way {
                let blink = (get_time() * 4.0).sin() > -0.3;
                if blink {
                    text_outlined_centered("Wrong way!", mx, my, 56.0 * k, PEACH);
                }
            }
            if r.finished {
                text_outlined_centered(&format!("Finished {}!", ordinal(r.place)), mx, my, 64.0 * k, BUTTER);
            } else if r.lap == race.laps && r.lap > 1 && race.clock - r.lap_start < 2.5 {
                text_outlined_centered("Final lap!", mx, my, 56.0 * k, BUTTER);
            } else if r.lap > 1 && race.clock - r.lap_start < 2.5 {
                if let Some(t) = r.lap_times.last() {
                    text_outlined_centered(&format!("Lap {}  {}", r.lap - 1, format_time(*t)), mx, my, 44.0 * k, CREAM);
                }
            }
            if r.reset_timer > 0.6 {
                text_outlined_centered("Respawning...", mx, my + 60.0 * k, 32.0 * k, CREAM);
            }
            if car.telemetry.airtime > 0.6 {
                text_outlined_centered(&format!("Big air! {:.1}s", car.telemetry.airtime), mx, vp.y + vp.h * 0.22, 36.0 * k, SKY);
            }
            if car.telemetry.drift_angle.abs() > 0.35 && car.telemetry.speed > 10.0 {
                text_outlined_centered("Drift!", mx, vp.y + vp.h * 0.22 + 40.0 * k, 32.0 * k, LAVENDER);
            }
        }
        RacePhase::Finished => {}
    }
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
    let k = scale();
    draw_circle(x + size * 0.5, y + size * 0.5 + 4.0 * k, size * 0.58, SHADOW);
    draw_circle(x + size * 0.5, y + size * 0.5, size * 0.58, Color::new(1.0, 0.965, 0.90, 0.85));
    let inner = size * 0.78;
    let off = vec2(x + (size - inner) * 0.5, y + (size - inner) * 0.5);
    let to = |p: Vec2| off + (p - min) / ext * inner + ((vec2(ext, ext) - (max - min)) / ext * inner) * 0.5;
    let n = t.samples.len();
    for i in (0..n).step_by(3) {
        let a = t.samples[i].pos;
        let b = t.samples[(i + 3) % n].pos;
        let pa = to(vec2(a.x, a.z));
        let pb = to(vec2(b.x, b.z));
        draw_line(pa.x, pa.y, pb.x, pb.y, 5.0 * k, Color::new(0.45, 0.40, 0.48, 0.9));
    }
    let s0 = t.samples[0].pos;
    let p0 = to(vec2(s0.x, s0.z));
    draw_circle(p0.x, p0.y, 4.0 * k, CREAM);
    let cols = [PEACH, SKY, BUTTER, MINT, LAVENDER, Color::new(0.98, 0.7, 0.4, 1.0), Color::new(0.9, 0.9, 0.9, 1.0), Color::new(0.4, 0.8, 0.8, 1.0)];
    let me = s.players[player].car;
    for (i, c) in s.world.cars.iter().enumerate() {
        if i == me {
            continue;
        }
        let p = to(vec2(c.pos.x, c.pos.z));
        draw_circle(p.x, p.y, 4.5 * k, cols[i % cols.len()]);
    }
    let c = &s.world.cars[me];
    let p = to(vec2(c.pos.x, c.pos.z));
    draw_circle(p.x, p.y, 7.5 * k, INK);
    draw_circle(p.x, p.y, 5.5 * k, BUTTER);
}
