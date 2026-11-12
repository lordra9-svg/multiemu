# Platforms

Unko ships one egui/eframe application for five targets: Linux, Windows,
macOS, iOS and Android. The NES core, the shell UI, the touch overlay and the
recording encoders are shared; only the platform glue differs.

| Target | Window/input | Audio | Package |
|---|---|---|---|
| Linux | winit + glow, SDL3 gamepads | cpal (ALSA/PulseAudio) | `unko-linux-x86_64.tar.gz` |
| Windows | winit + glow, SDL3 gamepads | cpal (WASAPI) | `unko-windows-x86_64.zip` |
| macOS | winit + glow, SDL3 gamepads | cpal (CoreAudio) | `Unko.app` in `unko-macos-arm64.zip` |
| iOS | winit + wgpu (Metal), touch overlay | cpal (CoreAudio) | Xcode app (static lib) |
| Android | winit + glow (EGL), touch overlay | cpal (AAudio) | Gradle APK (cdylib) |

## What is verified where

The repository's CI checks the full workspace on Linux, Windows and macOS, and
checks the app shell plus the NES core for `aarch64-linux-android` and
`aarch64-apple-ios` on Linux runners. Those checks compile the complete shell,
including eframe, winit, cpal and the platform backends.

What still needs a real toolchain (Xcode and the Android SDK/NDK), and is
therefore *not* run in this repository's CI, is packaging, signing and running
on a device:

- `crates/unko-app/ios/` is an XcodeGen project that links the Rust static
  library and calls `start_winit_app()` from `main.m`.
- `crates/unko-app/android/` is a Gradle project with a
  `android.app.NativeActivity` manifest; `cargo ndk` places
  `libunko_app.so` in `jniLibs`.

Both build scripts fail loudly when their platform tools are missing.

## Desktop

```sh
cargo build --release -p unko-cli -p unko-app
./target/release/unko-app --roms /path/to/roms
```

The release workflow (`.github/workflows/release.yml`) builds the three desktop
archives on tag pushes (`v*`) and attaches them to the GitHub release. The
macOS job bundles `Unko.app` with a minimal `Info.plist`; it is unsigned, so
Gatekeeper needs a one-time `xattr -dr com.apple.quarantine Unko.app` or an
`codesign`/notarisation step in a fork.

## iOS

```sh
cargo build --release --target aarch64-apple-ios --lib -p unko-app
crates/unko-app/ios/build.sh   # xcodegen + xcodebuild
```

Requirements: macOS, Xcode, the `aarch64-apple-ios` Rust target, and
XcodeGen (`brew install xcodegen`). The Rust library is built as a `staticlib`;
`ios/Sources/main.m` calls the exported `start_winit_app`. iOS uses the wgpu
(Metal) renderer because eframe's glow backend has no iOS support. Add a
`DEVELOPMENT_TEAM` to `project.yml` to sign for a device.

## Android

```sh
rustup target add aarch64-linux-android
cargo install cargo-ndk
crates/unko-app/android/build.sh
```

Requirements: Android SDK, NDK, Gradle 8.9+ (or generate the wrapper once with
`gradle wrapper --gradle-version 8.9`), and `cargo-ndk`. The build produces
`libunko_app.so` for `arm64-v8a` and wraps it in a `NativeActivity` APK. The
manifest uses the framework `android.app.NativeActivity` (no Java/Kotlin
code is needed). Signing uses the standard Gradle release configuration.

## Mobile behaviour

- **Input**: the touch overlay is the primary controller and is enabled by
  default on iOS/Android. SDL3 is a desktop-only dependency, so the mobile
  build uses the no-op gamepad backend in `src/gamepad/mobile.rs`; a native
  `GameController`/`Gamepad` backend can replace it without touching the UI.
- **Paths**: settings live in the platform config directory
  (`ProjectDirs`), saves/ROMs under the platform data directory; the shell
  creates the config directory when saving. Desktop keeps the portable
  `unko.conf` next to the executable.
- **Lifecycle**: losing focus pauses the game; the audio stream is paused
  rather than fed silence, and the UI repaints at 4 Hz while idle.
- **ROM import**: mobile has no file picker yet. ROMs must be present in the
  configured `rom_dir` under the app's data directory; a Storage Access
  Framework / `UIDocumentPicker` import is the next mobile feature.

## Known gaps

- Native gamepad support on iOS/Android.
- Mobile ROM import and share-sheet handling.
- App icons and launch screens (both scaffolds use platform defaults).
- Notarisation/signing credentials for macOS, iOS and Play Store releases.
- Physical-device validation: none of the mobile packages has been run on a
  device in this repository's environment.
