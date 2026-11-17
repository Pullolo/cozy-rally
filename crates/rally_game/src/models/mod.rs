//! Procedural low-poly models (cars, wheels, props). Everything is built from code.
mod car;
pub mod parts;
mod props;

pub use car::{car_model, wheel_mesh, CarModel};
pub use props::{add_prop, catalogue_kinds};

/// Car paint palette (players can pick)
pub const PAINTS: [[f32; 3]; 8] = [
    [0.95, 0.45, 0.40],
    [0.35, 0.62, 0.95],
    [0.98, 0.80, 0.30],
    [0.45, 0.75, 0.48],
    [0.75, 0.52, 0.90],
    [0.98, 0.62, 0.30],
    [0.95, 0.95, 0.92],
    [0.30, 0.78, 0.80],
];

