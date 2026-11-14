# Accuracy charter — NES

Decision: **reference parity, Rust, MIT with documented ISC adaptations**, no
deadline. `unko-nes` is the only NES core; TetaNES was retired after
the accuracy core met the mapper and save-state bar.

## 1. What "accurate" means

For every ROM in the corpus, observable behaviour must match the chosen
reference:

- framebuffer, per frame (bit-identical where practical)
- audio, per frame (sample-exact where practical)
- CPU/PPU/APU cycle counts and bus traces
- save-state evolution and rewind determinism

Public suites must be 100%:

| Suite | Bar |
|---|---|
| `nestest` | 0 diff |
| blargg `cpu_interrupts_v2` | 5/5 |
| blargg `instr_timing` / `instr_misc` | all |
| `ppu_vbl_nmi` | 10/10 |
| `ppu_sprites` | 19/19 |
| MMC3 A12 IRQ tests | all (currently 4/6; see `docs/accuracy-divergences.md`) |
| `apu_test`, `blargg_apu_2005` | all |
| AccuracyCoin | 146/146 non-DRAW rows (currently 146/146) |
| Commercial oracle corpus | byte-identical frames/audio |

Current results (2026-11-13): every public row above is green — `nestest`
8,980/8,980 lines, `instr_timing`/`instr_basics`, `ppu_vbl_nmi` 12/12,
`ppu_sprites` 19/19, blargg APU/DMC 13/13, and AccuracyCoin 146/146 non-DRAW
(146/151 total, 5 DRAW skips, 0 failed, 199,320 frames). The 7,101-image
commercial audit loads 7,073 (99.6%) with 0 unsupported mapper IDs and 28
truncated dumps; 6,490 images render by 120 frames. Details in
`docs/accuracy-status.md` and `docs/mapper-audit.md`.

"Reference parity" is stronger than "runs the library": a game that looks right
but has the wrong cycle count is a failure.

## 2. Oracle hierarchy

When references disagree, decide in this order:

1. **Real hardware capture** (logic analyzer / console) — authoritative.
2. **ares** — low-level, ISC, closest silicon model.
3. **MesenCE** — broadest compatibility and quirk coverage.
4. **RustyNES** — best public test evidence and ledger.

Every divergence gets a dated entry in `docs/accuracy-divergences.md` with the
ROM/test, the observed values, and the chosen rule. The current open list and
the intentional approximations live in that file.

## 3. Source rules (hard)

- Implementers of this core **do not read GPL emulator source**. That includes
  MesenCE, RustyNES and Dolphin.
- ares is ISC and may be read or adapted with attribution. The adapted areas
  are recorded in `crates/unko-nes/PROVENANCE.md`; the ares ISC
  notice is in `licenses/ares-LICENSE.txt` and `NOTICE`.
- Allowed inputs: NESdev wiki, chip datasheets, public hardware notes, test ROM
  behaviour, our own hardware captures, black-box runs of reference binaries,
  and ISC-licensed ares source.
- Forbidden: copying, translating, or mechanically extracting code, tables or
  comments from GPL reference emulators; generating tables from their binaries.
- `refs/` contains GPL clones for analysis of the *shell* and for running
  oracles. The accuracy core must not import, link or read them. Keep the
  boundary mechanical (separate crate, no path deps, provenance check in CI).
- Every module gets a provenance note: which public sources it was written from.

This matters because the project is MIT. A single translated GPL function makes
the whole distributed work GPL.

## 4. Architecture

```
master clock (u64, 6× colourburst)
  CPU start_cycle/end_cycle  ──►  bus access at φ2
  PPU run_to(master - offset)     one dot per iteration
  APU tick (every other CPU cycle)
  DMA engine (per-cycle, unified)
```

Rules:

1. **One canonical clock.** No component keeps an independent cycle counter.
2. **Every CPU cycle is a real bus access.** No coalesced instructions.
3. **The bus owns everything mutable.** CPU borrows the bus; PPU/APU/mappers do
   not own each other.
4. **PPU is dot-level**, with separate 2C02 / 2C07 / UA6538 raster data and
   revision-gated quirks.
5. **Mapper IRQs belong to the mapper**, fed by A12/scanline/CPU-cycle hooks.
6. **Determinism is a contract.** Same ROM + inputs + options ⇒ identical
   output, always.
7. **Save states are versioned and ROM-bound.**

## 5. Scope for v1

Everything selected: licensed library, FDS, VS System, PAL/Dendy, expansion
audio, peripherals, unlicensed mappers, NSF playback.

Order inside the core:

1. NTSC 2C02 + 2A03 + NROM/UxROM/CNROM/MMC1/MMC3.
2. Top 30 mapper families, then the long tail.
3. PAL/Dendy raster models.
4. Expansion audio.
5. FDS, VS, NSF.
6. Peripherals.

## 6. Milestones

### A0 — harness and skeleton (done)
- [x] Accuracy crate (`unko-nes`), now the only NES core.
- [x] Region timing table, CPU bus contract, CPU registers/reset.
- [x] Deterministic frame/audio trace format and `unko trace` / `diff`.
- [x] Provenance rules and crate-level `PROVENANCE.md`.
- [x] Exit: harness runs a ROM and hashes output; two runs diff clean.

Commands:

```sh
unko trace game.nes --core nes_accuracy_core --frames 600 --out ref.trace
unko trace game.nes --core <accuracy-id> --frames 600 --out new.trace
unko diff ref.trace new.trace
```

### A1 — CPU (done)
- [x] Cycle-stepped 6502/2A03, all official and unofficial opcodes.
- [x] `nestest` 0 diff (8980/8980 trace lines, including CYC).
- [x] blargg `instr_timing` and `instr_basics` pass.
- [x] Unit tests for reset, addressing, branches and stack.
- [x] Interrupt edge/branch-delay semantics and DMA: `cpu_interrupts_v2` 5/5
      and `dmc_dma_during_read4` pass.

See `docs/accuracy-status.md` for the measured log.

### A2 — PPU (done)
- Dot-level `Ppu`: registers, VRAM, palette, OAM, vblank/NMI, suppression,
  odd-frame skip, CHR-RAM, background rendering.
- [x] **12 / 12 `vbl_nmi` ROMs pass**, including `nmi_suppression` and
      `nmi_timing`, after implementing penultimate-cycle NMI polling.
- [x] First sprite pass: OAM evaluation, sprite-zero hit, overflow flag,
      sprite rendering, OAM DMA.
- [x] **19 / 19 `ppu_sprites` ROMs pass**, including all sprite-zero, overflow,
      OAM read/stress and `sprite_ram`.

### A3 — APU (done)
- Pulse/triangle/noise/DMC, frame counter, length/sweep/envelope, mixing, DMC
  DMA.
- Exit: APU suites green; audio hash matches the reference.

### A4 — mappers
- Board classes, A12 filter, IRQ timing, NES 2.0, battery.
- Exit: top 30 families and commercial oracle corpus boot.

### A5 — accuracy gate
- Exit: AccuracyCoin 146/146 non-DRAW rows and a documented divergence ledger.
- Progress: 146/146 non-DRAW rows pass as of 2026-11-12; see
  `docs/accuracy-status.md` and `docs/accuracy-divergences.md`.

### A6 — breadth
- FDS, VS, PAL/Dendy, expansion audio, peripherals, NSF.

### A7 — parity and tooling
- Differential harness against ares/Mesen/RustyNES binaries.
- Hardware capture comparisons.
- Optional debugger.

## 7. Performance posture

Accuracy first. Optimisation happens only after the accuracy gate, and only
when a benchmark proves a regression; there are no accuracy shortcuts,
frame-skip or approximation modes.

The core is single-threaded and deterministic. Its efficiency is measured as
CPU time per emulated frame plus idle shell CPU, with budgets recorded in
`docs/performance.md` and enforced in CI:

| Budget | Value |
|---|---|
| CI throughput, synthetic rendering NROM, shared runner | >= 300 fps (<= 3.33 ms/frame) |
| Local, worst case with every physical core contended (synthetic) | >= 300 fps |
| Local, worst case with every physical core contended (MMC2 title) | >= 250 fps (<= 4 ms/frame) |
| Idle shell | 4 UI repaints/s, no core work while paused or unfocused |

Measured on the development machine: 813-819 fps (1.22 ms/frame) for the
synthetic NROM, 375-416 fps (2.4-2.7 ms/frame) for the MMC2 title, and
1029-1040 fps with PGO. Dot-lockstep costs roughly 6-8% over a coarser
scheduler in published comparisons; that is accepted.

## 8. Known risks

- Clean-room parity is a multi-month effort even for NES.
- Hardware captures may be unavailable for late-discovered quirks.
- The unlicensed mapper long tail is effectively unbounded; define a mapper
  manifest and accept "best effort with save-state round-trip".
- FDS requires a user-supplied BIOS; never bundle it.
- The existing GPL clones under `refs/` are a provenance hazard; keep them out
  of the accuracy crate's build and review path.
