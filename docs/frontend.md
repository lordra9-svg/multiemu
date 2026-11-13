# Frontend and GUI/UX blueprint

The frontend follows **Dolphin's desktop UX**, not its code or assets. Dolphin is
GPL-2.0-or-later; this project is MIT. We take the *information architecture*
and interaction model — main window, game library, toolbar, settings categories,
per-game properties — and implement it originally in Rust with `egui`/`eframe`.

Nothing is copied from `refs/dolphin`: no source, no icons, no theme files, no
translated strings. Menu names below are functional labels, written for this app.

## Dolphin's structure (what we borrow)

```
MainWindow
├── MenuBar        File · Emulation · View · Movie · Tools · Help
├── ToolBar        Open · Refresh · Play/Pause · Stop · Fullscreen ·
│                  Screenshot · Config · Graphics · Controllers
├── central stack
│   ├── GameList   list view (table) or grid view (covers)
│   │              search bar, platform/region filters, columns, double-click to play
│   └── RenderWidget  emulation surface, fullscreen-capable
├── dock widgets   logs, registers, code, ...

SettingsWindow
├── General · Graphics · Controllers · Interface · On-Screen Display ·
│   Audio · Paths · GameCube · Wii · Advanced
└── per-game properties (separate dialog)

Game list columns
Platform · Banner · Title · Description · Maker · ID · Country · Size ·
FileName · FilePath · FileFormat · BlockSize · Compression · TimePlayed · Tags
```

Key interaction rules worth keeping:

1. **The library is the home screen.** Launching is double-click / Enter, not a
   blocking file dialog.
2. **One toolbar for the verbs that matter**: open, refresh, play/pause, stop,
   fullscreen, screenshot, settings.
3. **Settings are a single window with a category rail**, not a pile of modal
   dialogs. Categories are stable and discoverable.
4. **Per-game properties are separate from global settings** (Dolphin's
   Properties dialog), so a ROM can override region/filter/etc.
5. **Status is always visible**: current game, core, FPS, resolution, slot.
6. **Search is a mode of the library**, not a separate page.
7. **Fullscreen hides chrome and keeps hotkeys.**

## Unko mapping

| Dolphin | Unko (NES first) | Status |
|---|---|---|
| GameList (table/grid) | Library table + grid toggle | now |
| Search bar | Library search field | now |
| Platform column | System (`nes`) | now |
| Title / FileName / FilePath | Title, file name, path | now |
| Archive members | `.zip` / `.7z` / `.rar` entries read in place | now |
| Size / Country / ID | Size, region guess, mapper, CRC32 | now |
| TimePlayed | play-time counter (per game, persisted) | later |
| Banner / Description / Maker | not applicable yet | — |
| MenuBar: File | Open ROM, ROM folder, Rescan, Exit | now |
| MenuBar: Emulation | Play/Pause, Stop, Reset, States, Screenshot, Fullscreen | now |
| MenuBar: View | List/Grid, status bar, fullscreen | now |
| MenuBar: Tools | Settings, Controllers, Graphics | now |
| MenuBar: Help | About | now |
| ToolBar | Open, Refresh, Play/Pause, Stop, Reset, Fullscreen, Shot, Config | now |
| RenderWidget | `egui` texture viewport, aspect-correct | now |
| Settings: General/Interface | theme, pause on focus loss, confirm stop, detachable windows, reset-all | now |
| Settings: Graphics | filter, palette + `.pal` import, aspect, integer scale, supersampling, output shader, window presets | now |
| Settings: Audio | sample rate, latency target, resampler quality, output filter, volume/mute, per-channel mixer | now |
| Settings: Controllers | fixed keyboard map, rebinding UI | partial |
| Settings: Paths | ROM dir, save dir, per-system library roots | now |
| Settings: NES page | region, mapper/header info, deterministic RAM | now |
| Settings: Advanced | core id, log level, state format | now |
| Per-game properties | header/mapper/region overrides | later |
| NetPlay / Achievements / Debugger | separate milestones | later |

## Platforms

The shell is one egui/eframe codebase with platform backends behind it:

| Target | Input | Audio | Paths |
|---|---|---|---|
| Linux | SDL3 | cpal/ALSA or PulseAudio | portable `unko.conf` or platform dirs |
| Windows | SDL3 | cpal/WASAPI | portable `unko.conf` or platform dirs |
| macOS | SDL3 | cpal/CoreAudio | portable `unko.conf` or platform dirs |
| iOS | SDL3 + touch overlay | cpal/CoreAudio | `ProjectDirs` config/data |
| Android | SDL3 + touch overlay | cpal/AAudio | `ProjectDirs` config/data |

SDL3 is built from source by Cargo for the desktop targets. The mobile shells
use the touch overlay as their primary controller and a no-op gamepad backend,
because SDL3 needs the platform SDKs; a native `GameController`/`Gamepad`
backend can replace it without touching the UI. CI checks the full workspace
on Linux, Windows and macOS, and both the NES core and the complete app shell
for the Android and iOS Rust targets. The packaging scaffolds live in
`crates/unko-app/android` (Gradle + `cargo-ndk`, `NativeActivity`) and
`crates/unko-app/ios` (XcodeGen + static library); see `docs/platforms.md`.
Signed/notarised release builds and device validation still need those
platform toolchains.

## What is deliberately different

- **No Qt, no C++ UI.** `egui` keeps the shell in the same workspace and builds
  on Linux/Windows/macOS/wasm. If the app later needs native menus and a
  document-based macOS feel, the `Core` boundary does not change.
- **No ROM database dependency.** Titles come from file names; metadata comes
  from the iNES/NES 2.0 header. A game database can be added behind the same
  library model.
- **Native file dialogs are optional.** `rfd` is used for ROM/save folders and
  palette import on desktop; text entries remain for platforms without a
  dialog backend, so the UI works in a headless/CI environment.

## Implementation notes

- `egui`/`eframe` 0.31, pinned.
- Video: the core's RGBA frame is uploaded to a texture each frame. The
  viewport offers Fit, 4:3 and 8:7 aspect modes, an automatic or fixed 1-6x
  integer scale, and 1-3x output supersampling (the frame is upscaled before
  the linear downscale to the window; the PPU stays 256x240). Output shader
  presets (scanlines, LCD grid, CRT vignette) are painted over the image, on
  top of the core's Raw/Composite/CRT filters.
- Windows: Settings, Devices and the Debugger open as native OS windows on
  desktop (each remembers its own size/position); mobile embeds them as
  panels. The General page has a confirmed "Reset all settings" that restores
  the config, layout, hotkeys, bindings, devices and per-game profiles and
  removes state thumbnails, while never touching save states, SRAM or
  recordings.
- Pacing: a `Duration`-based accumulator targets 60.0988 Hz and runs at most
  four catch-up frames per repaint; vsync is left to the windowing backend.
  The UI sleeps until the next frame deadline (`request_repaint_after`), so an
  idle-but-running console wakes once per frame instead of spinning, and a
  suspended process never replays its stall: any backlog past the four-frame
  catch-up is discarded, keeping latency and audio in real time. Integer
  durations prevent fractional-frame drift, so a long session cannot start
  emitting double frames. Pausing drops the UI cadence to 4 Hz, and an
  unpause/load resets the clock so the first frame is presented immediately.
- Input: keyboard map is arrows + Z/X/A/S + Enter/Shift; SDL3 gamepads are
  hot-plugged, remappable, support analog-stick D-pad input, auto-fire and
  per-game profiles. A gamepad-to-player matrix routes each physical pad to
  P1-P4 (two pads on one player merge), and both the two NES ports and each
  physical pad can be disabled. The Devices window lists the port devices,
  every detected pad, its assignment and connection state. The input display
  and an on-screen touch controller are available for mobile and can be
  previewed on desktop. The library grid takes 1-8 columns and the list view
  shows any combination of system, file, size, region, mapper and modified
  columns.
- Controller-port devices: each port can be a gamepad, Zapper, Arkanoid
  paddle (mouse steering + fire), Power Pad (on-screen 12-pad mat), Family
  BASIC keyboard (host keys map to the documented 9x8 matrix), a Four Score
  adapter (four players across both ports), the Famicom microphone (`$4016`
  D2, held from the Devices window) or the Family BASIC data recorder
  (cassette bit loopback on `$4016`). Settings -> Controllers selects the
  device per port and applies it immediately.
- Audio: cpal output with a lock-free ring buffer, device-rate resampling,
  volume/mute and underrun statistics. The stream is opened once at startup
  and fed one mono sample per emulated frame. The Audio page selects the
  target buffer latency (15-250 ms, default 60), the resampler quality
  (Fast/Standard/High: 0.42/0.46/0.49 cutoff with Hann/Blackman/
  Blackman-Harris windows), and the output filter model (NES RF, Crisp,
  Warm, Linear, or a custom high-pass/low-pass pair). The per-channel mixer
  has a gain slider and a mute switch for pulse 1/2, triangle, noise, DMC and
  expansion audio; all values are host preferences, never part of a save
  state. The page also reports the measured queue depth, underruns and
  dropped samples.
- Debugger: a separate window with CPU registers/flags and a live
  disassembly, PPU timing, palette and OAM views, APU channel meters, a
  hex viewer/editor for CPU/PPU/OAM memory and PC breakpoints. Stepping works
  while paused (F10 = one instruction, F9 = one frame); with breakpoints set,
  the player executes one instruction at a time and pauses before the
  breakpoint instruction. Open it from `Emulation -> Debugger` or with
  `--debug`.
- Video: Raw, Composite/NTSC and CRT (composite + scanlines + bloom)
  filters, plus Canonical 2C02, Consumer TV, RGB monitor, Monochrome, the
  generated Vivid/Soft/PAL-like/Studio models and a Custom `.pal`/`.palette`
  import. The generated models are parameter variations of the ares NTSC
  waveform model (ISC), not hardware captures; Canonical + Raw is the default
  so reference traces stay comparable. See Settings -> Graphics.
- Recording: WAV audio, numbered PNG frames and animated GIF, written under
  `saves/recordings`; all encoders are pure Rust and work on every target.
- State/SRAM: `saves/<rom>.<core>.slotN.state` and `saves/<rom>.sav`; the core
  never touches the filesystem. States carry an `UNKOST05` header, a ROM
  fingerprint and the mapper state, so a state cannot be loaded into a
  different image or core. The former TetaNES state files are ignored.
- Audio playback is active through cpal; the Audio page reports the actual
  device rate, channel count, target and measured latency, underruns and
  dropped samples. The queue is primed on the first frame, trimmed after a
  stall, and an underrun repeats the last sample with a short decay instead
  of inserting a hard zero.
