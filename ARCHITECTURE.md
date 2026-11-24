# Cozy Rally — architecture & contributor guide

Cozy, pretty, procedural rally racing in Rust. Targets: **Windows PC** (cross-compiled
from Linux with `x86_64-pc-windows-gnu`) and **Android** (sideload APK, see `android/README.md`),
built from one code base. Online multiplayer comes later. Keep code portable (no Windows-only
APIs, GLSL `#version 100`, u16 index buffers); anything platform-specific goes in
`rally_game/src/platform/` behind `cfg(target_os = ...)`.

**Art direction: cozy and super pretty.** Soft pastel palettes, warm sun, gentle fog, low-poly
flat-shaded shapes with rounded silhouettes, lots of small life (flowers, critters, bunting,
spectators cheering). Nothing harsh, nothing grimy. Everything (models, sounds, music) is
generated **in code** — no asset files unless absolutely necessary.

## Workspace

```
crates/rally_core/   headless simulation (no graphics). glam 0.27 shared with macroquad.
  vehicle.rs   raycast car physics (suspension, tyres, drivetrain, assists). Local frame: +X right, +Y up, -Z forward
  track.rs     procedural closed-loop track + heightfield terrain + props placement;
               TrackParams { difficulty 0..2, surface: SurfaceMode }, Track::ground_at (blended surface)
  layout.rs    rally layout planner (corner complexes, exact loop closure), crests/jumps
  surfacemap.rs surface sections + organic patches with smooth transitions, road_color
  pacenotes.rs rally pace notes from the centre line (UI-agnostic, serialisable)
  biome.rs     5 biomes: palette + surfaces + terrain character
  race.rs      countdown, checkpoints, laps, placings, respawns
  world.rs     fixed 120 Hz tick (2 physics substeps), collisions, WorldEvent stream
  ai.rs        AI driver
  examples/    sim.rs (AI races, stats), dyno.rs (physics characterisation), trace.rs,
               stagesim.rs (stats per track difficulty), pacenotes.rs, surfaces.rs, trackstats.rs
crates/rally_net/    online multiplayer (no graphics; depends on rally_core)
  protocol.rs  ClientMsg / ServerMsg, PROTOCOL_VERSION (bump on any layout change), DEFAULT_PORT 27420
  conn.rs      non-blocking framed TCP (u32 LE length + bincode); UdpMsg datagrams carry the
               30 Hz snapshots on the same port (token-authenticated, TCP fallback)
  server.rs    lobby, leader, synchronised start (Loaded -> Go), snapshot relay, official results
  client.rs    NetClient used by the game (background connect, poll per frame)
  sync.rs      remote cars: simulated locally with the owner's inputs, softly corrected toward
               the owner's snapshot extrapolated by latency (snap beyond 8 m)
  bin/rally_server.rs  the self-hostable server
  tests/       lobby_race.rs (real sockets, full flow), remote_sync.rs (correction error vs latency)
crates/rally_game/   macroquad app
  render/gpu.rs    miniquad pipelines (opaque / alpha / overlay), lit+fogged vertex-colour shader,
                   static GpuMesh with frustum culling, stream buffer for dynamic geometry
  render/mesh.rs   MeshBuilder: flat-shaded low-poly primitives (box, frustum, blob, ...)
  render/scene.rs  builds terrain/road/props/water/sky/hills meshes for a Track
  models.rs        procedural car + prop models
  game.rs          RaceSession: World + players + AI + cameras + viewports + 3D drawing
  camera.rs        chase camera
  input.rs         keyboard + gamepads (gilrs on PC, platform::android on phones) -> CarInput
  touch.rs         on-screen touch controls (phones; COZY_TOUCH=1 on PC)
  platform/        platform layer: Android JNI glue (lifecycle, Back, pads, saves, logcat)
  hud.rs           in-race HUD
  pacenotes_hud.rs pace-notes strip (top centre), N toggles, PACE_NOTES_ENABLED / settings.pace_notes
  ui.rs            immediate-mode cozy UI widgets
  audio.rs         audio facade (API used by the game)
  fx.rs            particle/effects system (owned by RaceSession)
  online.rs        Online screen (connect form + lobby), OnlineRace: feeds NetEvents into the
                   RaceSession (pre_update / post_update around RaceSession::update)
  (circuits)       floodlit zones: PropKind::LampPost placed by track.rs `lamp_zones` (pit straight +
                   slow corners), lit by night.rs (nearest masts become spot lights, faded by rank)
  night.rs         night races: per-car headlamps -> gpu.set_lights (MAX_LIGHTS spot lights,
                   viewer gets both lamps), beams, lamp damage, fireflies. TrackParams::night,
                   scene.rs TimeOfDay::Night palette + grade, Environment::night (sky stars,
                   darker weather/clouds/fx). COZY_NIGHT=1 / COZY_HIGHBEAM=1 test hooks
  main.rs          app state machine: Title / Setup / Loading / Race / Results / Help
```

### Frame flow
`main loop` -> `RaceSession::update(dt)` (input, AI, `World::update`, events -> camera shake,
`fx.update`, `audio::race_frame`) -> `RaceSession::draw_3d` per viewport (sky, hills, terrain,
road, props, cars, shadows, `fx.draw`, water) -> HUD (macroquad 2D) on top.

Render interpolation: cars have `prev_pos/prev_rot`; use `game::interp(car, session.alpha)`.

### GPU gotchas (important)
* Call `gpu.begin_3d(..)` / `gpu.end_3d()` around raw drawing; it flushes macroquad's 2D batch.
* miniquad 0.4 only enables the depth test for pipelines with depth_write; `gpu.rs` re-enables it
  for the alpha pipeline (`force_depth_test`). Keep that if you add pipelines.
* glClear ignores depth while depth writes are masked -> `begin_3d` binds the opaque pipeline first.
* Shader uniforms are packed sequentially as floats; the Rust struct must match the order exactly.
* Keep shaders GLSL ES 1.00 (`#version 100`) for Android later.

## Building & testing
```
cargo build --release -p rally_game
cargo run --release -p rally_core --example sim -- 6 2      # AI races on 6 seeds: must finish, 0 NaN
cargo run --release -p rally_core --example dyno            # physics numbers
cargo build --release --target x86_64-pc-windows-gnu -p rally_game   # Windows exe
```
Visual playtest (headless, virtual display, autopilot drives player 1):
```
COZY_AUTOPLAY=1 COZY_SEED=3 COZY_SHOTS=5,12,20 COZY_SHOT_DIR=/some/dir COZY_EXIT_AFTER=21 \
  xvfb-run -a -s "-screen 0 1280x720x24" ./target/release/rally_game
```
Online test: run `rally_server --port 27999`, then two game instances with
`COZY_ONLINE=127.0.0.1:27999 COZY_NAME=Alice COZY_ONLINE_LEAD=2` (leader: picks a 1-lap race and
starts once 2 are in) and `COZY_ONLINE=127.0.0.1:27999 COZY_NAME=Bob`, plus `COZY_AUTOPLAY=1`
and separate `COZY_SETTINGS` files.
Other env hooks: `COZY_SPLIT=1` (2 players), `COZY_BIOME=0..5` (0 random, 1 Meadow, 2 Autumn,
3 Snow, 4 Desert, 5 Coast), `COZY_SCREEN=title|setup|help|race`, `COZY_MSAA=1`.
Software GL (llvmpipe) is slow — a few FPS is normal under Xvfb. **Look at your screenshots**
(open the PNGs) to judge visual work; it's the only way to know it looks good.

## Rules for parallel work
* Work only inside your own git worktree/branch. Commit early and often with clear messages.
* Stay inside the files you own; if you must touch a shared file (game.rs, main.rs, gpu.rs,
  Cargo.toml), keep the change minimal and mention it in your final report.
* Keep `cargo build` warning-light and the `sim` example passing.
* Performance matters: target 60 FPS on a mid laptop at 1080p, split-screen included.
