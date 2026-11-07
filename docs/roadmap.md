# Roadmap

## Accuracy track (parallel, NES)

The main effort is now the reference-parity accuracy core. Details in
`docs/accuracy.md`.

- [x] **A0** accuracy crate, deterministic frame/audio/state hashing, test-ROM
      runner, provenance firewall.
- [x] **A1** cycle-stepped 6502: `nestest` 0 diff, `instr_timing` and
      `instr_basics` pass.
- [x] **A2a** `vbl_nmi` 12/12; penultimate-cycle NMI polling.
- [x] **A2b** sprite suite 19/19: all `spr_hit_*`, all overflow ROMs,
      `oam_read`, `oam_stress` and `sprite_ram` pass.
- [x] **A3** APU + DMC: all 13 blargg APU/DMC ROMs in the suite pass
      (length/IRQ/jitter/reset/halt/reload, DMC basics/rates). Mixer scoring and
      DMC DMA cycle-stealing remain as follow-ups.
- [ ] **A4** top 30 mapper families + commercial oracle corpus. Progress:
      NROM/MMC1/UxROM/CNROM/MMC3(+IRQ)/AxROM/VRC(+IRQ)/Color Dreams/CPROM/
      GxROM/Camerica/UNROM-512/NINA/DxROM/Nanjing-163/Sunsoft-FME7/Sachen/
      Jaleco-SS88006/Namco-163/Namco-175-340/Action-53/J.Y.-ASIC/Waixing-FS305/
      Bandai-FCG/TXC/CoolBoy-MMC3-8025/MMC3-215/GTROM/Cony-83/RAMBO-1/VRC6/
      Taito-TC0190-TC0690/X1-005/Irem-G-101/JF-13/multicart latches/partial
      MMC5, the 143 protected NROM and the 212/235/286 multicarts, `System` bus,
      Core ABI adapter and a `corpus` runner.
      594/908 loaded ROMs render in a 1000-ROM sample.
      Remaining: full MMC5 ExRAM/audio, VRC/VRC6/N163 expansion audio, MMC2/MMC4
      PPU-latched CHR, ROM-nametable mappers, and the remaining small ASICs.
- [ ] **A5** AccuracyCoin gate and divergence ledger. Isolated runner exercises
      all 151 rows: **138 pass, 8 fail, 5 DRAW skips, 0 unrun** (146 non-DRAW).
      All A1–A3 public suites remain green. Remaining clusters: DMA/abort timing
      (`DMA + $2002 Read`, `DMC DMA + OAM DMA`, `Explicit DMA Abort`,
      `DMA Landing on Write`), NMI/BRK hijack, the `$2007` read buffer, and the
      scanline-0 stale-sprite path. The open list is
      `docs/accuracy-divergences.md`; the dated measured log is
      `docs/accuracy-status.md`.
- [ ] **A6** FDS, VS, PAL/Dendy, expansion audio, peripherals, NSF.
- [ ] **A7** differential harness, hardware captures, performance pass.

## M0 — core boundary (done)

- [x] Cargo workspace, MIT license, third-party notices.
- [x] `multiemu-core`: `Core`, `CoreInfo`, `Registry`, video/audio/input/state types.
- [x] `multiemu-nes`: TetaNES adapter (load, frames, RGBA video, f32 audio, input,
      save state, SRAM).
- [x] `multiemu-cli`: `info`, `run` (PPM screenshots), `bench`.
- [x] Synthetic NROM boot test; state round-trip test; registry tests.
- [x] Verified locally: 256×240 RGBA, ~937 fps / 15.6× realtime in release on a
      test ROM.

## M1 — NES product slice

- [x] Desktop shell with Dolphin-inspired UX (`multiemu-app`): menu bar, toolbar,
      game library (list + grid), search, player viewport, status bar, settings
      window with category rail, about dialog.
- [x] Library reads loose `.nes` and `.zip`/`.7z`/`.rar` members without
      extracting; default folder `roms/`.
- [x] Keyboard input (arrows, X/Z, Enter, Shift), pause/reset/stop, save/load
      state, screenshot, fullscreen, deterministic `verify`.
- [ ] GPU scaling/shader pass and a native file dialog (the current shell uses an
      `egui` texture and a path entry field).
- [ ] Gamepad support (gilrs/SDL), rebinding UI, hot-plug, four players.
- [ ] Audio device via `cpal`; ring buffer; sample-rate resampling; underrun policy.
- [ ] SRAM persistence keyed by ROM hash; save-state slots with thumbnails.
  - [x] Core exposes `sram()` / `load_sram`; the app writes `saves/<stem>.sav`.
- [ ] Frame advance, fast-forward and rewind ring in the host.
- [x] CLI regression mode: hash frames/audio, fresh-run vs state-resume, assert
      byte-identical tails (`multiemu verify`).
- [ ] Test-ROM harness: `nestest`, blargg CPU/PPU/APU, `ppu_vbl_nmi`, MMC3 A12,
      and any freely licensed suites. Record pass counts in `docs/accuracy-status.md`.
- [ ] Mapper audit against a licensed ROM library: log unsupported mapper IDs and
      boot failures; rank the gaps by prevalence.

## M2 — NES depth

- [ ] Zapper and other input devices.
- [ ] FDS (BIOS supplied by the user, never bundled), Vs. System, NSF/NSFE.
- [ ] Expansion audio levels and per-channel mixing.
- [ ] NTSC/composite filter pass on the GPU, palette selection.
- [ ] Debugger: CPU/PPU state, memory viewer, breakpoints, trace log.
- [ ] Game Genie / cheat engine.

## M3 — prove "multi"

- [ ] A second NES core behind the same ABI (pick by measured compatibility or
      performance gap), selected in settings.
- [ ] A second system behind the same ABI, starting with **SNES from ares**
      (ISC) with MesenCE as the GPL oracle, then **N64 from ares** with
      gopher64 as a GPL structural reference. See `docs/systems.md`.
- [ ] Only then consider a C ABI + dynamically loaded core plugins.
- [ ] GameCube/Wii is deferred: Dolphin is GPL-2.0+ and application-shaped, so
      it can only be a separate optional process, never a registry core.

## M4 — distribution

- [ ] Desktop packages (Linux AppImage/Flatpak, Windows, macOS).
- [ ] Web build (`wasm32`) if the frontend stack allows it.
- [ ] Optional mobile shells.
- [ ] Reproducible builds, checksums, license audit in CI.

## Accuracy policy

The NES backend is currently classed `Accuracy::High`, not `CycleAccurate`,
because TetaNES is not dot-locked to the same degree as ares/Mesen/RustyNES.
Do not raise the badge without a passing page of the test matrix and a
documented list of known approximations. `docs/accuracy-status.md` and
`docs/accuracy-divergences.md` hold that record.
