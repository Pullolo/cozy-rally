//! Render quality preset (kept free of game dependencies so tools/examples can use gpu.rs).

/// Render quality preset. `Low` draws straight to the screen (window MSAA, no shadow map,
/// no post-processing) and is the safe fallback for weak GPUs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// No shadow map, no post-processing
    Low,
    /// 1024 shadow map, bloom + grading + FXAA
    Medium,
    /// 3072 shadow map, bloom + grading, MSAA offscreen target when supported (FXAA otherwise)
    High,
}

impl Quality {
    pub fn shadows(self) -> bool {
        self != Quality::Low
    }

    pub fn post(self) -> bool {
        self != Quality::Low
    }

    pub fn shadow_size(self) -> u32 {
        match self {
            Quality::Low => 0,
            Quality::Medium => 1024,
            Quality::High => 3072,
        }
    }
}
