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
use crate::surface::{Surface, SurfaceProps};
use crate::track::{Track, TrackQuery};
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
    /// Rotational inertia of engine + flywheel (kg m^2), reflected through the gearbox
    pub engine_inertia: f32,
    /// Limited-slip strength (0 = open diff)
    pub lsd: f32,
    /// AI planner calibration: multiplies the grip the racing-line planner assumes, so that
    /// the top AI difficulty sits at this car's lap-time optimum (measured with
    /// `skilltest` SKILL_SWEEP)
    pub ai_grip: f32,
    /// Share of the aero downforce on the front axle (applied at the axles)
    pub aero_front: f32,
    /// Gear change time (s): ~0.22 rally box, ~0.05 paddle-shift race box
    pub shift_time: f32,
    pub brake_force: f32,
    pub brake_bias_front: f32,
    pub handbrake_force: f32,
    pub drag: f32,
    pub downforce: f32,
    /// 0..1 stability/counter-steer assist strength
    pub assist: f32,
    /// Base paint colour (render hint)
    pub color: [f32; 3],
    /// Rally cars (all surfaces) or circuit cars (tarmac racers: GT / hypercar / formula)
    pub class: CarClass,
    /// Tyre compound: rally tyres work everywhere; slicks are superb on tarmac only
    pub tyre: TyreType,
}

/// Car discipline (shared interface; gameplay/UI filter cars per track discipline).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CarClass {
    Rally,
    /// Tarmac circuit racers: GT, hypercar, open-wheel formula
    Circuit,
}

/// Tyre compound (shared interface; the tyre model scales surface grip by it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TyreType {
    /// Treaded rally tyres: good on every surface
    Rally,
    /// Racing slicks: much more grip on tarmac, very poor on loose/wet surfaces
    Slick,
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
            tire_grip: 1.2,
            peak_slip_angle: 0.15,
            rear_grip_bias: 1.04,
            max_steer: 0.62,
            drivetrain: Drivetrain::Fwd,
            awd_front_split: 1.0,
            peak_torque: 145.0,
            peak_rpm: 5600.0,
            idle_rpm: 900.0,
            redline: 7200.0,
            gears: &[2.3, 1.75, 1.38, 1.13, 0.95],
            final_drive: 3.5,
            reverse_ratio: 3.2,
            engine_inertia: 0.10,
            lsd: 0.1,
            ai_grip: 1.0,
            aero_front: 0.511,
            shift_time: 0.22,
            brake_force: 11500.0,
            brake_bias_front: 0.66,
            handbrake_force: 5000.0,
            drag: 0.62,
            downforce: 0.8,
            assist: 0.75,
            color: [0.95, 0.55, 0.45],
            class: CarClass::Rally,
            tyre: TyreType::Rally,
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
            tire_grip: 1.17,
            peak_slip_angle: 0.15,
            rear_grip_bias: 1.08,
            max_steer: 0.62,
            drivetrain: Drivetrain::Rwd,
            awd_front_split: 0.0,
            peak_torque: 325.0,
            peak_rpm: 4800.0,
            idle_rpm: 850.0,
            redline: 6800.0,
            gears: &[2.9, 2.0, 1.5, 1.2, 1.0, 0.84],
            final_drive: 3.6,
            reverse_ratio: 3.2,
            engine_inertia: 0.20,
            lsd: 0.35,
            ai_grip: 0.92,
            aero_front: 0.5,
            shift_time: 0.22,
            brake_force: 14500.0,
            brake_bias_front: 0.64,
            handbrake_force: 6500.0,
            drag: 0.62,
            downforce: 1.0,
            assist: 0.6,
            color: [0.35, 0.62, 0.95],
            class: CarClass::Rally,
            tyre: TyreType::Rally,
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
            awd_front_split: 0.28,
            peak_torque: 320.0,
            peak_rpm: 4500.0,
            idle_rpm: 900.0,
            redline: 7200.0,
            gears: &[3.0, 2.05, 1.5, 1.18, 0.97, 0.82],
            final_drive: 4.85,
            reverse_ratio: 3.2,
            engine_inertia: 0.15,
            lsd: 0.3,
            ai_grip: 0.9,
            aero_front: 0.51,
            shift_time: 0.22,
            brake_force: 16000.0,
            brake_bias_front: 0.62,
            handbrake_force: 7000.0,
            drag: 0.68,
            downforce: 1.5,
            assist: 0.65,
            color: [0.98, 0.82, 0.30],
            class: CarClass::Rally,
            tyre: TyreType::Rally,
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
            awd_front_split: 0.35,
            peak_torque: 420.0,
            peak_rpm: 3600.0,
            idle_rpm: 750.0,
            redline: 5400.0,
            gears: &[3.4, 2.2, 1.5, 1.12, 0.9],
            final_drive: 3.5,
            reverse_ratio: 3.5,
            engine_inertia: 0.25,
            lsd: 0.3,
            ai_grip: 0.9,
            aero_front: 0.5,
            shift_time: 0.22,
            brake_force: 19000.0,
            brake_bias_front: 0.62,
            handbrake_force: 8000.0,
            drag: 0.85,
            downforce: 0.5,
            assist: 0.8,
            color: [0.45, 0.72, 0.48],
            class: CarClass::Rally,
            tyre: TyreType::Rally,
        },
        CarSpec {
            id: 4,
            name: "Blossom",
            blurb: "Pink turbo pocket rocket. Tiny, twitchy, loves to rotate.",
            mass: 1080.0,
            half_extents: Vec3::new(0.84, 0.56, 1.95),
            // short and light: quick to change direction
            inertia_scale: Vec3::new(1.0, 0.86, 1.0),
            cg_to_front: 1.08,
            cg_to_rear: 1.18,
            half_track: 0.78,
            mount_height: -0.04,
            wheel_radius: 0.31,
            wheel_width: 0.22,
            susp_rest: 0.38,
            susp_travel: 0.26,
            spring_k: 36000.0,
            damp_bump: 2800.0,
            damp_rebound: 3700.0,
            arb_front: 9000.0,
            arb_rear: 11000.0,
            tire_grip: 1.15,
            peak_slip_angle: 0.15,
            rear_grip_bias: 0.98,
            max_steer: 0.64,
            drivetrain: Drivetrain::Awd,
            // rear-biased AWD
            awd_front_split: 0.25,
            // small turbo: strong mid-range, runs out of breath at the top
            peak_torque: 290.0,
            peak_rpm: 4000.0,
            idle_rpm: 950.0,
            redline: 7000.0,
            gears: &[2.6, 1.85, 1.42, 1.15, 0.95, 0.8],
            final_drive: 4.3,
            reverse_ratio: 3.2,
            engine_inertia: 0.12,
            lsd: 0.4,
            ai_grip: 0.87,
            aero_front: 0.522,
            shift_time: 0.22,
            brake_force: 13500.0,
            brake_bias_front: 0.64,
            handbrake_force: 6500.0,
            // boxy hatch: more drag, a little less top speed
            drag: 0.74,
            downforce: 1.0,
            assist: 0.62,
            color: [0.98, 0.60, 0.78],
            class: CarClass::Rally,
            tyre: TyreType::Rally,
        },
        // ---- circuit class (slicks, tarmac only) ----
        CarSpec {
            id: 5,
            name: "Comet",
            blurb: "Rear-engined GT racer with a big wing. Tarmac only!",
            mass: 1420.0,
            half_extents: Vec3::new(0.98, 0.5, 2.25),
            inertia_scale: Vec3::new(1.0, 1.0, 1.0),
            cg_to_front: 1.42,
            cg_to_rear: 1.03,
            half_track: 0.82,
            mount_height: 0.02,
            wheel_radius: 0.34,
            wheel_width: 0.30,
            susp_rest: 0.20,
            susp_travel: 0.10,
            spring_k: 95000.0,
            damp_bump: 6500.0,
            damp_rebound: 8500.0,
            arb_front: 32000.0,
            arb_rear: 26000.0,
            tire_grip: 1.42,
            peak_slip_angle: 0.11,
            rear_grip_bias: 1.06,
            max_steer: 0.55,
            drivetrain: Drivetrain::Rwd,
            awd_front_split: 0.0,
            peak_torque: 480.0,
            peak_rpm: 7000.0,
            idle_rpm: 1100.0,
            redline: 9000.0,
            gears: &[3.4, 2.55, 2.0, 1.62, 1.33, 1.1, 0.92],
            final_drive: 3.9,
            reverse_ratio: 3.2,
            engine_inertia: 0.12,
            lsd: 0.45,
            ai_grip: 0.9,
            aero_front: 0.34,
            shift_time: 0.08,
            brake_force: 34000.0,
            brake_bias_front: 0.66,
            handbrake_force: 4000.0,
            drag: 0.52,
            downforce: 1.03,
            assist: 0.5,
            color: [0.96, 0.95, 0.90],
            class: CarClass::Circuit,
            tyre: TyreType::Slick,
        },
        // hypercar
        CarSpec {
            id: 6,
            name: "Zephyr",
            blurb: "Hybrid hypercar. Silent shove, glued to the tarmac.",
            mass: 1500.0,
            half_extents: Vec3::new(1.0, 0.48, 2.3),
            inertia_scale: Vec3::new(1.0, 1.0, 1.0),
            cg_to_front: 1.36,
            cg_to_rear: 1.30,
            half_track: 0.85,
            mount_height: 0.03,
            wheel_radius: 0.35,
            wheel_width: 0.32,
            susp_rest: 0.18,
            susp_travel: 0.09,
            spring_k: 110000.0,
            damp_bump: 7500.0,
            damp_rebound: 9500.0,
            arb_front: 36000.0,
            arb_rear: 32000.0,
            tire_grip: 1.5,
            peak_slip_angle: 0.105,
            rear_grip_bias: 1.03,
            max_steer: 0.55,
            drivetrain: Drivetrain::Awd,
            awd_front_split: 0.32,
            peak_torque: 880.0,
            peak_rpm: 7200.0,
            idle_rpm: 1000.0,
            redline: 8600.0,
            gears: &[3.3, 2.4, 1.85, 1.5, 1.24, 1.04, 0.88],
            final_drive: 3.4,
            reverse_ratio: 3.2,
            engine_inertia: 0.14,
            lsd: 0.4,
            ai_grip: 0.9,
            aero_front: 0.40,
            shift_time: 0.06,
            brake_force: 44500.0,
            brake_bias_front: 0.66,
            handbrake_force: 4000.0,
            drag: 0.66,
            downforce: 1.73,
            assist: 0.5,
            color: [0.55, 0.85, 0.92],
            class: CarClass::Circuit,
            tyre: TyreType::Slick,
        },
        // formula
        CarSpec {
            id: 7,
            name: "Dragonfly",
            blurb: "Open-wheel formula racer. Featherlight, all wing.",
            mass: 800.0,
            half_extents: Vec3::new(0.95, 0.5, 2.5),
            inertia_scale: Vec3::new(0.9, 0.9, 0.9),
            cg_to_front: 1.75,
            cg_to_rear: 1.35,
            half_track: 0.80,
            mount_height: 0.12,
            wheel_radius: 0.33,
            wheel_width: 0.36,
            susp_rest: 0.10,
            susp_travel: 0.06,
            spring_k: 140000.0,
            damp_bump: 6000.0,
            damp_rebound: 7500.0,
            arb_front: 30000.0,
            arb_rear: 26000.0,
            tire_grip: 1.78,
            peak_slip_angle: 0.095,
            rear_grip_bias: 1.08,
            max_steer: 0.5,
            drivetrain: Drivetrain::Rwd,
            awd_front_split: 0.0,
            peak_torque: 560.0,
            peak_rpm: 12000.0,
            idle_rpm: 4000.0,
            redline: 15000.0,
            gears: &[3.0, 2.45, 2.05, 1.75, 1.52, 1.34, 1.19, 1.06],
            final_drive: 4.9,
            reverse_ratio: 3.2,
            engine_inertia: 0.05,
            lsd: 0.5,
            ai_grip: 0.9,
            aero_front: 0.36,
            shift_time: 0.05,
            brake_force: 31800.0,
            brake_bias_front: 0.60,
            handbrake_force: 2000.0,
            drag: 0.76,
            downforce: 5.2,
            assist: 0.45,
            color: [0.98, 0.78, 0.35],
            class: CarClass::Circuit,
            tyre: TyreType::Slick,
        },
        CarSpec {
            id: 8,
            name: "Kestrel",
            blurb: "Wide-body turbo rally hatch. Sharp turn-in, huge grip, a proper works car.",
            mass: 1190.0,
            half_extents: Vec3::new(0.9, 0.6, 2.0),
            inertia_scale: Vec3::new(1.0, 0.92, 1.0),
            cg_to_front: 1.2,
            cg_to_rear: 1.28,
            half_track: 0.82,
            mount_height: -0.03,
            wheel_radius: 0.32,
            wheel_width: 0.23,
            susp_rest: 0.40,
            susp_travel: 0.29,
            spring_k: 37000.0,
            damp_bump: 3000.0,
            damp_rebound: 4000.0,
            arb_front: 10000.0,
            arb_rear: 9500.0,
            tire_grip: 1.15,
            peak_slip_angle: 0.15,
            rear_grip_bias: 0.99,
            max_steer: 0.62,
            drivetrain: Drivetrain::Awd,
            awd_front_split: 0.3,
            // 1.6 turbo with a restrictor: big mid-range torque
            peak_torque: 330.0,
            peak_rpm: 4200.0,
            idle_rpm: 950.0,
            redline: 7300.0,
            gears: &[2.8, 1.95, 1.48, 1.18, 0.97, 0.82],
            final_drive: 4.6,
            reverse_ratio: 3.2,
            engine_inertia: 0.13,
            lsd: 0.4,
            ai_grip: 0.89,
            aero_front: 0.5,
            shift_time: 0.15,
            brake_force: 15500.0,
            brake_bias_front: 0.63,
            handbrake_force: 7000.0,
            drag: 0.7,
            // big roof wing and splitter
            downforce: 2.0,
            assist: 0.62,
            color: [0.35, 0.62, 0.95],
            class: CarClass::Rally,
            tyre: TyreType::Rally,
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
    /// Longitudinal slip ratio (wheel surface speed - ground speed) / max(|ground speed|, 3 m/s).
    /// > 0 wheelspin, < 0 braking / lock-up (-1 = locked)
    pub slip_ratio: f32,
    /// Tyre slip angle (rad)
    pub slip_angle: f32,
    /// 0..1: how far past peak friction the tyre is (combined longitudinal + lateral)
    pub traction_loss: f32,
    /// Current traction-control torque cut on this wheel (0..1)
    pub tc_cut: f32,
    /// Traction-control integrator state
    pub tc_i: f32,
    /// Slip ratio / slip angle at peak friction for the current ground (for normalising)
    pub peak_slip_ratio: f32,
    pub peak_slip_angle: f32,
    /// Current ABS brake release on this wheel (0..1)
    pub abs_cut: f32,
}

impl WheelState {
    pub fn is_front(idx: usize) -> bool {
        idx < 2
    }

    /// Traction loss split by cause, each 0..1: (sideways slide, wheelspin, lock-up).
    /// Zero while the tyre is below its peak, except a gentle sideways "scrub" in the last
    /// 15% before the peak (a correctly driven corner sounds alive but quiet).
    /// The one place FX / audio / HUD should read tyre slip from.
    pub fn traction_split(&self) -> (f32, f32, f32) {
        if !self.contact || self.peak_slip_ratio <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        let sx = self.slip_ratio / self.peak_slip_ratio;
        let sy = self.slip_angle / self.peak_slip_angle.max(1e-3);
        let s2 = sx * sx + sy * sy;
        if s2 < 1e-6 {
            return (0.0, 0.0, 0.0);
        }
        let s = s2.sqrt();
        let (lat_share, long_share) = (sy * sy / s2, sx * sx / s2);
        let scrub = 0.15 * ((s - 0.85) / 0.15).clamp(0.0, 1.0) * lat_share;
        let loss = self.traction_loss;
        let lat = (loss * lat_share + scrub).min(1.0);
        let long = loss * long_share;
        if self.slip_ratio >= 0.0 { (lat, long, 0.0) } else { (lat, 0.0, long) }
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
    /// 0..1 wheelspin of the driven wheels (max over wheels)
    pub wheelspin: f32,
    /// 0..1 brake lock-up (max over wheels)
    pub lockup: f32,
    /// 0..1 floor scraping the ground at speed (sparks for FX / grind for audio)
    pub floor_scrape: f32,
}

#[derive(Clone, Debug)]
pub struct Car {
    pub spec: CarSpec,
    pub pos: Vec3,
    pub rot: Quat,
    /// State at the previous fixed tick (for render interpolation)
    pub prev_pos: Vec3,
    pub prev_rot: Quat,
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
    /// Engine torque multiplier (1 = stock). Used by AI catch-up; players keep 1.
    pub power_scale: f32,
    /// Aero drag multiplier (slipstream behind another car < 1)
    pub drag_scale: f32,
    /// Seconds since the handbrake was last pulled (stability control stays out of drifts)
    pub handbrake_age: f32,
    /// Post-landing stability window (s left)
    pub landing_timer: f32,
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
            prev_pos: pos,
            prev_rot: rot,
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
            power_scale: 1.0,
            drag_scale: 1.0,
            handbrake_age: 10.0,
            landing_timer: 0.0,
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
        self.prev_pos = pos;
        self.prev_rot = rot;
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

        // ---------------- steering with assists (see steer_target)
        let steer_in = input.steer.clamp(-1.0, 1.0);
        let target = self.steer_target(steer_in);
        let rate = 4.0 * dt;
        self.steer += (target - self.steer).clamp(-rate, rate);

        // ---------------- engine + clutch
        // Engine speed is locked to the driven wheels through the gearbox when the clutch is
        // engaged; it slips (engine free-revs toward a launch rpm) when pulling away, below idle
        // or while shifting. Wheel speeds come from integrated wheel dynamics, so wheelspin
        // flares the revs and the rev limiter cuts power.
        let ratio = self.total_gear_ratio();
        let shares = drive_shares(&spec);
        let avg_wheel_omega: f32 = (0..4).map(|i| self.wheels[i].spin_speed * shares[i]).sum();
        let wheel_rpm = (avg_wheel_omega * ratio).abs() * 60.0 / std::f32::consts::TAU;
        let launch_rpm = spec.idle_rpm + throttle * (spec.peak_rpm * 0.75 - spec.idle_rpm);
        let shifting = self.shift_timer > 0.05;
        let clutch_slip = shifting || self.gear == 0 || self.frozen || (self.gear.abs() == 1 && wheel_rpm < launch_rpm) || wheel_rpm < spec.idle_rpm * 0.95;
        if clutch_slip {
            let target_rpm = launch_rpm.max(wheel_rpm).max(spec.idle_rpm);
            self.rpm += (target_rpm - self.rpm) * (1.0 - (-dt * 9.0).exp());
        } else {
            self.rpm += (wheel_rpm - self.rpm) * (1.0 - (-dt * 80.0).exp());
        }
        self.rpm = self.rpm.clamp(spec.idle_rpm * 0.9, spec.redline + 250.0);
        // auto shift
        if self.gear >= 1 && self.shift_timer <= 0.0 {
            let n = spec.gears.len() as i32;
            let all_on_ground = self.wheels.iter().all(|w| w.contact);
            let spinning = self.telemetry.wheelspin > 0.5;
            // short-shift through wheelspin only when banging on the limiter
            let up_rpm = if spinning { spec.redline * 0.985 } else { spec.redline * 0.94 };
            if self.rpm > up_rpm && self.gear < n && throttle > 0.2 && all_on_ground {
                self.gear += 1;
                self.shift_timer = spec.shift_time;
            } else if self.gear > 1 {
                let lower = spec.gears[(self.gear - 2) as usize] * spec.final_drive;
                let body_omega = v_long_body.abs() / spec.wheel_radius;
                let rpm_if_lower = body_omega * lower * 60.0 / std::f32::consts::TAU;
                // kickdown: flooring it drops a gear when the engine has room to rev
                let kickdown = throttle > 0.92 && rpm_if_lower < spec.redline * 0.84 && self.rpm < spec.peak_rpm * 1.05;
                if kickdown || (rpm_if_lower < spec.redline * 0.72 && (self.rpm < spec.peak_rpm * 0.62 || (brake > 0.3 && rpm_if_lower < spec.redline * 0.8))) {
                    self.gear -= 1;
                    self.shift_timer = spec.shift_time * 0.82;
                }
            }
        }
        let shifting = self.shift_timer > 0.05;
        let limiter = self.rpm >= spec.redline;
        let eff_throttle = if shifting || limiter { 0.0 } else { throttle };
        let mut engine_torque = torque_curve(&spec, self.rpm) * eff_throttle * self.power_scale;
        if clutch_slip && self.gear.abs() == 1 && !shifting {
            // dumping the clutch: the spinning flywheel adds a torque spike while it slows
            // toward the wheels (the launch "chirp")
            let slip_rpm = (self.rpm - wheel_rpm).max(0.0);
            engine_torque *= 1.0 + 0.25 * (spec.engine_inertia / 0.18).clamp(0.4, 1.0) * (slip_rpm / 2500.0).min(1.0);
        }
        if eff_throttle < 0.05 && self.gear != 0 && !clutch_slip {
            engine_torque -= spec.peak_torque * 0.12 * (self.rpm / spec.redline);
        }
        // total torque at the driven wheels
        let mut total_drive_torque = if self.frozen || shifting || self.gear == 0 { 0.0 } else { engine_torque * ratio * DRIVELINE_EFF };
        let reflected_inertia = if clutch_slip || shifting { 0.0 } else { spec.engine_inertia * ratio * ratio };

        // Circuit cars: traction-limited torque map (part of the car, like a race ECU's
        // per-gear torque maps, not an assist). Their engines make far more torque in the low
        // gears than slicks can put down, so with an on/off keyboard throttle the rear would
        // light up out of every slow corner. Wheel torque is capped near what the driven axle
        // can transmit (incl. downforce and the grip already used for cornering); lower assist
        // levels leave more surplus so the throttle still has to be respected.
        if spec.class == CarClass::Circuit && total_drive_torque > 0.0 && !self.frozen {
            let vl = v_long_body.max(0.0);
            let df = spec.downforce * vl * vl;
            let wb = spec.cg_to_front + spec.cg_to_rear;
            let (static_load, aero_load, bias) = match spec.drivetrain {
                Drivetrain::Rwd => (spec.mass * GRAVITY * spec.cg_to_front / wb, df * (1.0 - spec.aero_front), spec.rear_grip_bias),
                Drivetrain::Fwd => (spec.mass * GRAVITY * spec.cg_to_rear / wb, df * spec.aero_front, 1.0),
                Drivetrain::Awd => (spec.mass * GRAVITY, df, 1.0),
            };
            let load = static_load + aero_load;
            let ls = load_sensitivity(spec.tyre);
            let mu = spec.tire_grip * bias * (1.0 - ls * (load / static_load.max(1.0) - 1.0)).max(0.6);
            // grip already used sideways (steady-state lateral acceleration = yaw rate x speed)
            let a_max = mu * (spec.mass * GRAVITY + df) / spec.mass;
            let lat_use = (self.ang_vel.dot(up).abs() * speed / a_max.max(1.0)).min(1.0);
            let lon_room = (1.0 - lat_use * lat_use).max(0.0).sqrt().max(0.35);
            let surplus = 0.98 + (1.0 - tc_strength(self.assist_level)) * 0.28;
            let cap = mu * load * spec.wheel_radius * surplus * lon_room;
            total_drive_torque = total_drive_torque.min(cap);
        }

        // assists that follow the player's assist level: traction control, stability control, ABS
        let tc = tc_strength(self.assist_level);
        let abs = abs_strength(self.assist_level);
        if tc > 0.0 && speed > 6.0 && v_long_body > 0.0 && total_drive_torque > 0.0 {
            // stability control: back off the power when the car is sliding beyond a drift angle
            let beta = v_lat_body.atan2(v_long_body);
            // oversteer = rotating faster than the path: yaw rate in the sense of the slide
            let yaw_rate = self.ang_vel.dot(up);
            let lim = 0.2 + (1.0 - tc) * 0.45;
            let rot_into = (-yaw_rate * beta.signum()).max(0.0);
            let cut = ((beta.abs() - lim) / 0.2 + rot_into * 0.3 * tc).clamp(0.0, 1.0) * (0.5 + 0.5 * tc);
            total_drive_torque *= 1.0 - cut;
        }

        // ---------------- wheels
        let inv_i = self.inv_inertia_world();
        let mut force = Vec3::new(0.0, -GRAVITY * spec.mass, 0.0);
        let mut torque = Vec3::ZERO;
        let add_force_at = |f: Vec3, p: Vec3, force: &mut Vec3, torque: &mut Vec3, pos: Vec3| {
            *force += f;
            *torque += (p - pos).cross(f);
        };
        let max_len = spec.susp_rest + spec.wheel_radius;
        let wheel_inertia = 9.0 * spec.wheel_radius * spec.wheel_radius + 0.2;
        let r = spec.wheel_radius;
        let mut on_ground = 0u8;
        let mut max_skid: f32 = 0.0;
        let mut landing: f32 = 0.0;
        let mut compressions = [0.0f32; 4];
        let mut wheelspin: f32 = 0.0;
        let mut lockup: f32 = 0.0;
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
            let front = i < 2;
            let hand = !front && input.handbrake > 0.5;
            let is_driven = shares[i] > 0.0;
            let inertia = wheel_inertia + reflected_inertia * shares[i];
            // brake torque capacity on this wheel (ABS scales it)
            let bias = if front { spec.brake_bias_front } else { 1.0 - spec.brake_bias_front };
            let w = &mut self.wheels[i];
            let mut brake_t = brake * spec.brake_force * bias * 0.5 * r * (1.0 - w.abs_cut);
            if hand {
                brake_t += spec.handbrake_force * 0.5 * r * 1.25;
            }
            let drive_t = total_drive_torque * shares[i] * (1.0 - w.tc_cut);
            w.prev_compression = w.compression;
            w.steer_angle = if front { self.steer } else { 0.0 };
            if !hit {
                w.contact = false;
                w.compression = 0.0;
                w.load = 0.0;
                w.susp_length = spec.susp_rest;
                w.skid *= 0.9;
                w.slide_lat = 0.0;
                w.slide_long = 0.0;
                w.slip_ratio = 0.0;
                w.slip_angle = 0.0;
                w.traction_loss = 0.0;
                w.tc_cut *= 0.9;
                w.abs_cut *= 0.9;
                // free wheel: drive, bearing drag and brakes
                let net = drive_t - w.spin_speed * 1.5;
                w.spin_speed = integrate_wheel(w.spin_speed, inertia, net, 0.0, drive_t, brake_t, dt);
                w.spin_angle = (w.spin_angle + w.spin_speed * dt) % std::f32::consts::TAU;
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
            let (props, surface, q) = wheel_ground(track, contact, self.track_hint);
            if i == 0 {
                self.track_hint = Some(q.index);
            }
            w.surface = surface;
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
            let fz = w.load.min(spec.mass * GRAVITY * 4.0);
            let grip_bias = if front { 1.0 } else { spec.rear_grip_bias };
            // compound vs surface (slicks: great on tarmac, hopeless off it / in the wet) and
            // load sensitivity (grip grows less than linearly with load, so downforce helps
            // but not without bound)
            let wet = if track.conditions.keys.is_empty() { 0.0 } else { track.conditions.at(q.s).wetness };
            let compound = tyre_surface_factor(spec.tyre, &props, wet);
            let fz_nom = spec.mass * GRAVITY * 0.25;
            let load_k = (1.0 - load_sensitivity(spec.tyre) * (fz / fz_nom - 1.0)).clamp(0.55, 1.1);
            let mu = spec.tire_grip * props.grip * grip_bias * compound * load_k;
            let max_f = mu * fz;
            let tyre = Tyre { max_f, peak_alpha: spec.peak_slip_angle, peak_kappa: peak_slip_ratio(&props), slide: props.slide_grip, slide_long: props.slide_grip * 0.94 };
            let alpha = v_lat.atan2(v_long.abs().max(3.5));
            let v_ref = v_long.abs().max(LOW_SPEED_REF);

            // ---- wheel rotational dynamics (implicit, linearised tyre feedback)
            let omega0 = w.spin_speed;
            let kappa0 = (omega0 * r - v_long) / v_ref;
            let (fx0, _, _) = tyre.forces(kappa0, alpha);
            // stiffness of Fx w.r.t. omega in the linear region (upper bound keeps it stable)
            let k_omega = max_f * 2.0 / tyre.peak_kappa * r / v_ref;
            let (fx_lock, _, _) = tyre.forces(-v_long / v_ref, alpha);
            let omega = integrate_wheel(omega0, inertia, drive_t - r * fx0, r * k_omega, drive_t - r * fx_lock, brake_t, dt);
            w.spin_speed = omega;
            w.spin_angle = (w.spin_angle + omega * dt) % std::f32::consts::TAU;
            let kappa = (omega * r - v_long) / v_ref;
            let (mut fx, mut fy, combined) = tyre.forces(kappa, alpha);
            // arcade stability: keep a little lateral authority even when the tyre is spinning
            // or locked (more with assists), so cars stay catchable
            let assist = (spec.assist * self.assist_level).min(1.0);
            let (_, fy_pure, _) = tyre.forces(0.0, alpha);
            let keep = 0.12 + 0.22 * assist;
            if fy.abs() < fy_pure.abs() * keep {
                fy = fy_pure * keep;
            }
            // low speed lateral damping to kill creep
            if v_long.abs() < 2.0 {
                let damp = -v_lat * spec.mass * 0.25 * 6.0;
                let blend = 1.0 - v_long.abs() / 2.0;
                fy = fy * (1.0 - blend) + damp.clamp(-max_f, max_f) * blend;
            }
            // rolling resistance and standstill hold (body forces, not through the wheel)
            let low_speed = (v_long.abs() / 0.6).min(1.0);
            fx -= v_long.signum() * props.rolling * fz * low_speed;
            // static friction for a held wheel (brake at standstill or handbrake-locked) so
            // cars don't creep on slopes through the low-speed slip regularisation
            let held = omega == 0.0 && brake_t > 0.0;
            if (held || (brake > 0.1 && throttle < 0.1)) && v_long.abs() < 0.6 {
                fx = (fx - v_long * spec.mass * 25.0).clamp(-max_f, max_f);
            }
            let tire_force = f_dir * fx + r_dir * fy;
            // apply a little above the contact patch to reduce body roll (arcade stability)
            let apply_at = contact + up * (spec.wheel_radius * 0.9);
            add_force_at(tire_force, apply_at, &mut force, &mut torque, self.pos);

            // ---- traction control / ABS (per wheel, filtered)
            let kp = tyre.peak_kappa;
            // TC is a PI controller on slip ratio. The weak proportional part only *limits*
            // spin on milder settings (bigger surplus torque -> more slip, as in a real car);
            // the integral part tightens over ~a second of continuous spin, so a launch
            // gives a burst of wheelspin that TC then reins in. Cozy clamps hard and fast.
            let spinning_fwd = tc > 0.0 && is_driven && drive_t * kappa > 0.0;
            let tc_target = if spinning_fwd {
                let allow = kp * (1.0 + (1.0 - tc) * 2.5);
                let excess = (kappa.abs() - allow) / kp;
                let p_cut = excess / (1.2 + (1.0 - tc) * 16.0);
                // integral settles the tyre just past its peak (efficient, no visible spin)
                // rate-limited, so the time to rein in a spin grows with how much cut the
                // surface needs (longer on snow than on gravel)
                let ki = 0.15 + 1.25 * tc * tc;
                let i_err = (kappa.abs() / kp - (1.15 + 0.3 * (1.0 - tc))).clamp(-0.6, 1.0);
                w.tc_i = (w.tc_i + dt * ki * i_err).clamp(0.0, 1.0);
                // combined slip while cornering: tyre past its peak because of steering + drive
                let sy = alpha / spec.peak_slip_angle;
                let lat_frac = if combined > 1e-3 { (sy / combined).powi(2) } else { 0.0 };
                // (relaxed on lower assist levels so power oversteer can happen: Normal lets the rear
                // step out into a catchable slide, Cozy keeps it tidy)
                let by_combined = (combined - (1.0 + (1.0 - tc) * 2.6)) / 0.5 * lat_frac;
                (p_cut + w.tc_i).max(by_combined).clamp(0.0, 1.0) * (0.6 + 0.4 * tc)
            } else {
                w.tc_i = (w.tc_i - dt * 1.5).max(0.0);
                0.0
            };
            let rate = if tc_target > w.tc_cut { 35.0 } else { 10.0 };
            w.tc_cut += (tc_target - w.tc_cut) * (1.0 - (-dt * rate).exp());
            let abs_target = if abs > 0.0 && brake > 0.05 && kappa * v_long < 0.0 && v_long.abs() > 1.5 {
                let allow = kp * (1.0 + (1.0 - abs) * 4.0);
                ((kappa.abs() - allow) / kp).clamp(0.0, 1.0) * (0.5 + 0.45 * abs)
            } else {
                0.0
            };
            let rate = if abs_target > w.abs_cut { 40.0 } else { 12.0 };
            w.abs_cut += (abs_target - w.abs_cut) * (1.0 - (-dt * rate).exp());

            // ---- telemetry for FX / audio / HUD
            w.slip_ratio = kappa;
            w.slip_angle = alpha;
            w.peak_slip_ratio = tyre.peak_kappa;
            w.peak_slip_angle = tyre.peak_alpha;
            w.traction_loss = ((combined - 1.0) / 1.5).clamp(0.0, 1.0);
            let x = alpha.abs() / spec.peak_slip_angle;
            let slide = if x > 1.0 { v_lat.abs() } else { 0.0 };
            w.slide_lat = slide;
            w.slide_long = (omega * r - v_long).abs();
            if is_driven && kappa * v_long.signum() > 0.0 && drive_t.abs() > 1.0 {
                wheelspin = wheelspin.max(((kappa.abs() - kp) / (kp * 2.5)).clamp(0.0, 1.0));
            }
            if kappa * v_long < 0.0 && v_long.abs() > 2.0 {
                lockup = lockup.max(((kappa.abs() - kp) / (1.0 - kp).max(0.1)).clamp(0.0, 1.0));
            }
            let skid_amt = ((slide - 1.0) / 6.0).clamp(0.0, 1.0).max(((w.slide_long - 2.0) / 8.0).clamp(0.0, 1.0));
            w.skid += (skid_amt - w.skid) * (1.0 - (-dt * 12.0).exp());
            max_skid = max_skid.max(w.skid);
        }
        // limited-slip differentials: pull the wheels of each driven axle together
        if spec.lsd > 0.0 {
            let k = 1.0 - (-dt * spec.lsd * 60.0).exp();
            for (l, rr) in [(0usize, 1usize), (2, 3)] {
                if shares[l] > 0.0 {
                    let m = 0.5 * (self.wheels[l].spin_speed + self.wheels[rr].spin_speed);
                    self.wheels[l].spin_speed += (m - self.wheels[l].spin_speed) * k;
                    self.wheels[rr].spin_speed += (m - self.wheels[rr].spin_speed) * k;
                }
            }
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
            force -= self.vel.normalize() * spec.drag * self.drag_scale * v2;
        }
        if on_ground > 0 {
            // downforce at the axles: the front/rear balance changes the handling with speed
            let df = spec.downforce * v2;
            let pf = self.pos + self.rot * Vec3::new(0.0, spec.mount_height, -spec.cg_to_front);
            let pr = self.pos + self.rot * Vec3::new(0.0, spec.mount_height, spec.cg_to_rear);
            add_force_at(-up * df * spec.aero_front, pf, &mut force, &mut torque, self.pos);
            add_force_at(-up * df * (1.0 - spec.aero_front), pr, &mut force, &mut torque, self.pos);
        }

        // stability assist: damp yaw when not steering hard; gentle body-slip limiter
        if on_ground >= 2 {
            let yaw_rate = self.ang_vel.dot(up);
            let steer_input = input.steer.abs();
            let yaw_damp = spec.assist * self.assist_level * (0.25 + 0.6 * (1.0 - steer_input)) * 0.45;
            torque -= up * yaw_rate * yaw_damp * spec.mass * 1.2;
        }

        // electronic stability control (strength follows the assist level): when the car is
        // sliding past a drift angle and still rotating into the slide, apply a corrective yaw
        // moment (what per-wheel ESC braking achieves). Off while handbrake-drifting.
        if input.handbrake > 0.5 {
            self.handbrake_age = 0.0;
        } else {
            self.handbrake_age += dt;
        }
        if tc > 0.0 && on_ground >= 2 && speed > 5.0 && v_long_body > 0.0 {
            let beta = v_lat_body.atan2(v_long_body);
            let lim = 0.12 + (1.0 - tc) * 0.45;
            let yaw_rate = self.ang_vel.dot(up);
            // positive when the yaw rate increases the slip angle (oversteer)
            let rot_into = (-yaw_rate * beta.signum()).max(0.0);
            let excess = (beta.abs() - lim).max(0.0);
            let hb_fade = ((self.handbrake_age - 0.4) / 0.8).clamp(0.0, 1.0);
            if excess > 0.0 {
                // progressive: lets a slide develop, firmly stops it growing past ~25 deg
                let m = (excess * 22.0 + excess * excess * 90.0 + rot_into * excess.min(0.2) * 25.0) * spec.mass * tc * hb_fade;
                torque -= up * beta.signum() * m;
            }
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

        // air control: level the car, a little player yaw (rate-limited, not accumulating),
        // and with assists gently align the nose with the direction of travel so landings
        // are straight
        let assist_a = (spec.assist * self.assist_level).min(1.0);
        if on_ground == 0 {
            let (_, n) = track.terrain.sample(self.pos.x, self.pos.z);
            let level = up.cross(Vec3::Y.lerp(n, 0.5).normalize());
            let k = spec.mass * 6.0;
            torque += level * k * 2.0;
            torque -= (self.ang_vel - up * self.ang_vel.dot(up)) * k * 0.6;
            let yaw_rate = self.ang_vel.dot(up);
            let mut want = -steer_in * 0.6;
            let vh = Vec3::new(self.vel.x, 0.0, self.vel.z);
            let fh = Vec3::new(fwd.x, 0.0, fwd.z);
            if vh.length() > 5.0 && fh.length() > 0.3 {
                // signed angle of travel relative to the nose (+ = velocity to the right)
                let psi = fh.normalize().cross(vh.normalize()).y.asin() * -1.0;
                want += -psi * 2.0 * assist_a;
            }
            torque += up * (want - yaw_rate) * spec.mass * 2.5;
            self.landing_timer = 0.0;
        } else if self.telemetry.airtime > 0.08 {
            // just landed after a real jump: short stability window
            self.landing_timer = 0.5;
        }
        if on_ground >= 2 && self.landing_timer > 0.0 {
            // yaw tracks what the steering asks for (kinematic, grip-limited) for a moment,
            // so a slightly crossed-up or one-wheel-first landing doesn't snap into a spin
            let wb = spec.cg_to_front + spec.cg_to_rear;
            let v = v_long_body.max(0.0);
            let max_r = 9.0 / v.max(3.0);
            let want = (-v * self.steer.tan() / wb).clamp(-max_r, max_r);
            let yaw_rate = self.ang_vel.dot(up);
            let f = (self.landing_timer / 0.5) * assist_a;
            torque += up * (want - yaw_rate) * spec.mass * 12.0 * f;
            self.landing_timer = (self.landing_timer - dt).max(0.0);
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
        let mut scrape = 0.0f32;
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
                // the floor (lower corners) touching down at speed = sparks
                if c.y < 0.0 && speed > 6.0 {
                    scrape = scrape.max(((speed - 6.0) / 25.0).min(1.0) * (depth / 0.02).min(1.0));
                }
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
        t.wheelspin = wheelspin;
        t.floor_scrape = scrape;
        t.lockup = lockup;
        t.shifting = shifting;
        t.engine_load = (eff_throttle * (1.0 - self.rpm / spec.redline * 0.3)).clamp(0.0, 1.0);
        t.in_water = in_water;
    }

    /// Road-wheel angle the steering system aims for with `steer_in` (-1..1), including the
    /// assists: counter-steer and the slip-angle limiter (full input = front tyres at about
    /// their peak slip angle). Monotonic in `steer_in` (the AI inverts it).
    pub fn steer_target(&self, steer_in: f32) -> f32 {
        let spec = &self.spec;
        let steer_in = steer_in.clamp(-1.0, 1.0);
        let fwd = self.forward();
        let right = self.right();
        let speed = self.vel.length();
        let v_long_body = self.vel.dot(fwd);
        let v_lat_body = self.vel.dot(right);
        let speed_factor = 1.0 / (1.0 + (speed / 22.0).powi(2) * 0.9);
        let _ = v_lat_body;
        // The wheel angle follows the driver's input only: no automatic counter-steer (that
        // must come from the driver). Assists stay invisible (yaw torque / traction control).
        // Sim: raw angle with speed sensitivity only
        let mut target = (steer_in * spec.max_steer * speed_factor.max(0.22)).clamp(-spec.max_steer, spec.max_steer);
        // Slip-angle steering limiter (Forza-style): full input puts the front tyres at about
        // their peak slip angle relative to the front axle's actual direction of travel, so
        // the fronts never plough past the peak. It can only *reduce* the angle the driver
        // asks for, never flip its sign or add steering: centred input = straight wheels, and
        // full opposite lock in a slide reaches peak slip beyond the direction of travel
        // (the angle that catches it).
        if self.assist_level > 0.2 && self.assist_level < 0.8 {
            // Sport: the wheels follow the input exactly (no dependence on the slide, so they
            // never straighten or counter by themselves). Full input is capped by a fixed,
            // speed-only angle: the geometric angle for the car's peak cornering at this
            // speed plus the tyres' peak slip angle.
            let k = steer_limit_factor(self.assist_level).unwrap_or(1.1);
            let v = v_long_body.abs().max(1.0);
            let wb = spec.cg_to_front + spec.cg_to_rear;
            let a_max = GRAVITY * spec.tire_grip * (1.0 + spec.downforce * v * v / (spec.mass * GRAVITY));
            let mut cap = (wb * a_max / (v * v) + k * spec.peak_slip_angle).min(spec.max_steer);
            // countersteer room: steering toward the direction of travel (catching / holding a
            // drift) may go as far past the static cap as the front axle is sliding, so a
            // drift can be held with opposite lock. Steering into the slide is never reduced.
            if v_long_body > 1.0 {
                let v_f = self.vel + self.ang_vel.cross(fwd * spec.cg_to_front);
                let theta = v_f.dot(right).atan2(v_f.dot(fwd).max(0.5));
                cap = (cap + (steer_in.signum() * theta).max(0.0)).min(spec.max_steer);
            }
            let w = ((speed - 3.0) / 5.0).clamp(0.0, 1.0);
            let low = steer_in * spec.max_steer;
            return (low + (steer_in * cap - low) * w).clamp(-spec.max_steer, spec.max_steer);
        }
        if let Some(k) = steer_limit_factor(self.assist_level) {
            if v_long_body > 1.0 {
                let v_f = self.vel + self.ang_vel.cross(fwd * spec.cg_to_front);
                let theta = v_f.dot(right).atan2(v_f.dot(fwd).max(0.5));
                let half = k * spec.peak_slip_angle;
                let limited = if steer_in >= 0.0 { steer_in * (theta + half).max(0.0) } else { steer_in * (half - theta).max(0.0) };
                // full lock stays available when parking / turning around
                let w = ((speed - 3.0) / 5.0).clamp(0.0, 1.0);
                let low = steer_in * spec.max_steer;
                target = (low + (limited - low) * w).clamp(-spec.max_steer, spec.max_steer);
            }
        }
        target
    }

    /// Direction of travel of the front axle relative to the car's heading (rad, + = right).
    /// A front wheel steered to this angle runs at zero slip.
    pub fn front_travel_angle(&self) -> f32 {
        let fwd = self.forward();
        let v_f = self.vel + self.ang_vel.cross(fwd * self.spec.cg_to_front);
        v_f.dot(self.right()).atan2(v_f.dot(fwd).max(0.5))
    }

    /// Steering input (-1..1) that makes `steer_target` closest to `angle` (rad)
    pub fn steer_input_for_angle(&self, angle: f32) -> f32 {
        let (mut lo, mut hi) = (-1.0f32, 1.0f32);
        if self.steer_target(hi) <= angle {
            return 1.0;
        }
        if self.steer_target(lo) >= angle {
            return -1.0;
        }
        for _ in 0..14 {
            let mid = 0.5 * (lo + hi);
            if self.steer_target(mid) < angle {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
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

/// Driveline efficiency (gearbox + diff losses)
pub const DRIVELINE_EFF: f32 = 0.88;
/// Reference speed below which slip ratios are computed against this speed (m/s); keeps the
/// tyre model well-behaved at standstill.
pub const LOW_SPEED_REF: f32 = 3.0;

/// Fraction of drive torque sent to each wheel (FL, FR, RL, RR)
pub fn drive_shares(spec: &CarSpec) -> [f32; 4] {
    match spec.drivetrain {
        Drivetrain::Fwd => [0.5, 0.5, 0.0, 0.0],
        Drivetrain::Rwd => [0.0, 0.0, 0.5, 0.5],
        Drivetrain::Awd => {
            let f = spec.awd_front_split * 0.5;
            let r = (1.0 - spec.awd_front_split) * 0.5;
            [f, f, r, r]
        }
    }
}

/// Grip multiplier of a tyre compound on a surface. Rally tyres: 1. Slicks: full grip on
/// tarmac (and kerbs), collapsing on loose ground (gravel/grass/sand/snow/mud ~x0.4, ice
/// lower) and losing much more than treaded tyres in the wet (~x0.55 overall at full wet
/// together with the surface's own wet factor).
pub fn tyre_surface_factor(tyre: TyreType, p: &SurfaceProps, wetness: f32) -> f32 {
    match tyre {
        TyreType::Rally => 1.0,
        TyreType::Slick => {
            let loose = p.looseness();
            let icy = ((0.5 - p.grip) / 0.2).clamp(0.0, 1.0);
            (1.0 - 0.68 * loose - 0.4 * icy).max(0.28) * (1.0 - 0.24 * wetness.clamp(0.0, 1.0))
        }
    }
}

/// How much tyre friction falls as load rises above the static load (per unit of extra load)
pub fn load_sensitivity(tyre: TyreType) -> f32 {
    match tyre {
        TyreType::Rally => 0.0,
        TyreType::Slick => 0.07,
    }
}

/// Steering limiter: fraction of the tyre's peak slip angle that full steering input uses,
/// or None (Sim: raw steering angle). Cozy 0.85, Normal 0.9, Sport ~1.1.
pub fn steer_limit_factor(assist_level: f32) -> Option<f32> {
    if assist_level <= 0.2 {
        None
    } else if assist_level <= 1.0 {
        Some(1.25 - 0.35 * (assist_level - 0.2) / 0.8)
    } else {
        Some((0.9 - (assist_level - 1.0) * 0.17).max(0.8))
    }
}

/// Traction-control strength for an assist level (Sim 0 -> off, Sport minimal, Normal mild,
/// Cozy strong)
pub fn tc_strength(assist_level: f32) -> f32 {
    if assist_level <= 0.2 {
        0.0
    } else {
        ((assist_level - 0.2) / 1.1).clamp(0.0, 1.0).powf(1.3)
    }
}

/// ABS strength for an assist level (Sim -> off: wheels can lock)
pub fn abs_strength(assist_level: f32) -> f32 {
    if assist_level <= 0.2 {
        0.0
    } else {
        ((assist_level - 0.2) / 1.1).clamp(0.0, 1.0).powf(1.1)
    }
}

/// Ground under one wheel: (surface properties, dominant surface for FX, track query).
/// The ONLY place the tyre model reads the surface, so blended surfaces can be plugged in here
/// (grip must vary continuously as each wheel crosses a transition).
#[inline]
pub fn wheel_ground(track: &Track, contact: Vec3, hint: Option<usize>) -> (SurfaceProps, Option<Surface>, TrackQuery) {
    let g = track.ground_at(contact, hint);
    // ground_at already blends surface + weather (wet grip, puddles)
    (g.props, Some(g.primary), g.query)
}

/// Slip ratio at peak longitudinal friction. A continuous function of the surface properties
/// (so it blends across surface transitions): ~0.10 on tarmac, ~0.15 on loose surfaces where
/// the tyre digs in, lower on ice.
pub fn peak_slip_ratio(p: &SurfaceProps) -> f32 {
    let loose = p.looseness();
    let icy = ((0.5 - p.grip) / 0.2).clamp(0.0, 1.0);
    0.10 + 0.05 * loose - 0.02 * icy
}

/// Combined-slip tyre: normalised slip vector (slip ratio / peak, slip angle / peak) through
/// one friction curve, so wheelspin or lock-up eats lateral grip and vice versa.
#[derive(Clone, Copy, Debug)]
pub struct Tyre {
    /// Peak friction force (mu * Fz)
    pub max_f: f32,
    pub peak_alpha: f32,
    pub peak_kappa: f32,
    /// Fraction of peak left when fully sliding sideways / spinning
    pub slide: f32,
    pub slide_long: f32,
}

impl Tyre {
    /// (longitudinal force, lateral force, combined normalised slip)
    pub fn forces(&self, kappa: f32, alpha: f32) -> (f32, f32, f32) {
        let sx = kappa / self.peak_kappa;
        let sy = alpha / self.peak_alpha;
        let s2 = sx * sx + sy * sy;
        if s2 < 1e-12 {
            return (0.0, 0.0, 0.0);
        }
        let s = s2.sqrt();
        let f = if s < 1.0 {
            s * (2.0 - s)
        } else {
            let slide = (sx * sx * self.slide_long + sy * sy * self.slide) / s2;
            slide + (1.0 - slide) * (-(s - 1.0) * 1.2).exp()
        };
        let mag = self.max_f * f;
        (mag * sx / s, -mag * sy / s, s)
    }
}

/// One implicit step of wheel spin dynamics.
/// `net`: drive minus tyre torque at the current speed; `k`: d(tyre torque)/d(omega) (>= 0);
/// `net_locked`: net torque if the wheel were stopped; `brake`: brake torque capacity.
/// The brake is Coulomb friction: it can hold the wheel still but never reverses it.
fn integrate_wheel(omega: f32, inertia: f32, net: f32, k: f32, net_locked: f32, brake: f32, dt: f32) -> f32 {
    let hold_band = 0.3;
    if omega.abs() <= hold_band && net_locked.abs() <= brake {
        return 0.0;
    }
    let sgn = if omega.abs() > hold_band { omega.signum() } else { net_locked.signum() };
    let w1 = omega + dt * (net - brake * sgn) / (inertia + dt * k);
    if brake > 0.0 && w1 * sgn < 0.0 && omega * sgn >= 0.0 {
        return 0.0;
    }
    if w1.is_finite() { w1 } else { 0.0 }
}
