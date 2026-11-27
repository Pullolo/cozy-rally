//! Campaign gameplay helpers (headless): reference runs and medal times, replay hashing,
//! and leaderboard backends (a local file store now; an HTTP stub for a future server).
use crate::ai::{AiDriver, Personality, RubberBand};
use crate::campaign::{LeaderboardBackend, LeaderboardEntry, MedalTimes, StageDef};
use crate::race::{RaceEvent, RacePhase};
use crate::track::Track;
use crate::world::{Controller, RacerSetup, World, WorldEvent, TICK};
use std::path::PathBuf;
use std::sync::Arc;

/// Bump when physics / AI changes enough that cached reference times are stale.
pub const REFERENCE_VERSION: u32 = 2;
/// Car the reference runs use (the all-rounder), unless the stage restricts cars.
pub const REFERENCE_CAR: u8 = 2;
/// Assist level of the reference driver (Normal)
pub const REFERENCE_ASSIST: f32 = 1.0;

/// Medal margins over the reference time (fractions). Gold is hard but fair on Normal
/// assists: the reference is a near-optimal clean autopilot run.
pub const PLATINUM: f32 = -0.02;
pub const GOLD: f32 = 0.03;
pub const SILVER: f32 = 0.10;
pub const BRONZE: f32 = 0.22;

#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceRun {
    pub time: f32,
    /// Race clock at each checkpoint passed
    pub splits: Vec<f32>,
    pub car: u8,
    pub resets: u32,
}

pub fn medal_times(reference: f32) -> MedalTimes {
    MedalTimes { platinum: reference * (1.0 + PLATINUM), gold: reference * (1.0 + GOLD), silver: reference * (1.0 + SILVER), bronze: reference * (1.0 + BRONZE) }
}

/// Car a stage's reference uses
pub fn reference_car(stage: &StageDef) -> u8 {
    if stage.cars.is_empty() || stage.cars.contains(&REFERENCE_CAR) { REFERENCE_CAR } else { stage.cars[0] }
}

/// Drive a stage headless with a near-optimal autopilot (one lap / A to B).
pub fn reference_run(stage: &StageDef) -> Option<ReferenceRun> {
    let track = Arc::new(Track::generate(&stage.params));
    reference_run_laps(track, reference_car(stage), stage.laps.max(1))
}

pub fn reference_run_on(track: Arc<Track>, car: u8) -> Option<ReferenceRun> {
    reference_run_laps(track, car, 1)
}

/// Reference run over `laps` laps (circuits; stages are always one run)
pub fn reference_run_laps(track: Arc<Track>, car: u8, laps: u32) -> Option<ReferenceRun> {
    let mut world = World::new(track, &[RacerSetup { car_spec: car as usize, controller: Controller::Local(0) }], laps);
    world.cars[0].assist_level = REFERENCE_ASSIST;
    let mut ai = AiDriver::with_personality(Personality { skill: 1.0, aggression: 1.0, consistency: 1.0 }, RubberBand::default(), 7);
    let mut splits = Vec::new();
    let mut resets = 0;
    let max_ticks = ((20.0 + 6.0 * laps as f32) * 60.0 / TICK) as u64;
    let mut ticks = 0u64;
    while ticks < max_ticks {
        let inp = ai.drive(&world.cars[0], &world.track, TICK);
        world.set_input(0, inp);
        world.tick();
        ticks += 1;
        for e in world.drain_events() {
            match e {
                WorldEvent::Race(RaceEvent::Checkpoint { car: 0 }) => splits.push(world.race.clock),
                WorldEvent::Race(RaceEvent::Reset { car: 0 }) => resets += 1,
                _ => {}
            }
        }
        if !world.cars[0].pos.is_finite() {
            return None;
        }
        if world.race.racers[0].finished || world.race.phase == RacePhase::Finished {
            break;
        }
    }
    let time = world.race.racers[0].finish_time?;
    Some(ReferenceRun { time, splits, car, resets })
}

/// FNV-1a 64-bit hash of a replay/ghost blob (LeaderboardEntry::replay_hash)
pub fn replay_hash(blob: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in blob {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ------------------------------------------------------------------ local leaderboard

/// File-backed leaderboard: keeps the best `keep` entries per (stage, assists).
/// Format: one entry per line, tab-separated (see `encode`); unreadable lines are skipped.
pub struct LocalLeaderboard {
    pub path: Option<PathBuf>,
    pub entries: Vec<LeaderboardEntry>,
    pub keep: usize,
}

fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect()
}

fn encode(e: &LeaderboardEntry) -> String {
    let splits: Vec<String> = e.splits_ms.iter().map(|v| v.to_string()).collect();
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:016x}",
        clean(&e.stage_id),
        e.campaign_version,
        clean(&e.game_version),
        clean(&e.player),
        e.car,
        clean(&e.assists),
        e.time_ms,
        splits.join(","),
        e.timestamp,
        e.replay_hash
    )
}

fn decode(line: &str) -> Option<LeaderboardEntry> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() != 10 {
        return None;
    }
    Some(LeaderboardEntry {
        stage_id: f[0].to_string(),
        campaign_version: f[1].parse().ok()?,
        game_version: f[2].to_string(),
        player: f[3].to_string(),
        car: f[4].parse().ok()?,
        assists: f[5].to_string(),
        time_ms: f[6].parse().ok()?,
        splits_ms: if f[7].is_empty() { Vec::new() } else { f[7].split(',').map(|v| v.parse().ok()).collect::<Option<Vec<u32>>>()? },
        timestamp: f[8].parse().ok()?,
        replay_hash: u64::from_str_radix(f[9], 16).ok()?,
    })
}

impl LocalLeaderboard {
    /// In-memory board (tests / no writable data dir)
    pub fn in_memory() -> Self {
        LocalLeaderboard { path: None, entries: Vec::new(), keep: 50 }
    }

    /// Load from `path` (missing or damaged file = empty board)
    pub fn open(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path).map(|t| t.lines().filter_map(decode).collect()).unwrap_or_default();
        LocalLeaderboard { path: Some(path), entries, keep: 50 }
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(p) = &self.path else { return Ok(()) };
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text: String = self.entries.iter().map(|e| encode(e) + "\n").collect();
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, p).map_err(|e| e.to_string())
    }

    /// Best entries for a stage, optionally only one assist level, fastest first
    pub fn top_filtered(&self, stage_id: &str, assists: Option<&str>, n: usize) -> Vec<LeaderboardEntry> {
        let mut v: Vec<LeaderboardEntry> = self.entries.iter().filter(|e| e.stage_id == stage_id && assists.is_none_or(|a| e.assists == a)).cloned().collect();
        v.sort_by_key(|e| (e.time_ms, e.timestamp));
        v.truncate(n);
        v
    }

    /// 1-based rank a time would have on this stage (optionally per assist level)
    pub fn rank_of(&self, stage_id: &str, assists: Option<&str>, time_ms: u32) -> usize {
        self.entries.iter().filter(|e| e.stage_id == stage_id && assists.is_none_or(|a| e.assists == a) && e.time_ms < time_ms).count() + 1
    }
}

impl LeaderboardBackend for LocalLeaderboard {
    fn submit(&mut self, entry: LeaderboardEntry) -> Result<(), String> {
        if entry.time_ms == 0 || entry.stage_id.is_empty() {
            return Err("invalid entry".into());
        }
        let (stage, assists) = (entry.stage_id.clone(), entry.assists.clone());
        self.entries.push(entry);
        // keep the best `keep` per (stage, assists)
        let mut group: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].stage_id == stage && self.entries[i].assists == assists).collect();
        group.sort_by_key(|&i| (self.entries[i].time_ms, self.entries[i].timestamp));
        let drop: Vec<usize> = group.into_iter().skip(self.keep).collect();
        let mut k = 0;
        self.entries.retain(|_| {
            let keep = !drop.contains(&k);
            k += 1;
            keep
        });
        self.save()
    }

    fn top(&self, stage_id: &str, n: usize) -> Vec<LeaderboardEntry> {
        self.top_filtered(stage_id, None, n)
    }

    fn name(&self) -> &'static str {
        "local"
    }
}

// ------------------------------------------------------------------ HTTP leaderboard (stub)

/// Global leaderboard client — protocol defined, transport not wired up yet.
/// Enable with the `http-leaderboard` cargo feature.
///
/// Endpoints (JSON over HTTPS, UTF-8):
///
/// * `POST {base}/v1/stages/{stage_id}/entries`
///   body `{"entry": LeaderboardEntry, "replay": "<base64 ghost blob>"}`
///   where `entry.replay_hash == fnv1a64(replay bytes)`;
///   response `200 {"accepted": true, "rank": 12}` or `{"accepted": false, "reason": "..."}`.
///   The server may re-simulate / sanity-check the replay before accepting.
/// * `GET {base}/v1/stages/{stage_id}/top?n=10&campaign_version=1[&assists=Normal]`
///   response `200 {"entries": [LeaderboardEntry, ...]}` fastest first.
///
/// `LeaderboardEntry` serialises with serde field names
/// (`stage_id, campaign_version, game_version, player, car, assists, time_ms, splits_ms,
/// timestamp, replay_hash`).
#[cfg(feature = "http-leaderboard")]
pub mod http {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct SubmitRequest {
        pub entry: LeaderboardEntry,
        /// base64 (standard alphabet, padded) ghost blob
        pub replay: String,
    }

    #[derive(Serialize, Deserialize)]
    pub struct SubmitResponse {
        pub accepted: bool,
        #[serde(default)]
        pub rank: Option<u32>,
        #[serde(default)]
        pub reason: Option<String>,
    }

    #[derive(Serialize, Deserialize)]
    pub struct TopResponse {
        pub entries: Vec<LeaderboardEntry>,
    }

    pub fn base64(data: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for c in data.chunks(3) {
            let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            for k in 0..4 {
                if k <= c.len() {
                    out.push(T[((n >> (18 - 6 * k)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub struct HttpLeaderboard {
        pub base_url: String,
        /// Replay blob to attach to the next submit (set by the game before `submit`)
        pub pending_replay: Vec<u8>,
    }

    impl HttpLeaderboard {
        pub fn new(base_url: &str) -> Self {
            HttpLeaderboard { base_url: base_url.trim_end_matches('/').to_string(), pending_replay: Vec::new() }
        }

        /// (method, url, json body) for a submit — what a transport would send
        pub fn submit_request(&self, entry: &LeaderboardEntry) -> (String, String, String) {
            let body = SubmitRequest { entry: entry.clone(), replay: base64(&self.pending_replay) };
            ("POST".into(), format!("{}/v1/stages/{}/entries", self.base_url, entry.stage_id), serde_json::to_string(&body).unwrap_or_default())
        }

        pub fn top_request(&self, stage_id: &str, n: usize, assists: Option<&str>) -> String {
            let mut url = format!("{}/v1/stages/{}/top?n={}&campaign_version={}", self.base_url, stage_id, n, crate::campaign::CAMPAIGN_VERSION);
            if let Some(a) = assists {
                url += &format!("&assists={a}");
            }
            url
        }

        pub fn parse_top(json: &str) -> Vec<LeaderboardEntry> {
            serde_json::from_str::<TopResponse>(json).map(|r| r.entries).unwrap_or_default()
        }
    }

    impl LeaderboardBackend for HttpLeaderboard {
        fn submit(&mut self, entry: LeaderboardEntry) -> Result<(), String> {
            let (_m, url, _body) = self.submit_request(&entry);
            Err(format!("global leaderboard not available yet ({url})"))
        }
        fn top(&self, _stage_id: &str, _n: usize) -> Vec<LeaderboardEntry> {
            Vec::new()
        }
        fn name(&self) -> &'static str {
            "global"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(stage: &str, ms: u32, assists: &str) -> LeaderboardEntry {
        LeaderboardEntry {
            stage_id: stage.into(),
            campaign_version: crate::campaign::CAMPAIGN_VERSION,
            game_version: "0.5".into(),
            player: "Tab\tby".into(),
            car: 4,
            assists: assists.into(),
            time_ms: ms,
            splits_ms: vec![1000, 2000],
            timestamp: 1700000000 + ms as u64,
            replay_hash: replay_hash(b"ghost"),
        }
    }

    #[test]
    fn leaderboard_roundtrip_and_ranking() {
        let dir = std::env::temp_dir().join(format!("cozy_lb_test_{}", std::process::id()));
        let path = dir.join("lb.txt");
        let mut lb = LocalLeaderboard::open(path.clone());
        lb.keep = 3;
        for ms in [90_000, 80_000, 85_000, 95_000, 70_000] {
            lb.submit(entry("s1", ms, "Normal")).unwrap();
        }
        lb.submit(entry("s1", 60_000, "Cozy")).unwrap();
        lb.submit(entry("s2", 50_000, "Normal")).unwrap();
        let back = LocalLeaderboard::open(path);
        let top: Vec<u32> = back.top_filtered("s1", Some("Normal"), 10).iter().map(|e| e.time_ms).collect();
        assert_eq!(top, vec![70_000, 80_000, 85_000]);
        assert_eq!(back.top("s1", 1)[0].time_ms, 60_000);
        assert_eq!(back.rank_of("s1", Some("Normal"), 82_000), 3);
        assert_eq!(back.top("s1", 1)[0].player, "Tab by");
        assert_eq!(back.top("s1", 1)[0].splits_ms, vec![1000, 2000]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn medals_and_hash() {
        let m = medal_times(100.0);
        assert_eq!(m.medal_for(97.0), crate::campaign::Medal::Platinum);
        assert_eq!(m.medal_for(103.0), crate::campaign::Medal::Gold);
        assert_eq!(m.medal_for(130.0), crate::campaign::Medal::None);
        assert_ne!(replay_hash(b"a"), replay_hash(b"b"));
        assert!(decode("garbage").is_none());
    }
}
