use serde::{Deserialize, Serialize};

/// Normalised driver controls. Produced by keyboard / touch / gamepad / AI / network.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CarInput {
    /// 0..1
    pub throttle: f32,
    /// 0..1
    pub brake: f32,
    /// -1 (left) .. 1 (right)
    pub steer: f32,
    /// 0..1
    pub handbrake: f32,
    /// Request respawn on track
    pub reset: bool,
}
