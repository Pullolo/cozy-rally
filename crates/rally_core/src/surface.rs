//! Ground surface types and their tire/handling properties.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Surface {
    Asphalt,
    Gravel,
    Dirt,
    Grass,
    Sand,
    Snow,
    Ice,
    Mud,
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceProps {
    /// Peak friction coefficient multiplier
    pub grip: f32,
    /// How much of the peak grip remains when fully sliding (lower = slidier)
    pub slide_grip: f32,
    /// Rolling resistance coefficient (force = coeff * Fz)
    pub rolling: f32,
    /// Random bumpiness amplitude (m) used for suspension noise
    pub bumpiness: f32,
    /// Particle/dust colour (linear 0..1)
    pub dust: [f32; 3],
    /// Whether wheels throw debris (gravel/dirt/snow)
    pub loose: bool,
}

impl SurfaceProps {
    /// Linear blend between two surfaces' properties (t = weight of `b`). `loose` follows the
    /// dominant side.
    pub fn lerp(a: SurfaceProps, b: SurfaceProps, t: f32) -> SurfaceProps {
        let t = t.clamp(0.0, 1.0);
        let l = |x: f32, y: f32| x + (y - x) * t;
        SurfaceProps {
            grip: l(a.grip, b.grip),
            slide_grip: l(a.slide_grip, b.slide_grip),
            rolling: l(a.rolling, b.rolling),
            bumpiness: l(a.bumpiness, b.bumpiness),
            dust: [l(a.dust[0], b.dust[0]), l(a.dust[1], b.dust[1]), l(a.dust[2], b.dust[2])],
            loose: if t < 0.5 { a.loose } else { b.loose },
        }
    }
}

impl Surface {
    pub fn props(self) -> SurfaceProps {
        use Surface::*;
        match self {
            Asphalt => SurfaceProps { grip: 1.05, slide_grip: 0.80, rolling: 0.012, bumpiness: 0.0, dust: [0.55, 0.55, 0.58], loose: false },
            Gravel => SurfaceProps { grip: 0.82, slide_grip: 0.78, rolling: 0.020, bumpiness: 0.012, dust: [0.76, 0.66, 0.50], loose: true },
            Dirt => SurfaceProps { grip: 0.78, slide_grip: 0.76, rolling: 0.025, bumpiness: 0.018, dust: [0.60, 0.45, 0.30], loose: true },
            Grass => SurfaceProps { grip: 0.62, slide_grip: 0.75, rolling: 0.060, bumpiness: 0.02, dust: [0.45, 0.62, 0.30], loose: true },
            Sand => SurfaceProps { grip: 0.62, slide_grip: 0.80, rolling: 0.090, bumpiness: 0.01, dust: [0.93, 0.82, 0.58], loose: true },
            Snow => SurfaceProps { grip: 0.58, slide_grip: 0.82, rolling: 0.035, bumpiness: 0.008, dust: [0.95, 0.97, 1.0], loose: true },
            Ice => SurfaceProps { grip: 0.30, slide_grip: 0.85, rolling: 0.010, bumpiness: 0.0, dust: [0.85, 0.92, 1.0], loose: false },
            Mud => SurfaceProps { grip: 0.55, slide_grip: 0.80, rolling: 0.11, bumpiness: 0.025, dust: [0.35, 0.26, 0.18], loose: true },
        }
    }

    pub fn name(self) -> &'static str {
        use Surface::*;
        match self {
            Asphalt => "Asphalt",
            Gravel => "Gravel",
            Dirt => "Dirt",
            Grass => "Grass",
            Sand => "Sand",
            Snow => "Snow",
            Ice => "Ice",
            Mud => "Mud",
        }
    }
}
