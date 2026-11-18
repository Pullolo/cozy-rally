//! Weather & stage conditions (shared interface).
//!
//! A stage carries a deterministic *conditions timeline* along its arc length: e.g. dry
//! start → rain front at 2.1 km → fog bank in the valley → drying tarmac near the finish.
//! It is "unpredictable" for the player (generated from the seed, different every stage)
//! but identical for everyone on the same stage, so leaderboard times stay fair.
use crate::surface::Surface;
use serde::{Deserialize, Serialize};

/// Instantaneous conditions at a point on the stage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Weather {
    /// Precipitation intensity 0..1 (rain, or snowfall when `snow` is true)
    pub precipitation: f32,
    /// true = snowfall instead of rain
    pub snow: bool,
    /// Road wetness 0..1 (lags behind rain: puddles stay after a shower)
    pub wetness: f32,
    /// Fog density 0..1 (0 = biome default fog, 1 = ~60 m visibility)
    pub fog: f32,
    /// Cloud cover 0..1 (dims and cools the sun)
    pub overcast: f32,
    /// Wind (m/s, world XZ) for particles / foliage
    pub wind: [f32; 2],
}

impl Default for Weather {
    fn default() -> Self {
        Weather { precipitation: 0.0, snow: false, wetness: 0.0, fog: 0.0, overcast: 0.0, wind: [1.5, 0.5] }
    }
}

impl Weather {
    pub fn lerp(&self, o: &Weather, t: f32) -> Weather {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Weather {
            precipitation: l(self.precipitation, o.precipitation),
            snow: if t < 0.5 { self.snow } else { o.snow },
            wetness: l(self.wetness, o.wetness),
            fog: l(self.fog, o.fog),
            overcast: l(self.overcast, o.overcast),
            wind: [l(self.wind[0], o.wind[0]), l(self.wind[1], o.wind[1])],
        }
    }

    /// Grip multiplier for a surface under these conditions (physics reads this).
    pub fn grip_factor(&self, surface: Surface) -> f32 {
        let w = self.wetness.clamp(0.0, 1.0);
        match surface {
            Surface::Asphalt => 1.0 - 0.28 * w,
            Surface::Gravel | Surface::Dirt => 1.0 - 0.14 * w,
            Surface::Grass => 1.0 - 0.25 * w,
            Surface::Mud => 1.0 - 0.15 * w,
            Surface::Sand => 1.0 + 0.05 * w,
            Surface::Snow => 1.0 - 0.10 * w,
            Surface::Ice => 1.0 - 0.20 * w,
        }
    }
}

/// Named presets (also used for fixed-weather loop races).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WeatherPreset {
    #[default]
    Clear,
    Overcast,
    Drizzle,
    Rain,
    Storm,
    Fog,
    Snowfall,
}

impl WeatherPreset {
    pub fn weather(self) -> Weather {
        let base = Weather::default();
        match self {
            WeatherPreset::Clear => base,
            WeatherPreset::Overcast => Weather { overcast: 0.7, ..base },
            WeatherPreset::Drizzle => Weather { precipitation: 0.3, wetness: 0.45, overcast: 0.75, fog: 0.15, ..base },
            WeatherPreset::Rain => Weather { precipitation: 0.7, wetness: 0.85, overcast: 0.9, fog: 0.25, wind: [4.0, 1.0], ..base },
            WeatherPreset::Storm => Weather { precipitation: 1.0, wetness: 1.0, overcast: 1.0, fog: 0.35, wind: [9.0, 3.0], ..base },
            WeatherPreset::Fog => Weather { fog: 0.85, overcast: 0.5, wetness: 0.15, ..base },
            WeatherPreset::Snowfall => Weather { precipitation: 0.6, snow: true, overcast: 0.8, fog: 0.3, ..base },
        }
    }
}

/// How a track's conditions are chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConditionsSpec {
    /// Today's behaviour: biome-default clear weather everywhere
    #[default]
    BiomeDefault,
    /// One preset for the whole track
    Fixed(WeatherPreset),
    /// Seeded, changing conditions along the stage (rain fronts, fog banks, drying roads…)
    Dynamic,
    /// Like Dynamic, but the weather builds into a storm over the stage
    Storm,
}

/// A keyframe on the conditions timeline (arc length in metres).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConditionsKey {
    pub s: f32,
    pub weather: Weather,
}

/// Conditions along a track. For loops `s` wraps; for stages it is clamped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StageConditions {
    /// Sorted by s; empty = Weather::default() everywhere
    pub keys: Vec<ConditionsKey>,
}

impl StageConditions {
    pub fn uniform(w: Weather) -> Self {
        StageConditions { keys: vec![ConditionsKey { s: 0.0, weather: w }] }
    }

    /// Smoothly interpolated conditions at arc length `s`.
    pub fn at(&self, s: f32) -> Weather {
        match self.keys.len() {
            0 => Weather::default(),
            1 => self.keys[0].weather,
            _ => {
                if s <= self.keys[0].s {
                    return self.keys[0].weather;
                }
                for w in self.keys.windows(2) {
                    if s <= w[1].s {
                        let t = ((s - w[0].s) / (w[1].s - w[0].s).max(1e-3)).clamp(0.0, 1.0);
                        let t = t * t * (3.0 - 2.0 * t);
                        return w[0].weather.lerp(&w[1].weather, t);
                    }
                }
                self.keys.last().unwrap().weather
            }
        }
    }
}
