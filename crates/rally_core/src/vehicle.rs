//! Semi-realistic raycast vehicle.
//!
//! Model summary:
//! * 6-DOF rigid body (semi-implicit Euler, world-space inertia tensor).
//! * Per-wheel raycast suspension: spring + bump/rebound damping + bump stop + anti-roll bars,
//!   so weight transfer (pitch under braking, roll in corners) emerges naturally.
//! * Tyres: slip-angle based lateral curve with post-peak falloff, longitudinal force from the
//!   drivetrain/brakes, combined via a friction ellipse (wheelspin steals lateral grip ->
//!   power oversteer; handbrake locks the rear -> rotation).
//! * Engine torque curve, auto gearbox with shift cut, engine braking, reverse logic.
//! * Assists (Forza-Horizon-ish feel): speed-sensitive steering, counter-steer assist,
//!   yaw stability, air control that lands you wheels-down.
//! Car local frame: +X right, +Y up, -Z forward.
use crate::input::CarInput;
use crate::surface::Surface;
use crate::track::Track;
use glam::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

pub const GRAVITY: f32 = 9.81;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Drivetrain {
    Fwd,
    Rwd,
    /// AWD with fraction of torque to the front
    Awd,
}

#[derive(Clone, Debug)]
pub struct CarSpec {
    pub id: u8,
    pub name: &'static str,
    pub blurb: &'static str,
    pub mass: f32,
    /// Body half-extents (collision box), local
    pub half_extents: Vec3,
    /// Inertia multipliers (scale the box inertia; >1 = more sluggish rotation)
    pub inertia_scale: Vec3,
    pub cg_to_front: f32,
    pub cg_to_rear: f32,
    pub half_track: f32,
    /// Height of suspension top mounts relative to the CG (negative = below)
    pub mount_height: f32,
    pub wheel_radius: f32,
    pub wheel_width: f32,
    pub susp_rest: f32,
    pub susp_travel: f32,
    pub spring_k: f32,
    pub damp_bump: f32,
    pub damp_rebound: f32,
    pub arb_front: f32,
    pub arb_rear: f32,
    pub tire_grip: f32,
    pub peak_slip_angle: f32,
    pub rear_grip_bias: f32,
    pub max_steer: f32,
    pub drivetrain: Drivetrain,
    pub awd_front_split: f32,
    pub peak_torque: f32,
    pub peak_rpm: f32,
    pub idle_rpm: f32,
    pub redline: f32,
    pub gears: &'static [f32],
    pub final_drive: f32,
    pub reverse_ratio: f32,
    pub brake_force: f32,
    pub brake_bias_front: f32,
    pub handbrake_force: f32,
    pub drag: f32,
    pub downforce: f32,
    /// 0..1 stability/counter-steer assist strength
    pub assist: f32,
    /// Base paint colour (render hint)
    pub color: [f32; 3],
}

pub fn car_specs() -> Vec<CarSpec> {
    vec![
        CarSpec {
            id: 0,
            name: "Pip",
            blurb: "Tiny front-drive hatch. Nimble, forgiving, a little slow.",
            mass: 980.0,
            half_extents: Vec3::new(0.82, 0.55, 1.85),
            inertia_scale: Vec3::new(1.0, 1.0, 1.0),
            cg_to_front: 1.15,
            cg_to_rear: 1.20,
            half_track: 0.74,
            mount_height: -0.05,
            wheel_radius: 0.30,
            wheel_width: 0.20,
            susp_rest: 0.36,
            susp_travel: 0.24,
            spring_k: 32000.0,
            damp_bump: 2600.0,
            damp_rebound: 3400.0,
            arb_front: 9000.0,
            arb_rear: 7000.0,
            tire_grip: 1.12,
            peak_slip_angle: 0.15,
            rear_grip_bias: 1.04,
            max_steer: 0.62,
            drivetrain: Drivetrain::Fwd,
            awd_front_split: 1.0,
            peak_torque: 235.0,
            peak_rpm: 5200.0,
            idle_rpm: 900.0,
            redline: 7200.0,
            gears: &[3.3, 2.15, 1.6, 1.25, 1.02],
            final_drive: 4.6,
            reverse_ratio: 3.2,
            brake_force: 11500.0,
            brake_bias_front: 0.66,
            handbrake_force: 5000.0,
            drag: 0.62,
            downforce: 0.8,
            assist: 0.75,
            color: [0.95, 0.55, 0.45],
        },
        CarSpec {
            id: 1,
            name: "Bramble",
            blurb: "Rear-drive coupe. Loves to slide. Hold that drift!",
            mass: 1180.0,
            half_extents: Vec3::new(0.88, 0.52, 2.15),
            inertia_scale: Vec3::new(1.0, 0.95, 1.0),
            cg_to_front: 1.30,
            cg_to_rear: 1.30,
            half_track: 0.80,
            mount_height: -0.05,
            wheel_radius: 0.32,
            wheel_width: 0.24,
            susp_rest: 0.36,
            susp_travel: 0.24,
            spring_k: 40000.0,
            damp_bump: 3000.0,
            damp_rebound: 4000.0,
            arb_front: 12000.0,
            arb_rear: 6000.0,
            tire_grip: 1.08,
            peak_slip_angle: 0.15,
            rear_grip_bias: 0.97,
            max_steer: 0.62,
            drivetrain: Drivetrain::Rwd,
            awd_front_split: 0.0,
            peak_torque: 330.0,
            peak_rpm: 5000.0,
            idle_rpm: 850.0,
            redline: 7000.0,
            gears: &[3.2, 2.2, 1.65, 1.3, 1.08, 0.92],
            final_drive: 4.9,
            reverse_ratio: 3.2,
            brake_force: 14500.0,
            brake_bias_front: 0.64,
            handbrake_force: 6500.0,
            drag: 0.62,
            downforce: 1.0,
            assist: 0.6,
            color: [0.35, 0.62, 0.95],
        },
        CarSpec {
            id: 2,
            name: "Thistle",
            blurb: "All-wheel-drive rally legend. Fast and planted everywhere.",
            mass: 1250.0,
            half_extents: Vec3::new(0.88, 0.62, 2.1),
            inertia_scale: Vec3::new(1.0, 1.0, 1.0),
            cg_to_front: 1.25,
            cg_to_rear: 1.30,
            half_track: 0.80,
            mount_height: -0.02,
            wheel_radius: 0.33,
            wheel_width: 0.23,
            susp_rest: 0.42,
            susp_travel: 0.30,
            spring_k: 38000.0,
            damp_bump: 3100.0,
            damp_rebound: 4200.0,
            arb_front: 11000.0,
            arb_rear: 9000.0,
            tire_grip: 1.12,
            peak_slip_angle: 0.15,
            rear_grip_bias: 0.99,
            max_steer: 0.60,
            drivetrain: Drivetrain::Awd,
            awd_front_split: 0.4,
            peak_torque: 345.0,
            peak_rpm: 4500.0,
            idle_rpm: 900.0,
            redline: 7200.0,
            gears: &[3.1, 2.1, 1.55, 1.22, 1.0, 0.86],
            final_drive: 5.4,
            reverse_ratio: 3.2,
            brake_force: 16000.0,
            brake_bias_front: 0.62,
            handbrake_force: 7000.0,
            drag: 0.68,
            downforce: 1.5,
            assist: 0.65,
            color: [0.98, 0.82, 0.30],
        },
        CarSpec {
            id: 3,
            name: "Burrow",
            blurb: "Chunky 4x4 camper. Soft, bouncy, unstoppable off-road.",
            mass: 1650.0,
            half_extents: Vec3::new(0.95, 0.85, 2.2),
            inertia_scale: Vec3::new(1.05, 1.1, 1.05),
            cg_to_front: 1.35,
            cg_to_rear: 1.35,
            half_track: 0.88,
            mount_height: -0.10,
            wheel_radius: 0.40,
            wheel_width: 0.30,
            susp_rest: 0.50,
            susp_travel: 0.38,
            spring_k: 42000.0,
            damp_bump: 4200.0,
            damp_rebound: 5600.0,
            arb_front: 14000.0,
            arb_rear: 12000.0,
            tire_grip: 1.10,
            peak_slip_angle: 0.17,
            rear_grip_bias: 1.02,
            max_steer: 0.58,
            drivetrain: Drivetrain::Awd,
            awd_front_split: 0.5,
            peak_torque: 470.0,
            peak_rpm: 3800.0,
            idle_rpm: 750.0,
            redline: 5600.0,
            gears: &[3.6, 2.3, 1.6, 1.2, 0.95],
            final_drive: 5.4,
            reverse_ratio: 3.5,
            brake_force: 19000.0,
            brake_bias_front: 0.62,
            handbrake_force: 8000.0,
            drag: 0.85,
            downforce: 0.5,
            assist: 0.8,
            color: [0.45, 0.72, 0.48],
        },
    ]
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WheelState {
    /// Mount point in local space
    pub local_mount: Vec3,
    pub compression: f32,
    pub prev_compression: f32,
    pub contact: bool,
    pub contact_point: Vec3,
    pub normal: Vec3,
    pub surface: Option<Surface>,
    pub load: f32,
    /// Accumulated rotation (rad) for rendering
    pub spin_angle: f32,
    /// Angular velocity (rad/s)
    pub spin_speed: f32,
    pub steer_angle: f32,
    /// Lateral sliding speed (m/s) beyond grip
    pub slide_lat: f32,
    /// Longitudinal slip speed (wheelspin/lock) m/s
    pub slide_long: f32,
    /// 0..1 skid intensity for FX/audio
    pub skid: f32,
    /// Suspension length (mount to wheel centre) for rendering
    pub susp_length: f32,
}

impl WheelState {
    pub fn is_front(idx: usize) -> bool {
        idx < 2
    }
}

/// Values useful to audio / FX / HUD.
#[derive(Clone, Copy, Debug, Default)]
pub struct CarTelemetry {
    pub speed: f32,
    pub forward_speed: f32,
    pub rpm: f32,
    pub gear: i32,
    pub throttle: f32,
    pub brake: f32,
    pub wheels_on_ground: u8,
    pub airtime: f32,
    /// Peak downward speed of the last landing (m/s), reset after read by FX
    pub last_landing: f32,
    pub drift_angle: f32,
    pub max_skid: f32,
    pub shifting: bool,
    pub engine_load: f32,
    pub in_water: bool,
}

#[derive(Clone, Debug)]
pub struct Car {
    pub spec: CarSpec,
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub ang_vel: Vec3,
    pub wheels: [WheelState; 4],
    pub steer: f32,
    pub rpm: f32,
    pub gear: i32,
    pub shift_timer: f32,
    pub reverse_timer: f32,
    pub telemetry: CarTelemetry,
    pub track_hint: Option<usize>,
    inv_inertia_local: Vec3,
    pub frozen: bool,
    /// Player assist multiplier (0 = sim, 1 = default, 1.3 = cozy)
    pub assist_level: f32,
    /// Set by an input tick; physics substeps use it
    pub input: CarInput,
}

fn torque_curve(spec: &CarSpec, rpm: f32) -> f32 {
    let x = rpm / spec.peak_rpm;
    let f = if x < 1.0 {
        0.55 + 0.45 * (1.0 - (1.0 - x).powi(2))
    } else {
        let over = (rpm - spec.peak_rpm) / (spec.redline - spec.peak_rpm).max(1.0);
        1.0 - 0.28 * over * over
    };
    spec.peak_torque * f.max(0.0)
}

impl Car {
    pub fn new(spec: CarSpec, pos: Vec3, rot: Quat) -> Car {
        let he = spec.half_extents;
        let m = spec.mass;
        // box inertia
        let ix = m / 3.0 * (he.y * he.y + he.z * he.z) * spec.inertia_scale.x;
        let iy = m / 3.0 * (he.x * he.x + he.z * he.z) * spec.inertia_scale.y;
        let iz = m / 3.0 * (he.x * he.x + he.y * he.y) * spec.inertia_scale.z;
        let mut wheels = [WheelState::default(); 4];
        let mounts = [
            Vec3::new(-spec.half_track, spec.mount_height, -spec.cg_to_front),
            Vec3::new(spec.half_track, spec.mount_height, -spec.cg_to_front),
            Vec3::new(-spec.half_track, spec.mount_height, spec.cg_to_rear),
            Vec3::new(spec.half_track, spec.mount_height, spec.cg_to_rear),
        ];
        for (w, m) in wheels.iter_mut().zip(mounts) {
            w.local_mount = m;
            w.susp_length = spec.susp_rest;
        }
        Car {
            pos,
            rot,
            vel: Vec3::ZERO,
            ang_vel: Vec3::ZERO,
            wheels,
            steer: 0.0,
            rpm: spec.idle_rpm,
            gear: 1,
            shift_timer: 0.0,
            reverse_timer: 0.0,
            telemetry: CarTelemetry::default(),
            track_hint: None,
            inv_inertia_local: Vec3::new(1.0 / ix, 1.0 / iy, 1.0 / iz),
            frozen: false,
            assist_level: 1.0,
            input: CarInput::default(),
            spec,
        }
    }

    pub fn forward(&self) -> Vec3 {
        self.rot * Vec3::NEG_Z
    }
    pub fn right(&self) -> Vec3 {
        self.rot * Vec3::X
    }
    pub fn up(&self) -> Vec3 {
        self.rot * Vec3::Y
    }

    pub fn inv_inertia_world(&self) -> Mat3 {
        let r = Mat3::from_quat(self.rot);
        r * Mat3::from_diagonal(self.inv_inertia_local) * r.transpose()
    }

    /// Velocity of a world point attached to the body
    pub fn point_velocity(&self, p: Vec3) -> Vec3 {
        self.vel + self.ang_vel.cross(p - self.pos)
    }

    /// Apply an instantaneous impulse at a world point
    pub fn apply_impulse(&mut self, impulse: Vec3, at: Vec3) {
        self.vel += impulse / self.spec.mass;
        let r = at - self.pos;
        self.ang_vel += self.inv_inertia_world() * r.cross(impulse);
    }

    pub fn teleport(&mut self, pos: Vec3, rot: Quat) {
        self.pos = pos;
        self.rot = rot;
        self.vel = Vec3::ZERO;
        self.ang_vel = Vec3::ZERO;
        self.rpm = self.spec.idle_rpm;
        self.gear = 1;
        self.steer = 0.0;
        for w in self.wheels.iter_mut() {
            w.spin_speed = 0.0;
            w.compression = 0.0;
            w.prev_compression = 0.0;
        }
    }

    pub fn speed_kmh(&self) -> f32 {
        self.vel.length() * 3.6
    }

    fn total_gear_ratio(&self) -> f32 {
        let g = if self.gear < 0 {
            -self.spec.reverse_ratio
        } else if self.gear == 0 {
            0.0
        } else {
            self.spec.gears[(self.gear as usize - 1).min(self.spec.gears.len() - 1)]
        };
        g * self.spec.final_drive
    }

    /// Advance the car by dt. `track` provides ground + surfaces.
    pub fn step(&mut self, dt: f32, track: &Track) {
        let spec = self.spec.clone();
        let input = if self.frozen { CarInput { throttle: self.input.throttle, brake: 1.0, handbrake: 1.0, ..Default::default() } } else { self.input };
        let fwd = self.forward();
        let right = self.right();
        let up = self.up();
        let speed = self.vel.length();
        let v_long_body = self.vel.dot(fwd);
        let v_lat_body = self.vel.dot(right);

        // ---------------- gearbox / reverse logic
        self.shift_timer = (self.shift_timer - dt).max(0.0);
        let mut throttle = input.throttle.clamp(0.0, 1.0);
        let mut brake = input.brake.clamp(0.0, 1.0);
        if self.gear >= 1 {
            if brake > 0.3 && throttle < 0.1 && v_long_body < 0.8 && !self.frozen {
                self.reverse_timer += dt;
                if self.reverse_timer > 0.25 {
                    self.gear = -1;
                    self.reverse_timer = 0.0;
                }
            } else {
                self.reverse_timer = 0.0;
            }
        } else if self.gear == -1 {
            if throttle > 0.1 && v_long_body > -1.0 {
                self.gear = 1;
            }
        }
        if self.gear == -1 {
            // brake pedal drives backwards, throttle brakes
            std::mem::swap(&mut throttle, &mut brake);
        }
        if self.frozen {
            brake = 1.0;
        }

        // ---------------- steering with assists
        let speed_factor = 1.0 / (1.0 + (speed / 22.0).powi(2) * 0.9);
        let mut target = input.steer.clamp(-1.0, 1.0) * spec.max_steer * speed_factor.max(0.22);
        if speed > 3.0 && v_long_body > 0.0 {
            let beta = v_lat_body.atan2(v_long_body.abs());
            // counter-steer toward the direction of travel
            target += beta * (spec.assist * self.assist_level).min(1.0) * 0.85;
        }
        target = target.clamp(-spec.max_steer, spec.max_steer);
        let rate = 3.2 * dt;
        self.steer += (target - self.steer).clamp(-rate, rate);

        // ---------------- engine
        let ratio = self.total_gear_ratio();
        let driven: &[usize] = match spec.drivetrain {
            Drivetrain::Fwd => &[0, 1],
            Drivetrain::Rwd => &[2, 3],
            Drivetrain::Awd => &[0, 1, 2, 3],
        };
        let avg_wheel_omega = driven.iter().map(|&i| self.wheels[i].spin_speed).sum::<f32>() / driven.len() as f32;
        let wheel_rpm = (avg_wheel_omega * ratio).abs() * 60.0 / std::f32::consts::TAU;
        let clutch_rpm = spec.idle_rpm + throttle * (spec.peak_rpm * 0.75 - spec.idle_rpm);
        let target_rpm = if self.gear.abs() == 1 && wheel_rpm < clutch_rpm { clutch_rpm.max(wheel_rpm) } else { wheel_rpm.max(spec.idle_rpm) };
        self.rpm += (target_rpm - self.rpm) * (1.0 - (-dt * 18.0).exp());
        self.rpm = self.rpm.clamp(spec.idle_rpm * 0.9, spec.redline + 200.0);
        // auto shift
        if self.gear >= 1 && self.shift_timer <= 0.0 {
            let n = spec.gears.len() as i32;
            let all_on_ground = self.wheels.iter().all(|w| w.contact);
            if self.rpm > spec.redline * 0.94 && self.gear < n && throttle > 0.2 && all_on_ground {
                let spinning = driven.iter().any(|&i| self.wheels[i].slide_long > 3.0);
                if !spinning {
                    self.gear += 1;
                    self.shift_timer = 0.22;
                }
            } else if self.gear > 1 {
                let lower = spec.gears[(self.gear - 2) as usize] * spec.final_drive;
                let rpm_if_lower = avg_wheel_omega.abs() * lower * 60.0 / std::f32::consts::TAU;
                if rpm_if_lower < spec.redline * 0.72 && (self.rpm < spec.peak_rpm * 0.62 || (brake > 0.3 && rpm_if_lower < spec.redline * 0.8)) {
                    self.gear -= 1;
                    self.shift_timer = 0.18;
                }
            }
        }
        let shifting = self.shift_timer > 0.05;
        let limiter = self.rpm >= spec.redline;
        let eff_throttle = if shifting || limiter { 0.0 } else { throttle };
        let mut engine_torque = torque_curve(&spec, self.rpm) * eff_throttle;
        if eff_throttle < 0.05 && self.gear != 0 {
            engine_torque -= spec.peak_torque * 0.12 * (self.rpm / spec.redline);
        }
        let total_drive_force = if self.frozen { 0.0 } else { engine_torque * ratio * 0.88 / spec.wheel_radius };

        // ---------------- wheels
        let inv_i = self.inv_inertia_world();
        let mut force = Vec3::new(0.0, -GRAVITY * spec.mass, 0.0);
        let mut torque = Vec3::ZERO;
        let add_force_at = |f: Vec3, p: Vec3, force: &mut Vec3, torque: &mut Vec3, pos: Vec3| {
            *force += f;
            *torque += (p - pos).cross(f);
        };
        let max_len = spec.susp_rest + spec.wheel_radius;
        let mut on_ground = 0u8;
        let mut max_skid: f32 = 0.0;
        let mut landing: f32 = 0.0;
        let mut compressions = [0.0f32; 4];
        for i in 0..4 {
            let mount = self.pos + self.rot * self.wheels[i].local_mount;
            let down = -up;
            // iterative ray / heightfield intersection
            let mut t = max_len;
            let mut normal = Vec3::Y;
            let mut hit = false;
            {
                let mut p = mount + down * t;
                for _ in 0..3 {
                    let (h, n) = track.terrain.sample(p.x, p.z);
                    let plane_p = Vec3::new(p.x, h, p.z);
                    let denom = down.dot(n);
                    if denom > -0.2 {
                        break;
                    }
                    t = (plane_p - mount).dot(n) / denom;
                    normal = n;
                    p = mount + down * t;
                }
                if t < max_len && t > -0.5 {
                    hit = true;
                }
            }
            let w = &mut self.wheels[i];
            w.prev_compression = w.compression;
            let front = i < 2;
            w.steer_angle = if front { self.steer } else { 0.0 };
            if !hit {
                if w.contact {
                    // just left the ground
                }
                w.contact = false;
                w.compression = 0.0;
                w.load = 0.0;
                w.susp_length = spec.susp_rest;
                w.skid *= 0.9;
                w.slide_lat = 0.0;
                w.slide_long = 0.0;
                // free-spinning wheel drifts toward drive
                let is_driven = driven.contains(&i);
                if is_driven && !shifting {
                    w.spin_speed += (eff_throttle * 60.0 - w.spin_speed * 0.5) * dt;
                }
                if input.handbrake > 0.5 && !front || brake > 0.5 {
                    w.spin_speed *= 0.8;
                }
                w.spin_angle += w.spin_speed * dt;
                continue;
            }
            on_ground += 1;
            let was_contact = w.contact;
            w.contact = true;
            w.normal = normal;
            let raw_comp = max_len - t;
            let comp = raw_comp.clamp(0.0, spec.susp_travel);
            w.compression = comp;
            compressions[i] = comp;
            w.susp_length = (spec.susp_rest - comp).max(spec.susp_rest - spec.susp_travel);
            let contact = mount + down * t;
            w.contact_point = contact;
            let (surface, q) = track.surface_at(contact, self.track_hint);
            if i == 0 {
                self.track_hint = Some(q.index);
            }
            w.surface = Some(surface);
            let props = surface.props();
            let comp_vel = (w.compression - w.prev_compression) / dt;
            if !was_contact {
                landing = landing.max((-self.vel.dot(normal)).max(0.0));
            }
            let damper = if comp_vel > 0.0 { spec.damp_bump } else { spec.damp_rebound };
            let mut fs = spec.spring_k * comp + damper * comp_vel;
            // bump stop
            if raw_comp > spec.susp_travel {
                let over = raw_comp - spec.susp_travel;
                fs += spec.spring_k * 12.0 * over + spec.damp_bump * 4.0 * comp_vel.max(0.0);
            }
            // surface bumpiness
            if props.bumpiness > 0.0 && speed > 2.0 {
                let ph = (contact.x * 1.7 + contact.z * 2.3).sin() * (contact.z * 1.3 - contact.x * 0.7).cos();
                fs += ph * props.bumpiness * spec.spring_k * (speed / 20.0).min(1.0);
            }
            let fs = fs.max(0.0);
            w.load = fs * up.dot(normal).max(0.0);
            add_force_at(up * fs, contact, &mut force, &mut torque, self.pos);

            // ---- tyre
            let heading_local = Quat::from_rotation_y(-w.steer_angle) * Vec3::NEG_Z;
            let heading = self.rot * heading_local;
            let f_dir = (heading - normal * heading.dot(normal)).normalize_or_zero();
            let r_dir = f_dir.cross(normal).normalize_or_zero();
            let vc = self.vel + self.ang_vel.cross(contact - self.pos);
            let v_long = vc.dot(f_dir);
            let v_lat = vc.dot(r_dir);
            let fz = w.load.min(spec.mass * GRAVITY * 1.5);
            let grip_bias = if front { 1.0 } else { spec.rear_grip_bias };
            let mu = spec.tire_grip * props.grip * grip_bias;
            let max_f = mu * fz;

            // lateral curve
            let hand = !front && input.handbrake > 0.5;
            let alpha = v_lat.atan2(v_long.abs().max(3.5));
            let x = alpha.abs() / spec.peak_slip_angle;
            let g = if x < 1.0 { x * (2.0 - x) } else { props.slide_grip + (1.0 - props.slide_grip) * (-(x - 1.0) * 1.2).exp() };
            let mut fy = -alpha.signum() * max_f * g;
            if hand {
                fy *= 0.45;
            }
            // low speed lateral damping to kill creep
            if v_long.abs() < 2.0 {
                let damp = -v_lat * spec.mass * 0.25 * 6.0;
                let blend = 1.0 - v_long.abs() / 2.0;
                fy = fy * (1.0 - blend) + damp.clamp(-max_f, max_f) * blend;
            }
            // longitudinal request
            let is_driven = driven.contains(&i);
            let mut fx = 0.0;
            if is_driven {
                let share = match spec.drivetrain {
                    Drivetrain::Awd => {
                        if front {
                            spec.awd_front_split * 0.5
                        } else {
                            (1.0 - spec.awd_front_split) * 0.5
                        }
                    }
                    _ => 0.5,
                };
                let mut drive = total_drive_force * share;
                // traction / stability control: limit drive to the grip left after cornering
                let a = (spec.assist * self.assist_level).min(1.2);
                let lat_use = (fy.abs() / max_f.max(1.0)).min(1.0);
                let avail = max_f * (1.0 - lat_use * lat_use).max(0.0).sqrt();
                let cap = avail * (1.0 + (1.0 - a).max(0.0) * 0.9) + max_f * 0.06;
                if drive.abs() > cap {
                    drive = drive.signum() * cap;
                }
                fx += drive;
            }
            let bias = if front { spec.brake_bias_front } else { 1.0 - spec.brake_bias_front };
            let mut brake_f = brake * spec.brake_force * bias * 0.5;
            if hand {
                brake_f += spec.handbrake_force * 0.5;
            }
            let low_speed = (v_long.abs() / 0.6).min(1.0);
            fx -= v_long.signum() * brake_f * low_speed;
            fx -= v_long.signum() * props.rolling * fz * low_speed;
            // static hold when braking at standstill
            if brake > 0.1 && v_long.abs() < 0.6 && !is_driven_effective(throttle) {
                fx -= v_long * spec.mass * 0.25 * 8.0;
            }

            // combined: friction ellipse
            let mut slip_long = 0.0;
            let assist = spec.assist * self.assist_level;
            let fx_limit = max_f * 0.98;
            if fx.abs() > fx_limit {
                slip_long = (fx.abs() - fx_limit) / max_f.max(1.0);
                fx = fx.signum() * fx_limit * (1.0 - 0.15 * slip_long.min(1.0));
            }
            let fy_cap = max_f * (1.0 - (fx / max_f.max(1.0)).powi(2)).max(0.12 + 0.2 * assist).sqrt();
            fy = fy.clamp(-fy_cap, fy_cap);
            let tire_force = f_dir * fx + r_dir * fy;
            // apply lateral force a little above the contact patch to reduce body roll (arcade stability)
            let apply_at = contact + up * (spec.wheel_radius * 0.9);
            add_force_at(tire_force, apply_at, &mut force, &mut torque, self.pos);

            // wheel spin (kinematic)
            let mut omega = v_long / spec.wheel_radius;
            if slip_long > 0.0 && is_driven && total_drive_force.abs() > brake_f {
                omega += total_drive_force.signum() * slip_long * 25.0;
            }
            if hand || (brake > 0.5 && slip_long > 0.05 && !is_driven) || (brake > 0.9 && slip_long > 0.3) {
                omega *= 0.0;
            }
            w.spin_speed += (omega - w.spin_speed) * (1.0 - (-dt * 30.0).exp());
            w.spin_angle = (w.spin_angle + w.spin_speed * dt) % std::f32::consts::TAU;
            let slide = if x > 1.0 { v_lat.abs() } else { 0.0 };
            w.slide_lat = slide;
            w.slide_long = (w.spin_speed * spec.wheel_radius - v_long).abs();
            let skid_amt = ((slide - 1.0) / 6.0).clamp(0.0, 1.0).max(((w.slide_long - 2.0) / 8.0).clamp(0.0, 1.0));
            w.skid += (skid_amt - w.skid) * (1.0 - (-dt * 12.0).exp());
            max_skid = max_skid.max(w.skid);
        }

        // anti-roll bars
        for (l, r, k) in [(0usize, 1usize, spec.arb_front), (2, 3, spec.arb_rear)] {
            if self.wheels[l].contact && self.wheels[r].contact {
                let diff = compressions[l] - compressions[r];
                let f = diff * k;
                let pl = self.pos + self.rot * self.wheels[l].local_mount;
                let pr = self.pos + self.rot * self.wheels[r].local_mount;
                add_force_at(up * f, pl, &mut force, &mut torque, self.pos);
                add_force_at(-up * f, pr, &mut force, &mut torque, self.pos);
            }
        }

        // aero
        let v2 = speed * speed;
        if speed > 0.1 {
            force -= self.vel.normalize() * spec.drag * v2;
        }
        if on_ground > 0 {
            force -= up * spec.downforce * v2;
        }

        // stability assist: damp yaw when not steering hard; gentle body-slip limiter
        if on_ground >= 2 {
            let yaw_rate = self.ang_vel.dot(up);
            let steer_input = input.steer.abs();
            let yaw_damp = spec.assist * self.assist_level * (0.25 + 0.6 * (1.0 - steer_input)) * 0.45;
            torque -= up * yaw_rate * yaw_damp * spec.mass * 1.2;
        }

        // spin protection: beyond a generous drift angle, gently yaw the car back
        if on_ground >= 2 && speed > 5.0 && v_long_body > -1.0 {
            let beta = v_lat_body.atan2(v_long_body.abs());
            let limit = 0.55 + (1.0 - spec.assist * self.assist_level) * 0.6;
            if beta.abs() > limit {
                let excess = beta.abs() - limit;
                // velocity to the right of heading -> rotate heading right (negative yaw)
                torque -= up * beta.signum() * excess * spec.mass * 9.0 * spec.assist * self.assist_level;
            }
        }

        // air control
        if on_ground == 0 {
            let (_, n) = track.terrain.sample(self.pos.x, self.pos.z);
            let level = up.cross(Vec3::Y.lerp(n, 0.5).normalize());
            let k = spec.mass * 6.0;
            torque += level * k * 2.0;
            torque -= (self.ang_vel - up * self.ang_vel.dot(up)) * k * 0.6;
            torque += up * (-input.steer) * spec.mass * 1.2;
        }

        // body vs ground collision (keeps flipped cars from sinking)
        let he = spec.half_extents;
        let corners = [
            Vec3::new(-he.x, -he.y * 0.4, -he.z),
            Vec3::new(he.x, -he.y * 0.4, -he.z),
            Vec3::new(-he.x, -he.y * 0.4, he.z),
            Vec3::new(he.x, -he.y * 0.4, he.z),
            Vec3::new(-he.x, he.y, -he.z * 0.8),
            Vec3::new(he.x, he.y, -he.z * 0.8),
            Vec3::new(-he.x, he.y, he.z * 0.8),
            Vec3::new(he.x, he.y, he.z * 0.8),
            Vec3::new(0.0, he.y * 1.1, 0.0),
            Vec3::new(0.0, -he.y * 0.4, -he.z * 1.05),
            Vec3::new(0.0, -he.y * 0.4, he.z * 1.05),
        ];
        let mut body_hit = 0.0f32;
        for c in corners {
            let p = self.pos + self.rot * c;
            let (h, n) = track.terrain.sample(p.x, p.z);
            let depth = (h - p.y) * n.y;
            if depth > 0.0 {
                let v = self.point_velocity(p);
                let vn = v.dot(n);
                let k = spec.mass * 400.0;
                let c_d = spec.mass * 30.0;
                let fn_mag = (k * depth - c_d * vn).max(0.0);
                let vt = v - n * vn;
                let friction = if vt.length() > 0.01 { -vt.normalize() * fn_mag * 0.6 } else { Vec3::ZERO };
                add_force_at(n * fn_mag + friction, p, &mut force, &mut torque, self.pos);
                body_hit = body_hit.max(-vn);
            }
        }
        let _ = body_hit;

        // water: heavy drag + buoyancy
        let mut in_water = false;
        if let Some(wl) = track.water_level {
            if self.pos.y < wl + 0.2 {
                in_water = true;
                let depth = (wl + 0.2 - self.pos.y).min(1.5);
                force += Vec3::Y * spec.mass * GRAVITY * depth * 0.9;
                force -= self.vel * spec.mass * 1.5;
            }
        }

        // ---------------- integrate
        self.vel += force / spec.mass * dt;
        self.ang_vel += inv_i * torque * dt;
        // mild angular damping
        self.ang_vel *= 1.0 - 0.15 * dt;
        let max_ang = 12.0;
        if self.ang_vel.length() > max_ang {
            self.ang_vel = self.ang_vel.normalize() * max_ang;
        }
        self.pos += self.vel * dt;
        let w = self.ang_vel;
        let dq = Quat::from_xyzw(w.x, w.y, w.z, 0.0) * self.rot;
        self.rot = Quat::from_xyzw(
            self.rot.x + 0.5 * dt * dq.x,
            self.rot.y + 0.5 * dt * dq.y,
            self.rot.z + 0.5 * dt * dq.z,
            self.rot.w + 0.5 * dt * dq.w,
        )
        .normalize();

        // ---------------- telemetry
        let t = &mut self.telemetry;
        t.speed = self.vel.length();
        t.forward_speed = self.vel.dot(self.rot * Vec3::NEG_Z);
        t.rpm = self.rpm;
        t.gear = self.gear;
        t.throttle = eff_throttle;
        t.brake = brake;
        t.wheels_on_ground = on_ground;
        if on_ground == 0 {
            t.airtime += dt;
        } else {
            if landing > t.last_landing {
                t.last_landing = landing;
            }
            t.airtime = 0.0;
        }
        t.drift_angle = if speed > 4.0 { v_lat_body.atan2(v_long_body.abs()) } else { 0.0 };
        t.max_skid = max_skid;
        t.shifting = shifting;
        t.engine_load = (eff_throttle * (1.0 - self.rpm / spec.redline * 0.3)).clamp(0.0, 1.0);
        t.in_water = in_water;
    }

    /// Two collision spheres (centre, radius) in world space along the body length
    pub fn collision_spheres(&self) -> [(Vec3, f32); 2] {
        let he = self.spec.half_extents;
        let r = he.x * 1.02;
        let off = (he.z - r * 0.85).max(0.1);
        let fwd = self.forward();
        [(self.pos + fwd * off, r), (self.pos - fwd * off, r)]
    }

    pub fn is_upside_down(&self) -> bool {
        self.up().y < 0.2
    }
}

#[inline]
fn is_driven_effective(throttle: f32) -> bool {
    throttle > 0.1
}
