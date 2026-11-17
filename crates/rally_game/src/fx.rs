//! Visual effects (particles, skid marks, ambient life). Stub: owned by RaceSession.
use crate::render::gpu::{Gpu, View};
use rally_core::world::{World, WorldEvent};

#[derive(Default)]
pub struct Fx {}

impl Fx {
    pub fn new(_world: &World) -> Fx {
        Fx {}
    }
    /// Called once per frame after the world advanced. `alpha` = render interpolation.
    pub fn update(&mut self, _world: &World, _events: &[WorldEvent], _alpha: f32, _dt: f32) {}
    /// Called per viewport inside the 3D pass, after opaque geometry and cars.
    pub fn draw(&self, _gpu: &mut Gpu, _view: &View, _env: &crate::render::gpu::Environment) {}
}
