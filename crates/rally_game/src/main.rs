#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Cozy Rally — a cosy procedural rally racer.
mod audio;
mod camera;
mod game;
mod hud;
mod input;
mod models;
mod render;
mod ui;

use game::{PlayerSlot, RaceSession, SessionConfig};
use input::Scheme;
use macroquad::prelude::*;
use rally_core::biome::{Biome, ALL_BIOMES};
use rally_core::race::format_time;
use rally_core::track::TrackParams;
use rally_core::vehicle::car_specs;
use render::gpu::{DrawParams, Environment, Gpu, View};
use ui::*;

fn window_conf() -> Conf {
    Conf {
        window_title: "Cozy Rally".to_owned(),
        window_width: 1280,
        window_height: 720,
        high_dpi: true,
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
}

const DIFFICULTY: [(&str, f32); 3] = [("Chill", 0.35), ("Normal", 0.7), ("Spicy", 1.0)];
const ASSISTS: [(&str, f32); 4] = [("Cozy", 1.3), ("Normal", 1.0), ("Sport", 0.55), ("Sim", 0.0)];
const LENGTHS: [&str; 3] = ["Short", "Medium", "Long"];

impl Setup {
    fn config(&self) -> SessionConfig {
        let mut players = vec![PlayerSlot { car: self.cars[0], paint: self.paints[0], scheme: if self.split { Scheme::Wasd } else { Scheme::Both }, name: "Player 1".into() }];
        if self.split {
            players.push(PlayerSlot { car: self.cars[1], paint: self.paints[1], scheme: Scheme::Arrows, name: "Player 2".into() });
        }
        SessionConfig {
            track: TrackParams { seed: self.seed, biome: if self.biome == 0 { None } else { Some(ALL_BIOMES[self.biome - 1]) }, length_class: self.length },
            laps: self.laps,
            players,
            ai_count: self.opponents,
            ai_skill: DIFFICULTY[self.difficulty].1,
            assist_level: ASSISTS[self.assists].1,
        }
    }
}

fn random_seed() -> u64 {
    (macroquad::miniquad::date::now() * 1000.0) as u64 % 1_000_000
}

fn biome_name(i: usize, seed: u64) -> String {
    if i == 0 {
        let b = Biome::from_index((rally_core::rng::Rng::new(seed).next_u64() % 5) as usize);
        format!("Surprise ({})", b.style().name)
    } else {
        ALL_BIOMES[i - 1].style().name.to_string()
    }
}

/// Test hooks driven by environment variables (used for automated playtesting)
struct TestHooks {
    autoplay: bool,
    shots: Vec<f32>,
    shot_dir: String,
    exit_after: Option<f32>,
    split: bool,
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
        }
    }
}

struct App {
    gpu: Gpu,
    screen: Screen,
    setup: Setup,
    session: Option<RaceSession>,
    focus: Focus,
    preview_angle: f32,
    preview_models: Vec<(render::gpu::GpuMesh, render::gpu::GpuMesh, usize, usize)>,
    pause_focus: Focus,
    hooks: TestHooks,
    autopilots: Vec<rally_core::ai::AiDriver>,
    loading_frames: u32,
}

impl App {
    fn start_race(&mut self) {
        self.screen = Screen::Loading;
        self.loading_frames = 0;
    }

    fn finish_loading(&mut self) {
        if let Some(s) = self.session.take() {
            s.free(&mut self.gpu);
        }
        let cfg = self.setup.config();
        let s = RaceSession::new(cfg, &mut self.gpu);
        self.autopilots = (0..s.players.len()).map(|i| rally_core::ai::AiDriver::new(0.85, i as f32 * 1.5 - 0.7)).collect();
        self.session = Some(s);
        self.screen = Screen::Race;
        audio::set_music(audio::Music::Race);
    }

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

    fn draw_car_preview(&mut self, car: usize, paint: usize, x: f32, y: f32, w: f32, h: f32) {
        let idx = self.preview_model(car, paint);
        let spec = car_specs()[car].clone();
        let dpi = screen_dpi_scale();
        self.gpu.begin_3d((x * dpi) as i32, (y * dpi) as i32, (w * dpi) as i32, (h * dpi) as i32, true);
        let eye = vec3(4.2, 2.0, 4.8);
        let view = Mat4::look_at_rh(eye, vec3(0.0, 0.2, 0.0), Vec3::Y);
        let proj = Mat4::perspective_rh_gl(40f32.to_radians(), w / h, 0.1, 100.0);
        let v = View::new(view, proj, eye);
        let env = Environment { sun_dir: vec3(-0.4, 0.8, 0.5).normalize(), sun_color: [1.05, 0.98, 0.9], ambient: [0.5, 0.52, 0.58], fog_color: [1.0, 1.0, 1.0], fog_density: 0.0, time: 0.0 };
        let rot = Mat4::from_rotation_y(self.preview_angle);
        let ride = -spec.mount_height + spec.susp_rest * 0.8 + spec.wheel_radius - 0.6;
        let m = rot * Mat4::from_translation(vec3(0.0, ride, 0.0));
        let (body, wheel, _, _) = &self.preview_models[idx];
        // ground disc
        let mut disc = render::mesh::MeshBuilder::new();
        disc.cylinder(Mat4::from_translation(vec3(0.0, -0.62, 0.0)), 3.2, 0.03, 32, render::mesh::rgb([0.98, 0.9, 0.78]), render::mesh::rgb([0.98, 0.9, 0.78]));
        self.gpu.draw_builder(&disc, &v, &env, Mat4::IDENTITY, &DrawParams::default(), 0);
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
        self.gpu.end_3d();
    }

    fn title(&mut self) {
        cozy_background(get_time() as f32);
        let s = scale();
        let cx = screen_width() * 0.5;
        let t = get_time() as f32;
        let bob = (t * 1.6).sin() * 6.0 * s;
        text_outlined_centered("Cozy Rally", cx, screen_height() * 0.24 + bob, 110.0 * s, CREAM);
        text_centered("procedural dirt-road racing with friends", cx, screen_height() * 0.24 + 44.0 * s, 26.0 * s, INK);
        let bw = 340.0 * s;
        let bh = 62.0 * s;
        let x = cx - bw * 0.5;
        let mut y = screen_height() * 0.40;
        self.focus.begin(4);
        if button(&mut self.focus, "Solo race", x, y, bw, bh, PEACH) {
            self.setup.split = false;
            self.screen = Screen::Setup;
            self.focus = Focus::default();
            return;
        }
        y += bh + 18.0 * s;
        if button(&mut self.focus, "Split-screen (2P)", x, y, bw, bh, SKY) {
            self.setup.split = true;
            self.screen = Screen::Setup;
            self.focus = Focus::default();
            return;
        }
        y += bh + 18.0 * s;
        if button(&mut self.focus, "How to play", x, y, bw, bh, MINT) {
            self.screen = Screen::Help;
            self.focus = Focus::default();
            return;
        }
        y += bh + 18.0 * s;
        if button(&mut self.focus, "Quit", x, y, bw, bh, LAVENDER) {
            std::process::exit(0);
        }
        text_centered("v0.1", screen_width() - 40.0 * s, screen_height() - 16.0 * s, 18.0 * s, INK_SOFT);
    }

    fn help(&mut self) {
        cozy_background(get_time() as f32);
        let s = scale();
        let w = 760.0 * s;
        let h = 520.0 * s;
        let x = (screen_width() - w) * 0.5;
        let y = (screen_height() - h) * 0.5;
        panel(x, y, w, h);
        text_centered("How to play", x + w * 0.5, y + 56.0 * s, 46.0 * s, INK);
        let lines = [
            ("Drive", "W A S D  or  arrow keys"),
            ("Handbrake", "Space   (P2: Right Shift)"),
            ("Respawn on track", "R   (P2: Backspace)"),
            ("Change camera", "C   (P2: Right Ctrl)"),
            ("Pause", "Esc / P"),
            ("Split-screen", "P1 = WASD,  P2 = arrow keys"),
            ("Tip", "Lift off and flick the handbrake into hairpins!"),
            ("Tip", "Gravel and snow are slippery - be gentle."),
        ];
        for (i, (a, b)) in lines.iter().enumerate() {
            let ly = y + 110.0 * s + i as f32 * 42.0 * s;
            text(a, x + 50.0 * s, ly, 26.0 * s, INK);
            text(b, x + 290.0 * s, ly, 26.0 * s, INK_SOFT);
        }
        self.focus.begin(1);
        if button(&mut self.focus, "Back", x + w * 0.5 - 110.0 * s, y + h - 80.0 * s, 220.0 * s, 54.0 * s, PEACH) || is_key_pressed(KeyCode::Escape) {
            self.screen = Screen::Title;
            self.focus = Focus::default();
        }
    }

    fn setup_screen(&mut self) {
        cozy_background(get_time() as f32);
        self.preview_angle += get_frame_time() * 0.6;
        let s = scale();
        let sw = screen_width();
        let sh = screen_height();
        let w = (sw * 0.5).min(560.0 * s).max(420.0 * s).min(sw - 20.0);
        let x = 30.0 * s;
        let y = 24.0 * s;
        let h = sh - 48.0 * s;
        panel(x, y, w, h);
        text(if self.setup.split { "Split-screen race" } else { "Solo race" }, x + 24.0 * s, y + 48.0 * s, 40.0 * s, INK);
        let specs = car_specs();
        let rows = if self.setup.split { 13 } else { 11 };
        self.focus.begin(rows);
        let rh = ((h - 140.0 * s) / rows as f32 - 8.0 * s).min(48.0 * s);
        let mut ry = y + 72.0 * s;
        let rw = w - 48.0 * s;
        let rx = x + 24.0 * s;
        let step = rh + 8.0 * s;
        let wrap = |v: usize, d: i32, n: usize| ((v as i32 + d).rem_euclid(n as i32)) as usize;
        let players = if self.setup.split { 2 } else { 1 };
        for p in 0..players {
            let label = if players == 2 { format!("P{} car", p + 1) } else { "Car".into() };
            let d = selector(&mut self.focus, &label, specs[self.setup.cars[p]].name, rx, ry, rw, rh, [PEACH, SKY][p]);
            self.setup.cars[p] = wrap(self.setup.cars[p], d, specs.len());
            ry += step;
            let d = selector(&mut self.focus, if players == 2 { "  paint" } else { "Paint" }, &format!("#{}", self.setup.paints[p] + 1), rx, ry, rw, rh, [PEACH, SKY][p]);
            self.setup.paints[p] = wrap(self.setup.paints[p], d, models::PAINTS.len());
            ry += step;
        }
        let d = selector(&mut self.focus, "Scenery", &biome_name(self.setup.biome, self.setup.seed), rx, ry, rw, rh, MINT);
        self.setup.biome = wrap(self.setup.biome, d, ALL_BIOMES.len() + 1);
        ry += step;
        let d = selector(&mut self.focus, "Track", LENGTHS[self.setup.length as usize], rx, ry, rw, rh, MINT);
        self.setup.length = wrap(self.setup.length as usize, d, 3) as u8;
        ry += step;
        let d = selector(&mut self.focus, "Seed", &format!("{}", self.setup.seed), rx, ry, rw, rh, MINT);
        if d != 0 {
            self.setup.seed = random_seed();
        }
        ry += step;
        let d = selector(&mut self.focus, "Laps", &format!("{}", self.setup.laps), rx, ry, rw, rh, BUTTER);
        self.setup.laps = ((self.setup.laps as i32 + d - 1).rem_euclid(6) + 1) as u32;
        ry += step;
        let d = selector(&mut self.focus, "Opponents", &format!("{}", self.setup.opponents), rx, ry, rw, rh, BUTTER);
        self.setup.opponents = wrap(self.setup.opponents, d, 8 - players + 1);
        ry += step;
        let d = selector(&mut self.focus, "AI mood", DIFFICULTY[self.setup.difficulty].0, rx, ry, rw, rh, BUTTER);
        self.setup.difficulty = wrap(self.setup.difficulty, d, DIFFICULTY.len());
        ry += step;
        let d = selector(&mut self.focus, "Assists", ASSISTS[self.setup.assists].0, rx, ry, rw, rh, LAVENDER);
        self.setup.assists = wrap(self.setup.assists, d, ASSISTS.len());
        ry += step + 6.0 * s;
        let bw = (rw - 16.0 * s) * 0.5;
        let back = button(&mut self.focus, "Back", rx, ry, bw, rh + 6.0 * s, CREAM);
        // Back is drawn as a button but excluded from the row count above: fold it into Start's row
        let go = button(&mut self.focus, "Start!", rx + bw + 16.0 * s, ry, bw, rh + 6.0 * s, PEACH);
        if back || is_key_pressed(KeyCode::Escape) {
            self.screen = Screen::Title;
            self.focus = Focus::default();
            return;
        }
        if go {
            self.start_race();
            return;
        }

        // car previews + blurb on the right
        let px = x + w + 24.0 * s;
        let pw = sw - px - 24.0 * s;
        if pw > 200.0 * s {
            let ph = if players == 2 { (h - 24.0 * s) * 0.5 } else { h };
            for p in 0..players {
                let py = y + p as f32 * (ph + 24.0 * s);
                panel(px, py, pw, ph);
                let spec = &specs[self.setup.cars[p]];
                text(spec.name, px + 24.0 * s, py + 48.0 * s, 40.0 * s, INK);
                let drive = match spec.drivetrain {
                    rally_core::vehicle::Drivetrain::Fwd => "Front-wheel drive",
                    rally_core::vehicle::Drivetrain::Rwd => "Rear-wheel drive",
                    rally_core::vehicle::Drivetrain::Awd => "All-wheel drive",
                };
                text(drive, px + 24.0 * s, py + 76.0 * s, 22.0 * s, INK_SOFT);
                let bl = spec.blurb;
                text(bl, px + 24.0 * s, py + ph - 30.0 * s, (20.0 * s).min(pw / bl.len() as f32 * 1.9), INK_SOFT);
                // stat bars
                let stats = [
                    ("Power", spec.peak_torque / 470.0),
                    ("Grip", (spec.tire_grip - 0.9) / 0.3),
                    ("Weight", spec.mass / 1700.0),
                ];
                for (i, (name, v)) in stats.iter().enumerate() {
                    let by = py + 100.0 * s + i as f32 * 26.0 * s;
                    text(name, px + 24.0 * s, by + 14.0 * s, 20.0 * s, INK);
                    rounded_rect(px + 110.0 * s, by, 160.0 * s, 16.0 * s, 8.0 * s, Color::new(0.0, 0.0, 0.0, 0.08));
                    rounded_rect(px + 110.0 * s, by, 160.0 * s * v.clamp(0.08, 1.0), 16.0 * s, 8.0 * s, [PEACH, MINT, SKY][i]);
                }
                let car = self.setup.cars[p];
                let paint = self.setup.paints[p];
                let top = py + 180.0 * s;
                let hh = (ph - 220.0 * s).max(60.0);
                self.draw_car_preview(car, paint, px + 10.0 * s, top, pw - 20.0 * s, hh);
            }
        }
    }

    fn race(&mut self) {
        let dt = get_frame_time().min(0.05);
        let Some(sess) = self.session.as_mut() else {
            self.screen = Screen::Title;
            return;
        };
        if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::P) {
            sess.paused = !sess.paused;
            self.pause_focus = Focus::default();
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
        sess.update(dt, touch);
        if self.hooks.autoplay && sess.players.len() > 1 {
            let j = sess.players[1].car;
            let inp = self.autopilots[1].drive(&sess.world.cars[j], &sess.world.track, dt);
            sess.world.set_input(j, inp);
        }
        let horizon = sess.scene.sky_horizon;
        clear_background(Color::new(horizon[0], horizon[1], horizon[2], 1.0));
        sess.draw_3d(&mut self.gpu, |_, _, _| {});
        let vps = sess.viewports(screen_width(), screen_height());
        for (i, vp) in vps.iter().enumerate() {
            hud::draw_hud(sess, i, vp);
        }
        if vps.len() == 2 {
            if vps[0].w < screen_width() * 0.9 {
                draw_rectangle(screen_width() * 0.5 - 2.0, 0.0, 4.0, screen_height(), CREAM);
            } else {
                draw_rectangle(0.0, screen_height() * 0.5 - 2.0, screen_width(), 4.0, CREAM);
            }
        }
        if sess.paused {
            self.pause_menu();
        } else if sess.is_over() {
            self.screen = Screen::Results;
            self.focus = Focus::default();
            audio::set_music(audio::Music::Results);
        }
    }

    fn pause_menu(&mut self) {
        draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.2, 0.15, 0.25, 0.35));
        let s = scale();
        let w = 360.0 * s;
        let h = 330.0 * s;
        let x = (screen_width() - w) * 0.5;
        let y = (screen_height() - h) * 0.5;
        panel(x, y, w, h);
        text_centered("Paused", x + w * 0.5, y + 56.0 * s, 46.0 * s, INK);
        let mut f = std::mem::take(&mut self.pause_focus);
        f.begin(3);
        let bw = w - 60.0 * s;
        let bh = 56.0 * s;
        let resume = button(&mut f, "Resume", x + 30.0 * s, y + 90.0 * s, bw, bh, MINT);
        let restart = button(&mut f, "Restart", x + 30.0 * s, y + 160.0 * s, bw, bh, BUTTER);
        let quit = button(&mut f, "Quit to menu", x + 30.0 * s, y + 230.0 * s, bw, bh, PEACH);
        self.pause_focus = f;
        if resume {
            if let Some(sess) = self.session.as_mut() {
                sess.paused = false;
            }
        }
        if restart {
            self.start_race();
        }
        if quit {
            if let Some(sess) = self.session.take() {
                sess.free(&mut self.gpu);
            }
            audio::stop_cars();
            audio::set_music(audio::Music::Menu);
            self.screen = Screen::Title;
            self.focus = Focus::default();
        }
    }

    fn results(&mut self) {
        let Some(sess) = self.session.as_mut() else {
            self.screen = Screen::Title;
            return;
        };
        // keep the world alive in the background (cars coast on autopilot)
        sess.update(get_frame_time().min(0.05), None);
        let horizon = sess.scene.sky_horizon;
        clear_background(Color::new(horizon[0], horizon[1], horizon[2], 1.0));
        sess.draw_3d(&mut self.gpu, |_, _, _| {});
        let sess = self.session.as_ref().unwrap();
        let s = scale();
        let w = (720.0 * s).min(screen_width() - 20.0);
        let n = sess.world.cars.len();
        let h = 200.0 * s + n as f32 * 40.0 * s;
        let x = (screen_width() - w) * 0.5;
        let y = (screen_height() - h) * 0.5;
        panel(x, y, w, h);
        text_centered("Results", x + w * 0.5, y + 54.0 * s, 48.0 * s, INK);
        let local: Vec<usize> = sess.players.iter().map(|p| p.car).collect();
        for (row, &i) in sess.world.race.standings().iter().enumerate() {
            let r = &sess.world.race.racers[i];
            let ry = y + 100.0 * s + row as f32 * 40.0 * s;
            if local.contains(&i) {
                rounded_rect(x + 20.0 * s, ry - 28.0 * s, w - 40.0 * s, 38.0 * s, 10.0 * s, Color::new(0.99, 0.84, 0.45, 0.6));
            }
            let medal = ["🥇", "🥈", "🥉"];
            let _ = medal;
            text(&format!("{}.", row + 1), x + 34.0 * s, ry, 28.0 * s, INK);
            text(&sess.names[i], x + 80.0 * s, ry, 28.0 * s, INK);
            text(sess.world.cars[i].spec.name, x + 250.0 * s, ry, 24.0 * s, INK_SOFT);
            let t = r.finish_time.map(format_time).unwrap_or_else(|| "--:--".into());
            text(&t, x + 390.0 * s, ry, 28.0 * s, INK);
            if let Some(b) = r.best_lap {
                text(&format!("best {}", format_time(b)), x + w - 190.0 * s, ry, 22.0 * s, INK_SOFT);
            }
        }
        let by = y + h - 76.0 * s;
        let bw = (w - 80.0 * s) / 3.0;
        self.focus.begin(3);
        if button(&mut self.focus, "Race again", x + 20.0 * s, by, bw, 54.0 * s, MINT) {
            self.start_race();
        }
        if button(&mut self.focus, "New track", x + 40.0 * s + bw, by, bw, 54.0 * s, BUTTER) {
            self.setup.seed = random_seed();
            self.start_race();
        }
        if button(&mut self.focus, "Menu", x + 60.0 * s + bw * 2.0, by, bw, 54.0 * s, PEACH) {
            if let Some(sess) = self.session.take() {
                sess.free(&mut self.gpu);
            }
            audio::stop_cars();
            audio::set_music(audio::Music::Menu);
            self.screen = Screen::Title;
            self.focus = Focus::default();
        }
    }

    fn loading(&mut self) {
        cozy_background(get_time() as f32);
        let s = scale();
        text_outlined_centered("Building your track...", screen_width() * 0.5, screen_height() * 0.5, 48.0 * s, CREAM);
        text_centered(&biome_name(self.setup.biome, self.setup.seed), screen_width() * 0.5, screen_height() * 0.5 + 44.0 * s, 28.0 * s, INK);
        self.loading_frames += 1;
        if self.loading_frames >= 2 {
            self.finish_loading();
        }
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    audio::init();
    audio::set_music(audio::Music::Menu);
    let hooks = TestHooks::from_env();
    let mut app = App {
        gpu: Gpu::new(),
        screen: Screen::Title,
        setup: Setup { split: hooks.split, cars: [2, 1], paints: [0, 1], biome: 0, length: 1, seed: random_seed(), laps: 3, opponents: 3, difficulty: 1, assists: 1 },
        session: None,
        focus: Focus::default(),
        preview_angle: 0.6,
        preview_models: Vec::new(),
        pause_focus: Focus::default(),
        autopilots: Vec::new(),
        loading_frames: 0,
        hooks,
    };
    if let Ok(seed) = std::env::var("COZY_SEED") {
        app.setup.seed = seed.parse().unwrap_or(1);
    }
    if let Ok(b) = std::env::var("COZY_BIOME") {
        app.setup.biome = b.parse().unwrap_or(0);
    }
    if let Ok(scr) = std::env::var("COZY_SCREEN") {
        app.screen = match scr.as_str() {
            "setup" => Screen::Setup,
            "help" => Screen::Help,
            "race" => Screen::Loading,
            _ => Screen::Title,
        };
    } else if app.hooks.autoplay {
        app.screen = Screen::Loading;
    }
    let start = get_time();
    let mut shot_idx = 0;
    loop {
        match app.screen {
            Screen::Title => app.title(),
            Screen::Help => app.help(),
            Screen::Setup => app.setup_screen(),
            Screen::Loading => app.loading(),
            Screen::Race => app.race(),
            Screen::Results => app.results(),
        }
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
