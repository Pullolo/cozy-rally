use rally_core::ai::AiDriver;
use rally_core::track::{Track, TrackParams};
use rally_core::world::{Controller, RacerSetup, World};
use std::sync::Arc;
fn main() {
    let seed: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let secs: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(12.0);
    let car: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let track = Arc::new(Track::generate(&TrackParams { seed, biome: None, length_class: 1, ..Default::default() }));
    let racers: Vec<RacerSetup> = (0..4).map(|i| RacerSetup { car_spec: i, controller: Controller::Ai }).collect();
    let mut world = World::new(track.clone(), &racers, 2);
    let mut ai = AiDriver::new(0.9, 0.0);
    let s0 = track.samples[0];
    println!("start sample pos {:?} hw {}  terrain h there {:.2}", s0.pos, s0.half_width, track.terrain.height(s0.pos.x, s0.pos.z));
    for t in 0..(secs * 120.0) as i32 {
        for i in 0..4 { let inp = ai.drive(&world.cars[i], &world.track, 1.0/120.0); world.set_input(i, inp); }
        world.tick();
        if t % 30 == 0 {
            let c = &world.cars[car];
            let r = &world.race.racers[car];
            let w: Vec<String> = c.wheels.iter().map(|w| format!("{}{:.2}", if w.contact {"c"} else {"-"}, w.compression)).collect();
            println!("t{:5.2} pos ({:7.1},{:6.1},{:7.1}) ground {:6.1} v {:5.1} fwd {:5.1} up.y {:.2} gear {} rpm {:4.0} s {:6.1} lat {:5.1} cp {} rst {:.1} in {:?} wheels {:?} phase {:?}",
              t as f32/120.0, c.pos.x, c.pos.y, c.pos.z, track.terrain.height(c.pos.x,c.pos.z), c.vel.length(), c.telemetry.forward_speed, c.up().y, c.gear, c.rpm, r.s, r.lateral, r.next_cp, r.reset_timer, (c.input.throttle, c.input.brake, (c.input.steer*100.0).round()/100.0), w, world.race.phase);
        }
    }
}
