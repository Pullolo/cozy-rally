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
- Night races (Setup -> Time: Night): you see what your headlights reach. H toggles high /
  low beam (pad: D-pad up); big hits can smash a lamp; stars, moon, fireflies, glowing tail lights
- AI with racing lines, split-screen, gamepads, Time Trial with ghost
- Online races with up to 4 friends on a server you host yourself (`rally_server`)
- Procedural synth audio and music

## Build
```
cargo run --release -p rally_game
cargo build --release --target x86_64-pc-windows-gnu -p rally_game   # Windows exe
```
Linux needs `libasound2-dev libxi-dev libgl1-mesa-dev`. Gamepads on Linux: `--features gamepad-linux` (needs libudev).

## Online races with friends
One of you runs the server (any PC, it is tiny and headless):
```
cargo run --release -p rally_net --bin rally_server
cargo run --release -p rally_net --bin rally_server -- --port 27420 --name "Hazel's garage"
cargo build --release --target x86_64-pc-windows-gnu -p rally_net --bin rally_server   # rally_server.exe
```
It prints the address to share. Everyone (the host too) opens the game -> **Online**, types
that address and a name, and connects. The first to join is the **leader**: they pick the
race with **Race setup** (the normal setup screen), choose ghost cars or contact, and press
**Start race!**. Everyone loads the same track, waits at the line, and the countdown starts
when all are ready. Official finish times come from the server; then it's back to the lobby.

How friends reach the server:
- Same Wi-Fi / LAN: the host's LAN IP (printed by the server).
- Over the internet, easiest: install [Tailscale](https://tailscale.com) (or ZeroTier) on
  every PC and use the host's Tailscale IP. No router setup.
- Or forward **port 27420, TCP and UDP**, on the host's router and share the public IP.
  (UDP carries the car positions; if it is blocked the game falls back to TCP, which works
  but can stutter on bad Wi-Fi. The lobby shows UDP/TCP per player.)

Everyone needs the same game version (the server refuses mismatches). Join codes through a
relay are planned later.

## Controls
WASD / arrows, Space handbrake, R respawn, C camera, N pace notes, Esc pause.
Split-screen P2: arrows, Right Shift, Backspace. Gamepad: stick, RT/LT, A handbrake, Y respawn.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the code layout.
