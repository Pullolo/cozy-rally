# Cozy Rally: working notes for Claude

Read `ARCHITECTURE.md` (crates, frame flow, GPU gotchas, test hooks) and `android/README.md`
(APK build, phone specifics) before larger changes. This file covers how to update, verify and
ship the project from the state of 2026-11-24 (commit ee2d976, version 0.1.0).

## Branches

- `main` is the single source for **PC, Android and the server**. One codebase means PC and
  phones race together online, so never fork platform-specific branches that drift.
- Do feature work on a branch (`claude/<name>`), then fast-forward or merge into `main` when the
  checks below pass. Push to `main` only when the user asks for it.
- Platform-specific code stays behind `cfg(target_os = "android")` / `platform::MOBILE` in
  `crates/rally_game/src/platform/`, `touch.rs` and `android/`. A PC-only change must not break
  the APK build, and an Android change must not alter PC behaviour.

## Machine setup (Windows host, already done)

- Repo: `D:\Rust\cozy-rally`. Windows Rust builds the PC game and the Windows server natively.
- WSL **Ubuntu** is the default distro and has its own rustup (`~/.cargo`). Docker Desktop's WSL
  integration only covers the default distro. The APK builds there in Docker
  (`notfl3/cargo-apk`, NDK r25 / SDK 31) from a synced copy at `~/cozy-rally`.
- The Ubuntu user has no passwordless sudo. Install only user-space tools.
- `adb` runs on Windows. The test phone is a Xiaomi 24117RK2CG (HyperOS, Android 16, 2400x1080,
  120 Hz). It **blocks `adb shell input`** (tap/key injection), so the user taps on the phone
  and Claude reads logcat and screenshots.
- PowerShell 5.1 mangles `"` inside native-command arguments. Pass git commit messages with
  `git commit -F <file>`, and use single quotes inside `wsl ... bash -lc` strings.
- The release exe is a GUI-subsystem app. Use `Start-Process -Wait` when a script must wait for
  it (screenshot hooks).

## Everyday commands (PowerShell, repo root)

```powershell
cargo build --release -p rally_game                       # PC game
cargo test --release --workspace                          # all tests (incl. rally_net socket tests)
cargo run --release -p rally_core --example sim -- 6 2    # must finish: DNF 0, NaN 0
tools\android-deploy.ps1                                  # APK in WSL -> install -> launch on phone
tools\android-deploy.ps1 -Logcat                          # same, then follow the game's logcat
tools\release.ps1                                         # all artifacts -> dist\cozy-rally-<ver>-<hash>\ + zip
```

## Before merging to main (checklist)

1. `cargo build --release -p rally_game`: only the existing dead-code warnings.
2. `cargo test --release --workspace` passes. Examples (`showroom`) compile too: new struct
   fields must be added there.
3. The `sim` example reports `DNF 0 NaN 0`.
4. Android: `tools\android-deploy.ps1` builds. The link uses `--no-undefined`, so a missing
   native symbol fails the build here instead of crashing on launch. On the phone, the logcat
   shows no `PANIC`/`FATAL` and the `perf:` line (every 5 s) holds ~60 fps on Low.
5. UI or HUD changes: check the touch layout on PC with `COZY_TOUCH=1 COZY_WINDOW=1600x720`
   (phone aspect) plus `COZY_SHOTS` screenshots, and **look at the PNGs**. Keep touch widgets
   out of the bottom corners (controls) and the top-right row (pause, speedo, timer).
6. Online protocol changes: bump `PROTOCOL_VERSION` (rally_net/src/protocol.rs). Old clients
   and servers then refuse to talk, so ship new game and server builds together.

## Testing on the phone

- Logs: `adb logcat -v brief -s CozyRally SAPP AndroidRuntime DEBUG`. Rust panics are logged
  as `CozyRally: PANIC ...`.
- Screenshot: `adb shell screencap -p /sdcard/cozy.png; adb pull /sdcard/cozy.png <file>`
  (don't use `adb exec-out ... >` in PowerShell: it corrupts the binary).
- Background/resume: send Home with
  `adb shell am start -a android.intent.action.MAIN -c android.intent.category.HOME`, then
  start the game again. `dumpsys audio` should show the AAudio player paused, then started.
- Online with PC: run `target\release\rally_server.exe` (Bash `run_in_background`), then
  `adb reverse tcp:27420 tcp:27420`. The phone joins `127.0.0.1` (TCP fallback, since adb
  can't tunnel UDP). A PC client joins with env `COZY_ONLINE=127.0.0.1 COZY_NAME=PC
  COZY_ONLINE_LEAD=2`, picks a 1-lap race and starts once 2 players are in. Afterwards:
  `Stop-Process -Name rally_server, rally_game` (a running server locks its exe for cargo) and
  `adb reverse --remove-all`.
- Run PC test instances with `COZY_DATA_DIR` / `COZY_SETTINGS` pointing into `target\` so the
  user's own saves aren't touched.

## Releasing

- Bump `version` in `crates/rally_game/Cargo.toml` for a new public version. The Android
  `versionCode` is the git commit count, so every commit installs over the last.
- Commit, then run `tools\release.ps1`. It writes `dist\` (gitignored) with the Windows game,
  the Windows and Linux `rally_server`, the APK, a README (template:
  `tools\release-README.txt`) and SHA256SUMS. Smoke-test the exe and install the APK on the
  phone before handing it over.
- The APK is signed with the debug key in WSL `~/.android/cozy-debug.keystore`. Keep that file:
  a different key can't update the installed app. For a real release key, put
  `android/keystore/release.jks` (gitignored) in place and set `COZY_KEYSTORE_PASS`. Users must
  then uninstall once, which loses their saves. Ask the user before switching.

## Known follow-ups

- The title menu comes within ~25 px of the phone's camera cutout on the left edge (safe-area
  insets aren't passed to Rust).
- Gamepads on Android have not been tested on hardware yet.
- UDP from the phone hasn't been tested on real Wi-Fi (TCP fallback works).
- The Linux server is linked against WSL Ubuntu's glibc. Build it on an older distro, or with
  musl, if a host can't run it.
