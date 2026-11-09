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
      (length/IRQ/jitter/reset/halt/reload, DMC basics/rates). The nonlinear
      mixer curves are implemented and unit-tested. DMC DMA cycle stealing is
      validated by a synthetic reference benchmark: over 300 frames the same
      10,126 loop iterations are stolen with and without DMC (the only
      difference is a two-iteration start-up offset, present with DMC off too).
      The abort/interleave edge cases remain.
- [ ] **A4** top 30 mapper families + commercial oracle corpus. Progress:
      NROM/MMC1/UxROM/CNROM/MMC3(+IRQ)/AxROM/VRC(+IRQ)/Color Dreams/CPROM/
      GxROM/Camerica/UNROM-512/NINA/DxROM/Nanjing-163/Sunsoft-FME7/Sachen/
      Jaleco-SS88006/Namco-163/Namco-175-340/Action-53/J.Y.-ASIC/Waixing-FS305/
      Bandai-FCG/TXC/CoolBoy-MMC3-8025/MMC3-215/GTROM/Cony-83/RAMBO-1/VRC6/
      Taito-TC0190-TC0690/X1-005/Irem-G-101/JF-13/multicart latches/partial
      MMC5, MMC2/MMC4 PPU-latched CHR, Sunsoft-4 (including CHR-ROM
      nametables), the MMC3-8025/FS005 protection path (176), a corrected
      VRC2/VRC4 core (21/22/23/25), Sachen JV001 (136), Bandai LZ93D50 (159),
      MMC3 512 KiB CHR wiring (197), Waixing FS303 (195) with its `$5000`
      PRG-RAM, the MMC2/MMC3 hybrid (165), Super Mega JV001 (172), NTDEC
      TC-112 (193), full PPU mirroring modes, the 143
      protected NROM and the 212/235/286 multicarts, `System` bus, Core ABI
      adapter and a `corpus` runner. Full-library audit: 6,627/7,101 load,
      3,554 rendering, 446 unsupported, 0 panics; 136/193/197 render 7/7,
      159/165 8/8, 172 4/6, 195 8/8 (Tiexue needs Start on its title screen)
      (`docs/mapper-audit.md`).
      Remaining: FDS expansion audio (the FDS mapper/BIOS is still missing),
      the MMC5 >256 KiB CHR path, the Datach peripheral and the small six-image
      ASIC tail (triage in `docs/mapper-audit.md`). VRC6 expansion audio (two
      pulses + sawtooth), Sunsoft 5B / YM2149 (three tones + noise + envelope)
      Namco 163 (up to eight wave channels, shared $40-$7F sound RAM), MMC5
      (two pulses + PCM DAC) and the VRC7/YM2413 two-operator FM engine are
      implemented with unit tests (the built-in VRC7 patch table is an
      approximation; the custom patch registers are exact).
- [x] **A5** AccuracyCoin gate and divergence ledger. Isolated runner exercises
      all 151 rows: **146 pass, 0 fail, 5 DRAW skips, 0 unrun** — every
      non-DRAW test passes. `$2007 Stress Test` was closed with the PPU data
      state machine and per-dot fetch-buffer feed (170/170 answer-key bytes,
      confirmed against both RustyNES and MesenCE). The ledger
      (`docs/accuracy-divergences.md`) has no open failures and the dated
      measured log is `docs/accuracy-status.md`.
- [ ] **A6** FDS, VS, PAL/Dendy, expansion audio, peripherals, NSF.
      Progress: the accuracy core supports a per-port device type and the
      **NES Zapper** (`$4016`/`$4017` bit 4 trigger, bit 3 light, active-low;
      the sensor samples bright pixels in its 8x8 area as the beam passes),
      with unit coverage, and the desktop app maps the mouse to port 2 with a
      Controllers-tab toggle. **PAL/Dendy timing** is wired through
      `System::set_region` (312-line PPU, fractional 3.2-dot CPU/PPU ratio,
      NTSC-only odd-frame skip), with unit-test verification. An **NSF/NSFE
      parser and player** builds a synthetic NROM image with a player stub
      (INIT once, PLAY from the NMI) and is covered by header, image-layout and
      init/play unit tests. Remaining: FDS/VS, expansion audio, gamepad and the
      peripheral rebinding UI, and validation against real PAL/NSF content.
- [ ] **A7** differential harness, hardware captures, performance pass.
      Performance pass started: headless accuracy core ~535 -> ~640 fps
      (max-of-7, 2400 frames) via the sprite-list pixel loop, table-driven
      mixer, palette RGB cache and the 8-dot background span; profile shares
      logged in `docs/accuracy-status.md`. Single-ROM differential traces
      (`multiemu trace` / `diff`) and directory-level frame signatures
      (`multiemu signatures --check`) exist. Hardware captures still
      outstanding.

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
- [x] Native file dialog (`rfd`) for the ROM/save folders, integer scaling and
      a nearest/linear texture-filter option; the composite/NTSC display filter
      is offered through the core (`VideoFilter::Composite`).
- [ ] Gamepad support (gilrs/SDL), rebinding UI, hot-plug, four players.
- [ ] Audio device via `cpal`; ring buffer; sample-rate resampling; underrun policy.
- [x] SRAM persistence keyed by ROM content hash; save-state slots with
      thumbnails.
  - [x] Core exposes `sram()` / `load_sram`; the app writes `saves/<stem>.sav`.
- [ ] Frame advance, fast-forward and rewind ring in the host.
- [x] CLI regression mode: hash frames/audio, fresh-run vs state-resume, assert
      byte-identical tails (`multiemu verify`).
- [x] Test-ROM harness: `nestest`, blargg CPU/PPU/APU, `ppu_vbl_nmi`, MMC3 A12,
      and the AccuracyCoin isolated battery. Pass counts are recorded in
      `docs/accuracy-status.md`; the suites run in `crates/multiemu-nes-accuracy/tests/`.
- [x] Mapper audit against a licensed ROM library: 7,101 images in four
      batches, results and ranked gaps in `docs/mapper-audit.md`. 6,565 load,
      504 unsupported (191 IDs), 32 failures; MMC2/MMC4 followed from it.

## M2 — NES depth

- [x] Zapper (core sensor + app mouse aiming/trigger).
- [ ] Other input devices (gamepad, four-score, family BASIC keyboard).
- [ ] FDS (BIOS supplied by the user, never bundled), Vs. System.
- [x] NSF/NSFE parsing and player wiring (unit-tested; corpus validation
      pending).
- [ ] Expansion audio levels and per-channel mixing.
- [ ] NTSC/composite filter pass on the GPU, palette selection.
- [ ] Debugger: memory viewer and breakpoint UI. Headless pieces exist:
      `multiemu run --dump-ppu` prints per-frame CPU/PPU/APU state and
      `--break <hex>` stops on a PC.
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
