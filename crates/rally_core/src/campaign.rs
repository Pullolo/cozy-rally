//! Time-trial campaign definitions + leaderboard data model (shared interface).
//!
//! Campaign stages are point-to-point (A→B), several km long, with dynamic conditions.
//! Stage IDs are STABLE strings (never reuse/renumber): leaderboards key on
//! `(stage_id, CAMPAIGN_VERSION)`. Changing generation code that alters a stage's geometry
//! requires bumping `CAMPAIGN_VERSION` so old times are not compared against new roads.
use crate::biome::Biome;
use crate::surfacemap::SurfaceMode;
use crate::track::{LayoutKind, TrackParams};
use crate::weather::ConditionsSpec;
use serde::{Deserialize, Serialize};

pub const CAMPAIGN_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageDef {
    /// Stable unique id, e.g. "c1s2-hazel-hollow"
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub params: TrackParams,
    /// Allowed car ids (empty = any car)
    pub cars: Vec<u8>,
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
        },
        cars: cars.to_vec(),
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

pub fn find_stage(id: &str) -> Option<StageDef> {
    campaign().into_iter().flat_map(|c| c.stages).find(|s| s.id == id)
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
