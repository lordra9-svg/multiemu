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

## Portable data root

The shell resolves one data root before loading settings. It contains
`unko.conf`, `roms/`, `saves/` and `disksys.rom`; every state, SRAM file,
thumbnail, cheat list, input movie, screenshot and recording lives under
`saves/`. Moving the folder therefore moves the complete setup.

Resolution order:

1. `--data-dir DIR` / `--home DIR`;
2. `UNKO_HOME` or `UNKO_DATA_DIR`;
3. `--portable` (executable folder, AppImage folder, or the folder containing a
   macOS `.app`);
4. an `unko.conf` beside the executable;
5. an `unko.conf` in the current directory (legacy);
6. the executable folder when writable;
7. the platform data directory (`ProjectDirs`) for read-only installs.

Relative paths in `unko.conf` are resolved against the configuration file's
folder and written back relative to it when possible; absolute paths remain
absolute. On mobile the platform sandbox is the default, and `UNKO_HOME` can
point at an imported/exported folder.

The CLI does not keep hidden state: `--bios`, `--sram`, `--out` and
`--audio-dir` take explicit paths, so the same portable folder works in
scripts.

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

The release workflow (`.github/workflows/release.yml`) builds the desktop
packages on tag pushes (`v*`) and attaches them to the GitHub release:

| File | Contents |
|---|---|
| `unko-linux-x86_64.tar.gz` | `unko`, `unko-app`, desktop entry, icon, docs and licenses |
| `unko-<version>-x86_64.AppImage` | self-contained Linux app (linuxdeploy + appimagetool) |
| `unko-windows-x86_64.zip` | `unko.exe`, `unko-app.exe`, docs and licenses |
| `unko-macos-arm64.zip` | `Unko.app` (icon + `Info.plist`), `unko` CLI, docs and licenses |
| `SHA256SUMS` | checksums for the downloadable files |

The macOS bundle is unsigned, so Gatekeeper needs a one-time
`xattr -dr com.apple.quarantine Unko.app` or a `codesign`/notarisation step in
a fork.

### Reproducible artifacts

`scripts/package.py` builds archives with sorted entries, zeroed ownership and
`SOURCE_DATE_EPOCH` timestamps, so the same commit and toolchain produce
byte-identical `tar.gz`/zip files. The CI `Reproducible build check` job builds
the workspace twice in different directories with
`--remap-path-prefix=<path>=/build/unko`, then compares the binaries and the
Linux archive byte for byte. The AppImage is built for convenience but is not
claimed to be reproducible (appimagetool embeds timestamps).

### License audit

`scripts/license_audit.py` evaluates every dependency's SPDX expression from
`cargo metadata` against the allowlist in the script (MIT, Apache-2.0,
BSD-2/3, ISC, Zlib, Unicode-3.0, BSL-1.0, MPL-2.0, CC0, Unlicense, 0BSD,
OFL/Ubuntu font, bzip2 and the LLVM exception). It runs in CI and fails on any
unlisted or missing license.

### Per-platform build steps

```sh
scripts/package.py --platform linux --arch x86_64
scripts/package.py --platform windows --arch x86_64
scripts/package.py --platform macos --arch arm64   # run scripts/make_icns.sh first
scripts/appimage.sh dist                            # Linux AppImage
```

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
- **Paths**: mobile uses one platform data root (`ProjectDirs`) for
  `unko.conf`, ROMs and saves; `UNKO_HOME` can point at an imported/exported
  folder. Desktop portable installs keep the same layout beside the
  executable, and read-only installs fall back to the platform data root.
- **Lifecycle**: losing focus pauses the game; the audio stream is paused
  rather than fed silence, and the UI repaints at 4 Hz while idle.
- **ROM import**: mobile has no file picker yet. ROMs must be present in the
  configured `rom_dir` under the app's data directory; a Storage Access
  Framework / `UIDocumentPicker` import is the next mobile feature.

## Known gaps

- Native gamepad support on iOS/Android.
- Mobile ROM import and share-sheet handling.
- Flatpak packaging: the AppImage and tarball are the current Linux packages;
  a Flatpak manifest would need the crates vendored (`flatpak-cargo-generator`)
  for an offline build.
- Android/iOS app icons and launch screens (both scaffolds use platform
  defaults; the desktop icon is in `assets/`).
- Notarisation/signing credentials for macOS, iOS and Play Store releases.
- Physical-device validation: none of the mobile packages has been run on a
  device in this repository's environment.
