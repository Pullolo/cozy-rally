#!/usr/bin/env bash
# Build a sideloadable Cozy Rally APK (arm64-v8a).
#
# Runs inside the notfl3/cargo-apk Docker image, which provides the Android NDK r25, SDK
# platform 31 / build-tools 31 and a JDK. The image's own Rust (and cargo-quad-apk, which embeds
# an old Cargo) is too old for edition 2024, so instead the host's rustup toolchain is mounted
# in and android/build-inner.sh does what cargo-quad-apk would: build the game as a cdylib,
# compile miniquad's Java activity, package with aapt, zipalign and sign.
#
# Usage: android/build.sh [debug|release]      (default: release)
# Output: target/android/cozy-rally.apk        install: adb install -r target/android/cozy-rally.apk
#
# Signing: uses android/keystore/release.jks if present (env COZY_KEYSTORE_PASS), else a debug
# key generated once into ~/.android/cozy-debug.keystore. Keep the same key between builds or
# Android refuses to update an installed APK.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${1:-release}"
RUSTUP="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO="${CARGO_HOME:-$HOME/.cargo}"
TOOLCHAIN="$(cd "$ROOT" && rustup show active-toolchain | cut -d' ' -f1)"
IMAGE="${COZY_ANDROID_IMAGE:-notfl3/cargo-apk}"

rustup target add --toolchain "$TOOLCHAIN" aarch64-linux-android >/dev/null
# fetch crates on the host (network) so the container can build offline
cargo fetch --manifest-path "$ROOT/Cargo.toml" >/dev/null
mkdir -p "$HOME/.android"

docker run --rm \
  -v "$ROOT":/src -w /src \
  -v "$RUSTUP":/opt/host-rustup:ro \
  -v "$CARGO/registry":/usr/local/cargo/registry \
  -v "$HOME/.android":/root/.android \
  -e RUSTUP_HOME=/opt/host-rustup \
  -e HOST_TOOLCHAIN="$TOOLCHAIN" \
  -e CARGO_NET_OFFLINE=true \
  -e CARGO_TARGET_DIR=/src/target \
  -e COZY_VERSION_CODE="${COZY_VERSION_CODE:-$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || true)}" \
  -e COZY_KEYSTORE_PASS="${COZY_KEYSTORE_PASS:-}" \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  "$IMAGE" bash /src/android/build-inner.sh "$PROFILE"
