//! A running race: simulation world, local players (with cameras/viewports), AI,
//! and rendering of the 3D scene for each viewport.
use crate::camera::ChaseCam;
use crate::fx::Fx;
use crate::input::{PlayerInput, Scheme};
use crate::models::{car_model, PAINTS};
use crate::render::perf::{self, Sec};
use crate::render::gpu::{DrawParams, Gpu, GpuMesh, View};
use crate::render::mesh::{rgba, Vertex};
use crate::render::scene::Scene;
use macroquad::math::{Mat4, Quat, Vec3};
use rally_core::ai::AiDriver;
use rally_core::input::CarInput;
use rally_core::race::RacePhase;
use rally_core::track::{Track, TrackParams};
use rally_core::world::{Controller, RacerSetup, World, WorldEvent};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PlayerSlot {
    pub car: usize,
    pub paint: usize,
    pub scheme: Scheme,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub track: TrackParams,
    pub laps: u32,
    pub players: Vec<PlayerSlot>,
    pub ai_count: usize,
    /// 0..1
    pub ai_skill: f32,
    /// assist multiplier (0 sim .. 1.3 cozy)
    pub assist_level: f32,
    /// [gameplay] Race or Time Trial (see gameplay.rs)
    pub mode: crate::gameplay::GameMode,
}

pub struct LocalPlayer {
    pub car: usize,
    pub input: PlayerInput,
    pub cam: ChaseCam,
    pub name: String,
}

pub struct CarRender {
    pub body: GpuMesh,
    pub wheel: GpuMesh,
    pub glass: GpuMesh,
    pub head_lights: GpuMesh,
    pub brake_lights: GpuMesh,
    pub reverse_lights: GpuMesh,
}

pub struct RaceSession {
    pub config: SessionConfig,
    pub world: World,
    pub scene: Scene,
    pub cars: Vec<CarRender>,
    pub names: Vec<String>,
    pub ais: Vec<Option<AiDriver>>,
    pub players: Vec<LocalPlayer>,
    pub alpha: f32,
    pub time: f32,
    /// Events from the last update, for audio/FX/UI consumers
    pub frame_events: Vec<WorldEvent>,
    pub paused: bool,
    pub fx: Fx,
    /// [gameplay] time-trial state (ghost, deltas); None in races
    pub tt: Option<crate::gameplay::TimeTrialState>,
    /// Paint index per car (for UI colour coding: minimap dots, standings, results)
    pub paints: Vec<usize>,
    /// Cinematic camera override (title attract mode, results): (eye, target, fov radians).
    /// When set, a single full-screen viewport is rendered from this camera.
    pub cam_override: Option<(Vec3, Vec3, f32)>,
}

const AI_NAMES: [&str; 8] = ["Hazel", "Juniper", "Basil", "Clover", "Fern", "Rowan", "Maple", "Pebble"];

pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RaceSession {
    pub fn new(config: SessionConfig, gpu: &mut Gpu) -> RaceSession {
        let track = Arc::new(Track::generate(&config.track));
        let mut racers = Vec::new();
        let mut names = Vec::new();
        let mut paints = Vec::new();
        // players start at the back of the grid (cozy: you get to overtake!)
        let n_ai = config.ai_count;
        let mut rng = rally_core::rng::Rng::new(config.track.seed ^ 0xA1);
        for k in 0..n_ai {
            let spec = rng.range_i(0, 4) as usize;
            racers.push(RacerSetup { car_spec: spec, controller: Controller::Ai });
            names.push(AI_NAMES[k % AI_NAMES.len()].to_string());
            paints.push((k + 3) % PAINTS.len());
        }
        for (i, p) in config.players.iter().enumerate() {
            racers.push(RacerSetup { car_spec: p.car, controller: Controller::Local(i as u8) });
            names.push(p.name.clone());
            paints.push(p.paint);
        }
        let mut world = World::new(track.clone(), &racers, config.laps);
        for (i, c) in world.cars.iter_mut().enumerate() {
            if matches!(world.controllers[i], Controller::Local(_)) {
                c.assist_level = config.assist_level;
            }
        }
        let scene = Scene::build(&track, gpu);
        crate::render::bench::setup(&track, &format!("{} seed {}", track.style.name, config.track.seed));
        let cars = world
            .cars
            .iter()
            .zip(paints.iter())
            .map(|(c, &p)| {
                let m = car_model(&c.spec, PAINTS[p % PAINTS.len()]);
                CarRender {
                    body: gpu.upload(&m.body),
                    wheel: gpu.upload(&m.wheel),
                    glass: gpu.upload(&m.glass),
                    head_lights: gpu.upload(&m.head_lights),
                    brake_lights: gpu.upload(&m.brake_lights),
                    reverse_lights: gpu.upload(&m.reverse_lights),
                }
            })
            .collect();
        let ais = world
            .controllers
            .iter()
            .enumerate()
            .map(|(i, c)| match c {
                // [gameplay] personalities rolled around the difficulty
                Controller::Ai => Some(AiDriver::racer(i, config.ai_skill, config.track.seed)),
                _ => None,
            })
            .collect();
        let players = config
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| LocalPlayer { car: n_ai + i, input: PlayerInput::new(p.scheme), cam: ChaseCam::new(), name: p.name.clone() })
            .collect();
        let fx = Fx::new(&world);
        let tt = crate::gameplay::session_start(&config, &(n_ai..n_ai + config.players.len()).collect::<Vec<_>>(), gpu); // [gameplay]
        RaceSession { config, world, scene, cars, names, ais, players, alpha: 0.0, time: 0.0, frame_events: Vec::new(), paused: false, fx, tt, paints, cam_override: None }
    }

    pub fn free(self, gpu: &mut Gpu) {
        if let Some(t) = self.tt {
            t.free(gpu); // [gameplay]
        }
        self.scene.free(gpu);
        for c in self.cars {
            gpu.free(c.body);
            gpu.free(c.wheel);
            gpu.free(c.glass);
            gpu.free(c.head_lights);
            gpu.free(c.brake_lights);
            gpu.free(c.reverse_lights);
        }
    }

    pub fn track(&self) -> &Track {
        &self.world.track
    }

    /// `touch` provides extra input for player 0 (on-screen controls)
    pub fn update(&mut self, dt: f32, touch: Option<CarInput>) {
        perf::begin(Sec::Update);
        self.update_inner(dt, touch);
        perf::end(Sec::Update);
        if let Some(p) = self.players.first() {
            let idx = self.world.cars[p.car].track_hint.unwrap_or(0);
            crate::render::bench::set_position(idx, self.time);
        }
    }

    fn update_inner(&mut self, dt: f32, touch: Option<CarInput>) {
        self.frame_events.clear();
        if self.paused {
            // let audio fade engines immediately while paused
            crate::audio::race_frame(self, dt);
            return;
        }
        self.time += dt;
        self.scene.env.time = self.time;
        for (pi, p) in self.players.iter_mut().enumerate() {
            let inp = p.input.sample(dt, if pi == 0 { touch } else { None });
            if p.input.camera_pressed() {
                p.cam.mode = p.cam.mode.next();
                p.cam.snap();
            }
            self.world.set_input(p.car, inp);
        }
        // AI at frame rate is fine (inputs are held across ticks)
        let track = self.world.track.clone();
        rally_core::ai::drive_all(&mut self.ais, &mut self.world, dt); // [gameplay] traffic-aware AI
        // finished players coast: autopilot takes over
        for p in &self.players {
            let r = &self.world.race.racers[p.car];
            if r.finished && self.ais[p.car].is_none() {
                self.ais[p.car] = Some(AiDriver::new(0.5, 0.0));
            }
        }
        self.alpha = self.world.update(dt);
        self.frame_events = self.world.drain_events();
        for e in &self.frame_events {
            match *e {
                WorldEvent::Collision { car, other, strength, .. } => {
                    for p in self.players.iter_mut() {
                        if p.car == car || Some(p.car) == other {
                            p.cam.add_shake((strength / 12.0).min(0.8));
                        }
                    }
                }
                WorldEvent::Landing { car, strength } => {
                    for p in self.players.iter_mut() {
                        if p.car == car {
                            p.cam.add_shake((strength / 15.0).min(0.6));
                        }
                    }
                }
                WorldEvent::Race(rally_core::race::RaceEvent::Reset { car }) => {
                    for p in self.players.iter_mut() {
                        if p.car == car {
                            p.cam.snap();
                        }
                    }
                }
                _ => {}
            }
        }
        for p in self.players.iter_mut() {
            let (pos, _) = interp(&self.world.cars[p.car], self.alpha);
            p.cam.update(pos, &self.world.cars[p.car], &track, dt);
        }
        self.fx.update(&self.world, &self.frame_events, self.alpha, dt);
        crate::gameplay::frame(self, dt); // [gameplay] time trial + rumble
        if !self.players.is_empty() {
            crate::audio::race_frame(self, dt);
        }
    }

    pub fn is_over(&self) -> bool {
        self.world.race.phase == RacePhase::Finished
    }

    /// Viewports in logical pixels for each local player
    pub fn viewports(&self, sw: f32, sh: f32) -> Vec<Viewport> {
        if self.cam_override.is_some() {
            return vec![Viewport { x: 0.0, y: 0.0, w: sw, h: sh }];
        }
        match self.players.len() {
            0 | 1 => vec![Viewport { x: 0.0, y: 0.0, w: sw, h: sh }],
            2 => {
                if sw > sh * 1.9 {
                    vec![Viewport { x: 0.0, y: 0.0, w: sw * 0.5 - 1.0, h: sh }, Viewport { x: sw * 0.5 + 1.0, y: 0.0, w: sw * 0.5 - 1.0, h: sh }]
                } else {
                    vec![Viewport { x: 0.0, y: 0.0, w: sw, h: sh * 0.5 - 1.0 }, Viewport { x: 0.0, y: sh * 0.5 + 1.0, w: sw, h: sh * 0.5 - 1.0 }]
                }
            }
            _ => {
                let hw = sw * 0.5;
                let hh = sh * 0.5;
                (0..self.players.len().min(4)).map(|i| Viewport { x: (i % 2) as f32 * hw, y: (i / 2) as f32 * hh, w: hw - 1.0, h: hh - 1.0 }).collect()
            }
        }
    }

    pub fn view_for(&self, player: usize, vp: &Viewport) -> View {
        if let Some((eye, target, fov)) = self.cam_override {
            let view = Mat4::look_at_rh(eye, target, Vec3::Y);
            let proj = Mat4::perspective_rh_gl(fov, vp.w / vp.h.max(1.0), 0.25, 2200.0);
            return View::new(view, proj, eye);
        }
        let cam = &self.players[player].cam;
        if let Some((eye, target)) = crate::render::bench::pose() {
            let v = Mat4::look_at_rh(eye, target, Vec3::Y);
            let p = Mat4::perspective_rh_gl(64f32.to_radians(), vp.w / vp.h.max(1.0), 0.25, 2200.0);
            return View::new(v, p, eye);
        }
        if let Ok(spec) = std::env::var("COZY_ROAD_CAM") {
            let (eye, target) = crate::render::scene::road_viewpoint(self.track(), &spec);
            let v = Mat4::look_at_rh(eye, target, Vec3::Y);
            let p = Mat4::perspective_rh_gl(55f32.to_radians(), vp.w / vp.h.max(1.0), 0.1, 2200.0);
            return View::new(v, p, eye);
        }
        if std::env::var("COZY_WATER_CAM").is_ok() {
            if let Some((eye, target)) = crate::render::scene::water_viewpoint(self.track()) {
                let v = Mat4::look_at_rh(eye, target, Vec3::Y);
                let p = Mat4::perspective_rh_gl(62f32.to_radians(), vp.w / vp.h.max(1.0), 0.25, 2200.0);
                return View::new(v, p, eye);
            }
        }
        let (v, p) = cam.matrices(vp.w / vp.h.max(1.0));
        View::new(v, p, cam.eye)
    }

    /// Render the 3D world for every local player. Extra per-view drawing (particles etc.)
    /// is done through `extra(gpu, view, viewport_index)`.
    ///
    /// Per viewport: sun shadow map (props, structures, cars) -> world into the offscreen
    /// scene target -> after all viewports, bloom + colour grade + vignette composite to screen.
    pub fn draw_3d(&self, gpu: &mut Gpu, mut extra: impl FnMut(&mut Gpu, &View, usize)) {
        let dpi = macroquad::window::screen_dpi_scale();
        let sw = macroquad::window::screen_width();
        let sh = macroquad::window::screen_height();
        let vps = self.viewports(sw, sh);
        let shadows = gpu.begin_frame(crate::render::quality());
        for (i, vp) in vps.iter().enumerate() {
            let view = self.view_for(i, vp);
            if shadows {
                if let Some(light) = gpu.begin_shadow(&view, &self.scene.env) {
                    self.scene.draw_shadow_casters(gpu, &light, view.eye);
                    self.draw_cars(gpu, &light);
                    gpu.end_shadow();
                }
            }
            gpu.begin_scene((vp.x * dpi) as i32, (vp.y * dpi) as i32, (vp.w * dpi) as i32, (vp.h * dpi) as i32);
            self.scene.draw_opaque(gpu, &view);
            self.draw_cars(gpu, &view);
            self.draw_shadows(gpu, &view);
            gpu.set_shadow_receive(false);
            perf::end(Sec::Scene);
            perf::begin(Sec::Fx);
            self.fx.draw(gpu, &view, &self.scene.env);
            crate::gameplay::draw_world_overlays(self, gpu, &view, i); // [gameplay] racing line + ghost
            perf::end(Sec::Fx);
            perf::begin(Sec::Scene);
            gpu.set_shadow_receive(true);
            extra(gpu, &view, i);
            self.scene.draw_transparent(gpu, &view);
            gpu.end_scene();
        }
        gpu.finish_frame(&self.scene.grade, self.time);
        // everything drawn after the 3D (HUD, menus) is attributed to the HUD section
        perf::begin(Sec::Hud);
    }

    /// Draws every car (body + wheels) with its interpolated transform. Also used for the
    /// shadow pass (`gpu.in_shadow_pass()`), where only opaque meshes are rendered.
    fn draw_cars(&self, gpu: &mut Gpu, view: &View) {
        let env = self.scene.env;
        let shadow_pass = gpu.in_shadow_pass();
        for (i, car) in self.world.cars.iter().enumerate() {
            let (pos, rot) = interp(car, self.alpha);
            if !view.sphere_visible(pos, 3.5) {
                continue;
            }
            let ghost = self.world.race.racers[i].ghost > 0.0;
            let blink = ghost && (self.time * 10.0).sin() > 0.0;
            let he = car.spec.half_extents;
            // glossy paint + soft occlusion towards the underside
            let p = DrawParams {
                rim: 0.35,
                gloss: 0.65,
                ao_y: [-he.y * 0.9, he.y * 0.4],
                tint: if blink { [1.3, 1.3, 1.3, 1.0] } else { [1.0; 4] },
                ..Default::default()
            };
            let m = Mat4::from_rotation_translation(rot, pos);
            gpu.draw(&self.cars[i].body, view, &env, m, &p, false);
            let wp = DrawParams { gloss: 0.15, ..Default::default() };
            for w in &car.wheels {
                let side = if w.local_mount.x < 0.0 { -1.0 } else { 1.0 };
                let local = w.local_mount - Vec3::Y * w.susp_length + Vec3::X * side * 0.02;
                let wm = m * Mat4::from_translation(local) * Mat4::from_rotation_y(-w.steer_angle) * Mat4::from_rotation_x(-w.spin_angle);
                gpu.draw(&self.cars[i].wheel, view, &env, wm, &wp, false);
            }
            if shadow_pass {
                continue;
            }
            // lights: emissive lenses (unlit, over-bright tint so they glow / bloom).
            // Extra per-car meshes go here too, drawn with the car matrix `m`.
            let cr = &self.cars[i];
            let lamp = |tint: [f32; 4]| DrawParams { unlit: 1.0, tint, cast_shadow: false, receive_shadow: false, ..Default::default() };
            gpu.draw(&cr.head_lights, view, &env, m, &lamp([1.35, 1.3, 1.15, 1.0]), false);
            if car.telemetry.brake > 0.05 || car.frozen {
                gpu.draw(&cr.brake_lights, view, &env, m, &lamp([2.4, 0.28, 0.22, 1.0]), false);
            }
            if car.gear < 0 {
                gpu.draw(&cr.reverse_lights, view, &env, m, &lamp([1.6, 1.6, 1.6, 1.0]), false);
            }
            // translucent glass last so the drivers show through (with a sky/sun glint)
            gpu.draw(&cr.glass, view, &env, m, &DrawParams { rim: 0.8, gloss: 0.8, ..Default::default() }, true);
        }
    }

    /// Soft contact shadow right under each car (grounds the car even where the shadow map
    /// is low-res or disabled on Low quality).
    fn draw_shadows(&self, gpu: &mut Gpu, view: &View) {
        let mut verts: Vec<Vertex> = Vec::new();
        let mut idx: Vec<u16> = Vec::new();
        let track = self.track();
        let strength = if crate::render::quality().shadows() { 0.6 } else { 1.0 };
        for car in &self.world.cars {
            let (pos, rot) = interp(car, self.alpha);
            if pos.distance(view.eye) > 120.0 {
                continue;
            }
            let he = car.spec.half_extents;
            let fwd = rot * Vec3::NEG_Z;
            let f = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
            let r = Vec3::new(-f.z, 0.0, f.x);
            let gh = track.terrain.height(pos.x, pos.z);
            let height = (pos.y - gh).max(0.0);
            let a = (0.42 - height * 0.08).clamp(0.04, 0.42) * strength;
            let grow = 1.0 + height * 0.1;
            // soft shadow: dark core fading to a transparent ring
            let ring = 14;
            let base = verts.len() as u16;
            let c = Vec3::new(pos.x, gh + 0.08, pos.z);
            let col = [0.05, 0.04, 0.12];
            verts.push(Vertex { pos: c.to_array(), normal: [0.0, 1.0, 0.0], color: rgba(col, a) });
            for (k, (scale, alpha)) in [(0.75, a * 0.9), (1.0, 0.0)].iter().enumerate() {
                for j in 0..ring {
                    let ang = j as f32 / ring as f32 * std::f32::consts::TAU;
                    let p = pos + (r * ang.cos() * (he.x + 0.2) + f * ang.sin() * (he.z + 0.15)) * grow * *scale;
                    let y = track.terrain.height(p.x, p.z) + 0.08;
                    verts.push(Vertex { pos: [p.x, y, p.z], normal: [0.0, 1.0, 0.0], color: rgba(col, *alpha) });
                }
                let _ = k;
            }
            let ring16 = ring as u16;
            for j in 0..ring16 {
                let jn = (j + 1) % ring16;
                idx.extend_from_slice(&[base, base + 1 + j, base + 1 + jn]);
                let (i0, i1, o0, o1) = (base + 1 + j, base + 1 + jn, base + 1 + ring16 + j, base + 1 + ring16 + jn);
                idx.extend_from_slice(&[i0, o0, o1, i0, o1, i1]);
            }
        }
        gpu.draw_dynamic(&verts, &idx, view, &self.scene.env, Mat4::IDENTITY, &DrawParams { unlit: 1.0, receive_shadow: false, depth_bias: 4e-6, ..Default::default() }, 1);
    }
}

pub fn interp(car: &rally_core::vehicle::Car, alpha: f32) -> (Vec3, Quat) {
    (car.prev_pos.lerp(car.pos, alpha), car.prev_rot.slerp(car.rot, alpha))
}
