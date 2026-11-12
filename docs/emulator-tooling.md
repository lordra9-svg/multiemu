# Reference-emulator tooling

Host: Fedora-family Linux (immutable). Toolchains are installed in the user
Homebrew prefix, not layered into the OS.

## Installed toolchain

`brew install`:

```
cmake ninja ccache llvm lld lldb clang-format sdl2 sdl3 qt@6
vulkan-loader vulkan-headers vulkan-tools vulkan-validationlayers glslang
libusb hidapi libevdev libxkbcommon libx11 libxrandr libxi libxinerama
libxcursor libxext libxxf86vm libsm libice xorgproto mesa wayland
wayland-protocols libdecor pugixml enet miniupnpc mbedtls lzo curl ffmpeg
zstd alsa-lib dbus icu4c dotnet nasm yasm autoconf automake libtool meson
git-lfs
```

Versions at install time:

```
cmake 4.4.5          ninja 1.13.2       clang 23.1.5
gcc 16.2.1           dotnet 10.0.403    rustc/cargo 1.99.1
SDL2 2.32.74 (compat) SDL3 3.4.20        Qt 6.12.0
Vulkan 1.4.363       GTK3 3.24.52
```

Vulkan devices are visible (`vulkaninfo --summary` reports the AMD GPU).

### Symlinks added for missing Fedora devel files

```
libGLX.so     -> /usr/lib64/libGLX.so.0
libOpenGL.so  -> /usr/lib64/libOpenGL.so.0
libudev.so    -> $HOMEBREW_PREFIX/opt/systemd/lib/libudev.so
libudev.h     -> $HOMEBREW_PREFIX/opt/systemd/include/libudev.h
```

These live in `$(brew --prefix)/lib` and `include`.

## ares

```
cd refs/ares
cmake -S . -B build_headless -G Ninja \
  -DARES_BUILD_LOCAL=ON -DARES_BUILD_OPTIONAL_TARGETS=OFF \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH=$(brew --prefix) \
  -DOpenGL_GL_PREFERENCE=GLVND
ninja -C build_headless -j"$(nproc)"
```

Binary: `refs/ares/build_headless/desktop-ui/ares`.
CMake fetched the ares prebuilt dependencies into `.deps`.

## MesenCE

Mesen's default link uses static libstdc++, which Fedora's minimal image does
not ship for x86_64. Build dynamically with the system GCC:

```
cd refs/mesence
make STATICLINK=false USE_GCC=true -j"$(nproc)"
```

Binary: `refs/mesence/bin/linux-x64/Release/linux-x64/publish/Mesen`
(`MesenCore.so` is next to it).

Requires SDL2 at runtime (provided by `sdl2-compat`).

## gopher64

Submodules were initialized. Built with SDL3 + parallel-rdp:

```
cd refs/gopher64
cargo build --release
```

Binary: `refs/gopher64/target/release/gopher64`.

## RustyNES

```
cd refs/rustynes
cargo build --release
```

The workspace release build completes with the stable 1.99.1 toolchain.

## Dolphin

Dolphin submodules were initialized. Configuration succeeded with Qt6, SDL2,
Vulkan, FFmpeg, and libudev:

```
cd refs/dolphin
cmake -S . -B build_headless -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH="$(brew --prefix);$(brew --prefix)/opt/systemd"
ninja -C build_headless -j"$(nproc)"
```

Build directory used during validation: `build_headless`.
The full build completed (`1387/1387`) and produced:

```
build_headless/Binaries/dolphin-emu
Dolphin 66df2b4
```

## Submodule status

All reference-emulator submodules are initialized recursively:

```
refs/ares       0 uninitialized
refs/dolphin    0 uninitialized
refs/gopher64   0 uninitialized
refs/mesence    0 uninitialized
refs/rustynes   0 uninitialized
```

## Headless regression commands

`unko` carries the scriptable checks used for the audit and for change
gating:

- `corpus <dir> [--frames N] [--mapper ID] [--skip N] [--limit N] [--jobs N]`
  — smoke audit; reports loaded/rendering/blank/unsupported/failed, ranks
  unsupported mapper IDs and prints failing titles. Scans run in parallel:
  the default is logical CPUs minus one so the rest of the system keeps a
  core, `--jobs 1` restores the serial behaviour and `--jobs 0` uses every
  core. Results are merged by entry index, so the output is identical for
  every job count (7,101 images at 20 frames: 5.5 min serial, 42 s with the
  default; 120 frames: 4.9 min; 240 frames: 9.8 min). A full audit under
  eight competing CPU hogs still completes in 62 s with identical totals. `--parity` makes the quantitative
  differential audit the standard run: the oracle path comes from
  `--oracle PATH` or `UNKO_ORACLE`, `--frames` defaults to 120, and every
  image is executed by both the accuracy core and the reference producer with
  per-frame video/audio/geometry hashes compared at the same index. It prints
  overall and per-mapper identical-hash percentages, a palette-independent
  geometry percentage, and the lowest-parity titles. `--min-video PCT` /
  `--min-audio PCT` turn a shortfall into a non-zero exit status, so the same
  command gates CI.
- `boundary <rom|archive> [frames]` — prints the CPU cycle and PPU
  `(scanline, dot)` at which each frame completes, for comparing the frame
  boundary with a reference capture. The cycle deltas expose divider drift;
  `frame_cycle_accounting_holds_over_a_long_run` asserts the invariant in the
  unit suite.
- `signatures <dir> [--frames N] [--mapper ID] [--check FILE]` — hashes every
  frame of every image and compares against a local manifest, reporting
  changed/new/missing titles.
- `trace <rom> --frames N --out trace.txt [--no-state] [--audio-dir DIR]` and
  `diff a.trace b.trace [--skip N] [--video]` — per-frame video/audio hashes
  and the first divergent frame for one image. `--audio-dir` also writes the
  `f32` samples of each frame as `frame-NNNN.f32` (little-endian) for
  sample-level comparison with a reference capture. `--no-state` omits the
  save-state hash so a trace can be compared with a reference capture that has
  no state; `--skip N` ignores start-up frames and `--video` compares video
  only while the audio mixers are still being aligned.
- `parity a.trace b.trace [--window N]` — quantitative parity: same-index
  video/audio match counts, the first divergence of each, and the best
  alignment over ±N frames. This distinguishes a real rendering/audio defect
  from the known one-cycle frame-origin drift.
- `cargo run -p nes_accuracy_core --features apu-trace --example apudump --
  <rom> <frame>` — prints every channel's DAC output and the mixed value for
  each CPU cycle of one frame (`cycle p1 p2 tri noise dmc mixed`), matching
  the reference oracle's `APU_TRACE=<frame>` records. The feature is off by
  default so release builds carry no trace branch.
- `trace <disk.fds> --bios disksys.rom ... [--press-start N]` — traces a raw
  FDS disk image with a user-supplied BIOS. `--press-start N` holds Start for
  12 frames from frame `N` so a licence screen can be dismissed deterministically.
  The BIOS is never bundled or committed.

### Differential oracle

`trace` emits the `# unko trace v1` format: one `frame <index> <video>
<audio> [<geometry>]` line per frame using FNV-1a-64 over the RGBA8
framebuffer, over the `f32` audio samples and, when the producer emits it,
over a palette-independent frame description. A reference emulator only needs
to emit the same records; `diff` compares them frame by frame and reports the
first divergent video, geometry or audio hash.

The geometry hash (fourth field, optional) re-maps each distinct RGBA colour
to the index of its first appearance in scanline order and hashes those
indices. Any bijective palette therefore produces the same hash, while a real
tile, attribute, sprite or emphasis difference does not. It splits oracle
disagreement into "the RGB palette tables differ" and "the emulators render
different pixels": a video mismatch with an equal geometry hash is reported as
palette-only and is not a rendering defect. Traces from references that emit
only the original two hashes still compare; geometry is skipped for them.

The RustyNES adapter used for validation is a local, uncommitted black-box
tool (the GPL reference checkout never enters the repository): it loads the
ROM with `Nes::from_rom`, calls `run_frame`, and hashes the returned RGBA8
framebuffer, the `drain_audio()` samples and the canonical colour indices with
the same constants as `unko trace`. The ares/MesenCE wrappers emit the
original two-hash format, so their runs report video and audio only.

Current local result with the canonical 2C02 palette:

| Target | Identical frames |
|---|---|
| AccuracyCoin (60 frames) | 49/60 |
| Commercial NROM title (300 frames) | 297/300 |
| *Super Mario Bros.* NROM (600 frames) | video 599/600, audio 504/600 |
| *Super Mario Bros. 3* MMC3 (300 frames) | video 70/300, audio 228/300 (best audio shift −4: 259/300) |
| *The Legend of Zelda* MMC1 (600 frames) | video 598/600, audio 28/600 |

The Zelda figure is the clearest illustration of the residual limitation: the
video track matches almost exactly, while the continuously playing music makes
each frame's audio hash sensitive to where the 44.1 kHz drain lands. SMB's
sparser track tolerates the same sub-cycle phase difference far better, so the
audio number is a property of the drain-point alignment, not of a channel
implementation defect.

Two PPU defects were found and fixed with this harness:

- `$2007` completed reads masked the address with `$2FFF`, so CHR addresses
  above `$0FFF` read the wrong byte. A title-screen copy routine reading
  `$1EC0` obtained `$02` instead of `$20`; with `& $3FFF` the copied buffer
  and the whole title screen match the reference.
- 8x8 sprites selected their pattern table with PPUCTRL bit 4 (the background
  bit) instead of bit 3, garbling every sprite when the two tables differ.

The commercial title now differs only on frames 0, 9 and 27, where the first
scanline has a handful of backdrop-vs-black pixels while the PPU is still
starting up. Skipping those frames proves the long run explicitly:

```
unko diff video.trace reference.trace --skip 28 --video
ok: 272 frames (from 28, state not compared)
```

Audio now uses the same architecture as the reference: the non-linear mixer's
per-CPU-cycle value goes through a 256-phase, 32-tap Blackman-windowed-sinc
BLEP decimator to 44.1 kHz, then the NES front-loader's 90 Hz/440 Hz
high-pass and 14 kHz low-pass sections. The trace runs at 44.1 kHz and the
first `run_frame` reports the reset's power-on frame without executing, so
frame indices line up with the reference capture.

The per-CPU-cycle channel outputs and the mixed value are now bit-identical
on the frames compared (the pulse sequencers run at the APU rate, the
triangle DAC holds its step, and the console power-on phase/warm-up window
match). The system also leaves reset on CPU cycle 8 like the reference, which
keeps every frame start within one cycle of the reference capture. Audio
sample arrays are bit-identical for the first 38 frames of a commercial title
(28,428 of 85,818 samples over 120 frames); divergence starts when the
remaining one-cycle frame-boundary difference moves the drain point. On the
NROM title, 226 of 300 frame audio hashes match and 299 of 300 video hashes
match; frame-boundary alignment is within ±1 cycle (previously ±5).
- `run <rom> --dump-ppu` / `--break <hex>` — per-frame CPU/PPU/APU state and a
  PC breakpoint.
- `run <rom> --dump-chr PATH` — writes `PATH.chr` (the active 8 KiB CHR
  window, including shared mapper CHR), `PATH.oam`, `PATH.vram` and `PATH.ram`
  after the last frame. Useful for comparing a port against the oracle.

Manifests and traces are local artifacts (they derive from game content) and
are never committed.

## Second-reference capture hosts

Both references are patched to emit the same trace contract as the RustyNES
oracle (`# unko trace v1`, per-frame FNV video/audio hashes), so either can be
used as `UNKO_ORACLE`:

```
refs/refcap/mesen-refcap <rom> <frames> [raw-prefix]
refs/refcap/ares-refcap  <rom> <frames> [--system NAME]
```

- **MesenCE**: `InteropDLL` now exports `InitializeCapture`, the per-frame
  `Capture*` hash/copy functions and `CaptureSetMaximumSpeed`. The library is
  rebuilt with `make core USE_GCC=true STATICLINK=false`; the host seeds the
  default NES palette (`NesDefaultVideoFilter::GetDefaultPalette`) because a
  GUI config normally supplies it. Output is 256x240 RGBA8 plus mono f32
  audio at the configured rate; optional `raw-prefix` writes
  `<prefix>.frame.rgba` and `<prefix>.audio.f32`.
- **ares**: `desktop-ui` accepts `--frames N`, `--dump-trace PATH`,
  `--dump-frame PATH` and `--dump-audio PATH`; the wrapper runs it under
  `cage` with `WLR_BACKENDS=headless`/`pixman` and crops the Famicom
  283x242 canvas to the 256x240 viewport.

The reference palettes and audio mixers differ from RustyNES, so their hashes
are not byte-identical to the primary oracle. Use them for board behaviour,
boot/render checks, frame counts and the oracle-blind mappers, not as a
replacement for the RustyNES parity gate. The hosts live outside the
repository with the reference trees and take ROM paths only at runtime.

## Build status summary

| Emulator | Status | Binary |
|---|---|---|
| ares | built | `refs/ares/build_headless/desktop-ui/ares` |
| Dolphin | built | `build_headless/Binaries/dolphin-emu` |
| gopher64 | built | `refs/gopher64/target/release/gopher64` |
| MesenCE | built | `refs/mesence/bin/linux-x64/Release/linux-x64/publish/Mesen` |
| RustyNES | built | `refs/rustynes/target/release/` |

## ROM metadata and provenance

The repository contains no references to third-party ROM catalog providers and
distributes no ROM metadata. A local mapper audit may use catalog `.dat` files
and a licensed ROM library; those live outside the repository, are never
committed, and are not part of any build, test or published artifact. The
audit output recorded in `docs/accuracy-status.md` describes results only
(mapper IDs, counts), not the external paths.
