//! Android glue. The Java side (android/java/MainActivity.java.in, class `CozyNative`) calls the
//! `Java_com_cozyrally_game_CozyNative_*` functions below; the game reads the results through
//! `platform::*` and `input.rs`. Everything here must never panic across the JNI boundary.
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jfloat, jint};
use jni::JNIEnv;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static SUSPENDED: AtomicBool = AtomicBool::new(false);
static BACK: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32;
}

pub fn log(msg: &str) {
    let tag = c"CozyRally";
    if let Ok(text) = CString::new(msg.replace('\0', " ")) {
        // 4 = ANDROID_LOG_INFO
        unsafe { __android_log_write(4, tag.as_ptr(), text.as_ptr()) };
    }
}

pub fn suspended() -> bool {
    SUSPENDED.load(Ordering::Relaxed)
}

pub fn take_back() -> bool {
    BACK.swap(false, Ordering::Relaxed)
}

/// Called from MainActivity.onCreate *before* the game starts (QuadNative.activityOnCreate).
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_cozyrally_game_CozyNative_init<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, activity: JObject<'l>, files_dir: JString<'l>) {
    std::panic::set_hook(Box::new(|info| log(&format!("PANIC: {info}"))));
    // Saves in the app's private storage (settings.rs honours COZY_SETTINGS, gameplay.rs
    // COZY_DATA_DIR). Set before any game thread exists.
    if let Ok(dir) = env.get_string(&files_dir) {
        let dir: String = dir.into();
        unsafe {
            std::env::set_var("COZY_DATA_DIR", &dir);
            std::env::set_var("COZY_SETTINGS", format!("{dir}/settings.txt"));
        }
        log(&format!("data dir {dir}"));
    }
    // cpal's Android backend (oboe) looks the JavaVM + activity up through ndk-context
    if let (Ok(vm), Ok(act)) = (env.get_java_vm(), env.new_global_ref(&activity)) {
        let act_ptr = act.as_obj().as_raw();
        std::mem::forget(act); // lives for the whole process
        unsafe { ndk_context::initialize_android_context(vm.get_java_vm_pointer().cast(), act_ptr.cast()) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_cozyrally_game_CozyNative_onPause(_: JNIEnv, _: JClass) {
    SUSPENDED.store(true, Ordering::Relaxed);
    crate::audio::suspend(true);
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_cozyrally_game_CozyNative_onResume(_: JNIEnv, _: JClass) {
    SUSPENDED.store(false, Ordering::Relaxed);
    crate::audio::suspend(false);
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_cozyrally_game_CozyNative_onBack(_: JNIEnv, _: JClass) {
    BACK.store(true, Ordering::Relaxed);
}

// ------------------------------------------------------------------ gamepads

/// Raw state of one Android input device that behaves like a gamepad
#[derive(Clone, Copy, Debug, Default)]
pub struct RawPad {
    pub device: i32,
    pub lx: f32,
    pub ly: f32,
    pub lt: f32,
    pub rt: f32,
    pub hat_x: f32,
    pub hat_y: f32,
    /// Android KEYCODE_* of held buttons, as a bit set (see `KEYS`)
    pub keys: u32,
}

/// Android keycodes we track, in bit order
pub const KEYS: [i32; 17] = [
    96,  // BUTTON_A
    97,  // BUTTON_B
    99,  // BUTTON_X
    100, // BUTTON_Y
    102, // BUTTON_L1
    103, // BUTTON_R1
    104, // BUTTON_L2
    105, // BUTTON_R2
    108, // BUTTON_START
    109, // BUTTON_SELECT
    19,  // DPAD_UP
    20,  // DPAD_DOWN
    21,  // DPAD_LEFT
    22,  // DPAD_RIGHT
    23,  // DPAD_CENTER
    4,   // BACK (many pads send it for B / Select)
    82,  // MENU (some pads send it for Start)
];
pub fn key_bit(code: i32) -> u32 {
    KEYS.iter().position(|k| *k == code).map(|i| 1u32 << i).unwrap_or(0)
}

static PADS: Mutex<Vec<RawPad>> = Mutex::new(Vec::new());

fn with_pad(device: i32, f: impl FnOnce(&mut RawPad)) {
    if let Ok(mut pads) = PADS.lock() {
        let i = match pads.iter().position(|p| p.device == device) {
            Some(i) => i,
            None => {
                pads.push(RawPad { device, ..Default::default() });
                log(&format!("pad: device {device} connected (pad #{})", pads.len() - 1));
                pads.len() - 1
            }
        };
        f(&mut pads[i]);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_cozyrally_game_CozyNative_padAxes(_: JNIEnv, _: JClass, device: jint, lx: jfloat, ly: jfloat, lt: jfloat, rt: jfloat, hat_x: jfloat, hat_y: jfloat) {
    let fin = |v: f32| if v.is_finite() { v } else { 0.0 };
    with_pad(device, |p| {
        p.lx = fin(lx);
        p.ly = fin(ly);
        p.lt = fin(lt);
        p.rt = fin(rt);
        p.hat_x = fin(hat_x);
        p.hat_y = fin(hat_y);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_cozyrally_game_CozyNative_padKey(_: JNIEnv, _: JClass, device: jint, code: jint, down: jboolean) {
    let bit = key_bit(code);
    // controller debugging without the controller: every press is in logcat (`-s CozyRally`)
    if down != 0 {
        log(&format!("pad: device {device} key {code}{}", if bit == 0 { " (unmapped)" } else { "" }));
    }
    with_pad(device, |p| {
        if down != 0 {
            p.keys |= bit;
        } else {
            p.keys &= !bit;
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_cozyrally_game_CozyNative_padRemoved(_: JNIEnv, _: JClass, device: jint) {
    if let Ok(mut pads) = PADS.lock() {
        pads.retain(|p| p.device != device);
    }
}

/// Connected pads in connection order
pub fn pads() -> Vec<RawPad> {
    PADS.lock().map(|p| p.clone()).unwrap_or_default()
}

/// Rumble the pad's own motors (device id from `RawPad::device`); falls back to nothing if the
/// pad has none. Called from the game thread.
pub fn rumble(device: i32, strength: f32, ms: u32) {
    let activity = unsafe { macroquad::miniquad::native::android::ACTIVITY };
    if activity.is_null() {
        return;
    }
    let raw = unsafe { macroquad::miniquad::native::android::attach_jni_env() };
    let Ok(mut env) = (unsafe { JNIEnv::from_raw(raw.cast()) }) else {
        return;
    };
    let obj = unsafe { JObject::from_raw(activity.cast()) };
    let args = [JValue::Int(device), JValue::Int(ms as i32), JValue::Float(strength.clamp(0.0, 1.0))];
    if env.call_method(&obj, "rumble", "(IIF)V", &args).is_err() {
        let _ = env.exception_clear();
    }
    std::mem::forget(obj); // global ref owned by miniquad
}
