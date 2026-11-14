# Unko

A multi-platform, multi-core emulator shell written in Rust. The first system
is the NES, powered by the reference-parity `nes_accuracy_core` behind a
system-agnostic interface.

> **Rename:** this project was formerly `multi-emu` / `multiemu`. The emulator is
> now **Unko**, the host crates are `unko-core`, `unko-rom`, `unko-cli` and
> `unko-app`, and the NES core is `nes_accuracy_core` (`--core nes_accuracy_core`).

**Repository:** <https://github.com/lordra9-svg/unko>

## Status

The shell is usable end-to-end: library scanning, a Dolphin-inspired desktop
UI, GPU output shaders, speed controls, cheats, save states, exact rewind,
0-4 frame run-ahead and input-movie (TAS) record/playback. It runs the NES core
headlessly or in a desktop window, produces RGBA video and f32 audio, plays
audio through cpal, accepts keyboard, SDL3 gamepad, touch and peripheral input,
records WAV/PNG/GIF, and verifies determinism. FDS images load with a user-supplied `disksys.rom`
(**Settings → Paths → FDS BIOS**), with disk-side selection and persisted
disk writes. The same UI builds for Windows, macOS, Linux, iOS and Android.

The accuracy core is at milestone **A5** (accuracy gate), verified against the
public reference suites:

- `nestest` golden log: **8,980 / 8,980 lines** exact (PC, A/X/Y, SP, P and cycle count)
- blargg `instr_timing` and `instr_basics`: pass
- `ppu_vbl_nmi` **12/12**, `ppu_sprites` **19/19**, blargg APU/DMC **13/13**
- AccuracyCoin v3.1.0 isolated: **146/146 non-DRAW rows pass**, 0 failed,
  5 DRAW skips, 0 unrun (199,320 frames)
- NESdev archive baseline: **129 pass / 15 fail / 119 visual-or-input-only** (7 PAL APU failures pass when PAL timing is selected)
- RustyNES test tree: **149 pass / 15 fail / 177 visual-or-input-only** (156 / 8 with PAL APU in PAL mode)

Compatibility from the local licensed 7,101-image NES audit:

| Metric | Result |
|---|---:|
| Scanned | 7,101 |
| Loaded (mapper implemented) | 7,073 (**99.6%**) |
| Unsupported mapper IDs | **0** |
| Load/run failures | 28 (all truncated dumps) |
| Rendering at 20 frames | 3,898 |
| Rendering at 120 frames | 6,490 |
| Distinct mapper IDs in the set | 285 |

Only mapper 308 (NTDEC Batman) does not draw headless without Start input.
"Rendering" in that audit is a boot smoke signal, not a completion claim; the
20-frame window mostly measures how long a game stays on its boot screen.
Current figures, dated entries and the remaining limits are in
[`docs/compatibility.md`](docs/compatibility.md),
[`docs/accuracy-status.md`](docs/accuracy-status.md) and
[`docs/mapper-audit.md`](docs/mapper-audit.md); known divergences are in
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
  implementation-plan.md # current easy-to-hard feature queue
  mapper-audit.md        # 7,101-image compatibility and mapper coverage
  test-rom-inventory.md  # NESdev suite coverage and missing BIOS/ROMs
  compatibility.md       # systems, mappers, peripherals and feature support
  performance.md         # benchmark, state costs and efficiency budgets
  oracle-feature-matrix.md # comparison against the NES emulator field
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
renderer). Desktop packages are produced on tag pushes by
`.github/workflows/release.yml`: a Linux tarball and AppImage, a Windows zip
and a macOS `Unko.app` zip, each with `SHA256SUMS`. Archives are byte
reproducible from the same commit and toolchain, and CI audits dependency
licenses and verifies the reproducibility claim. Requirements and current
gaps are in [`docs/platforms.md`](docs/platforms.md).
Keyboard: arrows = D-pad, X = A, Z = B, Enter = Start, Shift = Select,
Space = pause, F5/F8 = save/load state, F9/F10 = step frame/instruction,
Backspace = rewind while paused, Tab = hold-to-fast-forward, F11 = fullscreen.

Playback and capture: 25-300% speed presets, frame advance, exact rewind
(64-256 MiB ring), 0-4 frame run-ahead, input-movie (TAS) recording/playback,
Game Genie and raw-RAM cheats, WAV/PNG/GIF capture, and screenshots.

Per-port devices (Settings -> Controllers): NES Zapper, Arkanoid paddle,
Power Pad / Family Trainer, Family BASIC keyboard, Subor/Dendy keyboard,
Famicom microphone, data-recorder tape loopback, Miracle Piano transport,
Datach scanner with a persisted X24C02 EEPROM, and the Four Score adapter.

The debugger window (`Emulation -> Debugger`, or `--debug` on the command
line) shows CPU registers and disassembly, PPU timing/palette/OAM, APU channel
outputs, a hex viewer/editor for CPU, PPU and OAM memory, and PC breakpoints
that pause before the instruction executes.

## Testing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/license_audit.py
```

Tests that need public test ROMs look for them under `test-roms/` and skip
cleanly when the folder is absent. No ROMs are tracked in this repository.

The synthetic performance floor is checked in CI; the current development
machine measures about **785 fps** (13.1x real time):

```sh
cargo run --release -p unko-cli -- bench --synthetic 3000 --min-fps 300
```

The compatibility smoke test needs a local library:

```sh
cargo run --release -p unko-cli -- corpus <library-dir> --frames 20
```

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
