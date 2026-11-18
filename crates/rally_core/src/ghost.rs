//! Time-trial ghosts: record a lap (pose at ~30 Hz + lap distance), replay it, compare
//! against it (live delta), and (de)serialise it to a tiny binary blob.
use crate::world::World;
use glam::{Quat, Vec3};

pub const GHOST_HZ: f32 = 30.0;
const MAGIC: &[u8; 4] = b"CRG1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GhostFrame {
    /// Seconds since the lap started
    pub t: f32,
    /// Distance along the track since the start line (m); slightly negative on a standing start
    pub dist: f32,
    pub pos: Vec3,
    pub rot: Quat,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GhostLap {
    pub time: f32,
    /// CarSpec id of the car that set it
    pub car_spec: u8,
    pub frames: Vec<GhostFrame>,
}

impl GhostLap {
    /// Interpolated pose at `t` seconds into the lap (None outside the recording)
    pub fn pose_at(&self, t: f32) -> Option<(Vec3, Quat)> {
        let f = &self.frames;
        if f.len() < 2 || t < f[0].t || t > f[f.len() - 1].t {
            return None;
        }
        let i = f.partition_point(|x| x.t <= t).clamp(1, f.len() - 1);
        let (a, b) = (&f[i - 1], &f[i]);
        let k = ((t - a.t) / (b.t - a.t).max(1e-4)).clamp(0.0, 1.0);
        Some((a.pos.lerp(b.pos, k), a.rot.slerp(b.rot, k)))
    }

    /// Time at which the ghost had covered `dist` metres of the lap (None outside range)
    pub fn time_at_dist(&self, dist: f32) -> Option<f32> {
        let f = &self.frames;
        if f.len() < 2 || dist < f[0].dist || dist > f[f.len() - 1].dist {
            return None;
        }
        // distance is monotonic except for tiny wobbles; first frame reaching dist
        let i = f.partition_point(|x| x.dist < dist).clamp(1, f.len() - 1);
        let (a, b) = (&f[i - 1], &f[i]);
        let k = ((dist - a.dist) / (b.dist - a.dist).max(1e-4)).clamp(0.0, 1.0);
        Some(a.t + (b.t - a.t) * k)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.frames.len() * 36);
        out.extend_from_slice(MAGIC);
        out.push(self.car_spec);
        out.extend_from_slice(&self.time.to_le_bytes());
        out.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        for f in &self.frames {
            for v in [f.t, f.dist, f.pos.x, f.pos.y, f.pos.z, f.rot.x, f.rot.y, f.rot.z, f.rot.w] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }

    /// Parse a blob written by `to_bytes`; None if it's damaged or from another version.
    pub fn from_bytes(b: &[u8]) -> Option<GhostLap> {
        if b.len() < 13 || &b[0..4] != MAGIC {
            return None;
        }
        let car_spec = b[4];
        let f32_at = |o: usize| -> Option<f32> { b.get(o..o + 4).map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]])) };
        let time = f32_at(5)?;
        let n = u32::from_le_bytes(b.get(9..13)?.try_into().ok()?) as usize;
        if n > 200_000 || b.len() < 13 + n * 36 || !time.is_finite() || time <= 0.0 {
            return None;
        }
        let mut frames = Vec::with_capacity(n);
        for k in 0..n {
            let o = 13 + k * 36;
            let v: Vec<f32> = (0..9).map(|j| f32_at(o + j * 4).unwrap_or(f32::NAN)).collect();
            if v.iter().any(|x| !x.is_finite()) {
                return None;
            }
            frames.push(GhostFrame { t: v[0], dist: v[1], pos: Vec3::new(v[2], v[3], v[4]), rot: Quat::from_xyzw(v[5], v[6], v[7], v[8]).normalize() });
        }
        Some(GhostLap { time, car_spec, frames })
    }
}

/// Lap distance of car `i` (m from the start line, negative before the first crossing)
pub fn lap_distance(world: &World, i: usize) -> f32 {
    let r = &world.race.racers[i];
    r.total - (r.lap as f32 - 1.0) * world.track.length
}

struct Recorder {
    car: usize,
    laps_done: usize,
    frames: Vec<GhostFrame>,
    next_t: f32,
}

/// Live time-trial state for the local players: records every lap, keeps the best one as
/// the ghost, and computes the running delta to it.
pub struct TimeTrial {
    /// Ghost being raced (best lap so far, possibly loaded from disk)
    pub best: Option<GhostLap>,
    /// Set when a lap beats the ghost; consumers (saving, UI) clear it
    pub new_best: bool,
    /// Per tracked car: current delta to the ghost (s, negative = ahead)
    pub deltas: Vec<Option<f32>>,
    /// Per tracked car: (last lap time, delta vs the ghost it was racing)
    pub last_lap: Vec<Option<(f32, Option<f32>)>>,
    recorders: Vec<Recorder>,
}

impl TimeTrial {
    pub fn new(cars: &[usize], best: Option<GhostLap>) -> TimeTrial {
        TimeTrial {
            best,
            new_best: false,
            deltas: vec![None; cars.len()],
            last_lap: vec![None; cars.len()],
            recorders: cars.iter().map(|&car| Recorder { car, laps_done: 0, frames: Vec::new(), next_t: 0.0 }).collect(),
        }
    }

    /// Seconds into the current lap for tracked player `k`
    pub fn lap_clock(&self, world: &World, k: usize) -> f32 {
        let r = &world.race.racers[self.recorders[k].car];
        world.race.clock - r.lap_start
    }

    /// Ghost pose to show to tracked player `k` (synchronised with that player's lap)
    pub fn ghost_pose(&self, world: &World, k: usize) -> Option<(Vec3, Quat)> {
        if !world.race.is_racing() || k >= self.recorders.len() {
            return None;
        }
        let r = &world.race.racers[self.recorders[k].car];
        if r.finished {
            return None;
        }
        self.best.as_ref()?.pose_at(self.lap_clock(world, k))
    }

    /// Call once per frame after the world update.
    pub fn update(&mut self, world: &World) {
        // keep running in the Finished phase so the final lap is still captured
        if matches!(world.race.phase, crate::race::RacePhase::Countdown(_)) {
            return;
        }
        for k in 0..self.recorders.len() {
            let car = self.recorders[k].car;
            let r = &world.race.racers[car];
            // lap completed since last frame?
            if r.lap_times.len() > self.recorders[k].laps_done {
                self.recorders[k].laps_done = r.lap_times.len();
                let time = *r.lap_times.last().unwrap();
                let mut frames = std::mem::take(&mut self.recorders[k].frames);
                if let Some(last) = frames.last().copied() {
                    // close the recording exactly on the line
                    frames.push(GhostFrame { t: time, dist: world.track.length, ..last });
                }
                let vs = self.best.as_ref().map(|b| time - b.time);
                self.last_lap[k] = Some((time, vs));
                let better = self.best.as_ref().is_none_or(|b| time < b.time);
                if better && frames.len() > 10 {
                    self.best = Some(GhostLap { time, car_spec: world.cars[car].spec.id, frames });
                    self.new_best = true;
                }
                self.recorders[k].next_t = 0.0;
            }
            if r.finished {
                self.deltas[k] = None;
                continue;
            }
            let t = world.race.clock - r.lap_start;
            let dist = lap_distance(world, car);
            let rec = &mut self.recorders[k];
            if t >= rec.next_t {
                let c = &world.cars[car];
                rec.frames.push(GhostFrame { t, dist, pos: c.pos, rot: c.rot });
                rec.next_t = t + 1.0 / GHOST_HZ;
            }
            self.deltas[k] = self.best.as_ref().and_then(|b| b.time_at_dist(dist)).map(|gt| t - gt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_sample() {
        let frames: Vec<GhostFrame> = (0..60).map(|i| GhostFrame { t: i as f32 / 30.0, dist: i as f32, pos: Vec3::new(i as f32, 0.0, 0.0), rot: Quat::IDENTITY }).collect();
        let g = GhostLap { time: 2.0, car_spec: 2, frames };
        let b = g.to_bytes();
        let g2 = GhostLap::from_bytes(&b).unwrap();
        assert_eq!(g, g2);
        let (p, _) = g.pose_at(0.5).unwrap();
        assert!((p.x - 15.0).abs() < 1e-3);
        assert!((g.time_at_dist(30.0).unwrap() - 1.0).abs() < 1e-3);
        assert!(GhostLap::from_bytes(&b[..20]).is_none());
        assert!(GhostLap::from_bytes(b"nope").is_none());
    }
}
