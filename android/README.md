# Cozy Rally on Android

Sideload build: one signed APK for arm64 phones and tablets (Android 8.0 / API 26 and newer).
It is built from the same code as the PC game. Everything Android-specific stays behind
`cfg(target_os = "android")` or in this folder, so PC builds never see it.

## Build

Requirements: Docker, rustup (the host toolchain is mounted into the build container).

```bash
android/build.sh            # release (default)
android/build.sh debug      # unoptimised, with symbols
```

Output: `target/android/cozy-rally.apk`.

The build runs in the `notfl3/cargo-apk` image, which provides Android NDK r25, SDK
platform 31, build-tools 31 and a JDK. The image's own Rust and `cargo-quad-apk` are too old
for edition 2024, so `build-inner.sh` replaces `cargo-quad-apk`:

1. `cargo build` for `aarch64-linux-android`. `rustc-wrapper.sh` compiles the game's bin crate
   as a cdylib (`librally_game.so`).
2. It compiles the Java activity (`java/*.java.in`, vendored from miniquad 0.4.11 plus our
   additions) and dexes it with d8.
3. It draws the launcher icon in code (`tools/MakeIcon.java`) and fills in
   `AndroidManifest.xml.in`.
4. It packages with aapt, then runs zipalign and apksigner.

`versionCode` is the git commit count, so every new build installs over the previous one.

The `.so` is linked with `--no-undefined` and oboe's C++ runtime (`libc++_static`, `libc++abi`)
is linked statically. A missing symbol fails the build instead of making
`System.loadLibrary` crash the app on launch.

**Windows / WSL:** run the build inside a WSL distro (Ubuntu) with rustup installed there, not
the Windows toolchain. Docker Desktop must have WSL integration on for that distro. The
"integrate with default distro" setting only covers the default distro, so either run
`wsl --set-default Ubuntu` or enable Ubuntu under Settings > Resources > WSL integration. The
build is faster from a copy of the repo on the WSL filesystem than from `/mnt/c` or `/mnt/d`.
`.gitattributes` keeps `*.sh` LF so the scripts run even with `core.autocrlf=true`. Install
from Windows with `adb install -r` on the APK path.

## Install

On the phone, enable Developer options, then USB debugging. Then:

```bash
adb install -r target/android/cozy-rally.apk
```

You can also copy the APK to the phone and open it. Android asks once to allow installs from
that app.

Android only updates an installed APK when the new one is signed with the same key:

- **Debug key (default):** generated once in `~/.android/cozy-debug.keystore`. Keep that file
  and always build on the same machine.
- **Release key:** put a keystore at `android/keystore/release.jks` (alias `cozyrally`, not
  committed) and set `COZY_KEYSTORE_PASS`.

If you switch keys, uninstall the old app first. Doing so deletes its saves.

## Logs and crashes

```bash
adb logcat -s CozyRally SAPP AndroidRuntime
```

Rust panics go to logcat with the `CozyRally` tag. Every 5 s the game also logs a `perf:` line
with fps, average and worst frame time, CPU time per section, draw calls, resolution and
quality. Phones have no F3 overlay, so use this line to check performance.

## What is different on the phone

| | PC | Android |
|---|---|---|
| Controls | keyboard, gamepads (gilrs) | touch controls; Bluetooth/USB gamepads are optional and hide the touch controls while used |
| Back | Esc | system Back button = Esc (never closes the app) |
| Background | n/a | race pauses, audio stream stops |
| Saves | `%APPDATA%\CozyRally`, `~/.config` | app private storage (`getFilesDir()`) |
| Defaults | High quality | Low quality, no grip widget; Settings > Graphics can switch |
| Menus | | no Split-screen, no Quit; "Touch steering": Arrows or Slider |

To test touch controls on PC, run with `COZY_TOUCH=1`. The mouse then acts as one finger.

## Code map

- `crates/rally_game/src/platform/`: the platform layer. `mod.rs` holds the shared API;
  `android.rs` holds the JNI exports (`CozyNative`), pad state, rumble and logcat.
- `crates/rally_game/src/touch.rs`: on-screen controls (layout, multitouch, drawing).
- `crates/rally_game/src/input.rs`: `android_pads()` maps Android pad events to the same
  `PadState` the PC uses.
- `android/java/MainActivity.java.in`: lines marked `Cozy` are ours. When bumping miniquad,
  re-vendor its `java/MainActivity.java` and re-apply those blocks.
