# Unko

A multi-platform, multi-core emulator shell written in Rust. The first system
is the NES, powered by the reference-parity `nes_accuracy_core` behind a
system-agnostic interface.

> **Rename:** this project was formerly `multi-emu` / `multiemu`. The emulator is
> now **Unko**, the host crates are `unko-core`, `unko-rom`, `unko-cli` and
> `unko-app`, and the NES core is `nes_accuracy_core` (`--core nes_accuracy_core`).

**Repository:** <https://github.com/lordra9-svg/unko>

## Status

The shell is at **M1**. It boots a NES ROM headlessly or in a desktop window,
runs frames at 60 fps, produces RGBA video and f32 audio, plays audio through
cpal, accepts keyboard and SDL3 gamepad input, records WAV/PNG/GIF, round-trips
save states, and verifies determinism. The same UI is being prepared for
Windows, macOS, Linux, iOS and Android (explicit touch controls are included).

The accuracy core is at milestone **A5** (accuracy gate):

- `nestest` 0 diff, blargg `instr_timing` / `instr_basics` pass
- `vbl_nmi` 12/12, `ppu_sprites` 19/19, all 13 blargg APU/DMC ROMs pass
- AccuracyCoin isolated battery: **146/151** (5 DRAW skips, 0 failures)

Cycle accuracy is not claimed yet. The remaining failures and every known
approximation are recorded in
[`docs/accuracy-status.md`](docs/accuracy-status.md) and
[`docs/accuracy-divergences.md`](docs/accuracy-divergences.md).

## Layout

```
crates/
  unko-core/         # SystemId, Core trait, video/audio/input/state types, registry
  unko-rom/          # discovery + .zip/.7z/.rar containers + iNES metadata
  nes_accuracy_core/ # reference-parity NES core + its Core ABI adapter (MIT + documented ISC adaptations)
  unko-cli/          # headless runner: info, scan, run, verify, trace, diff, bench, corpus
  unko-app/          # desktop shell (Dolphin-inspired UX)
docs/
  accuracy.md            # accuracy charter, gates and source rules
  accuracy-status.md     # measured results and dated experiment log
  accuracy-divergences.md # known divergences and the open failure list
  architecture.md        # core boundary and why it looks like this
  systems.md             # system/core entries, licenses, integration order
  roadmap.md             # A0..A7 and M0..M4
  decisions.md           # license/stack/scope decisions
  emulator-tooling.md    # installed reference emulators and toolchains
  frontend.md            # desktop UX blueprint and the GPL boundary
```

Target order: **NES → SNES (ares) → N64 (ares) → optional external Dolphin**.
The NES track uses one core: `nes_accuracy_core`, a Rust core built to
reference parity. It is MIT with documented ISC-licensed adaptations from ares;
MesenCE, RustyNES, gopher64 and Dolphin are GPL references or separate products.
Details in `docs/systems.md`.

## Build and run

A profile-guided build (about 10-13% faster) is available with
`scripts/pgo.sh <rom-directory>`; see `docs/performance.md`.

```sh
cargo build --release
cargo run -p unko-cli -- scan roms
cargo run -p unko-cli -- info "roms/.../game.zip"
cargo run -p unko-cli -- run "roms/.../game.zip" --frames 120 --out shot.ppm
```

ROMs may be loose `.nes` files or inside `.zip`, `.7z`, or `.rar` archives;
the CLI and GUI use the same reader. `scan` reports the library and mapper
metadata without extracting anything to disk.

`run` writes the last display frame as a binary PPM (`P6`) so you can inspect
output without a window.

## Desktop app

```sh
cargo run -p unko-app --release
cargo run -p unko-app --release -- --rom "roms/.../game.zip"
```

The library defaults to `roms/`; pass `--roms DIR` to point elsewhere. The shell follows Dolphin's desktop
UX: a game library as the home screen, menu bar, toolbar, player viewport,
status bar, and one settings window with a category rail. See
`docs/frontend.md` for the blueprint and the GPL boundary.

Gamepads are hot-plugged through SDL3; bindings, auto-fire, dead zone,
per-game profiles and keyboard shortcuts are configurable in Settings.

### Mobile and packaging

The same shell builds for iOS and Android: the crate is a library
(`unko_app`) plus a desktop binary, with `android_main` and `start_winit_app`
entry points, a touch overlay, and platform settings/save paths. Packaging
scaffolds live in `crates/unko-app/android` (Gradle + `cargo-ndk`,
`NativeActivity`) and `crates/unko-app/ios` (XcodeGen + static library, wgpu
renderer). Desktop release archives are produced on tag pushes by
`.github/workflows/release.yml`. Requirements and current gaps are in
[`docs/platforms.md`](docs/platforms.md).
Keyboard: arrows = D-pad, X = A, Z = B, Enter = Start, Shift = Select,
Space = pause, F5/F8 = save/load state, F11 = fullscreen.

## Testing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests that need public test ROMs look for them under `test-roms/` and skip
cleanly when the folder is absent. No ROMs are tracked in this repository.

To run the AccuracyCoin battery, supply the ROM and its catalog yourself:

```sh
cargo run -p nes_accuracy_core --release --example accuracycoin -- \
  test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated
```

## Licensing

Project code is MIT. Parts of the `nes_accuracy_core` NES core are adapted from
ares under its ISC licence; attribution is in `NOTICE`,
`licenses/ares-LICENSE.txt` and the crate's `PROVENANCE.md`. The desktop shell
uses SDL3 (zlib), cpal, egui/eframe and the Rust audio/image crates listed in
`NOTICE`. See `NOTICE` and `licenses/` for all third-party terms.

The retired TetaNES header-correction database was not carried over; the
accuracy core uses iNES/NES 2.0 headers directly. DAT-based bad-dump detection
and header repair are the next ROM-management feature.

This repository contains **no ROMs** and no copyrighted console firmware. Bring
your own legally obtained dumps and BIOS images.
