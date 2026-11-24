//! Time-trial campaign: stage sessions, medals, progress, splits, local leaderboard.
//!
//! API for the UI (campaign screens):
//! * `chapters()` -> chapters + stages with lock state, medals, best times, medal times
//! * `medal_times(id)` -> thresholds (computed once from a headless reference run, cached)
//! * `start_stage(id, car, paint, assists)` -> `SessionConfig` (also queued for main.rs)
//! * `last_result()` -> `StageResult` after a stage finishes
//! * `leaderboard(id, assists, n)` -> local top entries
//! * `request_restart()` / instant restart key **T** in a stage
//!
//! Files (in the user data dir, `campaign/`):
//! * `progress.txt`  — `stage_id\tcampaign_ver\tbest_ms\tmedal(0-4)\tcar\tassists\tsplit_ms,...`
//! * `medals.txt`    — `stage_id\tcampaign_ver\treference_ver\treference_s` (cache)
//! * `leaderboard.txt` — `rally_core::campaign_run::LocalLeaderboard` format
//! * `ghost_<stage_id>_v<campaign_ver>.bin` — best run ghost per stage
#![allow(dead_code)] // UI-facing API (campaign screens are built by the UI agent)
use crate::game::{PlayerSlot, RaceSession, SessionConfig, Viewport};
use crate::gameplay::GameMode;
use crate::input::Scheme;
use crate::ui::*;
use macroquad::prelude::*;
use rally_core::campaign::{campaign, find_stage, LeaderboardBackend, LeaderboardEntry, Medal, MedalTimes, CAMPAIGN_VERSION};
use rally_core::campaign_run::{medal_times as medals_from_reference, reference_run, replay_hash, LocalLeaderboard, REFERENCE_VERSION};
use rally_core::race::{format_time, RaceEvent};
use rally_core::world::WorldEvent;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct StageProgress {
    pub best_ms: u32,
    pub medal: Medal,
    pub car: u8,
    pub assists: String,
    pub splits_ms: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct StageView {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub km: f32,
    pub locked: bool,
    pub medal: Medal,
    pub best: Option<f32>,
    /// None until computed (call `medal_times(id)`; takes ~0.2-1 s the first time)
    pub medal_times: Option<MedalTimes>,
}

#[derive(Clone, Debug)]
pub struct ChapterView {
    pub name: String,
    pub unlocked: bool,
    /// Golds (incl. platinum) needed to unlock / currently owned
    pub golds_needed: u32,
    pub golds_have: u32,
    pub stages: Vec<StageView>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageResult {
    pub stage_id: String,
    pub time: f32,
    pub splits: Vec<f32>,
    pub medal: Medal,
    pub previous_best: Option<f32>,
    pub new_best: bool,
    /// 1-based rank on the local board (same assist level)
    pub rank_local: usize,
}

struct Active {
    id: String,
    assists: String,
    medals: Option<MedalTimes>,
    best_splits: Vec<f32>,
    splits: Vec<f32>,
    flash: Option<(usize, Option<f32>, f64)>,
    done: bool,
}

#[derive(Default)]
struct State {
    loaded: bool,
    progress: HashMap<String, StageProgress>,
    medals: HashMap<String, f32>,
    active: Option<Active>,
    last_result: Option<StageResult>,
    pending: Option<SessionConfig>,
    restart: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn dir() -> Option<PathBuf> {
    crate::gameplay::data_dir().map(|d| d.join("campaign"))
}

fn write_atomic(path: &PathBuf, text: &str) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn medal_index(m: Medal) -> u8 {
    m as u8
}

fn medal_from(i: u8) -> Medal {
    match i {
        1 => Medal::Bronze,
        2 => Medal::Silver,
        3 => Medal::Gold,
        4 => Medal::Platinum,
        _ => Medal::None,
    }
}

pub fn encode_progress(p: &HashMap<String, StageProgress>) -> String {
    let mut ids: Vec<&String> = p.keys().collect();
    ids.sort();
    ids.iter()
        .map(|id| {
            let e = &p[*id];
            let sp: Vec<String> = e.splits_ms.iter().map(|v| v.to_string()).collect();
            format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\n", id, CAMPAIGN_VERSION, e.best_ms, medal_index(e.medal), e.car, e.assists.replace(['\t', '\n'], " "), sp.join(","))
        })
        .collect()
}

pub fn decode_progress(text: &str) -> HashMap<String, StageProgress> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 7 || f[1].parse::<u32>().ok() != Some(CAMPAIGN_VERSION) {
            continue;
        }
        let (Ok(best_ms), Ok(m), Ok(car)) = (f[2].parse(), f[3].parse::<u8>(), f[4].parse()) else { continue };
        let splits_ms = if f[6].is_empty() { Vec::new() } else { f[6].split(',').filter_map(|v| v.parse().ok()).collect() };
        out.insert(f[0].to_string(), StageProgress { best_ms, medal: medal_from(m), car, assists: f[5].to_string(), splits_ms });
    }
    out
}

fn ensure_loaded(st: &mut State) {
    if st.loaded {
        return;
    }
    st.loaded = true;
    if let Some(d) = dir() {
        if let Ok(t) = std::fs::read_to_string(d.join("progress.txt")) {
            st.progress = decode_progress(&t);
        }
        if let Ok(t) = std::fs::read_to_string(d.join("medals.txt")) {
            for line in t.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() == 4 && f[1].parse::<u32>().ok() == Some(CAMPAIGN_VERSION) && f[2].parse::<u32>().ok() == Some(REFERENCE_VERSION) {
                    if let Ok(r) = f[3].parse::<f32>() {
                        st.medals.insert(f[0].to_string(), r);
                    }
                }
            }
        }
    }
}

fn save(st: &State) {
    let Some(d) = dir() else { return };
    write_atomic(&d.join("progress.txt"), &encode_progress(&st.progress));
    let mut ids: Vec<&String> = st.medals.keys().collect();
    ids.sort();
    let text: String = ids.iter().map(|id| format!("{}\t{}\t{}\t{}\n", id, CAMPAIGN_VERSION, REFERENCE_VERSION, st.medals[*id])).collect();
    write_atomic(&d.join("medals.txt"), &text);
}

fn leaderboard_file() -> Option<PathBuf> {
    dir().map(|d| d.join("leaderboard.txt"))
}

/// Ghost file for the active stage
pub fn active_ghost_file() -> Option<PathBuf> {
    let id = STATE.with(|s| s.borrow().active.as_ref().map(|a| a.id.clone()))?;
    dir().map(|d| d.join(format!("ghost_{id}_v{CAMPAIGN_VERSION}.bin")))
}

/// Medal thresholds for a stage (headless reference run on first use, then cached on disk)
pub fn medal_times(id: &str) -> Option<MedalTimes> {
    let cached = STATE.with(|s| {
        let mut s = s.borrow_mut();
        ensure_loaded(&mut s);
        s.medals.get(id).copied()
    });
    if let Some(r) = cached {
        return Some(medals_from_reference(r));
    }
    let stage = find_stage(id)?;
    let r = reference_run(&stage)?;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.medals.insert(id.to_string(), r.time);
        save(&s);
    });
    Some(medals_from_reference(r.time))
}

/// [ui] Store a reference time computed off-thread (the stage intro runs
/// `campaign_run::reference_run` in a worker so the UI never blocks)
pub fn cache_reference(id: &str, reference: f32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        ensure_loaded(&mut s);
        s.medals.insert(id.to_string(), reference);
        save(&s);
    });
}

fn golds(st: &State, ids: &[String]) -> u32 {
    ids.iter().filter(|id| st.progress.get(*id).is_some_and(|p| p.medal >= Medal::Gold)).count() as u32
}

/// All chapters with lock state, medals and best times
pub fn chapters() -> Vec<ChapterView> {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        ensure_loaded(&mut st);
        let mut out = Vec::new();
        let mut earlier: Vec<String> = Vec::new();
        for ch in campaign() {
            let have = golds(&st, &earlier);
            let unlocked = have >= ch.unlock_golds;
            let stages = ch
                .stages
                .iter()
                .map(|sd| {
                    let p = st.progress.get(&sd.id);
                    StageView {
                        id: sd.id.clone(),
                        name: sd.name.clone(),
                        blurb: sd.blurb.clone(),
                        km: match sd.params.layout {
                            rally_core::track::LayoutKind::PointToPoint { length_m } => length_m / 1000.0,
                            _ => 0.0,
                        },
                        locked: !unlocked,
                        medal: p.map(|p| p.medal).unwrap_or(Medal::None),
                        best: p.map(|p| p.best_ms as f32 / 1000.0),
                        medal_times: st.medals.get(&sd.id).map(|r| medals_from_reference(*r)),
                    }
                })
                .collect();
            earlier.extend(ch.stages.iter().map(|s| s.id.clone()));
            out.push(ChapterView { name: ch.name, unlocked, golds_needed: ch.unlock_golds, golds_have: have, stages });
        }
        out
    })
}

/// Set up a stage run. The config is also queued for main.rs (`take_pending_config`).
pub fn start_stage(id: &str, car: usize, paint: usize, assists: (&str, f32)) -> Option<SessionConfig> {
    let stage = find_stage(id)?;
    let medals = medal_times(id);
    let car = if stage.cars.is_empty() || stage.cars.contains(&(car as u8)) { car } else { stage.cars[0] as usize };
    let cfg = SessionConfig {
        track: stage.params.clone(),
        laps: 1,
        players: vec![PlayerSlot { car, paint, scheme: Scheme::Both, name: "Player 1".into() }],
        ai_count: 0,
        ai_skill: 0.7,
        assist_level: assists.1,
        mode: GameMode::Campaign,
        online: None,
    };
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        ensure_loaded(&mut st);
        let best_splits = st.progress.get(id).map(|p| p.splits_ms.iter().map(|v| *v as f32 / 1000.0).collect()).unwrap_or_default();
        st.active = Some(Active { id: id.to_string(), assists: assists.0.to_string(), medals, best_splits, splits: Vec::new(), flash: None, done: false });
        st.pending = Some(cfg.clone());
        st.last_result = None;
    });
    Some(cfg)
}

/// A stage config queued by `start_stage` (main.rs uses it instead of the setup screen's)
pub fn take_pending_config() -> Option<SessionConfig> {
    STATE.with(|s| s.borrow_mut().pending.take())
}

/// Re-queue the active stage (instant restart)
pub fn request_restart() {
    STATE.with(|s| s.borrow_mut().restart = true);
}

/// main.rs: restart wanted? (re-queues the same stage config)
pub fn take_restart(current: &SessionConfig) -> bool {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if !st.restart {
            return false;
        }
        st.restart = false;
        if let Some(a) = st.active.as_mut() {
            a.splits.clear();
            a.flash = None;
            a.done = false;
        }
        st.pending = Some(current.clone());
        true
    })
}

pub fn last_result() -> Option<StageResult> {
    STATE.with(|s| s.borrow().last_result.clone())
}

pub fn leaderboard(id: &str, assists: Option<&str>, n: usize) -> Vec<LeaderboardEntry> {
    leaderboard_file().map(LocalLeaderboard::open).unwrap_or_else(LocalLeaderboard::in_memory).top_filtered(id, assists, n)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Record a finished run: progress, leaderboard, result. Returns the result.
pub fn record_finish(id: &str, assists: &str, car: u8, time: f32, splits: &[f32], medals: Option<MedalTimes>, replay: &[u8]) -> StageResult {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        ensure_loaded(&mut st);
        let medal = medals.map(|m| m.medal_for(time)).unwrap_or(Medal::None);
        let prev = st.progress.get(id).cloned();
        let ms = (time * 1000.0).round() as u32;
        let new_best = prev.as_ref().is_none_or(|p| ms < p.best_ms);
        let best_medal = prev.as_ref().map(|p| p.medal).unwrap_or(Medal::None).max(medal);
        if new_best {
            st.progress.insert(id.to_string(), StageProgress { best_ms: ms, medal: best_medal, car, assists: assists.to_string(), splits_ms: splits.iter().map(|v| (v * 1000.0).round() as u32).collect() });
        } else if let Some(p) = st.progress.get_mut(id) {
            p.medal = best_medal;
        }
        save(&st);
        let mut lb = leaderboard_file().map(LocalLeaderboard::open).unwrap_or_else(LocalLeaderboard::in_memory);
        let entry = LeaderboardEntry {
            stage_id: id.to_string(),
            campaign_version: CAMPAIGN_VERSION,
            game_version: env!("CARGO_PKG_VERSION").to_string(),
            player: "Player 1".into(),
            car,
            assists: assists.to_string(),
            time_ms: ms,
            splits_ms: splits.iter().map(|v| (v * 1000.0).round() as u32).collect(),
            timestamp: now_secs(),
            replay_hash: replay_hash(replay),
        };
        let _ = lb.submit(entry);
        let rank_local = lb.rank_of(id, Some(assists), ms);
        let r = StageResult { stage_id: id.to_string(), time, splits: splits.to_vec(), medal, previous_best: prev.map(|p| p.best_ms as f32 / 1000.0), new_best, rank_local };
        st.last_result = Some(r.clone());
        r
    })
}

/// Per-frame stage logic (from gameplay::frame): splits, finish, restart key
pub fn frame(sess: &mut RaceSession) {
    if sess.config.mode != GameMode::Campaign || sess.players.is_empty() {
        return;
    }
    if is_key_pressed(KeyCode::T) && !sess.paused {
        request_restart();
    }
    let car = sess.players[0].car;
    let clock = sess.world.race.clock;
    let mut finished: Option<f32> = None;
    for e in &sess.frame_events {
        match *e {
            WorldEvent::Race(RaceEvent::Checkpoint { car: c }) if c == car => STATE.with(|s| {
                if let Some(a) = s.borrow_mut().active.as_mut() {
                    a.splits.push(clock);
                    let k = a.splits.len() - 1;
                    let d = a.best_splits.get(k).map(|b| clock - b);
                    a.flash = Some((k, d, get_time() + 3.0));
                    if let Some(d) = d {
                        crate::audio::split_chime(d < 0.0); // [audio]
                    }
                }
            }),
            WorldEvent::Race(RaceEvent::Finish { car: c, time, .. }) if c == car => finished = Some(time),
            _ => {}
        }
    }
    if let Some(time) = finished {
        let info = STATE.with(|s| s.borrow_mut().active.as_mut().filter(|a| !a.done).map(|a| {
            a.done = true;
            (a.id.clone(), a.assists.clone(), a.splits.clone(), a.medals)
        }));
        if let Some((id, assists, splits, medals)) = info {
            let replay = sess.tt.as_ref().and_then(|t| t.tt.last_ghost.as_ref()).map(|g| g.to_bytes()).unwrap_or_default();
            let r = record_finish(&id, &assists, sess.world.cars[car].spec.id, time, &splits, medals, &replay);
            // [audio] medal jingle, then the record fanfare when an earlier best was beaten
            crate::audio::medal_jingle(r.medal);
            if r.new_best && r.previous_best.is_some() {
                crate::audio::new_record();
            }
        }
    }
}

/// Small stage HUD: next medal target and split flashes (from gameplay::draw_hud)
pub fn draw_hud(sess: &RaceSession, player: usize, vp: &Viewport) {
    if sess.config.mode != GameMode::Campaign || player != 0 {
        return;
    }
    let k = scale() * (vp.h / screen_height()).max(0.6);
    let clock = sess.world.race.clock;
    STATE.with(|s| {
        let s = s.borrow();
        let Some(a) = s.active.as_ref() else { return };
        if let Some(m) = a.medals {
            let target = [(Medal::Platinum, m.platinum, LAVENDER), (Medal::Gold, m.gold, BUTTER), (Medal::Silver, m.silver, SKY), (Medal::Bronze, m.bronze, PEACH)]
                .into_iter()
                .rev()
                .filter(|(_, t, _)| *t >= clock)
                .last();
            if let Some((medal, t, col)) = target {
                let txt = format!("{:?} {}", medal, format_time(t));
                let w = 210.0 * k;
                let x = vp.x + vp.w - w - 16.0 * k;
                let y = vp.y + 16.0 * k + 96.0 * k + 62.0 * k;
                rounded_rect(x, y, w, 30.0 * k, 10.0 * k, Color::new(col.r, col.g, col.b, 0.85));
                text_centered(&txt, x + w * 0.5, y + 21.0 * k, 22.0 * k, INK);
            }
        }
        if let Some((i, d, until)) = a.flash {
            if get_time() < until {
                let msg = match d {
                    Some(d) => format!("Split {}  {}{:.2}", i + 1, if d <= 0.0 { "-" } else { "+" }, d.abs()),
                    None => format!("Split {}  {}", i + 1, format_time(a.splits[i])),
                };
                let col = match d {
                    Some(d) if d <= 0.0 => MINT,
                    Some(_) => PEACH,
                    None => CREAM,
                };
                text_outlined_centered(&msg, vp.x + vp.w * 0.5, vp.y + vp.h * 0.30, 36.0 * k, col);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_roundtrip() {
        let mut p = HashMap::new();
        p.insert("c1s1-buttercup-lane".to_string(), StageProgress { best_ms: 101_234, medal: Medal::Gold, car: 4, assists: "Normal".into(), splits_ms: vec![12_000, 30_500] });
        p.insert("c2s2-mushroom-dell".to_string(), StageProgress { best_ms: 99_000, medal: Medal::None, car: 0, assists: "Cozy".into(), splits_ms: vec![] });
        let back = decode_progress(&encode_progress(&p));
        assert_eq!(back, p);
        assert!(decode_progress("junk\nc1\t999\t1\t1\t1\tx\t").is_empty());
    }
}
