//! Debug one stage's reference run: stage_debug <stage-id> [car]
use rally_core::ai::{AiDriver, Personality, RubberBand};
use rally_core::campaign::find_stage;
use rally_core::track::Track;
use rally_core::world::{Controller, RacerSetup, World, TICK};
use std::sync::Arc;
fn main() {
    let id = std::env::args().nth(1).unwrap_or("c5s2-icicle-pass".into());
    let car: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(2);
    let st = find_stage(&id).expect("stage");
    let track = Arc::new(Track::generate(&st.params));
    println!("len {:.0} checkpoints {:?}", track.length, track.checkpoints);
    if let Ok(range) = std::env::var("PLAN") {
        let (a, b) = range.split_once('-').map(|(a, b)| (a.parse::<f32>().unwrap(), b.parse::<f32>().unwrap())).unwrap();
        let line = rally_core::racing_line::RacingLine::shared(&track);
        let spec = &rally_core::vehicle::car_specs()[car];
        let prof = rally_core::racing_line::SpeedProfile::build(&line, &rally_core::racing_line::SpeedParams::for_car(spec, 0.93 * spec.ai_grip, 1.05));
        let mut s = a;
        while s < b {
            let i = (s / 2.0) as usize % line.offset.len();
            let smp = &track.samples[i];
            println!("s{:5.0} R{:6.0} off{:5.1} hw{:4.1} grip{:4.2} surf {:?} limit{:5.1} bank{:5.2} vc{:6.3}", s, 1.0 / line.curvature[i].abs().max(1e-4), line.offset[i], smp.half_width, line.grip[i], smp.surface, prof.limit[i], line.bank_into[i], line.vcurv[i]);
            s += 6.0;
        }
        return;
    }
    let mut world = World::new(track, &[RacerSetup { car_spec: car, controller: Controller::Local(0) }], 1);
    let mut ai = AiDriver::with_personality(Personality { skill: 1.0, aggression: 1.0, consistency: 1.0 }, RubberBand::default(), 7);
    for t in 0..(600.0 / TICK) as i32 {
        let inp = ai.drive(&world.cars[0], &world.track, TICK);
        world.set_input(0, inp);
        world.tick();
        for e in world.drain_events() {
            if let rally_core::world::WorldEvent::Race(ev) = e { println!("  t{:6.1} {:?}", world.race.clock, ev); }
        }
        if std::env::var("WATCH").is_ok() && world.race.clock > 17.0 && world.race.clock < 27.0 && t % 30 == 0 {
            let r = &world.race.racers[0];
            let c = &world.cars[0];
            let d = ai.debug;
            let g = c.wheels.iter().map(|w| format!("{:?}", w.surface.map(|s| s.name()).unwrap_or("-"))).collect::<Vec<_>>().join(",");
            println!("   t{:5.2} s{:5.0} lat{:5.1} v{:4.1} vt{:4.1} beta{:5.1} thr{:.2} br{:.2} st{:5.2} steerang{:5.2} tl{:.2} {}", world.race.clock, r.s, r.lateral, c.vel.length(), d.target_speed, c.telemetry.drift_angle.to_degrees(), c.input.throttle, c.input.brake, c.input.steer, c.steer, c.wheels.iter().map(|w| w.traction_loss).fold(0.0f32, f32::max), g);
        }
        if t % (120 * 10) == 0 {
            let r = &world.race.racers[0];
            let c = &world.cars[0];
            println!("t{:6.1} s{:6.0} lat{:5.1} v{:4.1} lap{} cp{} off{} ai rec {}", world.race.clock, r.s, r.lateral, c.vel.length(), r.lap, r.next_cp, r.off_track, ai.stats.recoveries);
        }
        if world.race.racers[0].finished { println!("finished {:?}", world.race.racers[0].finish_time); break; }
    }
}
