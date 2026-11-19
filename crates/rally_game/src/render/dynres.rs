//! Dynamic resolution for the 3D scene.
//!
//! When the GPU is fill-rate bound (fullscreen at high resolution + heavy translucent
//! weather/dust), the offscreen scene is rendered into a smaller part of its target and the
//! composite upscales it. The HUD is drawn afterwards at native resolution, so text stays
//! crisp. Nothing is removed from the scene — only its pixel count changes, and only while
//! frames are too slow.
//!
//! The controller works from the measured frame time (GPU timer queries are often missing on
//! Windows). It lowers the scale quickly when frames are slow and probes back up slowly when
//! there is headroom; with vsync the frame time saturates at the refresh interval, so it
//! "probes" upward periodically and backs off for longer each time a probe fails.
//!
//! `COZY_DYNRES=0` disables it (always native resolution); `COZY_DYNRES_TARGET=<fps>` sets the
//! frame rate to protect (default 60).

pub const MIN_SCALE: f32 = 0.6;

/// Last scale used (f32 bits), for the F3 overlay
pub static LAST_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

pub fn last_scale() -> f32 {
    f32::from_bits(LAST_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

pub struct DynRes {
    pub enabled: bool,
    /// Current render scale (0.6..1.0) applied to the scene viewports
    pub scale: f32,
    target_ms: f32,
    avg_ms: f32,
    cooldown: f32,
    probe_wait: f32,
    probe_backoff: f32,
    /// scale before the last upward probe (to revert quickly if it fails)
    pre_probe: Option<f32>,
}

impl DynRes {
    pub fn new() -> DynRes {
        let enabled = std::env::var("COZY_DYNRES").map(|v| v != "0").unwrap_or(true);
        let fps: f32 = std::env::var("COZY_DYNRES_TARGET").ok().and_then(|v| v.parse().ok()).unwrap_or(60.0);
        DynRes {
            enabled,
            scale: 1.0,
            target_ms: 1000.0 / fps.max(10.0),
            avg_ms: 0.0,
            cooldown: 0.0,
            probe_wait: 2.0,
            probe_backoff: 2.0,
            pre_probe: None,
        }
    }

    /// Feed the last frame's duration (seconds). Returns the scale to use for this frame.
    pub fn update(&mut self, frame_dt: f32) -> f32 {
        if !self.enabled {
            self.scale = 1.0;
            return 1.0;
        }
        let ms = (frame_dt * 1000.0).clamp(0.0, 250.0);
        // ignore hitches from loading / alt-tab (> 250 ms clamps) by smoothing
        self.avg_ms = if self.avg_ms == 0.0 { ms } else { self.avg_ms + (ms - self.avg_ms) * 0.15 };
        self.cooldown = (self.cooldown - frame_dt).max(0.0);
        self.probe_wait -= frame_dt;
        let slow = self.avg_ms > self.target_ms * 1.25;
        let comfy = self.avg_ms < self.target_ms * 1.05;
        if slow && self.cooldown <= 0.0 && self.scale > MIN_SCALE {
            if let Some(prev) = self.pre_probe.take() {
                // the last step up was too much: go back and wait longer before probing again
                self.scale = prev;
                self.probe_backoff = (self.probe_backoff * 2.0).min(30.0);
            } else {
                // proportional step: pixel cost ~ scale^2
                let ratio = (self.target_ms / self.avg_ms).clamp(0.5, 1.0);
                let want = self.scale * ratio.sqrt();
                self.scale = (self.scale - 0.05).min(want).max(MIN_SCALE);
            }
            self.cooldown = 0.35;
            self.probe_wait = self.probe_backoff;
        } else if comfy && self.scale < 1.0 && self.probe_wait <= 0.0 && self.cooldown <= 0.0 {
            self.pre_probe = Some(self.scale);
            self.scale = (self.scale + 0.05).min(1.0);
            self.probe_wait = 1.0;
            self.cooldown = 0.35;
        } else if comfy && self.cooldown <= 0.0 {
            // a probe that held for a while is accepted; relax the back-off
            if self.pre_probe.is_some() && self.probe_wait <= 0.0 {
                self.pre_probe = None;
                self.probe_backoff = (self.probe_backoff * 0.75).max(2.0);
            }
        }
        // snap tiny scales differences to whole 1% steps (stable viewports)
        self.scale = (self.scale * 100.0).round() / 100.0;
        LAST_SCALE.store(self.scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
        self.scale
    }
}

/// Scale a full-resolution GL rect (origin bottom-left) into the reduced scene region.
pub fn scale_rect(x: i32, gl_y: i32, w: i32, h: i32, s: f32) -> (i32, i32, i32, i32) {
    if s >= 0.999 {
        return (x, gl_y, w, h);
    }
    let sx = (x as f32 * s).floor() as i32;
    let sy = (gl_y as f32 * s).floor() as i32;
    let sw = ((x + w) as f32 * s).ceil() as i32 - sx;
    let sh = ((gl_y + h) as f32 * s).ceil() as i32 - sy;
    (sx, sy, sw.max(1), sh.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_when_slow_and_recovers() {
        let mut d = DynRes { enabled: true, ..DynRes::new() };
        for _ in 0..120 {
            d.update(0.040); // 25 fps
        }
        assert!(d.scale < 0.8, "scale {}", d.scale);
        for _ in 0..2000 {
            d.update(0.006); // 165 fps
        }
        assert!(d.scale > 0.99, "scale {}", d.scale);
    }

    #[test]
    fn stays_native_when_fast() {
        let mut d = DynRes { enabled: true, ..DynRes::new() };
        for _ in 0..600 {
            d.update(0.006);
        }
        assert_eq!(d.scale, 1.0);
    }

    #[test]
    fn rect_scaling() {
        assert_eq!(scale_rect(0, 0, 1920, 1080, 1.0), (0, 0, 1920, 1080));
        let (x, y, w, h) = scale_rect(0, 540, 1920, 540, 0.5);
        assert_eq!((x, y, w, h), (0, 270, 960, 270));
    }
}
