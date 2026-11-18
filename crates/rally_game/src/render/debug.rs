//! Debug / benchmark switches read once from the environment (self-contained so tools that
//! include `gpu.rs` by path can include this too).
use std::sync::OnceLock;

static DEBUG_SKIP: OnceLock<String> = OnceLock::new();

/// Debug/benchmark switch: `COZY_SKIP=props,detail,...` skips drawing those scene layers
/// (sky, clouds, hills, terrain, water, road, structures, props, detail, pcf, shadows).
pub fn debug_skip(layer: &str) -> bool {
    let s = DEBUG_SKIP.get_or_init(|| std::env::var("COZY_SKIP").unwrap_or_default());
    !s.is_empty() && s.split(',').any(|x| x == layer)
}

static LEGACY: OnceLock<bool> = OnceLock::new();

/// Benchmark A/B switch: `COZY_PERF_LEGACY=1` restores the pre-optimisation paths
/// (single shared stream buffer, no prop LOD / distance culling).
pub fn perf_legacy() -> bool {
    *LEGACY.get_or_init(|| std::env::var("COZY_PERF_LEGACY").is_ok())
}
