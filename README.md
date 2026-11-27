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
- 7 biomes, shadows, bloom, wind, water, golden hour
- Night races (Setup -> Time: Night): you see what your headlights reach. H toggles high /
  low beam (pad: D-pad up); big hits can smash a lamp; stars, moon, fireflies, glowing tail lights.
  Circuits have floodlit zones (pit straight + a few slow corners); the rest of the lap is dark
- Circuits from 2.5 km sprints to the 20 km **Nordring** (Wolfsthal Nordring, or Circuit ->
  Size: Nordring): narrow, hilly, left-right forest road with flying brows and two carousels
- AI with racing lines, split-screen, gamepads, Time Trial with ghost
- Online races with up to 4 friends on a server you host yourself (`rally_server`)
- Procedural synth audio and music

## Build
```
cargo run --release -p rally_game
cargo build --release --target x86_64-pc-windows-gnu -p rally_game   # Windows exe
```
Android APK (sideload, arm64): `android/build.sh` (needs Docker), see [android/README.md](android/README.md).
Linux needs `libasound2-dev libxi-dev libgl1-mesa-dev`. Gamepads on Linux: `--features gamepad-linux` (needs libudev).

## Multiplayer

### Split-screen (same PC)
Title -> **Split-screen**. Player 1 drives with WASD (Space handbrake, R respawn, H lights),
player 2 with the arrow keys (Right Shift handbrake, Backspace respawn, Right Alt lights).
Gamepads work for both.

### Online with friends (up to 4 players)
Online races run through a small server that one of you hosts. It has no graphics, uses
almost no CPU and can run on the same PC you play on.

**1. The host starts the server**
```
cargo run --release -p rally_net --bin rally_server
```
Optional: `-- --port 27420 --name "Hazel's garage"`. To give a Windows friend the server as an
exe: `cargo build --release --target x86_64-pc-windows-gnu -p rally_net --bin rally_server`
(then share `target/x86_64-pc-windows-gnu/release/rally_server.exe`).
The server prints the address to share, e.g. `Friends on your network join with: 192.168.1.20:27420`.
Leave the window open while you play (Ctrl+C stops it).

**2. Make the server reachable** (pick one)
| Where are your friends? | What to do | Address to share |
|---|---|---|
| Same Wi-Fi / LAN | nothing | the LAN address the server prints |
| Anywhere (easiest) | everyone installs [Tailscale](https://tailscale.com) (or ZeroTier) and joins the same network | the host's Tailscale IP (`100.x.y.z`) |
| Anywhere (router) | forward port **27420 TCP and UDP** on the host's router to the host PC | the host's public IP (search "what is my ip") |

The port is 27420 unless you passed `--port`. UDP carries the car positions; if it is blocked
the game falls back to TCP automatically (works, but can stutter on bad Wi-Fi).

**3. Everyone joins (the host too)**
Start the game -> **Online** -> type the address (the port can be left out when it is 27420)
and your name -> **Connect**. You land in the lobby, which shows every player, ready state,
ping and whether they are on UDP or TCP. Car and paint are picked in the lobby once the
leader has chosen the race (rally cars for rally, circuit cars for circuits).

**4. The leader starts the race**
The first player to join is the **leader** (marked in the list; if they leave, the next player
takes over).
- **Race setup** opens the normal setup screen: discipline, track or circuit, laps,
  Day / Night... -> **Start!** sends the choice to the lobby (it doesn't start yet).
- **Cars**: *Ghosts* (default, cars drive through each other, never unfair with lag) or *Contact*.
- Others press **I'm ready**; the leader presses **Start race!**.

Everyone loads the same track and waits at the line; the countdown starts when all have
loaded. Finish times come from the server; afterwards **Back to lobby** for the next race.
Escape opens the pause menu but the race keeps going (your car brakes); **Leave race** returns
to the lobby. If a friend disconnects, an AI drives their car home.

**Troubleshooting**
- *Nobody is hosting there (connection refused)*: the server isn't running, or wrong address/port.
- *No answer*: wrong IP, the firewall blocks the server (allow `rally_server` / port 27420 on the
  host), or the port isn't forwarded. Try Tailscale.
- *Version mismatch*: everyone needs the same game build (pull and rebuild).
- *Server is full*: 4 players maximum.
- A friend's car stutters: check the lobby says UDP for them; on TCP, forward UDP 27420 too.

## Controls
WASD / arrows, Space handbrake, R respawn, C camera, N pace notes, H high/low beam (night), Esc pause.
Split-screen P2: arrows, Right Shift, Backspace. Gamepad: stick, RT/LT, A handbrake, Y respawn.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the code layout.
