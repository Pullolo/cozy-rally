//! Audio facade. (Stub: the procedural synth implementation lives behind this API.)
use rally_core::world::WorldEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiSound {
    Move,
    Select,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Music {
    Menu,
    Race,
    Results,
    None,
}

/// Per-car data the engine/tyre sounds need each frame
#[derive(Clone, Copy, Debug, Default)]
pub struct CarSound {
    pub rpm: f32,
    pub redline: f32,
    pub throttle: f32,
    pub speed: f32,
    pub skid: f32,
    pub loose_surface: bool,
    pub on_ground: bool,
    /// Distance to the closest local listener (m)
    pub distance: f32,
    /// -1 (left) .. 1 (right) relative to the listener
    pub pan: f32,
    pub is_local_player: bool,
    pub engine_kind: u8,
}

pub fn init() {}
pub fn ui_sound(_s: UiSound) {}
pub fn set_music(_m: Music) {}
pub fn set_volumes(_master: f32, _music: f32, _sfx: f32) {}
pub fn update_cars(_cars: &[CarSound], _dt: f32) {}
pub fn world_event(_e: &WorldEvent, _distance: f32) {}
pub fn stop_cars() {}
