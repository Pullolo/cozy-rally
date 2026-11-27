//! Gameplay glue owned by the gameplay team: game modes, time trial (ghost recording,
//! translucent ghost car, delta to best, ghost files per track), gamepad rumble.
//! game.rs / main.rs call into this module with one-liners.
use crate::game::{RaceSession, SessionConfig, Viewport};
use crate::models::car_model;
use crate::render::gpu::{DrawParams, Gpu, GpuMesh, View};
use crate::ui::*;
use macroquad::math::{Mat4, Vec3};
use macroquad::prelude::*;
use rally_core::ghost::{GhostLap, TimeTrial};
use rally_core::vehicle::car_specs;
use rally_core::world::WorldEvent;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    #[default]
    Race,
    /// Solo laps against your best-lap ghost
    TimeTrial,
    /// A campaign stage (see campaign_mode.rs): solo vs your best ghost for that stage
    Campaign,
}

impl GameMode {
    pub const ALL: [GameMode; 2] = [GameMode::Race, GameMode::TimeTrial];
    pub fn name(self) -> &'static str {
        match self {
            GameMode::Race => "Race",
            GameMode::TimeTrial => "Time Trial",
            GameMode::Campaign => "Campaign",
        }
    }
}

/// Ghost tint: soft, see-through, slightly blue-white
const GHOST_TINT: [f32; 4] = [0.85, 0.95, 1.15, 0.42];
const GHOST_PAINT: [f32; 3] = [0.92, 0.95, 1.0];

pub struct TimeTrialState {
    pub tt: TimeTrial,
    /// Ghost car meshes for every car model (the ghost may be set with any car)
    ghost_meshes: Vec<(u8, GpuMesh, GpuMesh)>,
    file: Option<PathBuf>,
    /// Seconds to show "New best lap!" for
    pub best_flash: f32,
    /// Best lap loaded from disk at session start (None = first time on this track)
    #[allow(dead_code)]
    pub loaded_best: Option<f32>,
}

/// Directory for ghost files: $COZY_DATA_DIR, else %APPDATA%\CozyRally (Windows),
/// else $XDG_DATA_HOME/cozy-rally or ~/.local/share/cozy-rally.
pub(crate) fn data_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("COZY_DATA_DIR") {
        return Some(PathBuf::from(d));
    }
    if cfg!(windows) {
        return std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("CozyRally"));
    }
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        return Some(PathBuf::from(x).join("cozy-rally"));
    }
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".local/share/cozy-rally"))
}

fn ghost_file(cfg: &SessionConfig) -> Option<PathBuf> {
    let t = &cfg.track;
    let biome = t.biome.map(|b| b as u8 + 1).unwrap_or(0);
    // everything that changes the generated stage is part of the key
    let surface = rally_core::surfacemap::SURFACE_MODES.iter().position(|m| *m == t.surface).unwrap_or(0);
    Some(data_dir()?.join("ghosts").join(format!("ghost_s{}_b{}_l{}_d{}_m{}.bin", t.seed, biome, t.length_class, t.difficulty, surface)))
}

fn upload_ghost_meshes(gpu: &mut Gpu) -> Vec<(u8, GpuMesh, GpuMesh)> {
    car_specs()
        .iter()
        .map(|spec| {
            let m = car_model(spec, GHOST_PAINT);
            (spec.id, gpu.upload(&m.body), gpu.upload(&m.wheel))
        })
        .collect()
}

impl TimeTrialState {
    pub fn new(cfg: &SessionConfig, player_cars: &[usize], gpu: &mut Gpu) -> TimeTrialState {
        let file = if cfg.mode == GameMode::Campaign { crate::campaign_mode::active_ghost_file() } else { ghost_file(cfg) };
        let best = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| GhostLap::from_bytes(&b));
        let loaded_best = best.as_ref().map(|g| g.time);
        TimeTrialState { tt: TimeTrial::new(player_cars, best), ghost_meshes: upload_ghost_meshes(gpu), file, best_flash: 0.0, loaded_best }
    }

    fn save(&self) {
        let (Some(f), Some(g)) = (self.file.as_ref(), self.tt.best.as_ref()) else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // write-then-rename so a crash never leaves a half-written ghost
        let tmp = f.with_extension("tmp");
        if std::fs::write(&tmp, g.to_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, f);
        }
    }

    pub fn free(self, gpu: &mut Gpu) {
        for (_, b, w) in self.ghost_meshes {
            gpu.free(b);
            gpu.free(w);
        }
    }
}

/// Create the per-session gameplay state (called from RaceSession::new)
pub fn session_start(cfg: &SessionConfig, player_cars: &[usize], gpu: &mut Gpu) -> Option<TimeTrialState> {
    // (campaign races against AI rivals have no ghost)
    (cfg.mode == GameMode::TimeTrial || (cfg.mode == GameMode::Campaign && cfg.ai_count == 0)).then(|| TimeTrialState::new(cfg, player_cars, gpu))
}

thread_local! {
    /// (message, until) for the racing-line toggle toast
    static LINE_TOAST: std::cell::Cell<(f64, crate::line_assist::LineMode)> = const { std::cell::Cell::new((0.0, crate::line_assist::LineMode::Full)) };
}

/// World-space gameplay overlays for one viewport (called from RaceSession::draw_3d):
/// the racing-line ribbon and the time-trial ghost.
pub fn draw_world_overlays(sess: &RaceSession, gpu: &mut Gpu, view: &View, player: usize) {
    static NO_HUD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*NO_HUD.get_or_init(|| std::env::var_os("COZY_NO_HUD").is_some()) {
        crate::line_assist::draw(sess, gpu, view, player);
    }
    draw_ghost(sess, gpu, view, player);
}

/// Per-frame gameplay update after the world step (called from RaceSession::update):
/// ghost recording / delta / saving, racing-line toggle (L / pad LB), rumble for players on pads.
pub fn frame(sess: &mut RaceSession, dt: f32) {
    frame_inner(sess, dt);
    // after the ghost recorder, so a finished stage's recording is available
    crate::campaign_mode::frame(sess);
}

fn frame_inner(sess: &mut RaceSession, dt: f32) {
    if let Some(p0) = sess.players.first() {
        let pad_lb = p0.input.pad.and_then(crate::input::pad).is_some_and(|p| p.pressed(crate::input::PadButton::Lb));
        if !sess.paused && (is_key_pressed(KeyCode::L) || pad_lb) {
            let m = crate::line_assist::cycle(&sess.world.cars[p0.car]);
            LINE_TOAST.with(|t| t.set((get_time() + 1.6, m)));
        }
    }
    // rumble on hits / landings
    for e in &sess.frame_events {
        match *e {
            WorldEvent::Collision { car, other, strength, .. } => {
                for p in &sess.players {
                    if p.car == car || Some(p.car) == other {
                        if let Some(pad) = p.input.pad {
                            crate::input::rumble(pad, (strength / 14.0).clamp(0.15, 0.9), 140 + (strength * 10.0).min(160.0) as u32);
                        }
                    }
                }
            }
            WorldEvent::Landing { car, strength } => {
                for p in &sess.players {
                    if p.car == car {
                        if let Some(pad) = p.input.pad {
                            crate::input::rumble(pad, (strength / 16.0).clamp(0.1, 0.7), 110);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let Some(t) = sess.tt.as_mut() else { return };
    t.best_flash = (t.best_flash - dt).max(0.0);
    t.tt.update(&sess.world);
    if t.tt.new_best {
        t.tt.new_best = false;
        t.best_flash = 3.0;
        t.save();
    }
}

/// Draw the translucent ghost for viewport/player `player` (called from draw_3d)
pub fn draw_ghost(sess: &RaceSession, gpu: &mut Gpu, view: &View, player: usize) {
    let Some(t) = sess.tt.as_ref() else { return };
    let Some(ghost) = t.tt.best.as_ref() else { return };
    let Some((_, body, wheel)) = t.ghost_meshes.iter().find(|m| m.0 == ghost.car_spec) else { return };
    let Some((pos, rot)) = t.tt.ghost_pose(&sess.world, player) else { return };
    // fade out when it overlaps the player's car so it never hides it
    let me = sess.players.get(player).map(|p| crate::game::interp(&sess.world.cars[p.car], sess.alpha).0);
    let fade = me.map_or(1.0, |m| ((m.distance(pos) - 2.0) / 6.0).clamp(0.25, 1.0));
    if !view.sphere_visible(pos, 3.5) {
        return;
    }
    let mut tint = GHOST_TINT;
    tint[3] *= fade;
    let p = DrawParams { tint, rim: 0.9, ..Default::default() };
    let m = Mat4::from_rotation_translation(rot, pos);
    let env = sess.scene.env;
    gpu.draw(body, view, &env, m, &p, true);
    let specs = car_specs();
    if let Some(spec) = specs.iter().find(|s| s.id == ghost.car_spec) {
        let mounts = [
            Vec3::new(-spec.half_track, spec.mount_height, -spec.cg_to_front),
            Vec3::new(spec.half_track, spec.mount_height, -spec.cg_to_front),
            Vec3::new(-spec.half_track, spec.mount_height, spec.cg_to_rear),
            Vec3::new(spec.half_track, spec.mount_height, spec.cg_to_rear),
        ];
        for mt in mounts {
            let wm = m * Mat4::from_translation(mt - Vec3::Y * spec.susp_rest * 0.8);
            gpu.draw(wheel, view, &env, wm, &p, true);
        }
    }
}

/// Small time-trial HUD line under the timer panel: live delta to the ghost, last lap result
pub fn draw_hud(sess: &RaceSession, player: usize, vp: &Viewport) {
    // [ui] Campaign stages: hud.rs draws the medal-target pill and split popups in one
    // column under the timer card, so the TT pill / campaign_mode HUD are skipped there.
    let campaign = sess.config.mode == GameMode::Campaign;
    if !campaign {
        crate::campaign_mode::draw_hud(sess, player, vp);
    }
    let (until, mode) = LINE_TOAST.with(|t| t.get());
    if get_time() < until && player == 0 {
        let k = scale() * (vp.h / screen_height()).max(0.6);
        text_outlined_centered(mode.name(), vp.x + vp.w * 0.5, vp.y + vp.h * 0.68, 30.0 * k, CREAM);
    }
    if campaign {
        return;
    }
    let Some(t) = sess.tt.as_ref() else { return };
    let k = scale() * (vp.h / screen_height()).max(0.6);
    let pad = 16.0 * k;
    let w = 210.0 * k;
    let x = vp.x + vp.w - w - pad;
    // below the HUD timer card and its checkpoint split chip
    let y = vp.y + pad + 96.0 * k + 14.0 * k + 52.0 * k;
    let h = 40.0 * k;
    if let Some(d) = t.tt.deltas.get(player).copied().flatten() {
        let col = if d <= 0.0 { MINT } else { PEACH };
        rounded_rect(x, y, w, h, 12.0 * k, Color::new(col.r, col.g, col.b, 0.92));
        text_centered(&format!("{}{:.2}", if d <= 0.0 { "-" } else { "+" }, d.abs()), x + w * 0.5, y + h * 0.72, 30.0 * k, INK);
    } else if t.tt.best.is_none() {
        rounded_rect(x, y, w, h, 12.0 * k, Color::new(1.0, 0.965, 0.90, 0.85));
        text_centered("set a lap time!", x + w * 0.5, y + h * 0.68, 22.0 * k, INK_SOFT);
    }
    if t.best_flash > 0.0 {
        let a = (t.best_flash / 0.5).min(1.0);
        let mut c = BUTTER;
        c.a = a;
        text_outlined_centered("New best lap!", vp.x + vp.w * 0.5, vp.y + vp.h * 0.24, 48.0 * k, c);
    } else if let Some((lt, Some(vs))) = t.tt.last_lap.get(player).copied().flatten() {
        let since = t.tt.lap_clock(&sess.world, player);
        if since < 3.0 {
            let s = format!("{}  ({}{:.2})", rally_core::race::format_time(lt), if vs <= 0.0 { "-" } else { "+" }, vs.abs());
            text_outlined_centered(&s, vp.x + vp.w * 0.5, vp.y + vp.h * 0.24, 36.0 * k, CREAM);
        }
    }
}
