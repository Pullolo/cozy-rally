//! Time-trial campaign definitions + leaderboard data model (shared interface).
//!
//! Campaign stages are point-to-point (A→B), several km long, with dynamic conditions.
//! Stage IDs are STABLE strings (never reuse/renumber): leaderboards key on
//! `(stage_id, CAMPAIGN_VERSION)`. Changing generation code that alters a stage's geometry
//! requires bumping `CAMPAIGN_VERSION` so old times are not compared against new roads.
use crate::biome::Biome;
use crate::surfacemap::SurfaceMode;
use crate::track::{LayoutKind, TrackParams};
use crate::vehicle::{car_specs, CarClass};
use crate::weather::{ConditionsSpec, WeatherPreset, WeatherTurn};
use serde::{Deserialize, Serialize};

pub const CAMPAIGN_VERSION: u32 = 1;

/// The two campaigns: rally stages (rally cars) and the circuit season (circuit cars).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CampaignKind {
    #[default]
    Rally,
    Circuit,
}

impl CampaignKind {
    pub const ALL: [CampaignKind; 2] = [CampaignKind::Rally, CampaignKind::Circuit];
    pub fn name(self) -> &'static str {
        match self {
            CampaignKind::Rally => "Rally",
            CampaignKind::Circuit => "Circuit",
        }
    }
    pub fn car_class(self) -> CarClass {
        match self {
            CampaignKind::Rally => CarClass::Rally,
            CampaignKind::Circuit => CarClass::Circuit,
        }
    }
}

/// What an event asks of you.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    /// Against the clock (stage A to B, or flying laps): medals from the reference time
    #[default]
    TimeTrial,
    /// Wheel to wheel against AI rivals, from the back of the grid. Medals by place:
    /// 3rd bronze, 2nd silver, a win gold, a win by `platinum_margin` seconds platinum.
    Race {
        rivals: u8,
        /// AI skill 0..1
        skill: f32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageDef {
    /// Stable unique id, e.g. "c1s2-hazel-hollow"
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub params: TrackParams,
    /// Allowed car ids (empty = any car)
    pub cars: Vec<u8>,
    #[serde(default)]
    pub kind: EventKind,
    /// Laps (circuits; stages are one run A to B)
    #[serde(default = "one_lap")]
    pub laps: u32,
}

fn one_lap() -> u32 {
    1
}

impl StageDef {
    pub fn is_race(&self) -> bool {
        matches!(self.kind, EventKind::Race { .. })
    }

    /// Race distance in km
    pub fn distance_km(&self) -> f32 {
        match self.params.layout {
            LayoutKind::PointToPoint { length_m } => length_m / 1000.0,
            LayoutKind::Loop => crate::circuits::approx_lap_km(self.params.length_class) * self.laps as f32,
        }
    }

    /// Winning margin (s) a race needs for platinum: grows with race length, 3..10 s
    pub fn platinum_margin(&self) -> f32 {
        (self.distance_km() * 0.45).clamp(3.0, 10.0)
    }

    /// Medal for finishing a race event in `place` (1-based), `margin` s ahead of second
    pub fn race_medal(&self, place: usize, margin: f32) -> Medal {
        match place {
            1 if margin >= self.platinum_margin() => Medal::Platinum,
            1 => Medal::Gold,
            2 => Medal::Silver,
            3 => Medal::Bronze,
            _ => Medal::None,
        }
    }
}

/// Car ids of a class (campaign car pools)
pub fn class_cars(class: CarClass) -> Vec<u8> {
    car_specs().iter().filter(|c| c.class == class).map(|c| c.id).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chapter {
    pub name: String,
    pub biome: Biome,
    pub stages: Vec<StageDef>,
    /// Gold medals needed (from previous chapters) to unlock this chapter
    pub unlock_golds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Medal {
    None,
    Bronze,
    Silver,
    Gold,
    /// Beat the developer/reference time by a margin
    Platinum,
}

/// Medal thresholds in seconds (computed from a reference run, see gameplay code).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MedalTimes {
    pub platinum: f32,
    pub gold: f32,
    pub silver: f32,
    pub bronze: f32,
}

impl MedalTimes {
    pub fn medal_for(&self, t: f32) -> Medal {
        if t <= self.platinum {
            Medal::Platinum
        } else if t <= self.gold {
            Medal::Gold
        } else if t <= self.silver {
            Medal::Silver
        } else if t <= self.bronze {
            Medal::Bronze
        } else {
            Medal::None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stage(id: &str, name: &str, blurb: &str, seed: u64, biome: Biome, km: f32, difficulty: u8, surface: SurfaceMode, cars: &[u8]) -> StageDef {
    stage_c(id, name, blurb, seed, biome, km, difficulty, surface, cars, ConditionsSpec::Dynamic)
}

/// Same as `stage` with an explicit conditions spec (e.g. `ConditionsSpec::Storm`)
#[allow(clippy::too_many_arguments)]
fn stage_c(id: &str, name: &str, blurb: &str, seed: u64, biome: Biome, km: f32, difficulty: u8, surface: SurfaceMode, cars: &[u8], conditions: ConditionsSpec) -> StageDef {
    // rally stages: rally cars only (slick-shod circuit racers are hopeless on gravel)
    let cars = if cars.is_empty() { class_cars(CarClass::Rally) } else { cars.to_vec() };
    StageDef {
        id: id.into(),
        name: name.into(),
        blurb: blurb.into(),
        params: TrackParams {
            seed,
            biome: Some(biome),
            length_class: 2,
            difficulty,
            surface,
            layout: LayoutKind::PointToPoint { length_m: km * 1000.0 },
            conditions,
            discipline: crate::track::Discipline::Rally,
            night: false,
        },
        cars,
        kind: EventKind::TimeTrial,
        laps: 1,
    }
}

/// The built-in campaign. (The tracks/gameplay agents may tune seeds, lengths and names,
/// but must keep ids stable once shipped.)
pub fn campaign() -> Vec<Chapter> {
    use Biome::{Autumn, Coast, Desert, Meadow};
    use SurfaceMode::{BiomeDefault, Mixed, Tarmac};
    vec![
        Chapter {
            name: "Meadow Rally".into(),
            biome: Meadow,
            unlock_golds: 0,
            stages: vec![
                stage("c1s1-buttercup-lane", "Buttercup Lane", "A gentle gravel warm-up through the meadows.", 1106, Meadow, 3.0, 0, BiomeDefault, &[]),
                stage("c1s2-hazel-hollow", "Hazel Hollow", "Narrow lanes, blind crests and a passing shower.", 1102, Meadow, 3.8, 1, Mixed, &[]),
                stage("c1s3-windmill-run", "Windmill Run", "Fast and flowing — until the fog rolls in.", 1103, Meadow, 4.5, 1, BiomeDefault, &[]),
            ],
        },
        Chapter {
            name: "Maple Woods".into(),
            biome: Autumn,
            unlock_golds: 2,
            stages: vec![
                stage("c2s1-copper-leaf", "Copper Leaf", "Slippery dirt under a carpet of leaves.", 2101, Autumn, 3.5, 1, BiomeDefault, &[]),
                stage("c2s2-mushroom-dell", "Mushroom Dell", "Rain-soaked hairpins and muddy ruts.", 2102, Autumn, 4.5, 1, Mixed, &[]),
                stage_c("c2s3-owl-ridge", "Owl Ridge", "A long, technical climb into the evening storm.", 2103, Autumn, 6.0, 2, BiomeDefault, &[], ConditionsSpec::Storm),
            ],
        },
        Chapter {
            name: "Seaside Tour".into(),
            biome: Coast,
            unlock_golds: 5,
            stages: vec![
                stage("c3s1-lighthouse-dash", "Lighthouse Dash", "Coastal tarmac with sea spray and gusts.", 3101, Coast, 4.0, 1, Tarmac, &[]),
                stage("c3s2-cliffside", "Cliffside", "Tarmac and gravel mixed above the waves.", 3102, Coast, 5.0, 2, Mixed, &[]),
                stage_c("c3s3-harbour-storm", "Harbour Storm", "Everything the coast can throw at you.", 3103, Coast, 6.5, 2, Mixed, &[], ConditionsSpec::Storm),
            ],
        },
        Chapter {
            name: "Canyon Classic".into(),
            biome: Desert,
            unlock_golds: 8,
            stages: vec![
                stage("c4s1-dune-hopper", "Dune Hopper", "Jumps, sand drifts and heat haze.", 4101, Desert, 4.5, 1, BiomeDefault, &[]),
                stage("c4s2-mesa-switchbacks", "Mesa Switchbacks", "Hairpins up the mesa — then a dust storm.", 4102, Desert, 5.5, 2, Mixed, &[]),
                stage("c4s3-red-rock-marathon", "Red Rock Marathon", "The long one. Pace yourself.", 4103, Desert, 8.0, 2, BiomeDefault, &[]),
            ],
        },
        Chapter {
            name: "Frosty Peaks".into(),
            biome: Biome::Snow,
            unlock_golds: 11,
            stages: vec![
                stage("c5s1-snowglobe", "Snowglobe", "Packed snow and gentle snowfall.", 5101, Biome::Snow, 4.0, 2, BiomeDefault, &[]),
                stage("c5s2-icicle-pass", "Icicle Pass", "Ice patches on the shady side of the pass.", 5102, Biome::Snow, 5.5, 2, Mixed, &[]),
                stage_c("c5s3-summit-blizzard", "Summit Blizzard", "The final test: a blizzard at the top.", 5103, Biome::Snow, 7.5, 2, SurfaceMode::Snow, &[], ConditionsSpec::Storm),
            ],
        },
    ]
}

/// Chapters of a campaign
pub fn campaign_of(kind: CampaignKind) -> Vec<Chapter> {
    match kind {
        CampaignKind::Rally => campaign(),
        CampaignKind::Circuit => circuit_campaign(),
    }
}

/// Stage / event by id (either campaign)
pub fn find_stage(id: &str) -> Option<StageDef> {
    CampaignKind::ALL.iter().flat_map(|k| campaign_of(*k)).flat_map(|c| c.stages).find(|s| s.id == id)
}

/// Which campaign an event belongs to
pub fn campaign_kind_of(id: &str) -> Option<CampaignKind> {
    CampaignKind::ALL.into_iter().find(|k| campaign_of(*k).iter().any(|c| c.stages.iter().any(|s| s.id == id)))
}

// ------------------------------------------------------------------ circuit season

/// Weather that turns during the race: `from` -> `to`, fully there at `at` s (built over
/// `ramp` s), clearing again after `lasts` s (0 = stays)
fn turn(from: WeatherPreset, to: WeatherPreset, at: u16, lasts: u16, ramp: u16) -> ConditionsSpec {
    ConditionsSpec::Timeline(WeatherTurn { from, to, at, lasts, ramp })
}

/// One circuit event on a named circuit (`circuits::circuits()` id) with its own weather,
/// time of day, laps and format.
#[allow(clippy::too_many_arguments)]
fn gp(id: &str, name: &str, blurb: &str, circuit: &str, laps: u32, kind: EventKind, cars: &[u8], conditions: ConditionsSpec, night: bool) -> StageDef {
    let c = crate::circuits::find_circuit(circuit).unwrap_or_else(|| panic!("unknown circuit {circuit}"));
    let mut params = c.params;
    params.conditions = conditions;
    params.night = night;
    let cars = if cars.is_empty() { class_cars(CarClass::Circuit) } else { cars.to_vec() };
    StageDef { id: id.into(), name: name.into(), blurb: blurb.into(), params, cars, kind, laps }
}

fn race(rivals: u8, skill: f32) -> EventKind {
    EventKind::Race { rivals, skill }
}

/// The circuit campaign: a season of club races, hot laps and night races where the
/// weather is part of the challenge. Mostly sunshine, but every chapter has a surprise: a
/// cloudburst that floods the track mid-race, sea fog you can't see through, black ice
/// under the floodlights. Races start from the back of the grid.
pub fn circuit_campaign() -> Vec<Chapter> {
    use WeatherPreset::*;
    const COMET: u8 = 5;
    const ZEPHYR: u8 = 6;
    const DRAGONFLY: u8 = 7;
    let sunny = ConditionsSpec::Fixed(Clear);
    vec![
        Chapter {
            name: "Club Season".into(),
            biome: Biome::Meadow,
            unlock_golds: 0,
            stages: vec![
                gp("gp1e1-daisy-hot-laps", "Daisy Park Hot Laps", "Three flying laps in the Comet GT. Sunshine, warm tarmac, no excuses.", "circuit-meadow-sprint", 3, EventKind::TimeTrial, &[COMET], sunny, false),
                gp("gp1e2-daisy-club-race", "Daisy Park Club Race", "Your first race: five rivals, four laps, and you start from the back.", "circuit-meadow-sprint", 4, race(5, 0.55), &[], sunny, false),
                gp("gp1e3-meadowbrook-cloudburst", "Meadowbrook Cloudburst", "Clear skies forecast. The farmers are bringing the cows in anyway.", "circuit-meadow-gp", 3, race(5, 0.6), &[], turn(Clear, Downpour, 170, 120, 25), false),
            ],
        },
        Chapter {
            name: "Coastal Series".into(),
            biome: Biome::Coast,
            unlock_golds: 2,
            stages: vec![
                gp("gp2e1-harbour-lights", "Harbour Lights", "A night race round the harbour. Floodlights on the pit straight, darkness by the pier.", "circuit-coast-sprint", 5, race(5, 0.65), &[], sunny, true),
                gp("gp2e2-lighthouse-sea-fret", "Sea Fret", "Two hot laps in the Zephyr. A sea fog rolls in off the bay, and it gets thick.", "circuit-coast-gp", 2, EventKind::TimeTrial, &[ZEPHYR], turn(Clear, ThickFog, 75, 0, 40), false),
                gp("gp2e3-lighthouse-monsoon", "Monsoon Grand Prix", "It has rained all week and it isn't stopping. Standing water everywhere: slicks float.", "circuit-coast-gp", 2, race(5, 0.6), &[], ConditionsSpec::Fixed(Downpour), false),
            ],
        },
        Chapter {
            name: "Desert Heat".into(),
            biome: Biome::Desert,
            unlock_golds: 4,
            stages: vec![
                gp("gp3e1-mirage-formula", "Mirage Formula", "Three laps in the Dragonfly, the open-wheel rocket. Brake later than feels sane.", "circuit-desert-sprint", 3, EventKind::TimeTrial, &[DRAGONFLY], sunny, false),
                gp("gp3e2-canyon-international", "Canyon International", "Full field, full sun, a proper Grand Prix at the foot of the mesas.", "circuit-desert-gp", 3, race(5, 0.72), &[], sunny, false),
                gp("gp3e3-canyon-haboob", "Haboob", "A night race. Halfway through, a wall of dust swallows the circuit.", "circuit-desert-gp", 3, race(5, 0.68), &[], turn(Clear, ThickFog, 150, 100, 20), true),
            ],
        },
        Chapter {
            name: "Mountain Endurance".into(),
            biome: Biome::Forest,
            unlock_golds: 7,
            stages: vec![
                gp("gp4e1-sudety-grand-prix", "Sudety Grand Prix", "Fast, wooded and unforgiving. The rivals are good now.", "circuit-forest-gp", 3, race(5, 0.8), &[], sunny, false),
                gp("gp4e2-black-forest-storm", "Black Forest Storm", "One lap of the ring in the Zephyr, ten kilometres. The storm hits somewhere in the middle.", "circuit-forest-ring", 1, EventKind::TimeTrial, &[ZEPHYR], turn(Clear, Downpour, 110, 140, 30), false),
                gp("gp4e3-sudety-black-ice", "Black Ice", "A freezing clear night. The road glitters under the lamps: there is almost no grip.", "circuit-forest-gp", 3, race(5, 0.72), &[], ConditionsSpec::Fixed(BlackIce), true),
            ],
        },
        Chapter {
            name: "Ring Masters".into(),
            biome: Biome::Autumn,
            unlock_golds: 10,
            stages: vec![
                gp("gp5e1-maplering-10k", "Maplering 10K", "Ten kilometres of crests and blind corners, wheel to wheel with the best.", "circuit-autumn-ring", 1, race(5, 0.85), &[], sunny, false),
                gp("gp5e2-clifftop-night-storm", "Clifftop Night Storm", "Night on the clifftop ring. The storm is coming in off the sea, and it won't pass.", "circuit-coast-ring", 1, race(5, 0.8), &[], turn(Clear, Downpour, 80, 0, 35), true),
                gp("gp5e3-the-green-hell", "The Green Hell", "Twenty kilometres of Nordring against the fastest field of the season. Fog in the valleys, then sun again. Win this.", "circuit-meadow-nordring", 1, race(5, 0.9), &[], turn(Clear, ThickFog, 260, 160, 40), false),
            ],
        },
    ]
}

// ------------------------------------------------------------------ leaderboards

/// One submitted time. Designed so a future global leaderboard server can verify runs:
/// the ghost replay (inputs/positions) can be re-simulated or sanity-checked server side.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaderboardEntry {
    pub stage_id: String,
    pub campaign_version: u32,
    pub game_version: String,
    pub player: String,
    pub car: u8,
    /// Assist level name (Cozy/Normal/Sport/Sim) — boards can be filtered by it
    pub assists: String,
    pub time_ms: u32,
    pub splits_ms: Vec<u32>,
    /// Unix seconds
    pub timestamp: u64,
    /// Hash of the ghost/replay blob for verification
    pub replay_hash: u64,
}

/// Backend abstraction: a local file store now, a global HTTP service later.
pub trait LeaderboardBackend {
    fn submit(&mut self, entry: LeaderboardEntry) -> Result<(), String>;
    /// Best entries for a stage, fastest first
    fn top(&self, stage_id: &str, n: usize) -> Vec<LeaderboardEntry>;
    fn name(&self) -> &'static str;
}
