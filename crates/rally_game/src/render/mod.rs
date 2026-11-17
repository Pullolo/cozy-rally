pub mod gpu;
pub mod mesh;
pub mod post;
pub mod scene;
pub mod shaders;
pub mod shadow;

use std::sync::atomic::{AtomicU8, Ordering};

/// Render quality preset. `Low` draws straight to the screen (window MSAA, no shadow map,
/// no post-processing) and is the safe fallback for weak GPUs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    /// No shadow map, no post-processing
    Low,
    /// 1024 shadow map, bloom + grading + FXAA
    Medium,
    /// 2048 shadow map, bloom + grading, MSAA offscreen target when supported (FXAA otherwise)
    High,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Low, Quality::Medium, Quality::High];

    pub fn name(self) -> &'static str {
        match self {
            Quality::Low => "Low",
            Quality::Medium => "Medium",
            Quality::High => "High",
        }
    }

    pub fn next(self) -> Quality {
        match self {
            Quality::Low => Quality::Medium,
            Quality::Medium => Quality::High,
            Quality::High => Quality::Low,
        }
    }

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
            Quality::High => 2048,
        }
    }
}

static QUALITY: AtomicU8 = AtomicU8::new(u8::MAX);

/// Current render quality. Defaults to High; `COZY_QUALITY=low|medium|high` overrides at startup.
pub fn quality() -> Quality {
    let q = QUALITY.load(Ordering::Relaxed);
    if q == u8::MAX {
        let q = match std::env::var("COZY_QUALITY").ok().as_deref().map(|s| s.to_ascii_lowercase()) {
            Some(ref s) if s == "low" || s == "0" => Quality::Low,
            Some(ref s) if s == "medium" || s == "1" => Quality::Medium,
            _ => Quality::High,
        };
        set_quality(q);
        return q;
    }
    match q {
        0 => Quality::Low,
        1 => Quality::Medium,
        _ => Quality::High,
    }
}

/// Change render quality at runtime (takes effect next frame; GPU targets are (re)built lazily).
pub fn set_quality(q: Quality) {
    QUALITY.store(
        match q {
            Quality::Low => 0,
            Quality::Medium => 1,
            Quality::High => 2,
        },
        Ordering::Relaxed,
    );
}
