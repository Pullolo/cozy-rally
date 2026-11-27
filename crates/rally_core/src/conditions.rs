//! Dynamic stage conditions: a seeded weather timeline along the road.
//!
//! "Unpredictable" for the player (every stage is different, fronts arrive mid-stage) but a
//! pure function of the stage seed, so every player drives exactly the same conditions and
//! leaderboard times stay fair. Conditions vary along arc length, not time.
//!
//! Ingredients: rain fronts that start mid-stage and stay, passing showers, roads that stay
//! wet after the rain and dry slowly, fog banks lying in valleys / low points, drifting
//! overcast, wind with gusts, snowfall (Snow), sand storms (Desert: fog + strong wind), and a
//! building storm for storm stages.
use crate::biome::Biome;
use crate::noise;
use crate::rng::Rng;
use crate::weather::{ConditionsKey, StageConditions, Weather};

/// Kinds of weather events (for stats / pace notes / UI)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherEvent {
    RainFront,
    Shower,
    Snowfall,
    FogBank,
    SandStorm,
    Storm,
}

pub struct ConditionsInput<'a> {
    pub seed: u64,
    pub biome: Biome,
    pub difficulty: u8,
    /// Build up to a storm over the stage
    pub storm: bool,
    /// Road elevation per sample (`spacing` apart)
    pub heights: &'a [f32],
    pub spacing: f32,
    /// Start / finish line arc lengths (loops: 0 / length)
    pub start_s: f32,
    pub finish_s: f32,
    pub closed: bool,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A bump that ramps up over `ramp` m, holds, and ramps down over `ramp_out` m.
fn plateau(s: f32, a: f32, b: f32, ramp: f32, ramp_out: f32) -> f32 {
    smoothstep(a - ramp, a, s) * (1.0 - smoothstep(b, b + ramp_out, s))
}

/// Generate the conditions timeline plus the list of events (s, kind) for notes / stats.
pub fn generate(inp: &ConditionsInput) -> (StageConditions, Vec<(f32, f32, WeatherEvent)>) {
    let mut rng = Rng::new(inp.seed ^ 0x00C0_4D17_1045);
    let n = inp.heights.len();
    let road_len = n as f32 * inp.spacing;
    let stage = (inp.finish_s - inp.start_s).max(100.0);
    let d = (inp.difficulty as usize).min(2);
    let nseed = (inp.seed as u32).wrapping_mul(747796405).wrapping_add(2891336453);
    let at_frac = |f: f32| inp.start_s + stage * f;

    let base_overcast = match inp.biome {
        Biome::Meadow => 0.15,
        Biome::Autumn => 0.35,
        Biome::Snow => 0.45,
        Biome::Desert => 0.05,
        Biome::Coast => 0.25,
        Biome::Forest => 0.4,
    };
    let base_wind = match inp.biome {
        Biome::Coast => rng.range(4.0, 6.5),
        Biome::Desert => rng.range(2.5, 4.0),
        Biome::Snow => rng.range(2.0, 3.5),
        _ => rng.range(1.0, 2.5),
    };
    let wind_dir = rng.range(0.0, std::f32::consts::TAU);
    let snowy = inp.biome == Biome::Snow;
    let desert = inp.biome == Biome::Desert;

    // ---- events: (start, end, kind, intensity)
    let mut events: Vec<(f32, f32, WeatherEvent, f32)> = Vec::new();
    let wet_kind = if snowy { WeatherEvent::Snowfall } else { WeatherEvent::Shower };
    if inp.storm {
        let a = at_frac(rng.range(0.45, 0.65));
        events.push((a, road_len + 1000.0, WeatherEvent::Storm, rng.range(0.85, 1.0)));
    } else {
        // a front that arrives mid-stage (more likely on harder stages)
        let front_p = [0.25, 0.45, 0.6][d];
        if !desert && rng.chance(front_p) {
            let a = at_frac(rng.range(0.25, 0.7));
            let b = if rng.chance(0.6) { road_len + 1000.0 } else { a + rng.range(900.0, 2200.0) };
            events.push((a, b, if snowy { WeatherEvent::Snowfall } else { WeatherEvent::RainFront }, rng.range(0.5, 0.9)));
        }
    }
    // passing showers
    let showers = [rng.range_i(0, 2), rng.range_i(1, 3), rng.range_i(1, 3)][d] as usize;
    for _ in 0..showers {
        if desert && !rng.chance(0.2) {
            continue;
        }
        let a = at_frac(rng.range(0.05, 0.85));
        let l = rng.range(220.0, 650.0);
        if events.iter().any(|e| a < e.1 + 300.0 && a + l > e.0 - 300.0) {
            continue;
        }
        events.push((a, a + l, wet_kind, rng.range(0.3, 0.75)));
    }
    // sand storms in the desert
    if desert && rng.chance([0.4, 0.7, 0.9][d]) {
        let a = at_frac(rng.range(0.3, 0.75));
        events.push((a, a + rng.range(400.0, 900.0), WeatherEvent::SandStorm, rng.range(0.55, 0.85)));
    }
    // fog banks in the valleys: local low points of the road
    let fog_count = match inp.biome {
        Biome::Desert => 0,
        _ => [rng.range_i(0, 2), rng.range_i(1, 3), rng.range_i(1, 3)][d] as usize,
    };
    if fog_count > 0 {
        let win = (300.0 / inp.spacing) as isize;
        let mut lows: Vec<(usize, f32)> = Vec::new();
        let i0 = ((inp.start_s + 150.0) / inp.spacing) as isize;
        let i1 = ((inp.finish_s - 100.0) / inp.spacing) as isize;
        let mut i = i0;
        while i < i1 {
            let y = inp.heights[i as usize];
            let mut is_min = true;
            let mut rim = f32::MIN;
            for k in (-win..=win).step_by(5) {
                let j = (i + k).clamp(0, n as isize - 1) as usize;
                if inp.heights[j] < y - 0.01 {
                    is_min = false;
                    break;
                }
                rim = rim.max(inp.heights[j]);
            }
            if is_min && rim - y > 5.0 {
                lows.push((i as usize, rim - y));
            }
            i += 5;
        }
        lows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let mut placed = 0;
        for (li, _) in lows {
            if placed >= fog_count {
                break;
            }
            let c = li as f32 * inp.spacing;
            if events.iter().any(|e| e.2 == WeatherEvent::FogBank && (e.0 + e.1) * 0.5 - c < 600.0 && c - (e.0 + e.1) * 0.5 < 600.0) {
                continue;
            }
            let hw = rng.range(130.0, 260.0);
            events.push((c - hw, c + hw, WeatherEvent::FogBank, rng.range(0.55, 0.9)));
            placed += 1;
        }
    }
    // dynamic stages always have *something* going on
    if events.is_empty() {
        let a = at_frac(rng.range(0.3, 0.7));
        let kind = if desert { WeatherEvent::SandStorm } else { wet_kind };
        events.push((a, a + rng.range(350.0, 800.0), kind, rng.range(0.45, 0.75)));
    }
    events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // ---- sample along the road and integrate wetness
    let step = 25.0;
    let count = (road_len / step).ceil() as usize + 1;
    let mut keys: Vec<ConditionsKey> = Vec::with_capacity(count);
    let mut wet: f32 = match inp.biome {
        Biome::Autumn | Biome::Forest => rng.range(0.0, 0.25), // damp woods
        Biome::Coast => rng.range(0.0, 0.15),
        _ => 0.0,
    };
    let base_wet = wet;
    let dry_rate = match inp.biome {
        Biome::Desert => 1.0 / 500.0,
        Biome::Snow => 1.0 / 2000.0,
        _ => 1.0 / rng.range(900.0, 1500.0),
    };
    for k in 0..count {
        let s = k as f32 * step;
        let mut precip: f32 = 0.0;
        let mut fog: f32 = 0.0;
        let mut overcast = (base_overcast + 0.3 * noise::fbm(s / 1400.0, 0.5, 2, nseed)).clamp(0.0, 1.0);
        let gust = noise::fbm(s / 90.0, 3.7, 2, nseed + 11).abs() * 2.0;
        let mut wind = base_wind * (1.0 + 0.6 * noise::fbm(s / 700.0, 1.3, 2, nseed + 5)) + gust * base_wind * 0.6;
        let mut snow = false;
        for &(a, b, kind, inten) in &events {
            match kind {
                WeatherEvent::RainFront | WeatherEvent::Shower | WeatherEvent::Snowfall => {
                    let ramp = if kind == WeatherEvent::Shower { 80.0 } else { 220.0 };
                    let f = plateau(s, a, b, ramp, 180.0);
                    // fronts flicker a little in intensity
                    let flick = 0.85 + 0.15 * noise::fbm(s / 120.0, 7.1, 2, nseed + 3);
                    precip = precip.max(f * inten * flick);
                    overcast = overcast.max(plateau(s, a - 250.0, b + 150.0, 300.0, 300.0) * (0.6 + 0.4 * inten));
                    if kind == WeatherEvent::Snowfall && f > 0.05 {
                        snow = true;
                    }
                    fog = fog.max(f * 0.2 * inten);
                    wind += f * inten * 3.0;
                }
                WeatherEvent::Storm => {
                    // clouds and wind build long before the rain
                    let build = smoothstep(a - 1500.0, a, s);
                    overcast = overcast.max(0.5 + 0.5 * build);
                    wind += build * 6.0 + smoothstep(a, a + 300.0, s) * 3.0;
                    let f = smoothstep(a - 200.0, a + 150.0, s);
                    precip = precip.max(f * inten * (0.85 + 0.15 * noise::fbm(s / 80.0, 2.2, 2, nseed + 9)));
                    fog = fog.max(f * 0.35);
                    snow |= snowy && f > 0.05;
                }
                WeatherEvent::FogBank => {
                    let f = plateau(s, a + 60.0, b - 60.0, 120.0, 120.0);
                    fog = fog.max(f * inten);
                    overcast = overcast.max(f * 0.5);
                }
                WeatherEvent::SandStorm => {
                    let f = plateau(s, a, b, 150.0, 200.0);
                    fog = fog.max(f * inten);
                    wind += f * rng_free_wind(inten);
                    overcast = overcast.max(f * 0.45);
                }
            }
        }
        // wetness lags the rain and dries slowly afterwards
        let target = if snow { precip * 0.45 } else { (precip * 1.15).min(1.0) };
        if target > wet {
            wet += (target - wet) * (step / 140.0).min(1.0);
        } else {
            wet = (wet - dry_rate * step).max(target.max(base_wet));
        }
        let w = Weather {
            precipitation: precip.clamp(0.0, 1.0),
            snow,
            wetness: wet.clamp(0.0, 1.0),
            fog: fog.clamp(0.0, 1.0),
            overcast: overcast.clamp(0.0, 1.0),
            wind: [wind * wind_dir.cos(), wind * wind_dir.sin()],
            ..Weather::default()
        };
        keys.push(ConditionsKey { s, weather: w });
    }
    if inp.closed && keys.len() > 4 {
        // loops: blend the last stretch back into the first so laps are seamless
        let first = keys[0].weather;
        let m = keys.len();
        let blend = (12).min(m / 3);
        for j in 0..blend {
            let t = (j + 1) as f32 / blend as f32;
            let k = m - blend + j;
            keys[k].weather = keys[k].weather.lerp(&first, t);
        }
    }
    let ev = events.iter().map(|e| (e.0.max(0.0), e.1.min(road_len), e.2)).collect();
    (StageConditions { keys, ..Default::default() }, ev)
}

fn rng_free_wind(inten: f32) -> f32 {
    6.0 + 8.0 * inten
}
