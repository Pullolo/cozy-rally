#!/usr/bin/env bash
# Runs inside the Android build container (see android/build.sh). Do not call directly.
set -euo pipefail
PROFILE="${1:-release}"
PACKAGE="com.cozyrally.game"
LIB="rally_game"
TRIPLE="aarch64-linux-android"
ABI="arm64-v8a"
MIN_SDK=26
SDK="$ANDROID_HOME"
BT="$SDK/build-tools/31.0.0"
ANDROID_JAR="$SDK/platforms/android-31/android.jar"
LLVM="$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64"
export PATH="/opt/host-rustup/toolchains/$HOST_TOOLCHAIN/bin:$PATH"

cd /src
OUT=target/android
WORK=$OUT/work
rm -rf "$WORK" && mkdir -p "$WORK"/{java,obj,apk/lib/$ABI,res}

# ---------------------------------------------------------------- 1. Rust -> lib<name>.so
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$LLVM/bin/${TRIPLE}${MIN_SDK}-clang"
export CC_aarch64_linux_android="$LLVM/bin/${TRIPLE}${MIN_SDK}-clang"
export CXX_aarch64_linux_android="$LLVM/bin/${TRIPLE}${MIN_SDK}-clang++"
export AR_aarch64_linux_android="$LLVM/bin/llvm-ar"
export CXXSTDLIB=c++
PROFILE_FLAG=""; PROFILE_DIR=debug
if [ "$PROFILE" = release ]; then PROFILE_FLAG="--release"; PROFILE_DIR=release; fi
# The game is a bin crate; the wrapper builds it as a cdylib (the .so Java loads).
[ "$PROFILE" = release ] && export CARGO_PROFILE_RELEASE_STRIP=symbols
RUSTC_WRAPPER=/src/android/rustc-wrapper.sh cargo build -p rally_game --bin "$LIB" --target "$TRIPLE" $PROFILE_FLAG
SO=target/$TRIPLE/$PROFILE_DIR/$LIB
cp "$SO" "$WORK/apk/lib/$ABI/lib${LIB}.so"
# C++ runtime (cpal's oboe backend links libc++_shared)
if "$LLVM/bin/llvm-readelf" -d "$SO" | grep -q 'libc++_shared.so'; then
  cp "$LLVM/sysroot/usr/lib/$TRIPLE/libc++_shared.so" "$WORK/apk/lib/$ABI/"
fi

# ---------------------------------------------------------------- 2. Java activity -> classes.dex
mkdir -p "$WORK/java/quad_native" "$WORK/java/${PACKAGE//.//}"
cp android/java/QuadNative.java "$WORK/java/quad_native/"
# miniquad's activity template: package + library name, `//%` plugin markers dropped
for f in MainActivity CozyNative; do
  sed -e "s/TARGET_PACKAGE_NAME/$PACKAGE/" -e "s/LIBRARY_NAME/$LIB/" -e '/^\s*\/\/%/d' \
    android/java/$f.java.in > "$WORK/java/${PACKAGE//.//}/$f.java"
done
javac -source 1.8 -target 1.8 -nowarn -Xlint:-options -bootclasspath "$ANDROID_JAR" \
  -d "$WORK/obj" $(find "$WORK/java" -name '*.java')
"$BT/d8" --min-api $MIN_SDK --lib "$ANDROID_JAR" --output "$WORK/apk" $(find "$WORK/obj" -name '*.class')

# ---------------------------------------------------------------- 3. resources + manifest
mkdir -p "$WORK/tools" && javac -nowarn -d "$WORK/tools" android/tools/MakeIcon.java
java -Djava.awt.headless=true -cp "$WORK/tools" MakeIcon "$WORK/res"
VERSION_NAME=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/rally_game/Cargo.toml | head -1)
VERSION_CODE="${COZY_VERSION_CODE:-1}"
sed -e "s/@PACKAGE@/$PACKAGE/" -e "s/@VERSION_CODE@/$VERSION_CODE/" -e "s/@VERSION_NAME@/$VERSION_NAME/" \
  android/AndroidManifest.xml.in > "$WORK/AndroidManifest.xml"

# ---------------------------------------------------------------- 4. package, align, sign
"$BT/aapt" package -f -M "$WORK/AndroidManifest.xml" -S "$WORK/res" -I "$ANDROID_JAR" \
  -F "$WORK/unaligned.apk"
( cd "$WORK/apk" && "$BT/aapt" add ../unaligned.apk classes.dex lib/$ABI/*.so >/dev/null )
"$BT/zipalign" -f -p 4 "$WORK/unaligned.apk" "$WORK/aligned.apk"

if [ -f android/keystore/release.jks ]; then
  KS=android/keystore/release.jks; ALIAS=cozyrally; PASS="${COZY_KEYSTORE_PASS:?set COZY_KEYSTORE_PASS}"
else
  KS=/root/.android/cozy-debug.keystore; ALIAS=androiddebugkey; PASS=android
  [ -f "$KS" ] || keytool -genkeypair -v -keystore "$KS" -storepass android -keypass android \
    -alias androiddebugkey -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Cozy Rally Debug" >/dev/null 2>&1
fi
"$BT/apksigner" sign --ks "$KS" --ks-pass "pass:$PASS" --key-pass "pass:$PASS" --ks-key-alias "$ALIAS" \
  --out "$OUT/cozy-rally.apk" "$WORK/aligned.apk"
"$BT/apksigner" verify "$OUT/cozy-rally.apk"
chown -R "${HOST_UID:-0}:${HOST_GID:-0}" "$OUT" target/$TRIPLE /root/.android 2>/dev/null || true
echo "APK: $OUT/cozy-rally.apk ($(du -h "$OUT/cozy-rally.apk" | cut -f1), versionCode $VERSION_CODE)"
