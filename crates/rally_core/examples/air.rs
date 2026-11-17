use rally_core::track::{Track, TrackParams};
use rally_core::world::{Controller, RacerSetup, World};
use std::sync::Arc;
fn main() {
    let track = Arc::new(Track::generate(&TrackParams { seed: 1, biome: None, length_class: 1 }));
    let racers: Vec<RacerSetup> = (0..4).map(|i| RacerSetup { car_spec: i, controller: Controller::Ai }).collect();
    let mut world = World::new(track.clone(), &racers, 2);
    for _ in 0..300 {
        world.tick();
    }
    for c in &world.cars {
        println!(
            "{} air {:.2} ground {} y {:.2} terrain {:.2} wheels {:?}",
            c.spec.name,
            c.telemetry.airtime,
            c.telemetry.wheels_on_ground,
            c.pos.y,
            track.terrain.height(c.pos.x, c.pos.z),
            c.wheels.iter().map(|w| (w.contact, (w.compression * 100.0).round())).collect::<Vec<_>>()
        );
    }
}
