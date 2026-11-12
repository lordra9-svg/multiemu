#!/usr/bin/env bash
# Builds the iOS app bundle.
#
# Requirements: macOS with Xcode, the aarch64-apple-ios Rust target, and
# XcodeGen (`brew install xcodegen`).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)

cd "$root"
cargo build --release --target aarch64-apple-ios --lib -p unko-app

cd "$here"
xcodegen generate --spec project.yml
xcodebuild \
  -project Unko.xcodeproj \
  -scheme Unko \
  -configuration Release \
  -sdk iphoneos \
  -derivedDataPath build \
  build

echo "App: $here/build/Build/Products/Release-iphoneos/Unko.app"
