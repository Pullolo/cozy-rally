//! Distance-to-road field: a coarse grid (default 10 m cells) holding the approximate XZ
//! distance from each cell to the nearest road centre-line sample.
//!
//! Generation uses it to spend full-resolution terrain work only in a corridor around the
//! road (long point-to-point stages), props use it to stay near the road, and renderers can
//! use it for corridor meshes / LOD / culling (`Track::road_distance(x, z)`).
use glam::Vec2;

#[derive(Clone, Debug, Default)]
pub struct RoadDistanceField {
    /// World XZ of cell (0,0)
    pub origin: Vec2,
    pub cell: f32,
    pub nx: usize,
    pub nz: usize,
    /// Row-major distances (m), saturated at `max_dist`
    pub dist: Vec<f32>,
    pub max_dist: f32,
}

impl RoadDistanceField {
    /// Chamfer distance transform seeded with the exact distance of cells near the road.
    pub fn build(road: &[Vec2], min: Vec2, max: Vec2, cell: f32, max_dist: f32) -> Self {
        let nx = ((max.x - min.x) / cell).ceil() as usize + 1;
        let nz = ((max.y - min.y) / cell).ceil() as usize + 1;
        let mut dist = vec![max_dist; nx * nz];
        // seed: exact distances within 2 cells of every road sample
        for p in road {
            let cx = ((p.x - min.x) / cell).round() as isize;
            let cz = ((p.y - min.y) / cell).round() as isize;
            for z in cz - 2..=cz + 2 {
                for x in cx - 2..=cx + 2 {
                    if x < 0 || z < 0 || x >= nx as isize || z >= nz as isize {
                        continue;
                    }
                    let c = Vec2::new(min.x + x as f32 * cell, min.y + z as f32 * cell);
                    let i = z as usize * nx + x as usize;
                    dist[i] = dist[i].min(c.distance(*p));
                }
            }
        }
        let (a, b) = (cell, cell * std::f32::consts::SQRT_2);
        // forward pass
        for z in 0..nz {
            for x in 0..nx {
                let i = z * nx + x;
                let mut d = dist[i];
                if x > 0 {
                    d = d.min(dist[i - 1] + a);
                }
                if z > 0 {
                    d = d.min(dist[i - nx] + a);
                    if x > 0 {
                        d = d.min(dist[i - nx - 1] + b);
                    }
                    if x + 1 < nx {
                        d = d.min(dist[i - nx + 1] + b);
                    }
                }
                dist[i] = d.min(max_dist);
            }
        }
        // backward pass
        for z in (0..nz).rev() {
            for x in (0..nx).rev() {
                let i = z * nx + x;
                let mut d = dist[i];
                if x + 1 < nx {
                    d = d.min(dist[i + 1] + a);
                }
                if z + 1 < nz {
                    d = d.min(dist[i + nx] + a);
                    if x + 1 < nx {
                        d = d.min(dist[i + nx + 1] + b);
                    }
                    if x > 0 {
                        d = d.min(dist[i + nx - 1] + b);
                    }
                }
                dist[i] = d.min(max_dist);
            }
        }
        RoadDistanceField { origin: min, cell, nx, nz, dist, max_dist }
    }

    /// Bilinear distance to the road (m); `max_dist` outside the field.
    pub fn at(&self, x: f32, z: f32) -> f32 {
        if self.dist.is_empty() {
            return 0.0;
        }
        let gx = (x - self.origin.x) / self.cell;
        let gz = (z - self.origin.y) / self.cell;
        if gx < 0.0 || gz < 0.0 || gx > (self.nx - 1) as f32 || gz > (self.nz - 1) as f32 {
            return self.max_dist;
        }
        let ix = (gx.floor() as usize).min(self.nx - 2);
        let iz = (gz.floor() as usize).min(self.nz - 2);
        let fx = gx - ix as f32;
        let fz = gz - iz as f32;
        let d = |x: usize, z: usize| self.dist[z * self.nx + x];
        let a = d(ix, iz) + (d(ix + 1, iz) - d(ix, iz)) * fx;
        let b = d(ix, iz + 1) + (d(ix + 1, iz + 1) - d(ix, iz + 1)) * fx;
        a + (b - a) * fz
    }
}
