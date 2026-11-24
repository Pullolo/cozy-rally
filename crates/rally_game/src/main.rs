#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Cozy Rally — a cosy procedural rally racer.
mod audio;
mod camera;
mod cinematic;
mod fx;
mod game;
mod campaign_mode;
mod gameplay;
mod line_assist;
mod campaign_data;
mod car_perf;
mod campaign_screens;
mod hud;
mod input;
mod models;
mod night;
mod online;
mod pacenotes_hud;
mod weather_view;
mod render;
mod settings;
mod ui;

use cinematic::Cinematic;
use game::{PlayerSlot, RaceSession, SessionConfig};
use hud::HudState;
use input::Scheme;
use macroquad::prelude::*;
use rally_core::biome::{Biome, ALL_BIOMES};
use rally_core::race::format_time;
use rally_core::track::TrackParams;
use rally_core::vehicle::car_specs;
use render::gpu::{DrawParams, Environment, Gpu, View};
use ui::*;

fn window_conf() -> Conf {
    let s = settings::load();
    // COZY_WINDOW=1920x1080 for resolution testing
    let (ww, wh) = std::env::var("COZY_WINDOW").ok().and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((1280, 720));
    Conf {
        window_title: "Cozy Rally".to_owned(),
        window_width: ww,
        window_height: wh,
        high_dpi: true,
        fullscreen: s.fullscreen,
        sample_count: std::env::var("COZY_MSAA").ok().and_then(|v| v.parse().ok()).unwrap_or(4),
        window_resizable: true,
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Title,
    Setup,
    Loading,
    Race,
    Results,
    Help,
    Settings,
    Campaign,
    StageIntro,
    StageResults,
    Leaderboard,
    Online,
}

impl Screen {
    fn is_menu(self) -> bool {
        matches!(self, Screen::Title | Screen::Setup | Screen::Help | Screen::Settings | Screen::Campaign | Screen::StageIntro | Screen::Leaderboard | Screen::Online)
    }
}

struct Setup {
    split: bool,
    cars: [usize; 2],
    paints: [usize; 2],
    biome: usize, // 0 = random
    length: u8,
    seed: u64,
    laps: u32,
    opponents: usize,
    difficulty: usize,
    assists: usize,
    /// Track surface preset (index into SURFACES == rally_core::surfacemap::SURFACE_MODES order)
    surface_mode: usize,
    /// Track difficulty (index into TRACK_DIFFICULTY)
    track_difficulty: usize,
    /// [gameplay] index into gameplay::GameMode::ALL
    mode: usize,
    /// 0 = Rally, 1 = Circuit
    discipline: usize,
    /// Circuit picker: 0 = Random, i = circuits()[i - 1]
    circuit: usize,
    /// Random circuit size: 0 Sprint, 1 Grand Prix, 2 Ring, 3 Nordring
    circuit_size: usize,
    /// Night race (headlights)
    night: bool,
}

const DIFFICULTY: [(&str, f32); 3] = [("Chill", 0.35), ("Normal", 0.7), ("Spicy", 1.0)];
const ASSISTS: [(&str, f32); 4] = [("Cozy", 1.3), ("Normal", 1.0), ("Sport", 0.55), ("Sim", 0.0)];
const LENGTHS: [&str; 3] = ["Short", "Medium", "Long"];
const CIRCUIT_SIZES: [&str; 4] = ["Sprint", "Grand Prix", "Ring", "Nordring"];
const DISCIPLINES: [&str; 2] = ["Rally", "Circuit"];
/// Sensible lap counts per circuit size
const CIRCUIT_LAPS: [u32; 4] = [5, 3, 1, 1];
const SURFACES: [&str; 5] = ["Biome default", "Gravel", "Tarmac", "Snow", "Mixed"];
const TRACK_DIFFICULTY: [&str; 3] = ["Relaxed", "Rally", "Expert"];
const PAINT_NAMES: [&str; 9] = ["Strawberry", "Blueberry", "Honey", "Pistachio", "Lilac", "Tangerine", "Coconut", "Lagoon", "Sakura"];
const TIPS: [&str; 8] = [
    "Lift off and flick the handbrake into hairpins!",
    "Circuits: slicks love tarmac - brake in a straight line, stay inside the white lines.",
    "Gravel and snow are slippery - be gentle.",
    "Stuck? Press R to hop back onto the road.",
    "Press C to try the other cameras.",
    "Big jumps: ease off the throttle before landing.",
    "The grip widget turns peach when a tyre slides.",
    "Gamepad: RT/LT drive, A handbrake, Y respawn, X camera, Start pause.",
];

impl Setup {
    /// [gameplay] selected game mode
    fn mode(&self) -> gameplay::GameMode {
        gameplay::GameMode::ALL[self.mode % gameplay::GameMode::ALL.len()]
    }

    fn is_circuit(&self) -> bool {
        self.discipline == 1
    }

    /// Named circuit selected in the picker (None = random)
    fn named_circuit(&self) -> Option<rally_core::circuits::CircuitDef> {
        if self.circuit == 0 {
            return None;
        }
        rally_core::circuits::circuits().into_iter().nth(self.circuit - 1)
    }

    fn circuit_size_now(&self) -> usize {
        self.named_circuit().map(|c| c.params.length_class as usize).unwrap_or(self.circuit_size).min(3)
    }

    /// Cars offered for the discipline (rally cars for rally, circuit cars for circuits)
    fn allowed_cars(&self) -> Vec<usize> {
        let want = if self.is_circuit() { rally_core::vehicle::CarClass::Circuit } else { rally_core::vehicle::CarClass::Rally };
        let v: Vec<usize> = car_specs().iter().enumerate().filter(|(_, c)| c.class == want).map(|(i, _)| i).collect();
        if v.is_empty() { (0..car_specs().len()).collect() } else { v }
    }

    /// Keep car choices valid for the discipline (remembered per discipline in settings)
    fn fix_cars(&mut self) {
        let allowed = self.allowed_cars();
        let st = settings::get();
        let pref = if self.is_circuit() { st.car_circuit } else { st.car_rally };
        for c in self.cars.iter_mut() {
            if !allowed.contains(c) {
                *c = if allowed.contains(&pref) { pref } else { allowed[0] };
            }
        }
    }

    fn track_params(&self) -> TrackParams {
        let mut p = self.track_params_base();
        p.night |= self.night;
        p
    }

    fn track_params_base(&self) -> TrackParams {
        if self.is_circuit() {
            return match self.named_circuit() {
                Some(c) => c.params,
                None => rally_core::circuits::circuit_params(self.seed, if self.biome == 0 { None } else { Some(ALL_BIOMES[self.biome - 1]) }, self.circuit_size.min(3) as u8, self.track_difficulty.min(2) as u8),
            }
            .with_env_overrides();
        }
        TrackParams { seed: self.seed, biome: if self.biome == 0 { None } else { Some(ALL_BIOMES[self.biome - 1]) }, length_class: self.length, difficulty: self.track_difficulty.min(2) as u8, surface: rally_core::surfacemap::SURFACE_MODES[self.surface_mode.min(4)], ..TrackParams::default() }.with_env_overrides()
    }

    fn config(&self) -> SessionConfig {
        let mut players = vec![PlayerSlot { car: self.cars[0], paint: self.paints[0], scheme: if self.split { Scheme::Wasd } else { Scheme::Both }, name: "Player 1".into() }];
        if self.split {
            players.push(PlayerSlot { car: self.cars[1], paint: self.paints[1], scheme: Scheme::Arrows, name: "Player 2".into() });
        }
        SessionConfig {
            track: self.track_params(),
            laps: self.laps,
            players,
            ai_count: if self.mode() == gameplay::GameMode::TimeTrial { 0 } else { self.opponents },
            ai_skill: DIFFICULTY[self.difficulty].1,
            assist_level: ASSISTS[self.assists].1,
            mode: self.mode(),
            online: None,
        }
    }
}

/// The on-screen pause button (bottom centre) is showing (the HUD keeps clear of it)
static PAUSE_BUTTON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn pause_button_visible() -> bool {
    PAUSE_BUTTON.load(std::sync::atomic::Ordering::Relaxed)
}

fn random_seed() -> u64 {
    (macroquad::miniquad::date::now() * 1000.0) as u64 % 1_000_000
}

/// The biome a seed will actually produce (reveals "Surprise!" on the loading screen)
fn biome_reveal(i: usize, seed: u64) -> String {
    if i == 0 {
        let b = Biome::from_index((rally_core::rng::Rng::new(seed).next_u64() % 5) as usize);
        b.style().name.to_string()
    } else {
        biome_name(i, seed)
    }
}

fn biome_name(i: usize, seed: u64) -> String {
    if i == 0 {
        let _ = seed;
        "Surprise!".to_string()
    } else {
        ALL_BIOMES[i - 1].style().name.to_string()
    }
}

fn paint_name(i: usize) -> &'static str {
    PAINT_NAMES.get(i).copied().unwrap_or("Custom")
}

fn cam_mode(i: usize) -> camera::CamMode {
    match i {
        1 => camera::CamMode::Far,
        2 => camera::CamMode::Hood,
        _ => camera::CamMode::Chase,
    }
}

/// Test hooks driven by environment variables (used for automated playtesting)
struct TestHooks {
    autoplay: bool,
    shots: Vec<f32>,
    shot_dir: String,
    exit_after: Option<f32>,
    split: bool,
    /// Extra state to enter once a race has loaded: "pause" or "results" (fast-forwarded)
    after_load: Option<String>,
}

impl TestHooks {
    fn from_env() -> TestHooks {
        let get = |k: &str| std::env::var(k).ok();
        TestHooks {
            autoplay: get("COZY_AUTOPLAY").is_some(),
            shots: get("COZY_SHOTS").map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect()).unwrap_or_default(),
            shot_dir: get("COZY_SHOT_DIR").unwrap_or_else(|| ".".into()),
            exit_after: get("COZY_EXIT_AFTER").and_then(|v| v.parse().ok()),
            split: get("COZY_SPLIT").is_some(),
            after_load: None,
        }
    }
}

/// Fade-through-cream between screens. Input is blocked while it runs.
struct Transition {
    target: Option<Screen>,
    /// 1 = fading out to cream, -1 = fading in, 0 = idle
    phase: i8,
    alpha: f32,
}

const FADE_OUT: f32 = 0.22;
const FADE_IN: f32 = 0.38;

struct App {
    gpu: Gpu,
    screen: Screen,
    setup: Setup,
    session: Option<RaceSession>,
    hud: Option<HudState>,
    focus: Focus,
    preview_angle: f32,
    preview_models: Vec<(render::gpu::GpuMesh, render::gpu::GpuMesh, usize, usize)>,
    pause_focus: Focus,
    hooks: TestHooks,
    autopilots: Vec<rally_core::ai::AiDriver>,
    loading_frames: u32,
    // --- presentation
    trans: Transition,
    /// Seconds since the current screen was entered (for pop-in animations)
    screen_t: f32,
    music: audio::Music,
    /// Title attract mode: AI race on a random track filmed by a cinematic camera
    attract: Option<RaceSession>,
    attract_wait: u32,
    attract_fade: f32,
    cine: Cinematic,
    results_cine: Cinematic,
    /// Where Settings / Help return to
    settings_from: Screen,
    help_from: Screen,
    tip: usize,
    fps: f32,
    pointer_seen: f32,
    /// Campaign screens state (chapter/stage selection, local progress, active stage)
    camp: campaign_screens::CampState,
    /// Online races (connection, lobby, the running online race)
    online: online::Online,
}

impl App {
    // ------------------------------------------------------------ flow

    /// Fade out, switch screen, fade in
    fn goto(&mut self, s: Screen) {
        if self.trans.phase == 0 {
            self.trans = Transition { target: Some(s), phase: 1, alpha: 0.0 };
            set_input_blocked(true);
        }
    }

    /// Switch immediately (overlays such as pause <-> settings)
    fn switch_now(&mut self, s: Screen) {
        self.enter(s);
    }

    fn enter(&mut self, s: Screen) {
        let prev = self.screen;
        if s == Screen::Race && prev == Screen::Loading {
            // the screen is fully cream now: build the track behind it, then fade in
            self.finish_loading();
            return;
        }
        self.screen = s;
        self.focus = Focus::default();
        self.screen_t = 0.0;
        if prev == Screen::Settings {
            settings::save();
        }
        // Settings/Help can be opened over a paused race: only the title/setup end a race
        if matches!(s, Screen::Title | Screen::Setup | Screen::Campaign | Screen::StageIntro | Screen::Leaderboard | Screen::Online) {
            // leaving an online race for a menu: tell the server
            self.online.leave_race();
            self.end_session();
        }
        if matches!(s, Screen::Title | Screen::Setup) {
            self.camp.active = None;
        }
        if s.is_menu() && self.session.is_none() {
            self.set_music(audio::Music::Menu);
        }
    }

    fn set_music(&mut self, m: audio::Music) {
        if self.music != m {
            self.music = m;
            audio::set_music(m);
        }
    }

    fn end_session(&mut self) {
        if let Some(sess) = self.session.take() {
            sess.free(&mut self.gpu);
        }
        self.hud = None;
        audio::stop_cars();
    }

    fn update_transition(&mut self) {
        let dt = get_frame_time().min(0.05);
        match self.trans.phase {
            1 => {
                self.trans.alpha += dt / FADE_OUT;
                if self.trans.alpha >= 1.0 {
                    self.trans.alpha = 1.0;
                    if let Some(t) = self.trans.target.take() {
                        self.enter(t);
                    }
                    self.trans.phase = -1;
                }
            }
            -1 => {
                self.trans.alpha -= dt / FADE_IN;
                if self.trans.alpha <= 0.0 {
                    self.trans.alpha = 0.0;
                    self.trans.phase = 0;
                    set_input_blocked(false);
                }
            }
            _ => {}
        }
        if self.trans.alpha > 0.0 {
            let a = ease_out(self.trans.alpha);
            screen_wash(Color::new(1.0, 0.97, 0.92, a), Color::new(0.99, 0.92, 0.86, a));
        }
    }

    fn start_race(&mut self) {
        self.goto(Screen::Loading);
        self.loading_frames = 0;
        self.tip = (self.tip + 1 + (random_seed() % 3) as usize) % TIPS.len();
    }

    fn finish_loading(&mut self) {
        self.end_session();
        if let Some(a) = self.attract.take() {
            a.free(&mut self.gpu);
        }
        // [gameplay] a campaign stage queued by campaign_mode::start_stage wins
        let assist = ASSISTS[self.setup.assists].1;
        let mut cfg = match self.online.take_config(assist) {
            Some(c) => c,
            None => campaign_mode::take_pending_config().unwrap_or_else(|| self.setup.config()),
        };
        if self.hooks.after_load.as_deref() == Some("results") {
            cfg.laps = 1;
        }
        let mut s = RaceSession::new(cfg, &mut self.gpu);
        self.online.session_ready(&mut s);
        let cam = cam_mode(settings::get().camera);
        for p in s.players.iter_mut() {
            p.cam.mode = cam;
        }
        self.autopilots = (0..s.players.len()).map(|i| rally_core::ai::AiDriver::new(0.85, i as f32 * 1.5 - 0.7)).collect();
        let mut hud = HudState::new(&s);
        if s.config.mode == gameplay::GameMode::Campaign {
            let (best, splits, medals) = self.camp.hud_reference();
            hud.set_reference(best, splits);
            hud.medals = medals;
        } else {
            self.camp.active = None;
        }
        self.hud = Some(hud);
        self.session = Some(s);
        self.screen = Screen::Race;
        self.screen_t = 0.0;
        self.focus = Focus::default();
        self.set_music(audio::Music::Race);
        if self.hooks.after_load.as_deref() == Some("results") {
            self.hooks.after_load = None;
            self.fast_forward_results();
        }
    }

    /// Test hook: let AI drive everyone until the race is over, then show results
    fn fast_forward_results(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        for i in 0..s.players.len() {
            let car = s.players[i].car;
            let skill = std::env::var("COZY_FF_SKILL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.95);
            s.ais[car] = Some(rally_core::ai::AiDriver::new(skill, 0.0));
        }
        s.world.race.finish_grace = 20.0;
        let mut guard = 0;
        while !s.is_over() && guard < 20000 {
            s.update(0.1, None);
            if let Some(h) = self.hud.as_mut() {
                h.update(s, 0.1);
            }
            guard += 1;
        }
        if self.camp.active.is_some() {
            self.stage_over();
            return;
        }
        self.screen = Screen::Results;
        self.results_cine = Cinematic::orbit(s.players[0].car);
        self.results_cine.frame = vec2(-0.45, 0.42);
        self.set_music(audio::Music::Results);
    }

    // ------------------------------------------------------------ backdrops

    /// Live 3D attract-mode backdrop for menus (2D fallback while it builds)
    fn menu_backdrop(&mut self) {
        let dt = get_frame_time().min(0.05);
        if self.attract.is_none() && self.session.is_none() {
            self.attract_wait += 1;
            if self.attract_wait > 2 && std::env::var("COZY_NO_ATTRACT").is_err() {
                let seed = random_seed();
                let cfg = SessionConfig {
                    track: TrackParams { seed, biome: None, length_class: 1, ..Default::default() },
                    laps: 99,
                    players: vec![],
                    ai_count: 6,
                    ai_skill: 0.65,
                    assist_level: 1.0,
                    mode: gameplay::GameMode::Race,
                    online: None,
                };
                let mut s = RaceSession::new(cfg, &mut self.gpu);
                if let rally_core::race::RacePhase::Countdown(_) = s.world.race.phase {
                    s.world.race.phase = rally_core::race::RacePhase::Countdown(1.5);
                }
                self.cine = Cinematic::new(seed);
                // keep the action to the right of the menu column
                self.cine.frame = vec2(0.30, 0.0);
                self.attract = Some(s);
                self.attract_fade = 1.0;
            }
        }
        match self.attract.as_mut() {
            Some(a) if self.session.is_none() => {
                a.update(dt, None);
                self.cine.update(a, dt);
                let horizon = a.scene.sky_horizon;
                clear_background(Color::new(horizon[0], horizon[1], horizon[2], 1.0));
                a.draw_3d(&mut self.gpu, |_, _, _| {});
                self.attract_fade = (self.attract_fade - dt / 0.9).max(0.0);
                let dip = (self.cine.dip * 0.9).max(self.attract_fade);
                if dip > 0.0 {
                    let a = ease_out(dip);
                    screen_wash(Color::new(1.0, 0.97, 0.92, a), Color::new(0.99, 0.92, 0.86, a));
                }
            }
            _ => cozy_background(get_time() as f32),
        }
    }

    // ------------------------------------------------------------ car preview

    fn preview_model(&mut self, car: usize, paint: usize) -> usize {
        if let Some(i) = self.preview_models.iter().position(|m| m.2 == car && m.3 == paint) {
            return i;
        }
        let spec = car_specs()[car].clone();
        let m = models::car_model(&spec, models::PAINTS[paint]);
        let b = self.gpu.upload(&m.body);
        let w = self.gpu.upload(&m.wheel);
        self.preview_models.push((b, w, car, paint));
        self.preview_models.len() - 1
    }

    /// Turntable preview: car framed ~60% of the panel width, 3/4 elevated view,
    /// on a soft pastel plinth with a blurred contact shadow.
    fn draw_car_preview(&mut self, car: usize, paint: usize, x: f32, y: f32, w: f32, h: f32) {
        use render::mesh::{rgba, Vertex};
        let idx = self.preview_model(car, paint);
        let spec = car_specs()[car].clone();
        let dpi = screen_dpi_scale();
        self.gpu.begin_3d((x * dpi) as i32, (y * dpi) as i32, (w * dpi) as i32, (h * dpi) as i32, true);
        let aspect = w / h.max(1.0);
        let fov = 34f32.to_radians();
        let he = spec.half_extents;
        // apparent width of the car seen from 3/4 is about its footprint diagonal
        let span = 2.0 * (he.x * he.x + he.z * he.z).sqrt();
        // projected width at this 3/4 angle is ~75% of the footprint diagonal
        let apparent = span * 0.75;
        let tan = (fov * 0.5).tan();
        let dist_w = apparent / 0.60 * 0.5 / (tan * aspect);
        let dist_h = (he.y * 2.0 + 1.0) / 0.55 * 0.5 / tan;
        let dist = dist_w.max(dist_h);
        let ground = -0.62;
        let target = vec3(0.0, ground + he.y * 0.9, 0.0);
        let eye = target + vec3(0.62, 0.42, 0.66).normalize() * dist;
        let view = Mat4::look_at_rh(eye, target, Vec3::Y);
        let proj = Mat4::perspective_rh_gl(fov, aspect, 0.1, 100.0);
        let v = View::new(view, proj, eye);
        let env = Environment { sun_dir: vec3(-0.4, 0.8, 0.5).normalize(), sun_color: [1.0, 0.95, 0.88], ambient: [0.48, 0.5, 0.56], fog_color: [1.0, 1.0, 1.0], fog_density: 0.0, time: 0.0, night: 0.0 };
        let rot = Mat4::from_rotation_y(self.preview_angle);
        let ride = -spec.mount_height + spec.susp_rest * 0.8 + spec.wheel_radius - 0.6;
        let m = rot * Mat4::from_translation(vec3(0.0, ride, 0.0));
        // plinth: soft peach-cream top with a slightly deeper rim (unlit so it never blows out)
        let r = span * 0.52;
        let unlit = DrawParams { unlit: 1.0, ..Default::default() };
        let mut disc = render::mesh::MeshBuilder::new();
        let rim = render::mesh::rgb([0.88, 0.79, 0.72]);
        let top = render::mesh::rgb([0.95, 0.88, 0.80]);
        disc.cylinder(Mat4::from_translation(vec3(0.0, ground - 0.12, 0.0)), r * 1.04, 0.12, 48, rim, rim);
        disc.cylinder(Mat4::from_translation(vec3(0.0, ground - 0.02, 0.0)), r, 0.03, 48, top, top);
        self.gpu.draw_builder(&disc, &v, &env, Mat4::IDENTITY, &unlit, 0);
        // blurred contact shadow (rotates with the car)
        let mut verts: Vec<Vertex> = Vec::new();
        let mut ids: Vec<u16> = Vec::new();
        let ring = 24u16;
        let sy = ground + 0.02;
        verts.push(Vertex { pos: [0.0, sy, 0.0], normal: [0.0, 1.0, 0.0], color: rgba([0.25, 0.15, 0.2], 0.38) });
        for k in 0..ring {
            let a = k as f32 / ring as f32 * std::f32::consts::TAU;
            let p = rot.transform_point3(vec3(a.cos() * (he.x + 0.35), 0.0, a.sin() * (he.z + 0.3)));
            verts.push(Vertex { pos: [p.x, sy, p.z], normal: [0.0, 1.0, 0.0], color: rgba([0.25, 0.15, 0.2], 0.0) });
        }
        for k in 0..ring {
            ids.extend_from_slice(&[0, 1 + k, 1 + (k + 1) % ring]);
        }
        self.gpu.draw_dynamic(&verts, &ids, &v, &env, Mat4::IDENTITY, &unlit, 1);
        let (body, wheel, _, _) = &self.preview_models[idx];
        self.gpu.draw(body, &v, &env, m, &DrawParams { rim: 0.5, ..Default::default() }, false);
        let mounts = [
            vec3(-spec.half_track, spec.mount_height, -spec.cg_to_front),
            vec3(spec.half_track, spec.mount_height, -spec.cg_to_front),
            vec3(-spec.half_track, spec.mount_height, spec.cg_to_rear),
            vec3(spec.half_track, spec.mount_height, spec.cg_to_rear),
        ];
        for mt in mounts {
            let wm = m * Mat4::from_translation(mt - Vec3::Y * spec.susp_rest * 0.8);
            self.gpu.draw(wheel, &v, &env, wm, &DrawParams::default(), false);
        }
        // headlights (unlit, glowing like in the race) + translucent glass last, cached per car/paint
        thread_local! {
            static PREVIEW_EXTRAS: std::cell::RefCell<Vec<(usize, usize, render::mesh::MeshBuilder, render::mesh::MeshBuilder)>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        PREVIEW_EXTRAS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let i = match cache.iter().position(|e| e.0 == car && e.1 == paint) {
                Some(i) => i,
                None => {
                    let cm = models::car_model(&spec, models::PAINTS[paint]);
                    cache.push((car, paint, cm.head_lights, cm.glass));
                    cache.len() - 1
                }
            };
            let (_, _, head, glass) = &cache[i];
            self.gpu.draw_builder(head, &v, &env, m, &DrawParams { unlit: 1.0, tint: [1.35, 1.3, 1.15, 1.0], ..Default::default() }, 0);
            self.gpu.draw_builder(glass, &v, &env, m, &DrawParams { rim: 0.8, ..Default::default() }, 1);
        });
        self.gpu.end_3d();
    }

    // ------------------------------------------------------------ screens

    fn title(&mut self) {
        self.menu_backdrop();
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let t = get_time() as f32;
        left_wash(sw * 0.55, Color::new(1.0, 0.96, 0.9, 0.55));
        let x = 64.0 * s;
        // logo: each letter bounces in, then bobs gently
        let logo_y = sh * 0.24;
        let tag = "cosy rally stages & circuit racing with friends";
        let tag_a = ((self.screen_t - 0.5) / 0.4).clamp(0.0, 1.0);
        let tw = text_size(tag, 22.0 * s).x;
        rounded_rect(x + 4.0 * s, logo_y + 26.0 * s, tw + 32.0 * s, 38.0 * s, 19.0 * s, with_alpha(CREAM, 0.9 * tag_a));
        text(tag, x + 20.0 * s, mid_baseline(logo_y + 45.0 * s, 22.0 * s), 22.0 * s, with_alpha(INK_SOFT, tag_a));
        draw_logo(x, logo_y, 100.0 * s, t, self.screen_t);
        
        let bw = 330.0 * s;
        let bh = 50.0 * s;
        let gap = 11.0 * s;
        let mut y = (sh * 0.38).max(logo_y + 84.0 * s);
        let items: [(&str, Color); 7] = [("Campaign", BUTTER), ("Quick race", PEACH), ("Split-screen", SKY), ("Online", BERRY), ("Settings", LAVENDER), ("How to play", MINT), ("Quit", CREAM)];
        self.focus.begin(items.len());
        let mut chosen = None;
        for (i, (label, col)) in items.iter().enumerate() {
            // staggered slide-in
            let k = ease_back(((self.screen_t - 0.15 - i as f32 * 0.06) / 0.4).clamp(0.0, 1.0));
            let bx = x - (1.0 - k) * 60.0 * s;
            if button(&mut self.focus, label, bx, y, bw, bh, *col) {
                chosen = Some(i);
            }
            y += bh + gap;
        }
        match chosen {
            Some(0) => self.goto(Screen::Campaign),
            Some(1) => {
                self.setup.split = false;
                self.goto(Screen::Setup);
            }
            Some(2) => {
                self.setup.split = true;
                self.goto(Screen::Setup);
            }
            Some(3) => {
                self.setup.split = false;
                self.goto(Screen::Online);
            }
            Some(4) => {
                self.settings_from = Screen::Title;
                self.goto(Screen::Settings);
            }
            Some(5) => {
                self.help_from = Screen::Title;
                self.goto(Screen::Help);
            }
            Some(6) => std::process::exit(0),
            _ => {}
        }
        // footer: now showing + hint
        if let Some(a) = &self.attract {
            let label = format!("Now showing: {}", a.track().style.name);
            let w = text_size(&label, 18.0 * s).x + 30.0 * s;
            rounded_rect(sw - w - 20.0 * s, sh - 50.0 * s, w, 32.0 * s, 16.0 * s, with_alpha(CREAM, 0.85));
            text_mid(&label, sw - w * 0.5 - 20.0 * s, sh - 34.0 * s, 18.0 * s, INK_SOFT);
        }
        let hint = "Arrows / mouse to choose  -  Enter to go";
        text(hint, x, sh - 26.0 * s, 17.0 * s, with_alpha(INK_SOFT, 0.9));
    }

    fn help(&mut self) {
        let over_race = self.session.is_some();
        if over_race {
            self.draw_race_frozen();
            draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.24, 0.18, 0.3, 0.35));
        } else {
            self.menu_backdrop();
            screen_wash(Color::new(1.0, 0.96, 0.9, 0.35), Color::new(1.0, 0.94, 0.88, 0.5));
        }
        let s = scale();
        let w = 820.0 * s;
        let h = 580.0 * s;
        let pop_k = ease_back((self.screen_t / 0.35).min(1.0));
        let x = (screen_width() - w) * 0.5;
        let y = (screen_height() - h) * 0.5 + (1.0 - pop_k) * 40.0 * s;
        panel(x, y, w, h);
        text_mid("How to play", x + w * 0.5, y + 44.0 * s, 40.0 * s, INK);
        text_mid("Rally stages on gravel, mud & snow  -  circuit races on tarmac with slicks", x + w * 0.5, y + 76.0 * s, 17.0 * s, INK_SOFT);
        let rows: [(&str, &[&str], &[&str]); 8] = [
            ("Drive", &["W", "A", "S", "D"], &["Up", "Left", "Down", "Right"]),
            ("Handbrake", &["Space"], &["R-Shift"]),
            ("Respawn on track", &["R"], &["Backspace"]),
            ("Change camera", &["C"], &["R-Ctrl"]),
            ("Pace notes on/off", &["N"], &[]),
            ("Racing line", &["L"], &["LB"]),
            ("Pause", &["Esc"], &["P"]),
            ("Menus", &["Enter"], &["Esc"]),
        ];
        let col1 = x + 44.0 * s;
        let col2 = x + 300.0 * s;
        let col3 = x + 560.0 * s;
        text("Solo  /  Player 1", col2, y + 104.0 * s, 18.0 * s, INK_SOFT);
        text("Also solo  /  Player 2", col3, y + 104.0 * s, 18.0 * s, INK_SOFT);
        let ks = 32.0 * s;
        for (i, (label, a, b)) in rows.iter().enumerate() {
            let ry = y + 140.0 * s + i as f32 * 41.0 * s;
            if i % 2 == 0 {
                rounded_rect(x + 24.0 * s, ry - 19.0 * s, w - 48.0 * s, 38.0 * s, 12.0 * s, with_alpha(CREAM_DEEP, 0.6));
            }
            text(label, col1, mid_baseline(ry, 22.0 * s), 22.0 * s, INK);
            let mut kx = col2;
            for k in a.iter() {
                kx += keycap(k, kx, ry, ks) + 6.0 * s;
            }
            let mut kx = col3;
            for k in b.iter() {
                kx += keycap(k, kx, ry, ks) + 6.0 * s;
            }
        }
        let ty = y + h - 128.0 * s;
        rounded_rect(x + 24.0 * s, ty, w - 48.0 * s, 44.0 * s, 14.0 * s, with_alpha(BUTTER, 0.55));
        text_mid(&format!("Tip: {}", TIPS[(get_time() / 5.0) as usize % TIPS.len()]), x + w * 0.5, ty + 22.0 * s, 20.0 * s, INK);
        self.focus.begin(1);
        let back = button_back(&mut self.focus, "Back", x + w * 0.5 - 110.0 * s, y + h - 70.0 * s, 220.0 * s, 50.0 * s, PEACH);
        if back || self.focus.back {
            if self.focus.back {
                audio::ui_sound(audio::UiSound::Back);
            }
            let to = self.help_from;
            if over_race {
                self.switch_now(to);
            } else {
                self.goto(to);
            }
        }
    }

    fn setup_screen(&mut self) {
        self.menu_backdrop();
        screen_wash(Color::new(1.0, 0.96, 0.9, 0.30), Color::new(1.0, 0.94, 0.88, 0.45));
        self.preview_angle += get_frame_time() * 0.6;
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let appear = ease_back((self.screen_t / 0.4).min(1.0));
        let specs = car_specs();
        let players = if self.setup.split { 2 } else { 1 };
        // left: options panel with two columns (car + race | track); right: car preview(s)
        let margin = 24.0 * s;
        let w = ((sw - 3.0 * margin) * 0.62).max(560.0 * s).min(sw - 2.0 * margin);
        let x = margin - (1.0 - appear) * 50.0 * s;
        let y = margin;
        let h = sh - 2.0 * margin;
        panel(x, y, w, h);
        text(if self.setup.mode() == gameplay::GameMode::TimeTrial { if self.setup.split { "Split-screen time trial" } else { "Time trial" } } else if self.setup.split { "Split-screen race" } else { "Solo race" }, x + 26.0 * s, y + 50.0 * s, 38.0 * s, INK);
        let tt = self.setup.mode() == gameplay::GameMode::TimeTrial; // [gameplay] no opponents in time trial
        let circuit = self.setup.is_circuit();
        let named = self.setup.named_circuit();
        // track column: Discipline + (rally: 5 rows | random circuit: 5 rows | named circuit: 1 row)
        let track_rows = 1 + if circuit && named.is_some() { 1 } else { 5 };
        let rows = players * 2 + 5 + track_rows + 2 + 1 - if tt { 2 } else { 0 };
        self.focus.begin(rows);
        let gap = 22.0 * s;
        let cw = (w - 2.0 * 26.0 * s - gap) * 0.5;
        let ax = x + 26.0 * s;
        let bx = ax + cw + gap;
        let head_h = 30.0 * s;
        let top = y + 72.0 * s;
        let button_h = 54.0 * s;
        // tallest column: car rows + race rows, plus two section headers
        let col_rows = (players * 2 + 6) as f32;
        let avail = h - (top - y) - 2.0 * head_h - button_h - 34.0 * s;
        let rh = (avail / col_rows - 8.0 * s).clamp(30.0 * s, 48.0 * s);
        let step = rh + 8.0 * s;
        let lfs = (rh * 0.44).min(27.0 * s);
        let label_w = ["Opponents", "Difficulty", "Discipline", "P1 paint", "Scenery"].iter().map(|l| text_size(l, lfs).x).fold(0.0, f32::max);
        self.focus.label_w = Some(label_w + 4.0 * s);
        let wrap = |v: usize, d: i32, n: usize| ((v as i32 + d).rem_euclid(n as i32)) as usize;
        let section = |label: &str, sx: f32, sy: f32| {
            text(label, sx + 4.0 * s, sy + 18.0 * s, 19.0 * s, INK_SOFT);
            let tw = text_size(label, 19.0 * s).x;
            rounded_rect(sx + tw + 16.0 * s, sy + 11.0 * s, cw - tw - 20.0 * s, 2.0 * s, 1.0, with_alpha(INK, 0.12));
        };
        // --- column A: cars, then race options
        let mut ay = top;
        section(if players == 2 { "Drivers" } else { "Your car" }, ax, ay);
        ay += head_h;
        for p in 0..players {
            let label = if players == 2 { format!("P{} car", p + 1) } else { "Car".into() };
            let allowed = self.setup.allowed_cars();
            let pos = allowed.iter().position(|c| *c == self.setup.cars[p]).unwrap_or(0);
            let d = selector(&mut self.focus, &label, specs[self.setup.cars[p]].name, ax, ay, cw, rh, [PEACH, SKY][p]);
            self.setup.cars[p] = allowed[wrap(pos, d, allowed.len())];
            ay += step;
            let paint = self.setup.paints[p];
            let d = selector_swatch(&mut self.focus, if players == 2 { &["P1 paint", "P2 paint"][p] } else { "Paint" }, paint_name(paint), Some(rgb3(models::PAINTS[paint])), ax, ay, cw, rh, [PEACH, SKY][p]);
            self.setup.paints[p] = wrap(self.setup.paints[p], d, models::PAINTS.len());
            ay += step;
        }
        ay += 6.0 * s;
        section("Race", ax, ay);
        ay += head_h;
        let d = selector(&mut self.focus, "Mode", self.setup.mode().name(), ax, ay, cw, rh, LAVENDER); // [gameplay]
        self.setup.mode = wrap(self.setup.mode, d, gameplay::GameMode::ALL.len());
        ay += step;
        let d = selector(&mut self.focus, "Laps", &format!("{}", self.setup.laps), ax, ay, cw, rh, BUTTER);
        let max_laps = if circuit { 12 } else { 6 };
        self.setup.laps = ((self.setup.laps as i32 + d - 1).rem_euclid(max_laps) + 1) as u32;
        ay += step;
        if !tt {
            let d = selector(&mut self.focus, "Opponents", &format!("{}", self.setup.opponents), ax, ay, cw, rh, BUTTER);
            self.setup.opponents = wrap(self.setup.opponents, d, 8 - players + 1);
            ay += step;
            let d = selector(&mut self.focus, "AI mood", DIFFICULTY[self.setup.difficulty].0, ax, ay, cw, rh, BUTTER);
            self.setup.difficulty = wrap(self.setup.difficulty, d, DIFFICULTY.len());
            ay += step;
        }
        let d = selector(&mut self.focus, "Assists", ASSISTS[self.setup.assists].0, ax, ay, cw, rh, LAVENDER);
        self.setup.assists = wrap(self.setup.assists, d, ASSISTS.len());
        if d != 0 {
            // keep the Settings screen's Assists in sync with the setup choice
            let a = self.setup.assists;
            settings::update(|s| s.assists = a);
            settings::save();
        }
        ay += step;
        let d = selector(&mut self.focus, "Time", if self.setup.night { "Night" } else { "Day" }, ax, ay, cw, rh, LAVENDER);
        if d != 0 {
            self.setup.night = !self.setup.night;
        }
        // --- column B: track options
        let mut by = top;
        section("Track", bx, by);
        by += head_h;
        let d = selector(&mut self.focus, "Discipline", DISCIPLINES[self.setup.discipline.min(1)], bx, by, cw, rh, SKY);
        if d != 0 {
            self.setup.discipline = 1 - self.setup.discipline.min(1);
            self.setup.fix_cars();
            self.setup.laps = if self.setup.is_circuit() { CIRCUIT_LAPS[self.setup.circuit_size_now()] } else { 3 };
            return;
        }
        by += step;
        if circuit {
            let circuits = rally_core::circuits::circuits();
            let name = named.as_ref().map(|c| c.name.replace("Grand Prix", "GP").replace("International", "Int'l")).unwrap_or_else(|| "Random".into());
            let d = selector(&mut self.focus, "Circuit", &name, bx, by, cw, rh, SKY);
            if d != 0 {
                self.setup.circuit = wrap(self.setup.circuit, d, circuits.len() + 1);
                self.setup.laps = CIRCUIT_LAPS[self.setup.circuit_size_now()];
                return;
            }
            by += step;
            if named.is_none() {
                let d = selector(&mut self.focus, "Size", CIRCUIT_SIZES[self.setup.circuit_size.min(3)], bx, by, cw, rh, MINT);
                if d != 0 {
                    self.setup.circuit_size = wrap(self.setup.circuit_size, d, 4);
                    self.setup.laps = CIRCUIT_LAPS[self.setup.circuit_size];
                }
                by += step;
                let d = selector(&mut self.focus, "Scenery", &biome_name(self.setup.biome, self.setup.seed), bx, by, cw, rh, MINT);
                self.setup.biome = wrap(self.setup.biome, d, ALL_BIOMES.len() + 1);
                by += step;
                let d = selector(&mut self.focus, "Difficulty", ["Flowing", "Technical", "Tricky"][self.setup.track_difficulty.min(2)], bx, by, cw, rh, MINT);
                self.setup.track_difficulty = wrap(self.setup.track_difficulty, d, 3);
                by += step;
                let d = selector(&mut self.focus, "Seed", &format!("{}", self.setup.seed), bx, by, cw, rh, MINT);
                if d != 0 {
                    self.setup.seed = random_seed();
                }
                by += step;
            }
        } else {
            let d = selector(&mut self.focus, "Scenery", &biome_name(self.setup.biome, self.setup.seed), bx, by, cw, rh, MINT);
            self.setup.biome = wrap(self.setup.biome, d, ALL_BIOMES.len() + 1);
            by += step;
            let d = selector(&mut self.focus, "Surface", SURFACES[self.setup.surface_mode], bx, by, cw, rh, MINT);
            self.setup.surface_mode = wrap(self.setup.surface_mode, d, SURFACES.len());
            by += step;
            let d = selector(&mut self.focus, "Difficulty", TRACK_DIFFICULTY[self.setup.track_difficulty], bx, by, cw, rh, MINT);
            self.setup.track_difficulty = wrap(self.setup.track_difficulty, d, TRACK_DIFFICULTY.len());
            by += step;
            let d = selector(&mut self.focus, "Length", LENGTHS[self.setup.length as usize], bx, by, cw, rh, MINT);
            self.setup.length = wrap(self.setup.length as usize, d, 3) as u8;
            by += step;
            let d = selector(&mut self.focus, "Seed", &format!("{}", self.setup.seed), bx, by, cw, rh, MINT);
            if d != 0 {
                self.setup.seed = random_seed();
            }
            by += step;
        }
        // a little track card under the track options: what you'll get
        let card_h = (ay - by + rh).min(if named.is_some() { 190.0 } else { 110.0 } * s);
        if card_h > 60.0 * s {
            rounded_rect(bx, by + 4.0 * s, cw, card_h - 8.0 * s, 14.0 * s, with_alpha(CREAM_DEEP, 0.7));
            let opp = if tt { "time trial".to_string() } else { format!("{} opponents", self.setup.opponents) };
            let (title, line, blurb) = match (circuit, &named) {
                (true, Some(c)) => (c.name.clone(), format!("{} - {} laps - {}", CIRCUIT_SIZES[c.params.length_class.min(3) as usize], self.setup.laps, opp), Some(c.blurb.clone())),
                (true, None) => ("A brand-new circuit".to_string(), format!("{} - {} laps - {}", CIRCUIT_SIZES[self.setup.circuit_size.min(3)], self.setup.laps, opp), None),
                _ => (if self.setup.biome == 0 { "A surprise biome".to_string() } else { ALL_BIOMES[self.setup.biome - 1].style().name.to_string() }, format!("{} laps  -  {} track  -  {}", self.setup.laps, LENGTHS[self.setup.length as usize].to_lowercase(), opp), None),
            };
            let tfs = (21.0 * s).min((cw - 32.0 * s) / text_size(&title, 21.0 * s).x.max(1.0) * 21.0 * s);
            text(&title, bx + 16.0 * s, by + 36.0 * s, tfs, INK);
            let fs = (16.0 * s).min((cw - 32.0 * s) / text_size(&line, 16.0 * s).x.max(1.0) * 16.0 * s);
            text(&line, bx + 16.0 * s, by + 62.0 * s, fs, INK_SOFT);
            if let Some(b) = blurb {
                // simple word wrap for the circuit blurb
                let mut lines = vec![String::new()];
                for word in b.split_whitespace() {
                    let cand = if lines.last().unwrap().is_empty() { word.to_string() } else { format!("{} {}", lines.last().unwrap(), word) };
                    if text_size(&cand, 16.0 * s).x > cw - 32.0 * s && !lines.last().unwrap().is_empty() {
                        lines.push(word.to_string());
                    } else {
                        *lines.last_mut().unwrap() = cand;
                    }
                }
                for (i, l) in lines.iter().enumerate().take(((card_h - 90.0 * s) / (22.0 * s)).max(0.0) as usize) {
                    text(l, bx + 16.0 * s, by + 92.0 * s + i as f32 * 22.0 * s, 16.0 * s, INK);
                }
            }
        }
        self.focus.label_w = None;
        // --- buttons
        let btn_y = y + h - button_h - 22.0 * s;
        let back = button_back(&mut self.focus, "Back", ax, btn_y, cw, button_h, CREAM);
        let go = button(&mut self.focus, "Start!", bx, btn_y, cw, button_h, PEACH);
        if back || self.focus.back {
            if self.focus.back {
                audio::ui_sound(audio::UiSound::Back);
            }
            self.goto(if self.online.setup_for_online { Screen::Online } else { Screen::Title });
            self.online.setup_for_online = false;
            return;
        }
        if go && self.online.setup_for_online {
            // leader picked the online race: send it to the lobby instead of starting
            self.online.setup_for_online = false;
            let laps = if self.setup.mode() == gameplay::GameMode::TimeTrial { 1 } else { self.setup.laps };
            let rs = rally_net::RaceSettings { track: self.setup.track_params(), laps, contact: self.online.contact };
            self.online.send(&rally_net::ClientMsg::SetSettings(rs));
            self.goto(Screen::Online);
            return;
        }
        if go {
            let su = &self.setup;
            let (a, dsc, c, cs, car, circ) = (su.assists, su.discipline, su.circuit, su.circuit_size, su.cars[0], su.is_circuit());
            settings::update(|st| {
                st.assists = a;
                st.discipline = dsc;
                st.circuit = c;
                st.circuit_size = cs;
                if circ {
                    st.car_circuit = car;
                } else {
                    st.car_rally = car;
                }
            });
            settings::save();
            self.start_race();
            return;
        }

        // --- car previews + stats on the right
        let px0 = x + w + margin;
        let pw = sw - px0 - margin;
        if pw < 180.0 * s {
            return;
        }
        let ph = if players == 2 { (h - margin) * 0.5 } else { h };
        for p in 0..players {
            let py = y + p as f32 * (ph + margin);
            let px = px0 + (1.0 - appear) * 60.0 * s;
            panel(px, py, pw, ph);
            let spec = &specs[self.setup.cars[p]];
            let compact = players == 2;
            let name_size = if compact { 32.0 } else { 42.0 } * s;
            text(spec.name, px + 22.0 * s, py + if compact { 42.0 } else { 54.0 } * s, name_size, INK);
            if compact {
                pill(&format!("P{}", p + 1), px + pw - 34.0 * s, py + 30.0 * s, 17.0 * s, [PEACH, SKY][p], INK);
            }
            let drive = match spec.drivetrain {
                rally_core::vehicle::Drivetrain::Fwd => "FWD",
                rally_core::vehicle::Drivetrain::Rwd => "RWD",
                rally_core::vehicle::Drivetrain::Awd => "AWD",
            };
            let dy = py + if compact { 64.0 } else { 80.0 } * s;
            // badges: class, tyres, drivetrain
            let mut bx2 = px + 22.0 * s;
            for (label, col) in [(hud::class_label(spec), hud::class_color(spec)), (hud::tyre_label(spec), with_alpha(BUTTER, 0.7)), (drive, with_alpha([PEACH, SKY][p], 0.55))] {
                let tw = text_size(label, 15.0 * s).x;
                pill(label, bx2 + tw * 0.5 + 8.0 * s, dy, 15.0 * s, col, INK);
                bx2 += tw + 26.0 * s;
            }
            // slicks on a loose rally road: say so clearly
            if !self.setup.is_circuit() && spec.tyre == rally_core::vehicle::TyreType::Slick {
                let warn = "Slicks: tarmac only!";
                let tw = text_size(warn, 15.0 * s).x;
                pill(warn, bx2 + tw * 0.5 + 8.0 * s, dy, 15.0 * s, BERRY, CREAM);
            }
            // stat bars scaled across every class (rally hatch .. formula)
            let cols = [PEACH, SKY, MINT, LAVENDER];
            let stats: Vec<(&str, f32, Color, String)> = hud::car_stats(spec).into_iter().zip(cols).map(|((n, v, val), c)| (n, v, c, val)).collect();
            let stat_top = dy + 22.0 * s;
            // solo: 2x2 stat grid above a big turntable; split: stacked stats left, car right
            let colw = if compact { pw * 0.42 } else { (pw - 44.0 * s - 16.0 * s) * 0.5 };
            for (i, (name, v, col, val)) in stats.iter().enumerate() {
                let (cx_i, cy_i) = if compact { (0, i) } else { (i % 2, i / 2) };
                let sx = px + 22.0 * s + cx_i as f32 * (colw + 16.0 * s);
                let sy = stat_top + cy_i as f32 * if compact { 34.0 } else { 40.0 } * s;
                text(name, sx, sy + 14.0 * s, 15.0 * s, INK_SOFT);
                text_right(val, sx + colw, sy + 14.0 * s, 14.0 * s, INK);
                let byy = sy + 21.0 * s;
                rounded_rect(sx, byy, colw, 11.0 * s, 5.5 * s, with_alpha(INK, 0.08));
                let f = v.clamp(0.08, 1.0);
                rounded_rect_gradient(sx, byy, colw * f, 11.0 * s, 5.5 * s, lighten(*col, 0.3), *col);
            }
            let car = self.setup.cars[p];
            let paint = self.setup.paints[p];
            if compact {
                let vx = px + 22.0 * s + colw + 10.0 * s;
                let vy = py + 50.0 * s;
                self.draw_car_preview(car, paint, vx, vy, px + pw - 8.0 * s - vx, py + ph - 10.0 * s - vy);
            } else {
                let top = stat_top + 84.0 * s;
                let hh = (py + ph - 52.0 * s - top).max(60.0);
                self.draw_car_preview(car, paint, px + 8.0 * s, top, pw - 16.0 * s, hh);
            }
            if !compact {
                let bl = spec.blurb;
                let fs = (17.0 * s).min((pw - 40.0 * s) / text_size(bl, 17.0 * s).x * 17.0 * s);
                text_mid(bl, px + pw * 0.5, py + ph - 52.0 * s * 0.55, fs, INK_SOFT);
            }
        }
    }

    fn settings_screen(&mut self) {
        let over_race = self.settings_from == Screen::Race && self.session.is_some();
        if over_race {
            self.draw_race_frozen();
            draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.24, 0.18, 0.3, 0.35));
        } else {
            self.menu_backdrop();
            screen_wash(Color::new(1.0, 0.96, 0.9, 0.35), Color::new(1.0, 0.94, 0.88, 0.5));
        }
        let s = scale();
        let w = 960.0 * s;
        let h = 600.0 * s;
        let k = ease_back((self.screen_t / 0.35).min(1.0));
        let x = (screen_width() - w) * 0.5;
        let y = (screen_height() - h) * 0.5 + (1.0 - k) * 40.0 * s;
        panel(x, y, w, h);
        text_mid("Settings", x + w * 0.5, y + 46.0 * s, 42.0 * s, INK);
        let mut st = settings::get();
        let before = st;
        self.focus.begin(14);
        let cw = (w - 3.0 * 32.0 * s) * 0.5;
        let lx = x + 32.0 * s;
        let rx = lx + cw + 32.0 * s;
        let rh = 44.0 * s;
        let step = rh + 9.0 * s;
        let header = |label: &str, hx: f32, hy: f32| {
            text(label, hx + 4.0 * s, hy, 20.0 * s, INK_SOFT);
            rounded_rect(hx + text_size(label, 20.0 * s).x + 16.0 * s, hy - 7.0 * s, cw - text_size(label, 20.0 * s).x - 20.0 * s, 2.0 * s, 1.0, with_alpha(INK, 0.12));
        };
        // left column: sound + display
        let mut ly = y + 96.0 * s;
        header("Sound", lx, ly);
        ly += 14.0 * s;
        slider(&mut self.focus, "Master", &mut st.master, lx, ly, cw, rh, PEACH);
        ly += step;
        slider(&mut self.focus, "Music", &mut st.music, lx, ly, cw, rh, PEACH);
        ly += step;
        slider(&mut self.focus, "Effects", &mut st.sfx, lx, ly, cw, rh, PEACH);
        ly += step + 30.0 * s;
        header("Display", lx, ly);
        ly += 14.0 * s;
        if toggle(&mut self.focus, "Fullscreen", &mut st.fullscreen, lx, ly, cw, rh, SKY) {
            set_fullscreen(st.fullscreen);
        }
        ly += step;
        let d = selector(&mut self.focus, "Graphics", st.quality.name(), lx, ly, cw, rh, SKY);
        if d != 0 {
            st.quality = if st.quality == settings::Quality::High { settings::Quality::Low } else { settings::Quality::High };
        }
        ly += step;
        toggle(&mut self.focus, "Show FPS", &mut st.show_fps, lx, ly, cw, rh, SKY);
        // right column: gameplay
        let mut ry = y + 96.0 * s;
        header("Driving", rx, ry);
        ry += 14.0 * s;
        let d = selector(&mut self.focus, "Assists", settings::ASSIST_NAMES[st.assists], rx, ry, cw, rh, MINT);
        st.assists = ((st.assists as i32 + d).rem_euclid(settings::ASSIST_NAMES.len() as i32)) as usize;
        ry += step;
        let d = selector(&mut self.focus, "Camera", settings::CAMERA_NAMES[st.camera], rx, ry, cw, rh, MINT);
        st.camera = ((st.camera as i32 + d).rem_euclid(settings::CAMERA_NAMES.len() as i32)) as usize;
        ry += step;
        toggle(&mut self.focus, "Grip widget", &mut st.show_traction, rx, ry, cw, rh, BUTTER);
        ry += step;
        toggle(&mut self.focus, "Pace notes", &mut st.pace_notes, rx, ry, cw, rh, BUTTER);
        ry += step;
        toggle(&mut self.focus, "Notes on circuits", &mut st.pace_notes_circuit, rx, ry, cw, rh, BUTTER);
        ry += step;
        // [gameplay] racing-line assist (key L in race)
        let d = selector(&mut self.focus, "Racing line", settings::RACING_LINE_NAMES[st.racing_line], rx, ry, cw, rh, BUTTER);
        st.racing_line = ((st.racing_line as i32 + d).rem_euclid(settings::RACING_LINE_NAMES.len() as i32)) as usize;
        ry += step + 30.0 * s;
        header("Help", rx, ry);
        ry += 14.0 * s;
        let controls = button(&mut self.focus, "Controls", rx, ry, cw, rh, LAVENDER);
        let back = button_back(&mut self.focus, "Back", x + w * 0.5 - 120.0 * s, y + h - 72.0 * s, 240.0 * s, 50.0 * s, PEACH);
        if st != before {
            if st.assists != before.assists {
                self.setup.assists = st.assists;
            }
            settings::set(st);
        }
        if controls {
            settings::save();
            self.help_from = Screen::Settings;
            if over_race {
                self.switch_now(Screen::Help);
            } else {
                self.goto(Screen::Help);
            }
        } else if back || self.focus.back {
            if self.focus.back {
                audio::ui_sound(audio::UiSound::Back);
            }
            let to = self.settings_from;
            if over_race {
                settings::save();
                // apply changes made from the pause menu to the race in progress
                if let Some(sess) = self.session.as_mut() {
                    let st = settings::get();
                    let cam = cam_mode(st.camera);
                    for p in sess.players.iter_mut() {
                        p.cam.mode = cam;
                    }
                    // campaign runs keep the assists they started with (times are tagged with them)
                    if sess.config.mode != gameplay::GameMode::Campaign {
                        let lvl = ASSISTS[st.assists].1;
                        sess.config.assist_level = lvl;
                        for p in sess.players.iter() {
                            sess.world.cars[p.car].assist_level = lvl;
                        }
                    }
                }
                self.screen = Screen::Race;
                self.pause_focus = Focus::default();
                self.pause_focus.index = 2;
            } else {
                self.goto(to);
            }
        }
    }

    /// The paused race drawn without advancing it (backdrop for pause overlays)
    fn draw_race_frozen(&mut self) {
        let Some(sess) = self.session.as_ref() else { return };
        let horizon = sess.scene.sky_horizon;
        clear_background(Color::new(horizon[0], horizon[1], horizon[2], 1.0));
        sess.draw_3d(&mut self.gpu, |_, _, _| {});
        if let Some(h) = &self.hud {
            let vps = sess.viewports(screen_width(), screen_height());
            for (i, vp) in vps.iter().enumerate() {
                hud::draw_hud(sess, h, i, vp);
                gameplay::draw_hud(sess, i, vp); // [gameplay] time-trial delta
            }
        }
    }

    fn race(&mut self) {
        let dt = get_frame_time().min(0.05);
        let Some(sess) = self.session.as_mut() else {
            self.switch_now(Screen::Title);
            return;
        };
        if !input_blocked() && (is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::P) || input::pause_pressed()) {
            sess.paused = !sess.paused;
            self.pause_focus = Focus::default();
            audio::ui_sound(if sess.paused { audio::UiSound::Select } else { audio::UiSound::Back });
        }
        // test hook: open the pause menu once the race is showing
        if matches!(self.hooks.after_load.as_deref(), Some("pause") | Some("pause_settings")) && self.screen_t > 2.0 {
            let to_settings = self.hooks.after_load.as_deref() == Some("pause_settings");
            self.hooks.after_load = None;
            sess.paused = true;
            if to_settings {
                self.settings_from = Screen::Race;
                self.switch_now(Screen::Settings);
                return;
            }
        }
        // [gameplay] test hook: COZY_WARP=<seconds> fast-forwards the race once (autopilot drives)
        if self.hooks.autoplay {
            if let Some(w) = std::env::var("COZY_WARP").ok().and_then(|v| v.parse::<f32>().ok()).filter(|_| sess.time < 0.5) {
                let step = 1.0 / 60.0;
                for _ in 0..(w / step) as usize {
                    let i = sess.players[0].car;
                    let t = self.autopilots[0].drive(&sess.world.cars[i], &sess.world.track, step);
                    let n_ap = self.autopilots.len();
                    for k in 1..sess.players.len() {
                        let c = sess.players[k].car;
                        let inp = self.autopilots[k.min(n_ap - 1)].drive(&sess.world.cars[c], &sess.world.track, step);
                        sess.world.set_input(c, inp);
                    }
                    sess.update(step, Some(t));
                    // (player inputs are re-sampled inside update: re-apply P2's autopilot)
                    for k in 1..sess.players.len() {
                        let c = sess.players[k].car;
                        let inp = self.autopilots[k.min(n_ap - 1)].drive(&sess.world.cars[c], &sess.world.track, step);
                        sess.world.set_input(c, inp);
                    }
                }
            }
        }
        // autopilot for automated playtests
        let mut touch = None;
        if self.hooks.autoplay && !sess.paused {
            let i = sess.players[0].car;
            touch = Some(self.autopilots[0].drive(&sess.world.cars[i], &sess.world.track, dt));
            if sess.players.len() > 1 {
                let j = sess.players[1].car;
                let inp = self.autopilots[1].drive(&sess.world.cars[j], &sess.world.track, dt);
                sess.world.set_input(j, inp);
            }
        }
        self.online.pre_update(sess, dt);
        sess.update(dt, touch);
        self.online.post_update(sess, dt);
        // [gameplay] campaign instant restart (T)
        if campaign_mode::take_restart(&sess.config) {
            self.start_race();
            return;
        }
        if self.hooks.autoplay && sess.players.len() > 1 {
            let j = sess.players[1].car;
            let inp = self.autopilots[1].drive(&sess.world.cars[j], &sess.world.track, dt);
            sess.world.set_input(j, inp);
        }
        if let Some(h) = self.hud.as_mut() {
            h.update(sess, if sess.paused { 0.0 } else { dt });
        }
        let horizon = sess.scene.sky_horizon;
        clear_background(Color::new(horizon[0], horizon[1], horizon[2], 1.0));
        sess.draw_3d(&mut self.gpu, |_, _, _| {});
        let vps = sess.viewports(screen_width(), screen_height());
        if let Some(h) = &self.hud {
            for (i, vp) in vps.iter().enumerate() {
                hud::draw_hud(sess, h, i, vp);
                gameplay::draw_hud(sess, i, vp); // [gameplay] time-trial delta
            }
        }
        if vps.len() == 2 {
            if vps[0].w < screen_width() * 0.9 {
                draw_rectangle(screen_width() * 0.5 - 2.0, 0.0, 4.0, screen_height(), CREAM);
            } else {
                draw_rectangle(0.0, screen_height() * 0.5 - 2.0, screen_width(), 4.0, CREAM);
            }
        }
        if let Some(n) = &sess.night {
            let s = scale();
            for (i, vp) in vps.iter().enumerate() {
                let Some(p) = sess.players.get(i) else { continue };
                let cx = vp.x + vp.w * 0.5;
                let y = vp.y + vp.h - 34.0 * s;
                if n.lamp_msg[p.car] < 3.0 {
                    pill("Headlight smashed! It'll flicker back...", cx, vp.y + vp.h * 0.36, 20.0 * s, with_alpha(BERRY, 0.92), CREAM);
                }
                let beam = if n.high_beam(p.car) { "High beam (H)" } else { "Low beam (H)" };
                pill(beam, cx - vp.w * 0.17, y, 16.0 * s, with_alpha(if n.high_beam(p.car) { SKY } else { CREAM }, 0.85), INK);
            }
        }
        if let Some(line) = self.online.status_line(sess) {
            let s = scale();
            pill(&line, screen_width() * 0.5, screen_height() * 0.30, 22.0 * s, with_alpha(CREAM, 0.92), INK);
        }
        // small on-screen pause button for mouse / touch players
        PAUSE_BUTTON.store(!sess.paused && self.pointer_seen > 0.0, std::sync::atomic::Ordering::Relaxed);
        if !sess.paused && self.pointer_seen > 0.0 {
            let s = scale();
            let r = 22.0 * s;
            let (bx, by) = (screen_width() * 0.5, screen_height() - r - 14.0 * s);
            let a = self.pointer_seen.min(1.0) * 0.85;
            circle(bx, by + 2.0 * s, r, with_alpha(SHADOW, a * 0.5));
            circle(bx, by, r, with_alpha(CREAM, a));
            rounded_rect(bx - 7.0 * s, by - 8.0 * s, 5.0 * s, 16.0 * s, 2.0 * s, with_alpha(INK, a));
            rounded_rect(bx + 2.0 * s, by - 8.0 * s, 5.0 * s, 16.0 * s, 2.0 * s, with_alpha(INK, a));
            let hit = Rect::new(bx - r, by - r, r * 2.0, r * 2.0);
            let clicked = (is_mouse_button_pressed(MouseButton::Left) && hit.contains(mouse_position().into())) || touches().iter().any(|t| t.phase == TouchPhase::Started && hit.contains(t.position));
            if clicked && !input_blocked() {
                sess.paused = true;
                self.pause_focus = Focus::default();
                audio::ui_sound(audio::UiSound::Select);
            }
        }
        let paused = sess.paused;
        if paused {
            self.pause_menu();
        } else if sess.is_over() && self.camp.active.is_some() {
            self.stage_over();
        } else if sess.is_over() {
            let car = sess.players.first().map(|p| p.car).unwrap_or(0);
            self.results_cine = Cinematic::orbit(car);
            self.results_cine.frame = vec2(-0.45, 0.42);
            self.switch_now(Screen::Results);
            self.set_music(audio::Music::Results);
        }
    }

    fn online_screen(&mut self) {
        self.menu_backdrop();
        // the lobby offers the cars of the race's discipline (rally cars until a race is picked)
        let circuit = self.online.client.as_ref().and_then(|c| c.settings.as_ref()).map(|st| st.track.discipline == rally_core::track::Discipline::Circuit).unwrap_or(self.setup.is_circuit());
        let want = if circuit { rally_core::vehicle::CarClass::Circuit } else { rally_core::vehicle::CarClass::Rally };
        let allowed: Vec<usize> = car_specs().iter().enumerate().filter(|(_, c)| c.class == want).map(|(i, _)| i).collect();
        let (mut car, mut paint) = (self.setup.cars[0], self.setup.paints[0]);
        if !allowed.contains(&car) && !allowed.is_empty() {
            car = allowed[0];
        }
        let mut f = std::mem::take(&mut self.focus);
        let action = self.online.screen(&mut f, &mut car, &mut paint, &allowed);
        self.focus = f;
        self.setup.cars[0] = car;
        self.setup.paints[0] = paint;
        match action {
            online::LobbyAction::Back => {
                audio::ui_sound(audio::UiSound::Back);
                self.goto(Screen::Title);
            }
            online::LobbyAction::PickRace => {
                self.online.setup_for_online = true;
                self.setup.split = false;
                self.goto(Screen::Setup);
            }
            online::LobbyAction::None => {}
        }
    }

    fn pause_menu(&mut self) {
        let sw = screen_width();
        let sh = screen_height();
        rounded_rect_gradient(0.0, 0.0, sw, sh, 0.0, Color::new(0.24, 0.18, 0.3, 0.30), Color::new(0.24, 0.18, 0.3, 0.45));
        let s = scale();
        let w = 380.0 * s;
        let h = 410.0 * s;
        let x = (sw - w) * 0.5;
        let y = (sh - h) * 0.5;
        panel(x, y, w, h);
        text_mid("Paused", x + w * 0.5, y + 46.0 * s, 44.0 * s, INK);
        if let Some(sess) = &self.session {
            let r = &sess.world.race;
            let me = sess.players.first().map(|p| &r.racers[p.car]);
            let sub = match me {
                Some(m) => format!("{}  -  Lap {}/{}  -  {}", hud::race_title(sess.track()), m.lap.min(r.laps), r.laps, hud::ordinal(m.place)),
                None => hud::race_title(sess.track()),
            };
            text_mid(&sub, x + w * 0.5, y + 82.0 * s, 18.0 * s, INK_SOFT);
        }
        let mut f = std::mem::take(&mut self.pause_focus);
        f.begin(4);
        let bw = w - 60.0 * s;
        let bh = 54.0 * s;
        let bx = x + 30.0 * s;
        let resume = button_back(&mut f, "Resume", bx, y + 112.0 * s, bw, bh, MINT);
        let online = self.online.in_race();
        let restart = button(&mut f, if online { "Racing online" } else { "Restart" }, bx, y + 180.0 * s, bw, bh, BUTTER) && !online;
        let sett = button(&mut f, "Settings", bx, y + 248.0 * s, bw, bh, SKY);
        let quit = button_back(&mut f, if online { "Leave race" } else { "Quit to menu" }, bx, y + 316.0 * s, bw, bh, PEACH);
        self.pause_focus = f;
        if resume {
            if let Some(sess) = self.session.as_mut() {
                sess.paused = false;
            }
        }
        if restart {
            self.start_race();
        }
        if sett {
            self.settings_from = Screen::Race;
            self.switch_now(Screen::Settings);
        }
        if quit {
            self.goto(if online { Screen::Online } else if self.camp.active.is_some() { Screen::Campaign } else { Screen::Title });
        }
    }

    fn results(&mut self) {
        let Some(sess) = self.session.as_mut() else {
            self.switch_now(Screen::Title);
            return;
        };
        // keep the world alive in the background (cars coast on autopilot), orbit the player
        let dt = get_frame_time().min(0.05);
        self.online.pre_update(sess, dt);
        sess.update(dt, None);
        self.online.post_update(sess, dt);
        self.results_cine.update(sess, dt);
        let horizon = sess.scene.sky_horizon;
        clear_background(Color::new(horizon[0], horizon[1], horizon[2], 1.0));
        sess.draw_3d(&mut self.gpu, |_, _, _| {});
        let sess = self.session.as_ref().unwrap();
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let t = self.screen_t;
        let fade = ease_out((t / 0.5).min(1.0));
        rounded_rect_gradient(0.0, 0.0, sw, sh, 0.0, Color::new(0.24, 0.18, 0.3, 0.05 * fade), Color::new(0.24, 0.18, 0.3, 0.35 * fade));
        let order = sess.world.race.standings();
        let local: Vec<usize> = sess.players.iter().map(|p| p.car).collect();
        let local_podium = order.iter().take(3).any(|c| local.contains(c));
        if local_podium {
            confetti(t);
        }
        // title
        let ty = 56.0 * s;
        let title_k = pop(t * 1.2);
        let headline = match order.iter().position(|c| local.contains(c)) {
            Some(0) => "You win!",
            Some(1) | Some(2) => "On the podium!",
            _ => "Race complete",
        };
        text_outlined_centered(headline, sw * 0.5, ty + 22.0 * s, (58.0 * s * title_k).max(1.0), BUTTER);
        text_outlined_centered(&format!("{}  -  {} lap{}", hud::race_title(sess.track()), sess.world.race.laps, if sess.world.race.laps == 1 { "" } else { "s" }), sw * 0.5, ty + 54.0 * s, 22.0 * s, with_alpha(CREAM, fade));

        // podium (left)
        let total_w = (1100.0 * s).min(sw - 40.0 * s);
        let left = (sw - total_w) * 0.5;
        let pod_w = total_w * 0.42;
        let base_y = sh - 150.0 * s;
        let col_w = pod_w / 3.0;
        let heights = [150.0 * s, 110.0 * s, 80.0 * s];
        let slots = [(1usize, 0usize), (0, 1), (2, 2)]; // (column, place index)
        for (col, place) in slots {
            let Some(&c) = order.get(place) else { continue };
            let delay = 0.3 + [0.4, 0.2, 0.0][place];
            let k = ease_back(((t - delay) / 0.45).clamp(0.0, 1.0));
            if k <= 0.0 {
                continue;
            }
            let bh = heights[place] * k;
            let bx = left + col as f32 * col_w + 6.0 * s;
            let bw = col_w - 12.0 * s;
            let col_c = [BUTTER, Color::new(0.86, 0.88, 0.95, 1.0), PEACH][place];
            soft_shadow(bx, base_y - bh, bw, bh, 14.0 * s, 1.0);
            rounded_rect_gradient(bx, base_y - bh, bw, bh + 20.0 * s, 14.0 * s, lighten(col_c, 0.3), col_c);
            text_mid(&format!("{}", place + 1), bx + bw * 0.5, base_y - bh + 40.0 * s, 46.0 * s * k.min(1.0), with_alpha(INK, 0.75));
            // racer badge on top
            let by = base_y - bh - 46.0 * s;
            let carc = hud::car_color(sess, c);
            let is_local = local.contains(&c);
            let bob = if place == 0 { (get_time() as f32 * 3.0).sin() * 3.0 * s } else { 0.0 };
            circle(bx + bw * 0.5, by + 3.0 * s + bob, 30.0 * s, with_alpha(SHADOW, 0.6));
            circle(bx + bw * 0.5, by + bob, 30.0 * s, if is_local { BUTTER } else { CREAM });
            circle(bx + bw * 0.5, by + bob, 23.0 * s, carc);
            let initial: String = sess.names[c].chars().next().map(|ch| ch.to_string()).unwrap_or_default();
            text_mid(&initial, bx + bw * 0.5, by + bob, 26.0 * s, INK);
            if place == 0 {
                crown(bx + bw * 0.5, by - 38.0 * s + bob, 20.0 * s);
            }
            let name_y = base_y + 40.0 * s;
            text_outlined_centered(&sess.names[c], bx + bw * 0.5, name_y, 24.0 * s, if is_local { BUTTER } else { CREAM });
            text_outlined_centered(sess.world.cars[c].spec.name, bx + bw * 0.5, name_y + 24.0 * s, 17.0 * s, with_alpha(CREAM, 0.9));
        }

        // standings table (right)
        let tx = left + pod_w + 24.0 * s;
        let tw = total_w - pod_w - 24.0 * s;
        let n = order.len();
        let row_h = (34.0 * s).min((sh - 330.0 * s) / n.max(1) as f32);
        let th = 56.0 * s + n as f32 * row_h + 16.0 * s;
        let ty0 = 120.0 * s;
        let slide = ease_out(((t - 0.2) / 0.5).clamp(0.0, 1.0));
        let tx = tx + (1.0 - slide) * 80.0 * s;
        panel(tx, ty0, tw, th);
        let cols = [tx + 20.0 * s, tx + 64.0 * s, tx + tw * 0.40, tx + tw * 0.66, tx + tw - 20.0 * s];
        let hy = ty0 + 34.0 * s;
        for (i, h) in ["#", "Driver", "Car", "Time", "Best lap"].iter().enumerate() {
            if i == 4 {
                text_right(h, cols[4], hy, 16.0 * s, INK_SOFT);
            } else {
                text(h, cols[i], hy, 16.0 * s, INK_SOFT);
            }
        }
        let winner_time = order.first().and_then(|&c| sess.world.race.racers[c].finish_time);
        let fastest = sess.world.race.racers.iter().filter_map(|r| r.best_lap).fold(f32::MAX, f32::min);
        for (row, &i) in order.iter().enumerate() {
            let r = &sess.world.race.racers[i];
            let ry = ty0 + 50.0 * s + row as f32 * row_h;
            let ra = ((t - 0.35 - row as f32 * 0.05) / 0.3).clamp(0.0, 1.0);
            if ra <= 0.0 {
                continue;
            }
            let mid = ry + row_h * 0.5;
            if local.contains(&i) {
                rounded_rect(tx + 10.0 * s, ry + 2.0 * s, tw - 20.0 * s, row_h - 4.0 * s, 10.0 * s, with_alpha(BUTTER, 0.55 * ra));
            } else if row % 2 == 1 {
                rounded_rect(tx + 10.0 * s, ry + 2.0 * s, tw - 20.0 * s, row_h - 4.0 * s, 10.0 * s, with_alpha(CREAM_DEEP, 0.5 * ra));
            }
            let fs = (row_h * 0.52).min(20.0 * s);
            let ink = with_alpha(INK, ra);
            circle(cols[0] + 10.0 * s, mid, row_h * 0.33, with_alpha(hud::car_color(sess, i), ra));
            text_mid(&format!("{}", row + 1), cols[0] + 10.0 * s, mid, fs * 0.85, ink);
            text(&sess.names[i], cols[1], mid_baseline(mid, fs), fs, ink);
            text(sess.world.cars[i].spec.name, cols[2], mid_baseline(mid, fs * 0.9), fs * 0.9, with_alpha(INK_SOFT, ra));
            let time = match (r.finish_time, winner_time) {
                (Some(ft), Some(wt)) if row > 0 => format!("+{:.2}", ft - wt),
                (Some(ft), _) => format_time(ft),
                _ => "DNF".into(),
            };
            text(&time, cols[3], mid_baseline(mid, fs), fs, ink);
            if let Some(b) = r.best_lap {
                let is_fastest = (b - fastest).abs() < 1e-4;
                let txt = format_time(b);
                if is_fastest {
                    let w = text_size(&txt, fs * 0.9).x;
                    rounded_rect(cols[4] - w - 10.0 * s, mid - fs * 0.62, w + 14.0 * s, fs * 1.24, fs * 0.62, with_alpha(LAVENDER, 0.8 * ra));
                    text_right(&txt, cols[4] - 3.0 * s, mid_baseline(mid, fs * 0.9), fs * 0.9, ink);
                } else {
                    text_right(&txt, cols[4], mid_baseline(mid, fs * 0.9), fs * 0.9, with_alpha(INK_SOFT, ra));
                }
            }
        }
        // buttons under the table
        let by = (ty0 + th + 20.0 * s).min(sh - 76.0 * s);
        let bw = (tw - 32.0 * s) / 3.0;
        let bh = 52.0 * s;
        if self.online.in_race() || self.online.connected() {
            let official = self.online.race.as_ref().is_some_and(|r| r.official.is_some());
            text(if official { "Official times from the server" } else { "Waiting for everyone to finish..." }, tx, by - 8.0 * s, 16.0 * s, INK_SOFT);
            self.focus.begin(1);
            if button(&mut self.focus, "Back to lobby", tx, by, tw, bh, MINT) || self.focus.back {
                self.goto(Screen::Online);
            }
            return;
        }
        self.focus.begin(3);
        let again = button(&mut self.focus, "Race again", tx, by, bw, bh, MINT);
        let new_track = button(&mut self.focus, "New track", tx + bw + 16.0 * s, by, bw, bh, BUTTER);
        let menu = button_back(&mut self.focus, "Menu", tx + 2.0 * (bw + 16.0 * s), by, bw, bh, PEACH);
        if again {
            self.start_race();
        }
        if new_track {
            self.setup.seed = random_seed();
            self.start_race();
        }
        if menu || self.focus.back {
            if self.focus.back {
                audio::ui_sound(audio::UiSound::Back);
            }
            self.goto(Screen::Title);
        }
    }

    fn loading(&mut self) {
        cozy_background(get_time() as f32);
        let s = scale();
        let cx = screen_width() * 0.5;
        let cy = screen_height() * 0.45;
        wheel_icon(cx, cy - 70.0 * s, 34.0 * s, get_time() as f32 * 4.0);
        text_outlined_centered("Building your track...", cx, cy + 10.0 * s, 46.0 * s, CREAM);
        let what = match (self.setup.is_circuit(), self.setup.named_circuit()) {
            (true, Some(c)) => c.name,
            (true, None) => format!("{} {}", biome_reveal(self.setup.biome, self.setup.seed), CIRCUIT_SIZES[self.setup.circuit_size.min(3)]),
            _ => biome_reveal(self.setup.biome, self.setup.seed),
        };
        text_mid(&what, cx, cy + 46.0 * s, 26.0 * s, INK);
        let tip = format!("Tip: {}", TIPS[self.tip % TIPS.len()]);
        let w = text_size(&tip, 20.0 * s).x + 40.0 * s;
        rounded_rect(cx - w * 0.5, cy + 86.0 * s, w, 40.0 * s, 20.0 * s, with_alpha(CREAM, 0.9));
        text_mid(&tip, cx, cy + 106.0 * s, 20.0 * s, INK_SOFT);
        // show this screen briefly, then fade out and build the track behind the cream
        if self.trans.phase == 0 {
            self.loading_frames += 1;
            if self.loading_frames >= 2 && self.screen_t > 0.5 {
                self.goto(Screen::Race);
            }
        }
    }

    fn draw_overlays(&mut self) {
        let st = settings::get();
        render::perf::overlay(&format!("{:?}", render::quality())); // [visuals] F3 perf overlay
        if get_frame_time() > 0.0 {
            self.fps = approach(self.fps, 1.0 / get_frame_time().max(1e-4), 3.0, get_frame_time().min(0.1));
        }
        if st.show_fps {
            let s = scale();
            let label = format!("{:.0} fps", self.fps);
            pill(&label, screen_width() * 0.5, screen_height() - 66.0 * s, 15.0 * s, with_alpha(CREAM, 0.85), INK);
        }
        // pointer presence (for the on-screen pause button): fades out when idle
        if mouse_delta_position().length() > 0.0 || !touches().is_empty() {
            self.pointer_seen = 3.0;
        } else {
            self.pointer_seen = (self.pointer_seen - get_frame_time()).max(0.0);
        }
    }
}

/// The bouncy "Cozy Rally" logo: each letter pops in, then bobs on a wave
fn draw_logo(x: f32, baseline: f32, size: f32, t: f32, age: f32) {
    let word = "Cozy Rally";
    let cols = [PEACH, BUTTER, MINT, SKY, CREAM, LAVENDER, PEACH, BUTTER, MINT, SKY];
    let mut cx = x;
    for (i, ch) in word.chars().enumerate() {
        let st = ch.to_string();
        let adv = text_size(&st, size).x + size * 0.02;
        if ch == ' ' {
            cx += size * 0.28;
            continue;
        }
        let k = pop((age - 0.05 * i as f32) * 1.3);
        let bob = (t * 2.2 - i as f32 * 0.55).sin() * size * 0.04;
        let sz = (size * k).max(1.0);
        let w = text_size(&st, sz).x;
        let lx = cx + (adv - w) * 0.5;
        // chunky ink outline + sticker-like cream border
        let o = size * 0.07;
        let y = baseline + bob;
        text(&st, lx, y + o * 0.9, sz, Color::new(0.2, 0.12, 0.25, 0.35));
        for a in 0..12 {
            let ang = a as f32 / 12.0 * std::f32::consts::TAU;
            text(&st, lx + ang.cos() * o, y + ang.sin() * o, sz, INK);
        }
        for a in 0..8 {
            let ang = a as f32 / 8.0 * std::f32::consts::TAU;
            text(&st, lx + ang.cos() * o * 0.45, y + ang.sin() * o * 0.45, sz, CREAM);
        }
        text(&st, lx, y, sz, cols[i % cols.len()]);
        cx += adv;
    }
}

fn wheel_icon(cx: f32, cy: f32, r: f32, ang: f32) {
    circle(cx, cy + 4.0, r, with_alpha(SHADOW, 0.6));
    circle(cx, cy, r, INK);
    circle(cx, cy, r * 0.62, CREAM_DEEP);
    for k in 0..5 {
        let a = ang + k as f32 / 5.0 * std::f32::consts::TAU;
        draw_line(cx, cy, cx + a.cos() * r * 0.6, cy + a.sin() * r * 0.6, r * 0.14, shade(CREAM_DEEP, 0.8));
    }
    circle(cx, cy, r * 0.2, PEACH);
}

fn crown(cx: f32, cy: f32, size: f32) {
    let w = size * 1.6;
    let h = size;
    let base = cy + h * 0.5;
    let pts = [vec2(cx - w * 0.5, base), vec2(cx - w * 0.5, cy - h * 0.2), vec2(cx - w * 0.25, cy + h * 0.15), vec2(cx, cy - h * 0.5), vec2(cx + w * 0.25, cy + h * 0.15), vec2(cx + w * 0.5, cy - h * 0.2), vec2(cx + w * 0.5, base)];
    for i in 1..pts.len() - 1 {
        draw_triangle(vec2(cx, base), pts[i], pts[i + 1], BUTTER);
    }
    draw_triangle(pts[0], pts[1], vec2(cx, base), BUTTER);
    rounded_rect(cx - w * 0.5, base - h * 0.18, w, h * 0.28, h * 0.1, shade(BUTTER, 0.85));
    for p in [pts[1], pts[3], pts[5]] {
        circle(p.x, p.y, size * 0.12, PEACH);
    }
}

/// Stateless confetti shower (deterministic per particle index)
fn confetti(t: f32) {
    let sw = screen_width();
    let sh = screen_height();
    let s = scale();
    let cols = [PEACH, MINT, SKY, BUTTER, LAVENDER, CREAM];
    for i in 0..90 {
        let h = (i as f32 * 12.9898).sin() * 43758.547;
        let r1 = h.fract().abs();
        let r2 = (h * 1.37).fract().abs();
        let r3 = (h * 2.71).fract().abs();
        let speed = 70.0 + r2 * 90.0;
        let start = r3 * 2.0;
        let tt = t - start;
        if tt < 0.0 {
            continue;
        }
        let y = (tt * speed * s) % (sh + 40.0 * s) - 20.0 * s;
        let x = r1 * sw + (tt * (1.0 + r2) + r3 * 6.0).sin() * 30.0 * s;
        let rot = tt * (2.0 + r1 * 3.0);
        let w = (6.0 + r2 * 5.0) * s;
        let flip = (tt * (3.0 + r3 * 4.0)).cos().abs().max(0.25);
        let a = (t / 0.6).min(1.0) * 0.95;
        draw_rectangle_ex(x, y, w, w * 0.6 * flip, DrawRectangleParams { offset: vec2(0.5, 0.5), rotation: rot, color: with_alpha(cols[i % cols.len()], a) });
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    ui::init_font();
    car_perf::start();
    audio::init();
    settings::apply_audio();
    let hooks = TestHooks::from_env();
    let st = settings::get();
    let mut app = App {
        gpu: Gpu::new(),
        screen: Screen::Title,
        setup: Setup { split: hooks.split, cars: [2, 1], paints: [0, 1], biome: 0, length: 1, seed: random_seed(), laps: 3, opponents: 3, difficulty: 1, assists: st.assists.min(ASSISTS.len() - 1), surface_mode: 0, track_difficulty: 1, mode: 0, discipline: st.discipline.min(1), circuit: st.circuit.min(rally_core::circuits::circuits().len()), circuit_size: st.circuit_size.min(3), night: false },
        session: None,
        hud: None,
        focus: Focus::default(),
        preview_angle: 0.6,
        preview_models: Vec::new(),
        pause_focus: Focus::default(),
        autopilots: Vec::new(),
        loading_frames: 0,
        trans: Transition { target: None, phase: -1, alpha: 1.0 },
        screen_t: 0.0,
        music: audio::Music::None,
        attract: None,
        attract_wait: 0,
        attract_fade: 0.0,
        cine: Cinematic::new(1),
        results_cine: Cinematic::orbit(0),
        settings_from: Screen::Title,
        help_from: Screen::Title,
        tip: (random_seed() % TIPS.len() as u64) as usize,
        fps: 60.0,
        pointer_seen: 0.0,
        camp: campaign_screens::CampState::new(),
        online: online::Online::new(),
        hooks,
    };
    app.set_music(audio::Music::Menu);
    set_input_blocked(true);
    if let Ok(seed) = std::env::var("COZY_SEED") {
        app.setup.seed = seed.parse().unwrap_or(1);
    }
    // circuits test hooks also pick the Circuit discipline in the setup screen
    if std::env::var("COZY_DISCIPLINE").is_ok_and(|v| v.eq_ignore_ascii_case("circuit")) {
        app.setup.discipline = 1;
        app.setup.circuit = 0;
    }
    if let Ok(id) = std::env::var("COZY_CIRCUIT") {
        if let Some(i) = rally_core::circuits::circuits().iter().position(|c| c.id == id) {
            app.setup.discipline = 1;
            app.setup.circuit = i + 1;
        }
    }
    if app.setup.is_circuit() {
        app.setup.laps = CIRCUIT_LAPS[app.setup.circuit_size_now()];
    }
    app.setup.fix_cars();
    // test hook: COZY_CAR=<car id> picks player 1's car
    if let Some(c) = std::env::var("COZY_CAR").ok().and_then(|v| v.parse::<usize>().ok()) {
        if c < rally_core::vehicle::car_specs().len() {
            app.setup.cars[0] = c;
        }
    }
    // [gameplay] test hook: COZY_STAGE=<stage id> starts a campaign stage directly
    if let Ok(id) = std::env::var("COZY_STAGE") {
        if campaign_mode::start_stage(&id, app.setup.cars[0], app.setup.paints[0], (ASSISTS[app.setup.assists].0, ASSISTS[app.setup.assists].1)).is_some() {
            app.screen = Screen::Loading;
        }
    }
    if std::env::var("COZY_MODE").is_ok_and(|m| m == "tt" || m == "timetrial") {
        app.setup.mode = 1; // [gameplay] time trial test hook
    }
    if let Ok(b) = std::env::var("COZY_BIOME") {
        app.setup.biome = b.parse().unwrap_or(0);
    }
    // COZY_CAMPAIGN_STAGE=<stage id> preselects a campaign stage (for stage_intro / stage / stage_results)
    if let Ok(id) = std::env::var("COZY_CAMPAIGN_STAGE") {
        for (ci, c) in campaign_data::chapters().iter().enumerate() {
            if let Some(si) = c.stages.iter().position(|st| st.id == id) {
                app.camp.chapter = ci;
                app.camp.stage = si;
            }
        }
    }
    if let Ok(scr) = std::env::var("COZY_SCREEN") {
        app.screen = match scr.as_str() {
            "setup" => Screen::Setup,
            "help" => Screen::Help,
            "settings" => Screen::Settings,
            "race" | "loading" => Screen::Loading,
            "campaign" => Screen::Campaign,
            "stage_intro" => Screen::StageIntro,
            "leaderboard" => Screen::Leaderboard,
            "stage" | "stage_results" => {
                app.camp.begin();
                if scr == "stage_results" {
                    app.hooks.after_load = Some("results".into());
                }
                Screen::Loading
            }
            "pause" | "pause_settings" | "results" => {
                app.hooks.after_load = Some(scr.clone());
                Screen::Loading
            }
            _ => Screen::Title,
        };
    } else if app.hooks.autoplay {
        app.screen = Screen::Loading;
    }
    let start = get_time();
    // test hooks: COZY_ONLINE=<address> joins a server at startup (COZY_NAME sets the name);
    // COZY_ONLINE_LEAD=<n> makes this client pick a 1-lap race and start once n players are in
    let online_lead: Option<usize> = std::env::var("COZY_ONLINE_LEAD").ok().and_then(|v| v.parse().ok());
    let mut online_lead_done = false;
    if let Ok(addr) = std::env::var("COZY_ONLINE") {
        let name = std::env::var("COZY_NAME").unwrap_or_else(|_| "Tester".into());
        app.online.client = Some(rally_net::NetClient::connect(&addr, &name, app.setup.cars[0] as u8, app.setup.paints[0] as u8));
        app.screen = Screen::Online;
    }
    let mut shot_idx = 0;
    loop {
        if let (Some(n), false) = (online_lead, online_lead_done) {
            if app.online.client.as_ref().is_some_and(|c| c.is_leader() && c.players.len() >= n) {
                online_lead_done = true;
                let rs = rally_net::RaceSettings { track: app.setup.track_params(), laps: 1, contact: false };
                app.online.send(&rally_net::ClientMsg::SetSettings(rs));
                app.online.send(&rally_net::ClientMsg::StartRace);
            }
        }
        input::poll(); // [gameplay] gamepads
        app.screen_t += get_frame_time().min(0.05);
        app.online.poll();
        // the leader started a race: everyone in the lobby (or on results) goes to the line.
        // Retried every frame: `goto` is ignored while a fade is still playing.
        if app.online.pending_start.is_some() && app.screen != Screen::Loading && app.trans.target != Some(Screen::Loading) && app.trans.phase == 0 {
            app.start_race();
        }
        match app.screen {
            Screen::Title => app.title(),
            Screen::Help => app.help(),
            Screen::Setup => app.setup_screen(),
            Screen::Settings => app.settings_screen(),
            Screen::Loading => app.loading(),
            Screen::Race => app.race(),
            Screen::Results => app.results(),
            Screen::Campaign => app.campaign_screen(),
            Screen::StageIntro => app.stage_intro(),
            Screen::StageResults => app.stage_results(),
            Screen::Leaderboard => app.leaderboard_screen(),
            Screen::Online => app.online_screen(),
        }
        app.draw_overlays();
        app.update_transition();
        let elapsed = (get_time() - start) as f32;
        if shot_idx < app.hooks.shots.len() && elapsed >= app.hooks.shots[shot_idx] {
            // flush 2D so the HUD is in the capture
            unsafe { get_internal_gl().flush() };
            let img = get_screen_data();
            let path = format!("{}/shot_{:02}_{:.0}s.png", app.hooks.shot_dir, shot_idx, elapsed);
            img.export_png(&path);
            println!("saved {path}");
            shot_idx += 1;
        }
        if let Some(e) = app.hooks.exit_after {
            if elapsed > e {
                if let Some(s) = &app.session {
                    let r = &s.world.race;
                    for i in r.standings() {
                        let rr = &r.racers[i];
                        println!("P{} {} lap {} finished {:?} best {:?}", rr.place, s.names[i], rr.lap, rr.finish_time, rr.best_lap);
                    }
                }
                std::process::exit(0);
            }
        }
        next_frame().await;
    }
}
