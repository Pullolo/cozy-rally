//! Player settings: global, cheap to read from any module (`settings::get()`), persisted
//! to a small key=value file in the per-user config dir (falls back to the exe dir).
//! Saving never panics; an unwritable location just means settings don't persist.
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Low,
    High,
}

impl Quality {
    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "Low",
            Quality::High => "High",
        }
    }
}

/// Default chase camera mode for new races: 0 = Chase, 1 = Far, 2 = Hood
pub const CAMERA_NAMES: [&str; 3] = ["Chase", "Far", "Hood"];
/// Assist presets (index into main.rs ASSISTS): Cozy / Normal / Sport / Sim
pub const ASSIST_NAMES: [&str; 4] = ["Cozy", "Normal", "Sport", "Sim"];
/// Racing-line assist: 0 Auto (Full, or Off with Sim assists), 1 Off, 2 Braking only, 3 Full
pub const RACING_LINE_NAMES: [&str; 4] = ["Auto", "Off", "Braking only", "Full"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// 0..1 volumes, passed to `audio::set_volumes`
    pub master: f32,
    pub music: f32,
    pub sfx: f32,
    /// Default assist preset for new races (index into ASSIST_NAMES)
    pub assists: usize,
    /// Default camera mode (index into CAMERA_NAMES)
    pub camera: usize,
    pub fullscreen: bool,
    pub show_fps: bool,
    /// Rendering quality; the renderer reads it via `settings::quality()`
    pub quality: Quality,
    /// Show the tyre-grip widget next to the speedo
    pub show_traction: bool,
    /// Rally pace-notes strip at the top of the race HUD (key N toggles in race)
    pub pace_notes: bool,
    /// Racing-line assist (index into RACING_LINE_NAMES; key L cycles in race)
    pub racing_line: usize,
    /// Pace notes on circuits (separate from rally stages; default off)
    pub pace_notes_circuit: bool,
    /// Last race setup choices: 0 = Rally, 1 = Circuit
    pub discipline: usize,
    /// Circuit picker: 0 = Random, 1.. = index+1 into rally_core::circuits::circuits()
    pub circuit: usize,
    /// Random circuit size: 0 Sprint, 1 Grand Prix, 2 Ring, 3 Nordring
    pub circuit_size: usize,
    /// Last car per discipline
    pub car_rally: usize,
    pub car_circuit: usize,
    /// On-screen steering style on phones (index into touch::STEER_NAMES)
    pub touch_steer: usize,
    /// Rally route: 0 = loop (laps), 1 = stage A to B
    pub rally_route: usize,
    /// Stage length (index into main.rs STAGE_KM)
    pub stage_len: usize,
    /// Race weather (index into main.rs WEATHERS)
    pub weather: usize,
}

impl Settings {
    pub const DEFAULT: Settings = Settings { master: 0.8, music: 0.7, sfx: 0.9, assists: 1, camera: 0, fullscreen: false, show_fps: false, quality: if crate::platform::MOBILE { Quality::Low } else { Quality::High }, show_traction: !crate::platform::MOBILE, pace_notes: true, racing_line: 0, pace_notes_circuit: false, discipline: 0, circuit: 1, circuit_size: 1, car_rally: 2, car_circuit: 5, touch_steer: 0, rally_route: 0, stage_len: 1, weather: 0 };
}

impl Default for Settings {
    fn default() -> Self {
        Settings::DEFAULT
    }
}

static SETTINGS: RwLock<Settings> = RwLock::new(Settings::DEFAULT);

/// Current settings (a cheap copy)
pub fn get() -> Settings {
    SETTINGS.read().map(|s| *s).unwrap_or_default()
}

/// Current graphics quality (convenience for the renderer)
#[allow(dead_code)]
pub fn quality() -> Quality {
    get().quality
}

/// Replace the settings and apply the side effects that are live (audio volumes).
pub fn set(s: Settings) {
    if let Ok(mut g) = SETTINGS.write() {
        *g = s;
    }
    apply_audio();
}

/// Modify the settings in place
pub fn update(f: impl FnOnce(&mut Settings)) {
    let mut s = get();
    f(&mut s);
    set(s);
}

pub fn apply_audio() {
    let s = get();
    crate::audio::set_volumes(s.master, s.music, s.sfx);
}

fn config_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        env("APPDATA").map(|p| p.join("CozyRally"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|p| p.join("Library/Application Support/CozyRally"))
    } else if cfg!(target_os = "android") {
        None
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config"))).map(|p| p.join("cozy-rally"))
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

/// Candidate settings files, preferred first
fn paths() -> Vec<PathBuf> {
    if let Some(p) = std::env::var_os("COZY_SETTINGS") {
        return vec![PathBuf::from(p)];
    }
    config_dir().into_iter().chain(exe_dir()).map(|d| d.join("settings.txt")).collect()
}

fn parse(text: &str) -> Settings {
    let mut s = Settings::DEFAULT;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let f = || v.parse::<f32>().ok().filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0));
        let b = || matches!(v, "1" | "true" | "yes" | "on");
        match k {
            "master" => s.master = f().unwrap_or(s.master),
            "music" => s.music = f().unwrap_or(s.music),
            "sfx" => s.sfx = f().unwrap_or(s.sfx),
            "assists" => s.assists = v.parse::<usize>().unwrap_or(s.assists).min(ASSIST_NAMES.len() - 1),
            "camera" => s.camera = v.parse::<usize>().unwrap_or(s.camera).min(CAMERA_NAMES.len() - 1),
            "fullscreen" => s.fullscreen = b(),
            "show_fps" => s.show_fps = b(),
            "show_traction" => s.show_traction = b(),
            "pace_notes" => s.pace_notes = b(),
            "pace_notes_circuit" => s.pace_notes_circuit = b(),
            "discipline" => s.discipline = v.parse::<usize>().unwrap_or(0).min(1),
            "circuit" => s.circuit = v.parse::<usize>().unwrap_or(s.circuit),
            "circuit_size" => s.circuit_size = v.parse::<usize>().unwrap_or(1).min(3),
            "car_rally" => s.car_rally = v.parse::<usize>().unwrap_or(s.car_rally),
            "car_circuit" => s.car_circuit = v.parse::<usize>().unwrap_or(s.car_circuit),
            "racing_line" => s.racing_line = v.parse::<usize>().unwrap_or(s.racing_line).min(RACING_LINE_NAMES.len() - 1),
            "touch_steer" => s.touch_steer = v.parse::<usize>().unwrap_or(0).min(crate::touch::STEER_NAMES.len() - 1),
            "quality" => s.quality = if v.eq_ignore_ascii_case("low") { Quality::Low } else { Quality::High },
            "rally_route" => s.rally_route = v.parse::<usize>().unwrap_or(0).min(1),
            "stage_len" => s.stage_len = v.parse::<usize>().unwrap_or(s.stage_len),
            "weather" => s.weather = v.parse::<usize>().unwrap_or(0),
            _ => {}
        }
    }
    s
}

fn serialize(s: &Settings) -> String {
    format!(
        "# Cozy Rally settings\nmaster={:.2}\nmusic={:.2}\nsfx={:.2}\nassists={}\ncamera={}\nfullscreen={}\nshow_fps={}\nshow_traction={}\npace_notes={}\nracing_line={}\npace_notes_circuit={}\ndiscipline={}\ncircuit={}\ncircuit_size={}\ncar_rally={}\ncar_circuit={}\ntouch_steer={}\nquality={}\nrally_route={}\nstage_len={}\nweather={}\n",
        s.master,
        s.music,
        s.sfx,
        s.assists,
        s.camera,
        s.fullscreen,
        s.show_fps,
        s.show_traction,
        s.pace_notes,
        s.racing_line,
        s.pace_notes_circuit,
        s.discipline,
        s.circuit,
        s.circuit_size,
        s.car_rally,
        s.car_circuit,
        s.touch_steer,
        s.quality.name().to_lowercase(),
        s.rally_route,
        s.stage_len,
        s.weather
    )
}

/// Load settings from disk into the global (defaults if missing/unreadable). Does not touch audio.
pub fn load() -> Settings {
    let s = paths().iter().find_map(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default();
    if let Ok(mut g) = SETTINGS.write() {
        *g = s;
    }
    s
}

/// Persist the current settings. Errors are ignored (read-only installs still work).
pub fn save() {
    let text = serialize(&get());
    for p in paths() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&p, &text).is_ok() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let s = Settings { master: 0.3, music: 0.0, sfx: 1.0, assists: 3, camera: 2, fullscreen: true, show_fps: true, quality: Quality::Low, show_traction: false, pace_notes: false, racing_line: 2, pace_notes_circuit: true, discipline: 1, circuit: 3, circuit_size: 2, car_rally: 1, car_circuit: 7, touch_steer: 1, rally_route: 1, stage_len: 2, weather: 5 };
        assert_eq!(parse(&serialize(&s)), s);
        assert_eq!(parse("garbage\nmaster=nan\nassists=99"), Settings { assists: 3, ..Settings::DEFAULT });
    }
}

/// Online prefs (last server address + player name), kept in `online.txt` next to settings.txt
/// because `Settings` is `Copy`. Returns ("", "") when missing.
pub fn load_online() -> (String, String) {
    let text = paths().iter().find_map(|p| std::fs::read_to_string(p.with_file_name("online.txt")).ok()).unwrap_or_default();
    let mut out = (String::new(), String::new());
    for line in text.lines() {
        match line.split_once('=') {
            Some(("address", v)) => out.0 = v.trim().to_string(),
            Some(("name", v)) => out.1 = v.trim().to_string(),
            _ => {}
        }
    }
    out
}

pub fn save_online(address: &str, name: &str) {
    let clean = |s: &str| s.replace(['\n', '\r'], "");
    let text = format!("address={}\nname={}\n", clean(address), clean(name));
    for p in paths() {
        let p = p.with_file_name("online.txt");
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&p, &text).is_ok() {
            break;
        }
    }
}
