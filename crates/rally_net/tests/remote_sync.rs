//! The remote-car model: world A drives car 0 with an AI; world B only sees car 0 through
//! 30 Hz snapshots delayed by a simulated network (one-way latency + jitter). The remote copy
//! must stay close to the real car and must never blow up.
use rally_core::ai::AiDriver;
use rally_core::track::{Track, TrackParams};
use rally_core::world::{Controller, RacerSetup, World, TICK};
use rally_net::sync::{snapshot_of, RemoteCar};
use std::sync::Arc;

fn run(seed: u64, one_way: f32, jitter: f32) -> (f32, f32, f32) {
    let track = Arc::new(Track::generate(&TrackParams { seed, ..Default::default() }));
    let mut a = World::new(track.clone(), &[RacerSetup { car_spec: 2, controller: Controller::Ai }], 1);
    let mut b = World::new(track.clone(), &[RacerSetup { car_spec: 2, controller: Controller::Remote }], 1);
    let mut ai = AiDriver::new(0.8, 0.0);
    let mut remote = RemoteCar::default();
    let mut in_flight: Vec<(f32, rally_net::CarSnapshot)> = Vec::new();
    let frame = 1.0 / 60.0;
    let (mut t, mut send_acc) = (0.0f32, 0.0f32);
    let mut rng = 12345u32;
    let (mut sum, mut n, mut worst) = (0.0f32, 0, 0.0f32);
    for step in 0..(60 * 60) {
        t += frame;
        let inp = ai.drive(&a.cars[0], &a.track, frame);
        a.set_input(0, inp);
        a.update(frame);
        send_acc += frame;
        if send_acc >= 1.0 / 30.0 {
            send_acc -= 1.0 / 30.0;
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let j = (rng >> 8) as f32 / (1u32 << 24) as f32 * jitter;
            in_flight.push((t + one_way + j, snapshot_of(&a.cars[0], t)));
        }
        // TCP keeps order: deliver in send order once due
        while in_flight.first().is_some_and(|(due, _)| *due <= t) {
            let (_, s) = in_flight.remove(0);
            remote.receive(s, t);
        }
        if let Some(i) = remote.input() {
            b.set_input(0, i);
        }
        remote.correct(&mut b.cars[0], t, one_way + jitter * 0.5 + 0.5 / 30.0, frame);
        b.update(frame);
        assert!(b.cars[0].pos.is_finite(), "remote car blew up");
        if step > 300 {
            let e = (a.cars[0].pos - b.cars[0].pos).length();
            sum += e;
            n += 1;
            worst = worst.max(e);
        }
    }
    let _ = TICK;
    let speed = a.cars[0].telemetry.speed;
    (sum / n as f32, worst, speed)
}

#[test]
fn remote_car_tracks_the_real_one() {
    for (seed, lat, jit) in [(3, 0.02, 0.01), (7, 0.06, 0.03), (11, 0.12, 0.05)] {
        let (mean, worst, _) = run(seed, lat, jit);
        println!("seed {seed} latency {:.0} ms jitter {:.0} ms: mean error {mean:.2} m, worst {worst:.2} m", lat * 1000.0, jit * 1000.0);
        assert!(mean < 1.5, "mean error {mean}");
        assert!(worst < rally_net::sync::SNAP_DIST * 1.5, "worst error {worst}");
    }
}
