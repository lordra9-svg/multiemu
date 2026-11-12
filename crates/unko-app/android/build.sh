#!/usr/bin/env bash
# Builds the Android APK.
#
# Requirements: Android SDK, an installed NDK, and `cargo-ndk`
# (`cargo install cargo-ndk`). Gradle 8.9+ must be on PATH, or generate the
# wrapper once with `gradle wrapper --gradle-version 8.9`.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)

cd "$root"
cargo ndk -t arm64-v8a -o crates/unko-app/android/app/src/main/jniLibs \
  build --release --lib -p unko-app

cd "$here"
if [ -x ./gradlew ]; then
  ./gradlew assembleRelease
else
  gradle assembleRelease
fi
echo "APK: $here/app/build/outputs/apk/release/app-release-unsigned.apk"
