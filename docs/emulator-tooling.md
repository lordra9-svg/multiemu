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
- `trace <rom> --frames N --out trace.txt` and `diff a.trace b.trace` —
  per-frame video/audio hashes and the first divergent frame for one image.
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
