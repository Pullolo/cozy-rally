//! AI driver: pure-pursuit steering toward a point ahead on a simple racing line,
//! plus curvature-based speed planning with braking distances.
use crate::input::CarInput;
use crate::track::{Track, SAMPLE_SPACING};
use crate::vehicle::{Car, GRAVITY};

#[derive(Clone, Debug)]
pub struct AiDriver {
    /// 0..1, scales cornering speed and reaction
    pub skill: f32,
    /// Preferred lateral offset (m) to spread cars out
    pub lane_bias: f32,
    stuck_timer: f32,
    reverse_timer: f32,
    phase: f32,
}

impl AiDriver {
    pub fn new(skill: f32, lane_bias: f32) -> Self {
        AiDriver { skill, lane_bias, stuck_timer: 0.0, reverse_timer: 0.0, phase: lane_bias * 3.1 }
    }

    pub fn drive(&mut self, car: &Car, track: &Track, dt: f32) -> CarInput {
        let q = track.query(car.pos, car.track_hint);
        let speed = car.vel.length();
        let fwd_speed = car.vel.dot(car.forward());
        self.phase += dt;

        // racing line: offset toward the inside of upcoming curvature
        let lookahead = 7.0 + speed * 0.55;
        let s_target = q.s + lookahead;
        let n = track.samples.len();
        let idx = ((s_target / SAMPLE_SPACING) as usize) % n;
        let mut curv_ahead = 0.0;
        for k in 0..12 {
            curv_ahead += track.samples[(idx + k * 2) % n].curvature;
        }
        curv_ahead /= 12.0;
        let hw = track.samples[idx].half_width;
        // positive curvature = left turn -> inside is left (negative lateral)
        let line = (-curv_ahead * 260.0).clamp(-hw * 0.55, hw * 0.55) + self.lane_bias * (1.0 - (curv_ahead.abs() * 60.0).min(1.0));
        let (p, _, r, _) = track.frame_at(s_target);
        let target = p + r * line;
        let to = target - car.pos;
        let local_x = to.dot(car.right());
        let local_z = to.dot(car.forward());
        let angle = local_x.atan2(local_z.max(0.1));
        let mut steer = (angle * 2.2).clamp(-1.0, 1.0);

        // speed planning
        let mu = car.spec.tire_grip * track.style.road_surface.props().grip * 0.92;
        let decel = mu * GRAVITY * 0.75;
        let horizon = (speed * speed / (2.0 * decel) + 25.0).min(220.0);
        let base_idx = q.index;
        let mut target_speed: f32 = 60.0;
        let steps = (horizon / SAMPLE_SPACING) as usize;
        for k in (0..steps).step_by(2) {
            let smp = &track.samples[(base_idx + k) % n];
            let c = smp.curvature.abs().max(1e-4);
            let v_corner = (mu * GRAVITY / c).sqrt() * (0.72 + 0.28 * self.skill);
            let d = k as f32 * SAMPLE_SPACING;
            let v_allowed = (v_corner * v_corner + 2.0 * decel * d).sqrt();
            target_speed = target_speed.min(v_allowed);
        }
        // be careful approaching jumps
        for &j in &track.jumps {
            let d = (track.samples[j].s - q.s).rem_euclid(track.length);
            if d < 60.0 {
                target_speed = target_speed.min(24.0 + 6.0 * self.skill);
            }
        }
        target_speed *= 0.9 + 0.1 * self.skill;

        let mut throttle = 0.0;
        let mut brake = 0.0;
        let err = target_speed - speed;
        if err > 0.0 {
            throttle = (err * 0.5).clamp(0.25, 1.0);
        } else if err < -1.5 {
            brake = (-err * 0.18).clamp(0.0, 1.0);
        } else {
            throttle = 0.15;
        }
        // traction control-ish: ease off when sliding
        if car.telemetry.drift_angle.abs() > 0.35 {
            throttle *= 0.6;
        }

        // stuck recovery: reverse out
        if self.reverse_timer > 0.0 {
            self.reverse_timer -= dt;
            return CarInput { throttle: 0.0, brake: 1.0, steer: -steer, handbrake: 0.0, reset: false };
        }
        if fwd_speed < 1.0 && throttle > 0.2 && !car.frozen {
            self.stuck_timer += dt;
        } else {
            self.stuck_timer = 0.0;
        }
        if self.stuck_timer > 2.0 {
            self.stuck_timer = 0.0;
            self.reverse_timer = 1.3;
        }
        if car.gear < 0 && self.reverse_timer <= 0.0 {
            throttle = 1.0;
            brake = 0.0;
        }
        steer = steer.clamp(-1.0, 1.0);
        CarInput { throttle, brake, steer, handbrake: 0.0, reset: false }
    }
}
