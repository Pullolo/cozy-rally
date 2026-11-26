//! Named racing circuits (Circuit mode). Ids are STABLE (lap records key on them).
use crate::biome::Biome;
use crate::surfacemap::SurfaceMode;
use crate::track::{Discipline, LayoutKind, TrackParams};
use crate::weather::ConditionsSpec;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CircuitDef {
    /// Stable unique id, e.g. "circuit-meadow-gp"
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub params: TrackParams,
}

/// Params for a circuit: `size` 0 = Sprint (~2.5 km), 1 = Grand Prix (~4.5 km), 2 = Ring (~10 km),
/// 3 = Nordring (~20 km of narrow, hilly forest road)
pub fn circuit_params(seed: u64, biome: Option<Biome>, size: u8, difficulty: u8) -> TrackParams {
    TrackParams {
        seed,
        biome,
        length_class: size.min(3),
        difficulty,
        surface: SurfaceMode::Tarmac,
        layout: LayoutKind::Loop,
        conditions: ConditionsSpec::BiomeDefault,
        discipline: Discipline::Circuit,
        night: false,
    }
}

fn def(id: &str, name: &str, blurb: &str, seed: u64, biome: Biome, size: u8, difficulty: u8) -> CircuitDef {
    CircuitDef { id: id.into(), name: name.into(), blurb: blurb.into(), params: circuit_params(seed, Some(biome), size, difficulty) }
}

pub fn circuits() -> Vec<CircuitDef> {
    use Biome::{Autumn, Coast, Desert, Forest, Meadow};
    vec![
        def("circuit-meadow-sprint", "Daisy Park", "A short, punchy club circuit between the flower fields.", 7101, Meadow, 0, 1),
        def("circuit-meadow-gp", "Meadowbrook Grand Prix", "Long straights, a tight hairpin and fast esses.", 7102, Meadow, 1, 1),
        def("circuit-coast-sprint", "Harbour Loop", "Seaside sprint with a chicane by the pier.", 7201, Coast, 0, 1),
        def("circuit-coast-gp", "Lighthouse Grand Prix", "Sweeping coastal GP track — heavy braking into the bay.", 7202, Coast, 1, 2),
        def("circuit-desert-sprint", "Mirage Speedway", "Flat out across the dunes, then the heat haze hairpin.", 7301, Desert, 0, 1),
        def("circuit-desert-gp", "Canyon International", "A modern GP circuit at the foot of the mesas.", 7302, Desert, 1, 2),
        def("circuit-autumn-ring", "Maplering", "The green-gold hell: ten kilometres of crests, compressions and blind corners through the maple forest.", 7401, Autumn, 2, 2),
        def("circuit-meadow-ring", "Hillside Ring", "A rolling old-school ring road over the meadow hills.", 7402, Meadow, 2, 1),
        def("circuit-coast-ring", "Clifftop Ring", "Endless clifftop sweepers and a banked carousel.", 7403, Coast, 2, 2),
        def("circuit-meadow-nordring", "Wolfsthal Nordring", "Twenty kilometres of the green hell: narrow, hilly, blind brows that throw you in the air, two banked carousels and barriers right at the edge. Learn it, or it learns you.", 7501, Meadow, 3, 2),
        def("circuit-forest-gp", "Sudety Grand Prix", "A mountain GP circuit cut through tall beech and spruce woods, with a dive into the valley.", 7601, Forest, 1, 2),
        def("circuit-forest-ring", "Black Forest Ring", "Ten kilometres of forest ring: blind crests under the canopy, cambered sweepers and a long valley straight.", 7602, Forest, 2, 2),
    ]
}

pub fn find_circuit(id: &str) -> Option<CircuitDef> {
    circuits().into_iter().find(|c| c.id == id)
}
