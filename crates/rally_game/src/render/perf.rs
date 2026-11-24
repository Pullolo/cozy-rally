//! Frame profiler: CPU time per section, draw calls / vertices / stream uploads, and GPU time
//! per pass via timer queries (GL 3.3+ desktop; silently absent on GLES2/WebGL).
//!
//! * `F3` (or the "Show FPS" setting + `F3`) toggles a detailed overlay so players can send a
//!   screenshot when a slowdown happens.
//! * Benchmarks (`COZY_BENCH`) switch to synchronous timing: every section ends with `glFinish`,
//!   so CPU+GPU cost is attributed to the section exactly (slower, but precise).
use std::cell::RefCell;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sec {
    /// simulation, AI, cameras (RaceSession::update)
    Update = 0,
    /// shadow map pass(es)
    Shadow,
    /// world into the scene target: terrain, props, cars...
    Scene,
    /// fx + overlays drawn inside the scene pass
    Fx,
    /// bloom + grading + composite
    Post,
    /// everything after the 3D (HUD, menus, macroquad 2D)
    Hud,
}

pub const SECS: usize = 6;
pub const SEC_NAMES: [&str; SECS] = ["update", "shadow", "scene", "fx", "post", "hud"];
/// GPU-timed passes
const GPU_PASSES: usize = 3; // shadow, scene (incl. fx), post
const GPU_NAMES: [&str; GPU_PASSES] = ["shadow", "scene", "post"];
const RING: usize = 4;

#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub draws: u32,
    pub verts: u64,
    pub dyn_calls: u32,
    pub dyn_bytes: u64,
    pub shadow_draws: u32,
    pub shadow_verts: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub cpu_ms: [f32; SECS],
    pub gpu_ms: [f32; GPU_PASSES],
    pub gpu_valid: bool,
    pub frame_ms: f32,
    pub c: Counters,
}

/// GL_TIME_ELAPSED query (miniquad's ElapsedQuery is a stub in 0.4)
struct ElapsedQuery(u32);

impl ElapsedQuery {
    fn is_supported() -> bool {
        use macroquad::miniquad::gl;
        if cfg!(any(target_os = "android", target_os = "ios", target_arch = "wasm32")) {
            return false;
        }
        let v = unsafe { gl::glGetString(gl::GL_VERSION) };
        if v.is_null() {
            return false;
        }
        let v = unsafe { std::ffi::CStr::from_ptr(v as *const std::ffi::c_char) }.to_string_lossy().to_string();
        // timer queries are core in GL 3.3+
        let mut it = v.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty());
        let (maj, min) = (it.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0), it.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0));
        !v.contains("OpenGL ES") && (maj > 3 || (maj == 3 && min >= 3))
    }
    fn new() -> ElapsedQuery {
        let mut id = 0;
        unsafe { macroquad::miniquad::gl::glGenQueries(1, &mut id) };
        ElapsedQuery(id)
    }
    fn begin_query(&self) {
        unsafe { macroquad::miniquad::gl::glBeginQuery(macroquad::miniquad::gl::GL_TIME_ELAPSED, self.0) }
    }
    fn end_query(&self) {
        unsafe { macroquad::miniquad::gl::glEndQuery(macroquad::miniquad::gl::GL_TIME_ELAPSED) }
    }
    fn is_available(&self) -> bool {
        let mut a = 0;
        unsafe { macroquad::miniquad::gl::glGetQueryObjectiv(self.0, macroquad::miniquad::gl::GL_QUERY_RESULT_AVAILABLE, &mut a) };
        a != 0
    }
    fn get_result(&self) -> u64 {
        let mut t: u64 = 0;
        unsafe { macroquad::miniquad::gl::glGetQueryObjectui64v(self.0, macroquad::miniquad::gl::GL_QUERY_RESULT, &mut t) };
        t
    }
}

struct GpuTimers {
    queries: Vec<[ElapsedQuery; GPU_PASSES]>,
    used: [[bool; GPU_PASSES]; RING],
    frame: usize,
    active: Option<usize>,
}

struct Perf {
    started: [Option<Instant>; SECS],
    cur: FrameStats,
    last: FrameStats,
    /// smoothed for display
    avg: FrameStats,
    worst_frame_ms: f32,
    worst_timer: f32,
    frame_start: Option<Instant>,
    overlay: bool,
    sync: bool,
    gpu: Option<GpuTimers>,
    gpu_last: [f32; GPU_PASSES],
}

thread_local! {
    static PERF: RefCell<Perf> = RefCell::new(Perf {
        started: [None; SECS],
        cur: FrameStats::default(),
        last: FrameStats::default(),
        avg: FrameStats::default(),
        worst_frame_ms: 0.0,
        worst_timer: 0.0,
        frame_start: None,
        overlay: std::env::var("COZY_PERF").is_ok(),
        sync: std::env::var("COZY_BENCH").is_ok(),
        gpu: None,
        gpu_last: [0.0; GPU_PASSES],
    });
}

fn finish_gpu() {
    unsafe { macroquad::miniquad::gl::glFinish() }
}

/// Synchronous (benchmark) timing: sections include their GPU work.
pub fn sync_mode() -> bool {
    PERF.with(|p| p.borrow().sync)
}

pub fn begin(s: Sec) {
    PERF.with(|p| {
        let mut p = p.borrow_mut();
        if p.sync {
            finish_gpu();
        }
        p.started[s as usize] = Some(Instant::now());
    })
}

pub fn end(s: Sec) {
    PERF.with(|p| {
        let mut p = p.borrow_mut();
        if p.sync {
            finish_gpu();
        }
        if let Some(t) = p.started[s as usize].take() {
            p.cur.cpu_ms[s as usize] += t.elapsed().as_secs_f32() * 1000.0;
        }
    })
}

pub fn count_draw(indices: i32, shadow: bool) {
    PERF.with(|p| {
        let mut p = p.borrow_mut();
        if shadow {
            p.cur.c.shadow_draws += 1;
            p.cur.c.shadow_verts += indices as u64;
        } else {
            p.cur.c.draws += 1;
            p.cur.c.verts += indices as u64;
        }
    })
}

pub fn count_upload(bytes: usize) {
    PERF.with(|p| {
        let mut p = p.borrow_mut();
        p.cur.c.dyn_calls += 1;
        p.cur.c.dyn_bytes += bytes as u64;
    })
}

/// GPU pass timing (0 shadow, 1 scene, 2 post). Only one query is active at a time; nested or
/// repeated calls within a frame (split-screen) accumulate into the same slot.
pub fn gpu_begin(pass: usize) {
    PERF.with(|p| {
        let mut p = p.borrow_mut();
        if p.sync {
            return;
        }
        if p.gpu.is_none() {
            if !ElapsedQuery::is_supported() {
                return;
            }
            p.gpu = Some(GpuTimers { queries: (0..RING).map(|_| [ElapsedQuery::new(), ElapsedQuery::new(), ElapsedQuery::new()]).collect(), used: [[false; GPU_PASSES]; RING], frame: 0, active: None });
        }
        let g = p.gpu.as_mut().unwrap();
        let slot = g.frame % RING;
        // one query per pass per frame (the first occurrence); good enough for 1 viewport and a
        // fair sample in split-screen
        if g.active.is_none() && !g.used[slot][pass] {
            g.queries[slot][pass].begin_query();
            g.used[slot][pass] = true;
            g.active = Some(pass);
        }
    })
}

pub fn gpu_end(pass: usize) {
    PERF.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(g) = p.gpu.as_mut() {
            if g.active == Some(pass) {
                let slot = g.frame % RING;
                g.queries[slot][pass].end_query();
                g.active = None;
            }
        }
    })
}

/// Close the frame: called once per frame from the overlay hook.
pub fn frame_end() {
    PERF.with(|p| {
        let mut p = p.borrow_mut();
        let now = Instant::now();
        if let Some(t) = p.frame_start {
            p.cur.frame_ms = (now - t).as_secs_f32() * 1000.0;
        }
        p.frame_start = Some(now);
        // read back the oldest GPU queries in the ring
        let mut gpu_ms = p.gpu_last;
        let mut valid = false;
        if let Some(g) = p.gpu.as_mut() {
            g.frame += 1;
            let slot = g.frame % RING; // oldest = about to be reused
            for k in 0..GPU_PASSES {
                if g.used[slot][k] {
                    if g.queries[slot][k].is_available() {
                        gpu_ms[k] = g.queries[slot][k].get_result() as f32 / 1.0e6;
                        valid = true;
                    }
                    g.used[slot][k] = false;
                } else {
                    gpu_ms[k] = 0.0;
                }
            }
        }
        p.gpu_last = gpu_ms;
        let mut done = p.cur;
        done.gpu_ms = gpu_ms;
        done.gpu_valid = valid;
        p.last = done;
        let k = 0.1;
        let a = &mut p.avg;
        for i in 0..SECS {
            a.cpu_ms[i] += (done.cpu_ms[i] - a.cpu_ms[i]) * k;
        }
        for i in 0..GPU_PASSES {
            a.gpu_ms[i] += (done.gpu_ms[i] - a.gpu_ms[i]) * k;
        }
        a.frame_ms += (done.frame_ms - a.frame_ms) * k;
        a.c = done.c;
        a.gpu_valid = done.gpu_valid;
        // worst frame over the last ~2 s
        p.worst_timer -= done.frame_ms / 1000.0;
        if p.worst_timer <= 0.0 || done.frame_ms > p.worst_frame_ms {
            if p.worst_timer <= 0.0 {
                p.worst_timer = 2.0;
                p.worst_frame_ms = 0.0;
            }
            p.worst_frame_ms = p.worst_frame_ms.max(done.frame_ms);
        }
        p.cur = FrameStats::default();
        super::bench::record(&done);
        #[cfg(target_os = "android")]
        log_summary(&done, &p.avg);
    })
}

/// Android: a one-line perf summary in logcat every 5 s (`adb logcat -s CozyRally`), since
/// phones have no F3 and GPU timer queries are unavailable on GLES.
#[cfg(target_os = "android")]
fn log_summary(done: &FrameStats, avg: &FrameStats) {
    use std::cell::Cell;
    thread_local! { static ACC: Cell<(u32, f32, f32)> = const { Cell::new((0, 0.0, 0.0)) }; }
    ACC.with(|acc| {
        let (n, t, worst) = acc.get();
        let (n, t, worst) = (n + 1, t + done.frame_ms, worst.max(done.frame_ms));
        if t < 5000.0 {
            acc.set((n, t, worst));
            return;
        }
        acc.set((0, 0.0, 0.0));
        let c = &avg.cpu_ms;
        crate::platform::log(&format!(
            "perf: {:.1} fps (avg {:.1} ms, worst {:.1} ms) | cpu ms update {:.1} shadow {:.1} scene {:.1} fx {:.1} post {:.1} hud {:.1} | {} draws {}k verts | {}x{} {:?}",
            n as f32 * 1000.0 / t,
            t / n as f32,
            worst,
            c[0], c[1], c[2], c[3], c[4], c[5],
            done.c.draws,
            done.c.verts / 1000,
            macroquad::prelude::screen_width(),
            macroquad::prelude::screen_height(),
            super::quality(),
        ));
    });
}

/// Stats of the last completed frame
pub fn last() -> FrameStats {
    PERF.with(|p| p.borrow().last)
}

pub fn overlay_enabled() -> bool {
    PERF.with(|p| p.borrow().overlay)
}

/// Per-frame hook (call once per frame after all drawing): handles F3, closes the HUD section
/// and the frame, and draws the overlay when enabled.
pub fn overlay(quality_label: &str) {
    use macroquad::prelude::*;
    end(Sec::Hud);
    if is_key_pressed(KeyCode::F3) {
        PERF.with(|p| {
            let mut p = p.borrow_mut();
            p.overlay = !p.overlay;
        });
    }
    frame_end();
    let (on, avg, worst) = PERF.with(|p| {
        let p = p.borrow();
        (p.overlay, p.avg, p.worst_frame_ms)
    });
    if !on {
        return;
    }
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{:.1} ms  ({:.0} fps)   worst 2s {:.1} ms   quality {}   scene res {:.0}%", avg.frame_ms, 1000.0 / avg.frame_ms.max(0.01), worst, quality_label, super::dynres::last_scale() * 100.0));
    let mut cpu = String::from("cpu ms ");
    for i in 0..SECS {
        cpu += &format!(" {} {:.2}", SEC_NAMES[i], avg.cpu_ms[i]);
    }
    lines.push(cpu);
    if avg.gpu_valid {
        let mut g = String::from("gpu ms ");
        for i in 0..GPU_PASSES {
            g += &format!(" {} {:.2}", GPU_NAMES[i], avg.gpu_ms[i]);
        }
        lines.push(g);
    } else {
        lines.push("gpu ms  (timer queries unavailable)".to_string());
    }
    let c = avg.c;
    lines.push(format!("draws {}  verts {}k   shadow draws {} verts {}k   stream {} calls {} KB", c.draws, c.verts / 1000, c.shadow_draws, c.shadow_verts / 1000, c.dyn_calls, c.dyn_bytes / 1024));
    let x = 10.0;
    let mut y = 18.0;
    let h = 17.0;
    let w = lines.iter().map(|l| l.len()).max().unwrap_or(10) as f32 * 7.6 + 12.0;
    draw_rectangle(x - 6.0, y - 14.0, w, h * lines.len() as f32 + 8.0, Color::new(0.1, 0.08, 0.14, 0.72));
    for l in &lines {
        draw_text(l, x, y, 16.0, Color::new(1.0, 0.97, 0.9, 1.0));
        y += h;
    }
}
