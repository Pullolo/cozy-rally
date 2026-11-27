# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Static HTML/CSS (no build step), served by GitHub Pages from the `docs/` folder on `main`. Confirmed by the user.

## Users

The public website (`docs/`) is for people who don't know the game yet: they arrive from a link, need to see what Cozy Rally is in a few seconds, then download it for Windows or Android. Online players also need the self-hosted server.

## Product Purpose

Cozy Rally is a free, cozy, procedural low-poly rally racer written in Rust (macroquad). The site exists to show the game and send visitors to the right download on GitHub Releases.

## Positioning

Everything is generated in code: stages, scenery, car models, engine sound and music. Every stage is new, yet the driving has semi-realistic physics (suspension, slip-based tyres, assists from Cozy to Sim). One codebase runs on PC and Android, and they race each other online.

## Operating Context

- Downloads live on GitHub Releases (`Pullolo/cozy-rally`) with stable asset names, so `releases/latest/download/<name>` always points at the newest build:
  `CozyRally-windows-x64.exe`, `CozyRally-android-arm64.apk` (sideload), `rally_server-windows-x64.exe`, `rally_server-linux-x64`, `SHA256SUMS.txt`.
- Android: sideloaded arm64 APK (not on Play Store); the install must allow unknown sources.
- Online: up to 4 players on a server one of them hosts (port 27420 TCP+UDP); LAN, Tailscale/ZeroTier or port forwarding.

## Capabilities and Constraints

- Rally stages and loops, circuits (2.5 km sprints up to the 20 km Nordring), 7 biomes (Sunny Meadow, Maple Woods, Frosty Peaks, Canyon Dunes, Seaside Cliffs, Mountain Forest, Snowy Taiga), night races with headlights, weather.
- 9 cars: Pip, Bramble, Thistle, Burrow, Blossom, Kestrel (rally); Comet, Zephyr, Dragonfly (circuit).
- Pace notes with co-driver calls, AI opponents with racing lines, split-screen, gamepads, time trial with ghost, campaign, online multiplayer.
- Android: touch controls; holds 60–120 fps on a recent phone on Low.
- Price and licence: not stated anywhere in the repo; the site must not claim a licence. "Free download" is true (public GitHub releases).

## Brand Commitments

- Name: Cozy Rally. Tone of the game: cozy, warm, playful, not macho motorsport.
- In-game UI uses rounded cream cards, pastel accent colours and a rounded font (`assets/fonts`).
- No real car brands, logos or sponsors anywhere (made-up cozy sponsor brands only).

## Evidence on Hand

- Real in-game screenshots can be captured with the game's test hooks (`COZY_SHOTS`, `COZY_BIOME`, `COZY_CAR`).
- No testimonials, reviews, download counts or press exist: never invent them.

## Product Principles

1. Show the real game: actual screenshots, never mock-ups.
2. Getting to the right download takes one click for each platform.
3. Honest and specific: real features, real requirements, no invented social proof.
4. Cozy, not aggressive.

## Accessibility & Inclusion

No specific requirement given; aim for WCAG AA contrast and keyboard-usable links.
