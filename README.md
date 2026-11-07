# multiemu

A multi-platform, multi-core emulator shell written in Rust. The first system
is the NES; the compatibility core is
[`tetanes-core`](https://crates.io/crates/tetanes-core) behind a
system-agnostic interface, and a second NES core
(`multiemu-nes-accuracy`) is being built toward reference cycle parity.

**Repository:** <https://github.com/lordra9-svg/multiemu>

## Status

The shell is at **M1**. It boots a NES ROM headlessly or in a desktop window,
runs frames at 60 fps, produces RGBA video and f32 audio, accepts keyboard
input, round-trips save states, and verifies determinism. Audio playback and
gamepad support are the next product increments.

The accuracy core is at milestone **A5** (accuracy gate):

- `nestest` 0 diff, blargg `instr_timing` / `instr_basics` pass
- `vbl_nmi` 12/12, `ppu_sprites` 19/19, all 13 blargg APU/DMC ROMs pass
- AccuracyCoin isolated battery: **139/151** (5 DRAW skips, 7 failures)

Cycle accuracy is not claimed yet. The remaining failures and every known
approximation are recorded in
[`docs/accuracy-status.md`](docs/accuracy-status.md) and
[`docs/accuracy-divergences.md`](docs/accuracy-divergences.md).

## Layout

```
crates/
  multiemu-core/         # SystemId, Core trait, video/audio/input/state types, registry
  multiemu-rom/          # discovery + .zip/.7z/.rar containers + iNES metadata
  multiemu-nes/          # NES compatibility core adapter (tetanes-core)
  multiemu-nes-accuracy/ # reference-parity NES core (MIT + documented ISC adaptations)
  multiemu-nes-accuracy-core/ # Core ABI adapter for the accuracy backend
  multiemu-cli/          # headless runner: info, scan, run, verify, trace, diff, bench, corpus
  multiemu-app/          # desktop shell (Dolphin-inspired UX)
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
The NES track has two cores: TetaNES for compatibility and a Rust core built to
reference parity. The accuracy core is MIT with documented ISC-licensed
adaptations from ares; MesenCE, RustyNES, gopher64 and Dolphin are GPL
references or separate products. Details in `docs/systems.md`.

## Build and run

```sh
cargo build --release
cargo run -p multiemu-cli -- scan roms
cargo run -p multiemu-cli -- info "roms/.../game.zip"
cargo run -p multiemu-cli -- run "roms/.../game.zip" --frames 120 --out shot.ppm
```

ROMs may be loose `.nes` files or inside `.zip`, `.7z`, or `.rar` archives;
the CLI and GUI use the same reader. `scan` reports the library and mapper
metadata without extracting anything to disk.

`run` writes the last display frame as a binary PPM (`P6`) so you can inspect
output without a window.

## Desktop app

```sh
cargo run -p multiemu-app --release
cargo run -p multiemu-app --release -- --rom "roms/.../game.zip"
```

The library defaults to `roms/`; pass `--roms DIR` to point elsewhere. The shell follows Dolphin's desktop
UX: a game library as the home screen, menu bar, toolbar, player viewport,
status bar, and one settings window with a category rail. See
`docs/frontend.md` for the blueprint and the GPL boundary.

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
cargo run -p multiemu-nes-accuracy --release --example accuracycoin -- \
  test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated
```

## Licensing

Project code is MIT. The NES compatibility backend, `tetanes-core`, is
dual-licensed MIT OR Apache-2.0 and is used here under the MIT option. Parts of
the `multiemu-nes-accuracy` NES core are adapted from ares under its ISC
licence; the attribution is in `NOTICE`, `licenses/ares-LICENSE.txt` and the
crate's `PROVENANCE.md`. See `NOTICE` and `licenses/` for all third-party terms.

This repository contains **no ROMs** and no copyrighted console firmware. Bring
your own legally obtained dumps and BIOS images.
