//! Reference runs + medal thresholds for every campaign stage.
//! cargo run --release -p rally_core --example campaign_ref [-- all-cars]
use rally_core::campaign::campaign;
use rally_core::campaign_run::{medal_times, reference_run_on, reference_car};
use rally_core::race::format_time;
use rally_core::track::Track;
use rally_core::vehicle::car_specs;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let all_cars = std::env::args().nth(1).as_deref() == Some("all-cars");
    println!("{:24} {:>6} {:>4} | {:>9} {:>9} {:>9} {:>9} {:>9} | resets splits  gen+run", "stage", "km", "car", "reference", "platinum", "gold", "silver", "bronze");
    for ch in campaign() {
        println!("-- {} (unlock at {} golds)", ch.name, ch.unlock_golds);
        for st in ch.stages {
            let t0 = Instant::now();
            let track = Arc::new(Track::generate(&st.params));
            let car = reference_car(&st);
            match reference_run_on(track.clone(), car) {
                Some(r) => {
                    let m = medal_times(r.time);
                    let mut extra = String::new();
                    if all_cars {
                        for c in 0..car_specs().len() as u8 {
                            if let Some(rc) = reference_run_on(track.clone(), c) {
                                extra += &format!(" {}:{:+.1}%", car_specs()[c as usize].name, (rc.time / r.time - 1.0) * 100.0);
                            }
                        }
                    }
                    println!(
                        "{:24} {:6.2} {:>4} | {:>9} {:>9} {:>9} {:>9} {:>9} | {:6} {:6} {:5}ms{extra}",
                        st.id,
                        track.length / 1000.0,
                        car_specs()[car as usize].name,
                        format_time(r.time),
                        format_time(m.platinum),
                        format_time(m.gold),
                        format_time(m.silver),
                        format_time(m.bronze),
                        r.resets,
                        r.splits.len(),
                        t0.elapsed().as_millis()
                    );
                }
                None => println!("{:24} FAILED (no finish)", st.id),
            }
        }
    }
}
