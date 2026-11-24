//! Remote cars: simulated locally with the owner's inputs (so wheels, suspension, dust and
//! engine sound all behave naturally) and pulled softly toward the owner's latest state,
//! extrapolated forward by the estimated latency. Big errors (respawns, lag spikes) snap.
use crate::protocol::CarSnapshot;
use rally_core::input::CarInput;
use rally_core::vehicle::Car;
use glam::{Quat, Vec3};

/// Beyond this error the car is teleported instead of blended (metres)
pub const SNAP_DIST: f32 = 8.0;
/// Correction stiffness (1/s): ~95% of an error is gone after 3/RATE seconds
pub const RATE: f32 = 7.0;
/// Never extrapolate further than this (seconds)
pub const MAX_EXTRAPOLATE: f32 = 0.5;

pub fn snapshot_of(car: &Car, t: f32) -> CarSnapshot {
    CarSnapshot {
        t,
        pos: car.pos.to_array(),
        rot: car.rot.to_array(),
        vel: car.vel.to_array(),
        ang_vel: car.ang_vel.to_array(),
        throttle: car.input.throttle,
        brake: car.input.brake,
        steer: car.input.steer,
        handbrake: car.input.handbrake,
    }
}

#[derive(Clone, Debug, Default)]
pub struct RemoteCar {
    pub last: Option<CarSnapshot>,
    /// Local time the last snapshot arrived
    pub recv_at: f32,
    /// Newest sender time seen (drops stale/out-of-order snapshots)
    pub newest_t: f32,
    /// Snapshots received (for the HUD "lagging" indicator)
    pub count: u32,
    pub fresh: bool,
}

impl RemoteCar {
    pub fn receive(&mut self, snap: CarSnapshot, now: f32) {
        if self.last.is_some() && snap.t < self.newest_t {
            return;
        }
        self.newest_t = snap.t;
        self.last = Some(snap);
        self.recv_at = now;
        self.count += 1;
        self.fresh = true;
    }

    /// Seconds since the last snapshot arrived
    pub fn silence(&self, now: f32) -> f32 {
        if self.last.is_none() { f32::INFINITY } else { now - self.recv_at }
    }

    /// Inputs the remote driver is holding
    pub fn input(&self) -> Option<CarInput> {
        self.last.map(|s| CarInput { throttle: s.throttle, brake: s.brake, steer: s.steer, handbrake: s.handbrake, reset: false })
    }

    /// Pull `car` toward the predicted remote state. `latency` = one-way delay estimate (s).
    pub fn correct(&mut self, car: &mut Car, now: f32, latency: f32, dt: f32) {
        let Some(s) = self.last else { return };
        let ahead = ((now - self.recv_at) + latency).clamp(0.0, MAX_EXTRAPOLATE);
        let vel = Vec3::from(s.vel);
        let ang = Vec3::from(s.ang_vel);
        let target_pos = Vec3::from(s.pos) + vel * ahead;
        let rot0 = Quat::from_array(s.rot).normalize();
        let target_rot = if ang.length_squared() > 1e-8 { (Quat::from_scaled_axis(ang * ahead) * rot0).normalize() } else { rot0 };
        let err = target_pos - car.pos;
        if self.fresh && (err.length() > SNAP_DIST || !car.pos.is_finite()) {
            car.pos = target_pos;
            car.rot = target_rot;
            car.vel = vel;
            car.ang_vel = ang;
            car.prev_pos = car.pos;
            car.prev_rot = car.rot;
        } else {
            let k = 1.0 - (-dt * RATE).exp();
            car.pos += err * k;
            car.vel += (vel - car.vel) * k;
            car.ang_vel += (ang - car.ang_vel) * k;
            car.rot = car.rot.slerp(target_rot, k).normalize();
        }
        self.fresh = false;
    }
}
