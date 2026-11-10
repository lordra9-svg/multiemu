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
| Settings: General/Interface | theme, pause on focus loss, confirm stop | now |
| Settings: Graphics | filter (raw/composite), integer scale | now |
| Settings: Audio | sample rate, volume/mute (playback later) | partial |
| Settings: Controllers | fixed keyboard map, rebinding UI | partial |
| Settings: Paths | ROM dir, save dir | now |
| Settings: NES page | region, mapper/header info, deterministic RAM | now |
| Settings: Advanced | core id, log level, state format | now |
| Per-game properties | header/mapper/region overrides | later |
| NetPlay / Achievements / Debugger | separate milestones | later |

## What is deliberately different

- **No Qt, no C++ UI.** `egui` keeps the shell in the same workspace and builds
  on Linux/Windows/macOS/wasm. If the app later needs native menus and a
  document-based macOS feel, the `Core` boundary does not change.
- **No ROM database dependency.** Titles come from file names; metadata comes
  from the iNES/NES 2.0 header. A game database can be added behind the same
  library model.
- **Native file dialogs are deferred.** The environment used for this milestone
  has no GTK/portal development packages, so the app uses a path entry field and
  folder scanning. `rfd` can replace it without touching the rest of the UI.

## Implementation notes

- `egui`/`eframe` 0.31, pinned.
- Video: the core's RGBA frame is uploaded to a nearest-neighbour texture each
  frame; the viewport preserves the NES 8:7 pixel aspect inside a 4:3 box.
- Pacing: an accumulator targets 60.0988 Hz and runs at most four catch-up
  frames per repaint; vsync is left to the windowing backend.
- Input: keyboard map is arrows + Z/X/A/S + Enter/Shift, shown in Controllers.
- State/SRAM: `saves/<rom>.state` and `saves/<rom>.sav`; the core never touches
  the filesystem.
- Audio playback is the next frontend increment; the core already produces f32
  samples and the Audio page carries the settings.
