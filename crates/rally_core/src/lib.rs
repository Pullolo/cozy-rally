//! Cozy Rally core: headless simulation (physics, track generation, race rules, AI,
//! network protocol). No rendering or platform dependencies.
pub mod ai;
pub mod biome;
pub mod input;
pub mod layout;
pub mod noise;
pub mod pacenotes;
pub mod race;
pub mod rng;
pub mod surface;
pub mod surfacemap;
pub mod terrain;
pub mod track;
pub mod vehicle;
pub mod world;

pub use glam;
