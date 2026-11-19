pub mod bench;
pub mod debug;
pub use debug::{debug_skip, perf_legacy};
pub mod gpu;
pub mod mesh;
pub mod perf;
pub mod quality;
pub mod post;
pub mod scene;
pub mod shaders;
pub mod shadow;
pub mod weather;

pub use quality::Quality;
use std::sync::OnceLock;

static ENV_QUALITY: OnceLock<Option<Quality>> = OnceLock::new();

/// Current render quality: follows the player's setting (`settings::quality()`, Low/High).
/// `COZY_QUALITY=low|medium|high` overrides it for testing (Medium is only reachable this way).
pub fn quality() -> Quality {
    let env = ENV_QUALITY.get_or_init(|| match std::env::var("COZY_QUALITY").ok().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("low") | Some("0") => Some(Quality::Low),
        Some("medium") | Some("1") => Some(Quality::Medium),
        Some("high") | Some("2") => Some(Quality::High),
        _ => None,
    });
    if let Some(q) = env {
        return *q;
    }
    match crate::settings::quality() {
        crate::settings::Quality::Low => Quality::Low,
        crate::settings::Quality::High => Quality::High,
    }
}

