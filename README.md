# Cozy Rally

A cozy, procedural low-poly rally racer in Rust (macroquad). Race AI or a friend in
split-screen on generated rally stages with mixed surfaces, pace notes and semi-realistic
physics. Everything (models, sounds, music) is generated in code.

## Features
- Raycast vehicle physics: suspension, slip-based tyres, wheel dynamics, clutch, LSDs,
  assists (Cozy / Normal / Sport / Sim): steering limiter, TCS, ABS
- Procedural rally stages: hairpins, squares, chicanes, crests, jumps; Relaxed / Rally / Expert
- Surfaces: gravel, tarmac, dirt, snow, ice, mud, sand, Mixed stages with smooth transitions
- Rally pace notes with co-driver calls (N)
- 5 biomes, shadows, bloom, wind, water, golden hour
- AI with racing lines, split-screen, gamepads, Time Trial with ghost
- Procedural synth audio and music

## Build
```
cargo run --release -p rally_game
cargo build --release --target x86_64-pc-windows-gnu -p rally_game   # Windows exe
```
Linux needs `libasound2-dev libxi-dev libgl1-mesa-dev`. Gamepads on Linux: `--features gamepad-linux` (needs libudev).

## Controls
WASD / arrows, Space handbrake, R respawn, C camera, N pace notes, Esc pause.
Split-screen P2: arrows, Right Shift, Backspace. Gamepad: stick, RT/LT, A handbrake, Y respawn.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the code layout.
