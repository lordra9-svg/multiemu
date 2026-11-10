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

`multiemu` carries the scriptable checks used for the audit and for change
gating:

- `corpus <dir> [--frames N] [--mapper ID] [--skip N] [--limit N]` — smoke
  audit; reports loaded/rendering/blank/unsupported/failed, ranks unsupported
  mapper IDs and prints failing titles.
- `signatures <dir> [--frames N] [--mapper ID] [--check FILE]` — hashes every
  frame of every image and compares against a local manifest, reporting
  changed/new/missing titles.
- `trace <rom> --frames N --out trace.txt [--no-state]` and
  `diff a.trace b.trace [--skip N] [--video]` — per-frame video/audio hashes
  and the first divergent frame for one image. `--no-state` omits the
  save-state hash so a trace can be compared with a reference capture that has
  no state; `--skip N` ignores start-up frames and `--video` compares video
  only while the audio mixers are still being aligned.
- `parity a.trace b.trace [--window N]` — quantitative parity: same-index
  video/audio match counts, the first divergence of each, and the best
  alignment over ±N frames. This distinguishes a real rendering/audio defect
  from the known one-cycle frame-origin drift.
- `trace <disk.fds> --bios disksys.rom ... [--press-start N]` — traces a raw
  FDS disk image with a user-supplied BIOS. `--press-start N` holds Start for
  12 frames from frame `N` so a licence screen can be dismissed deterministically.
  The BIOS is never bundled or committed.

### Differential oracle

`trace` emits the `# multiemu trace v1` format: one `frame <index> <video>
<audio>` line per frame using FNV-1a-64 over the RGBA8 framebuffer and over the
`f32` audio samples. A reference emulator only needs to emit the same records;
`diff` compares them frame by frame and reports the first divergent video or
audio hash.

The RustyNES adapter used for validation is a local, uncommitted black-box
tool (the GPL reference checkout never enters the repository): it loads the
ROM with `Nes::from_rom`, calls `run_frame`, and hashes the returned RGBA8
framebuffer and `drain_audio()` samples with the same FNV-1a constants.

Current local result with the canonical 2C02 palette:

| Target | Identical frames |
|---|---|
| AccuracyCoin (60 frames) | 49/60 |
| Commercial NROM title (300 frames) | 297/300 |
| *Super Mario Bros.* NROM (600 frames) | video 599/600, audio 504/600 |
| *Super Mario Bros. 3* MMC3 (300 frames) | video 70/300, audio 228/300 (best audio shift −4: 259/300) |

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
multiemu diff video.trace reference.trace --skip 28 --video
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

Manifests and traces are local artifacts (they derive from game content) and
are never committed.

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
