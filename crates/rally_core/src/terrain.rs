//! Regular-grid heightfield. Physics and rendering both use the exact same triangles
//! (each cell split along the (0,0)-(1,1) diagonal), so wheels sit exactly on the mesh.
use glam::{Vec2, Vec3};

#[derive(Clone, Debug)]
pub struct Heightfield {
    /// World position of vertex (0,0) in XZ
    pub origin: Vec2,
    pub cell: f32,
    pub nx: usize,
    pub nz: usize,
    pub heights: Vec<f32>,
}

impl Heightfield {
    pub fn new(origin: Vec2, cell: f32, nx: usize, nz: usize) -> Self {
        Heightfield { origin, cell, nx, nz, heights: vec![0.0; nx * nz] }
    }

    #[inline]
    pub fn idx(&self, ix: usize, iz: usize) -> usize {
        iz * self.nx + ix
    }

    #[inline]
    pub fn get(&self, ix: usize, iz: usize) -> f32 {
        self.heights[self.idx(ix.min(self.nx - 1), iz.min(self.nz - 1))]
    }

    pub fn vertex_pos(&self, ix: usize, iz: usize) -> Vec3 {
        Vec3::new(
            self.origin.x + ix as f32 * self.cell,
            self.get(ix, iz),
            self.origin.y + iz as f32 * self.cell,
        )
    }

    pub fn size(&self) -> Vec2 {
        Vec2::new((self.nx - 1) as f32 * self.cell, (self.nz - 1) as f32 * self.cell)
    }

    pub fn contains(&self, x: f32, z: f32) -> bool {
        let s = self.size();
        x >= self.origin.x && z >= self.origin.y && x <= self.origin.x + s.x && z <= self.origin.y + s.y
    }

    /// Height and (unnormalized-free) surface normal at world XZ.
    pub fn sample(&self, x: f32, z: f32) -> (f32, Vec3) {
        let gx = ((x - self.origin.x) / self.cell).clamp(0.0, (self.nx - 1) as f32 - 0.0001);
        let gz = ((z - self.origin.y) / self.cell).clamp(0.0, (self.nz - 1) as f32 - 0.0001);
        let ix = gx.floor() as usize;
        let iz = gz.floor() as usize;
        let fx = gx - ix as f32;
        let fz = gz - iz as f32;
        let h00 = self.get(ix, iz);
        let h10 = self.get(ix + 1, iz);
        let h01 = self.get(ix, iz + 1);
        let h11 = self.get(ix + 1, iz + 1);
        let c = self.cell;
        if fx >= fz {
            // triangle (0,0) (1,0) (1,1)
            let h = h00 + (h10 - h00) * fx + (h11 - h10) * fz;
            let n = Vec3::new(-(h10 - h00) / c, 1.0, -(h11 - h10) / c).normalize();
            (h, n)
        } else {
            // triangle (0,0) (1,1) (0,1)
            let h = h00 + (h11 - h01) * fx + (h01 - h00) * fz;
            let n = Vec3::new(-(h11 - h01) / c, 1.0, -(h01 - h00) / c).normalize();
            (h, n)
        }
    }

    pub fn height(&self, x: f32, z: f32) -> f32 {
        self.sample(x, z).0
    }
}
