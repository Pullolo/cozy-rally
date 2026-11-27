//! UI-side adapter for the time-trial campaign.
//!
//! The screens only talk to this module: stage definitions from `rally_core::campaign`,
//! progress / medals / boards from the gameplay team's `campaign_mode` (single storage),
//! plus UI-only helpers (forecast text, weather classification).
use rally_core::campaign::{campaign_of, CampaignKind, Chapter, Medal, StageDef};
use rally_core::conditions::WeatherEvent;
use rally_core::weather::{ConditionsSpec, Weather, WeatherPreset};
use std::cell::Cell;

thread_local! {
    /// Campaign the screens are showing (Rally stages or the Circuit season)
    static KIND: Cell<CampaignKind> = const { Cell::new(CampaignKind::Rally) };
}

/// Campaign the screens show (chapters / views / unlock state follow it)
pub fn kind() -> CampaignKind {
    KIND.with(|k| k.get())
}

pub fn set_kind(k: CampaignKind) {
    KIND.with(|c| c.set(k));
}

pub fn chapters() -> Vec<Chapter> {
    campaign_of(kind())
}

/// Stage length, or the whole race distance (laps x lap) on circuits
pub fn stage_length_m(def: &StageDef) -> f32 {
    def.distance_km() * 1000.0
}

pub fn difficulty_name(d: u8) -> &'static str {
    ["Relaxed", "Rally", "Expert"][d.min(2) as usize]
}

// ------------------------------------------------------------------ weather helpers

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WxKind {
    Clear,
    Cloudy,
    Wet,
    Showers,
    Rain,
    Storm,
    Fog,
    Snow,
    Sand,
    /// Cloudburst: standing water, spray, aquaplaning
    Downpour,
    /// Black ice / frost
    Ice,
}

impl WxKind {
    pub fn label(self) -> &'static str {
        match self {
            WxKind::Clear => "Clear",
            WxKind::Cloudy => "Cloudy",
            WxKind::Wet => "Wet road",
            WxKind::Showers => "Showers",
            WxKind::Rain => "Rain",
            WxKind::Storm => "Storm",
            WxKind::Fog => "Fog",
            WxKind::Snow => "Snowfall",
            WxKind::Sand => "Sand storm",
            WxKind::Downpour => "Downpour",
            WxKind::Ice => "Black ice",
        }
    }
}

/// Classify the weather at a point for the HUD chip
pub fn classify(w: &Weather) -> WxKind {
    if w.ice > 0.25 {
        WxKind::Ice
    } else if w.water > 0.45 && w.precipitation > 0.6 {
        WxKind::Downpour
    } else if w.snow && w.precipitation > 0.15 {
        WxKind::Snow
    } else if w.precipitation > 0.8 {
        WxKind::Storm
    } else if w.precipitation > 0.45 {
        WxKind::Rain
    } else if w.precipitation > 0.12 {
        WxKind::Showers
    } else if w.fog > 0.5 {
        WxKind::Fog
    } else if w.wetness > 0.3 {
        WxKind::Wet
    } else if w.overcast > 0.5 {
        WxKind::Cloudy
    } else {
        WxKind::Clear
    }
}

pub fn event_kind(e: WeatherEvent) -> WxKind {
    match e {
        WeatherEvent::RainFront => WxKind::Rain,
        WeatherEvent::Shower => WxKind::Showers,
        WeatherEvent::Snowfall => WxKind::Snow,
        WeatherEvent::FogBank => WxKind::Fog,
        WeatherEvent::SandStorm => WxKind::Sand,
        WeatherEvent::Storm => WxKind::Storm,
    }
}

/// Forecast for a stage card (cheap: no track generation). Returns (icon, hint text).
pub fn forecast(def: &StageDef) -> (WxKind, String) {
    let p = &def.params;
    let biome = p.biome.unwrap_or(rally_core::biome::Biome::Meadow);
    let night = |t: String| if p.night { format!("Night, {}", t.to_lowercase()) } else { t };
    match p.conditions {
        ConditionsSpec::Fixed(preset) => {
            let k = classify(&preset.weather());
            (k, night(if preset == WeatherPreset::Clear { if p.night { "Clear skies".into() } else { "Sunny".into() } } else { format!("{} all race", k.label()) }))
        }
        // a weather turn is a surprise: the forecast only says it might change
        ConditionsSpec::Timeline(t) => (classify(&t.from.weather()), night(format!("{}, changeable", if p.night && t.from == WeatherPreset::Clear { "Clear" } else { t.from.name() }))),
        ConditionsSpec::BiomeDefault => (WxKind::Clear, night("Fair weather".into())),
        ConditionsSpec::Dynamic | ConditionsSpec::Storm => {
            let len = stage_length_m(def);
            let spacing = 20.0;
            let heights = vec![0.0; (len / spacing) as usize + 2];
            let inp = rally_core::conditions::ConditionsInput { seed: p.seed, biome, difficulty: p.difficulty, storm: p.conditions == ConditionsSpec::Storm, heights: &heights, spacing, start_s: 0.0, finish_s: len, closed: false };
            let (_, events) = rally_core::conditions::generate(&inp);
            if p.conditions == ConditionsSpec::Storm {
                return (WxKind::Storm, "Storm brewing".into());
            }
            match events.first() {
                Some((s0, _, e)) => {
                    let k = event_kind(*e);
                    let when = if *s0 < len * 0.3 { "early" } else if *s0 < len * 0.65 { "mid-stage" } else { "late" };
                    let txt = match k {
                        WxKind::Showers => format!("Showers likely {when}"),
                        WxKind::Rain => format!("Rain front {when}"),
                        WxKind::Fog => format!("Fog banks {when}"),
                        WxKind::Snow => format!("Snow {when}"),
                        WxKind::Sand => format!("Dust storm {when}"),
                        _ => format!("Storm {when}"),
                    };
                    (k, txt)
                }
                None => (WxKind::Cloudy, "Changeable".into()),
            }
        }
    }
}

// ------------------------------------------------------------------ campaign_mode bridge

pub use crate::campaign_mode::{ChapterView, StageView};

/// Chapters with lock state, medals, best times and (cached) medal targets
pub fn views() -> Vec<ChapterView> {
    crate::campaign_mode::chapters_of(kind())
}

/// Gold-or-better medals across the campaign
pub fn golds(views: &[ChapterView]) -> u32 {
    views.iter().flat_map(|c| c.stages.iter()).filter(|s| s.medal >= Medal::Gold).count() as u32
}

pub fn stage_view<'a>(views: &'a [ChapterView], id: &str) -> Option<&'a StageView> {
    views.iter().flat_map(|c| c.stages.iter()).find(|s| s.id == id)
}

pub fn unlocked(views: &[ChapterView], ch: usize) -> bool {
    std::env::var("COZY_UNLOCK_ALL").is_ok() || views.get(ch).is_some_and(|c| c.unlocked)
}

/// Best run on the local board (time, splits) — read before a run starts so the results
/// screen and HUD can compare against the *previous* best
pub fn best_run(id: &str) -> Option<(f32, Vec<f32>)> {
    crate::campaign_mode::leaderboard(id, None, 1).first().map(|e| (e.time_ms as f32 / 1000.0, e.splits_ms.iter().map(|v| *v as f32 / 1000.0).collect()))
}
