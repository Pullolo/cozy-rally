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
    /// Standing water 0..1 (a downpour floods the road faster than it drains): tyres
    /// aquaplane at speed, see `aquaplane_grip`
    #[serde(default)]
    pub water: f32,
    /// Black ice / frost on the road 0..1 (cold nights): grip collapses on every surface
    #[serde(default)]
    pub ice: f32,
}

impl Default for Weather {
    fn default() -> Self {
        Weather { precipitation: 0.0, snow: false, wetness: 0.0, fog: 0.0, overcast: 0.0, wind: [1.5, 0.5], water: 0.0, ice: 0.0 }
    }
}

/// Grip left (0..1) for a tyre running through standing water `water` at `speed` m/s:
/// fine below ~60 km/h, then the tyre rides up on a film of water. Treaded rally tyres
/// clear water; slicks have no tread and float early. Shared by physics, AI and HUD.
pub fn aquaplane_grip(water: f32, speed: f32, slick: bool) -> f32 {
    if water <= 0.0 {
        return 1.0;
    }
    let (v0, v1, depth) = if slick { (17.0, 45.0, 0.75) } else { (22.0, 55.0, 0.6) };
    let t = ((speed - v0) / (v1 - v0)).clamp(0.0, 1.0);
    1.0 - depth * water.clamp(0.0, 1.0) * t * t * (3.0 - 2.0 * t)
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
            water: l(self.water, o.water),
            ice: l(self.ice, o.ice),
        }
    }

    /// Grip multiplier for a surface under these conditions (physics reads this).
    pub fn grip_factor(&self, surface: Surface) -> f32 {
        let w = self.wetness.clamp(0.0, 1.0);
        let wet = match surface {
            Surface::Asphalt => 1.0 - 0.28 * w,
            Surface::Gravel | Surface::Dirt => 1.0 - 0.14 * w,
            Surface::Grass => 1.0 - 0.25 * w,
            Surface::Mud => 1.0 - 0.15 * w,
            Surface::Sand => 1.0 + 0.05 * w,
            Surface::Snow => 1.0 - 0.10 * w,
            Surface::Ice => 1.0 - 0.20 * w,
        };
        // black ice glazes sealed roads the most; loose ground keeps a little bite
        let ice = self.ice.clamp(0.0, 1.0);
        let ice_k = match surface {
            Surface::Asphalt => 0.72,
            Surface::Gravel | Surface::Dirt | Surface::Mud => 0.5,
            Surface::Grass | Surface::Sand | Surface::Snow => 0.4,
            Surface::Ice => 0.2,
        };
        // a flooded road is a little slipperier even at low speed (aquaplaning is separate)
        wet * (1.0 - ice_k * ice) * (1.0 - 0.06 * self.water.clamp(0.0, 1.0))
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
    /// A cloudburst: rain so heavy the spray hides the car ahead and water stands on the
    /// racing line (aquaplaning)
    Downpour,
    /// Fog so thick you see ~25 m: a race by memory
    ThickFog,
    /// Clear, freezing night air: a glaze of ice on the road
    BlackIce,
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
            WeatherPreset::Downpour => Weather { precipitation: 1.0, wetness: 1.0, water: 1.0, overcast: 1.0, fog: 0.93, wind: [7.0, 2.5], ..base },
            WeatherPreset::ThickFog => Weather { fog: 1.0, overcast: 0.6, wetness: 0.25, wind: [0.4, 0.2], ..base },
            WeatherPreset::BlackIce => Weather { ice: 0.6, overcast: 0.2, fog: 0.2, wetness: 0.1, wind: [0.6, 0.3], ..base },
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            WeatherPreset::Clear => "Sunny",
            WeatherPreset::Overcast => "Overcast",
            WeatherPreset::Drizzle => "Drizzle",
            WeatherPreset::Rain => "Rain",
            WeatherPreset::Storm => "Storm",
            WeatherPreset::Fog => "Fog",
            WeatherPreset::Snowfall => "Snowfall",
            WeatherPreset::Downpour => "Downpour",
            WeatherPreset::ThickFog => "Thick fog",
            WeatherPreset::BlackIce => "Black ice",
        }
    }
}

/// How weather changes over race time on a circuit (`ConditionsSpec::Timeline`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WeatherTurn {
    /// Weather at the start
    pub from: WeatherPreset,
    /// What it turns into
    pub to: WeatherPreset,
    /// Race clock (s) when the change has fully arrived (it builds over `ramp` s before)
    pub at: u16,
    /// Seconds it lasts before clearing back to `from` (0 = stays to the end)
    pub lasts: u16,
    /// Build-up time (s)
    pub ramp: u16,
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
    /// Weather over race TIME (circuits: the same everywhere, changing lap by lap): e.g. a
    /// sunny race that turns into a downpour on lap 3. Roads soak, flood and dry gradually.
    Timeline(WeatherTurn),
}

/// A keyframe on the conditions timeline (arc length in metres).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConditionsKey {
    pub s: f32,
    pub weather: Weather,
}

/// Race clock shared by everyone reading a track's time-based weather (the `World` sets it
/// every tick). Interior-mutable so `Track` stays immutable / shareable.
#[derive(Debug, Default)]
pub struct WeatherClock(std::sync::atomic::AtomicU32);

impl WeatherClock {
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }
    pub fn set(&self, t: f32) {
        self.0.store(t.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

impl Clone for WeatherClock {
    fn clone(&self) -> Self {
        let c = WeatherClock::default();
        c.set(self.get());
        c
    }
}

impl PartialEq for WeatherClock {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Conditions along a track. For loops `s` wraps; for stages it is clamped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StageConditions {
    /// Sorted by s; empty = Weather::default() everywhere
    pub keys: Vec<ConditionsKey>,
    /// Time-based weather (circuits, `ConditionsSpec::Timeline`): one sample every
    /// `time_step` seconds of race clock. When present it replaces the position keys.
    #[serde(default)]
    pub time_keys: Vec<Weather>,
    #[serde(default)]
    pub time_step: f32,
    #[serde(skip)]
    pub clock: WeatherClock,
}

impl StageConditions {
    pub fn uniform(w: Weather) -> Self {
        StageConditions { keys: vec![ConditionsKey { s: 0.0, weather: w }], ..Default::default() }
    }

    /// Weather that changes over race time (see `WeatherTurn`), sampled every 2 s for 40 min.
    /// Precipitation, fog and cloud follow the turn; the road soaks up rain with a lag,
    /// floods in heavy rain and dries out slowly afterwards.
    pub fn timeline(turn: WeatherTurn) -> Self {
        let (a, b) = (turn.from.weather(), turn.to.weather());
        let step = 2.0f32;
        let n = (2400.0 / step) as usize;
        let ramp = (turn.ramp as f32).max(4.0);
        let at = turn.at as f32;
        let end = if turn.lasts == 0 { f32::INFINITY } else { at + turn.lasts as f32 };
        let ss = |e0: f32, e1: f32, x: f32| {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let mut keys = Vec::with_capacity(n);
        let (mut wet, mut water) = (a.wetness, a.water);
        for k in 0..n {
            let t = k as f32 * step;
            let clearing = if end.is_finite() { ss(end, end + ramp * 1.5, t) } else { 0.0 };
            let f = if t < at - ramp { 0.0 } else { ss(at - ramp, at, t) * (1.0 - clearing) };
            let mut w = a.lerp(&b, f);
            // the road lags the sky: soaks in ~20-40 s of rain, dries over minutes
            let rain = if w.snow { 0.0 } else { (w.precipitation * 1.1).min(1.0) };
            let wet_target = w.wetness.max(rain);
            if wet_target > wet {
                wet += (wet_target - wet) * (step / (14.0 + 30.0 * (1.0 - w.precipitation))).min(1.0);
            } else {
                wet = (wet - step / 240.0).max(wet_target);
            }
            if w.water > water {
                water += (w.water - water) * (step / 35.0).min(1.0);
            } else {
                water = (water - step / 90.0).max(w.water);
            }
            w.wetness = wet.clamp(0.0, 1.0);
            w.water = water.clamp(0.0, 1.0);
            keys.push(w);
        }
        StageConditions { keys: vec![ConditionsKey { s: 0.0, weather: keys[0] }], time_keys: keys, time_step: step, clock: WeatherClock::default() }
    }

    /// Time-based weather? (then `at` ignores the position)
    pub fn is_timed(&self) -> bool {
        !self.time_keys.is_empty()
    }

    /// Set the race clock for time-based weather (World does this every tick)
    pub fn set_clock(&self, t: f32) {
        if self.is_timed() {
            self.clock.set(t);
        }
    }

    /// Time-based weather at race clock `t`
    pub fn at_time(&self, t: f32) -> Weather {
        let n = self.time_keys.len();
        if n == 0 {
            return self.keys.first().map(|k| k.weather).unwrap_or_default();
        }
        let f = (t.max(0.0) / self.time_step.max(0.1)).min((n - 1) as f32);
        let i = (f as usize).min(n - 1);
        let j = (i + 1).min(n - 1);
        self.time_keys[i].lerp(&self.time_keys[j], f - i as f32)
    }

    /// Smoothly interpolated conditions at arc length `s` (time-based weather: now).
    pub fn at(&self, s: f32) -> Weather {
        if self.is_timed() {
            return self.at_time(self.clock.get());
        }
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
