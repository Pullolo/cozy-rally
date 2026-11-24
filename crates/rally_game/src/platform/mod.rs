//! Thin platform layer: everything that differs between PC and phone lives here, so the rest
//! of the game stays one shared code path. On PC every function is a no-op / constant.
//!
//! Android pieces (see `android.rs` and `android/java/MainActivity.java.in`):
//! * the Java activity calls `CozyNative.init(filesDir)` before the game starts: saves go to the
//!   app's private storage, cpal/oboe gets its JNI context, panics are logged to logcat;
//! * lifecycle: app sent to background -> `suspended()` (game pauses, audio stops);
//! * system Back button -> `take_back()` (acts like Esc);
//! * gamepads arrive as raw Android input events -> `android::pads()` (read by `input.rs`);
//! * rumble -> `android::rumble`.

#[cfg(target_os = "android")]
pub mod android;

/// True on phones/tablets: touch controls, mobile quality defaults, no split-screen.
pub const MOBILE: bool = cfg!(target_os = "android");

/// The app is in the background (Android onPause). Always false on PC.
pub fn suspended() -> bool {
    #[cfg(target_os = "android")]
    {
        android::suspended()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// System Back button pressed since the last call (Android). Always false on PC.
pub fn take_back() -> bool {
    #[cfg(target_os = "android")]
    {
        android::take_back()
    }
    #[cfg(not(target_os = "android"))]
    {
        false
    }
}

/// Write a line to the platform log (logcat on Android, stderr elsewhere).
#[allow(dead_code)]
pub fn log(msg: &str) {
    #[cfg(target_os = "android")]
    android::log(msg);
    #[cfg(not(target_os = "android"))]
    eprintln!("{msg}");
}
