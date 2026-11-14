# Provenance — `unko-nes`

This crate is **MIT-licensed original code with documented ISC-licensed
adaptations from ares**. It is *not* a strict clean-room implementation.
The licence rules are in `docs/accuracy.md`; this file records the sources
actually used so that the attribution stays accurate.

## Sources used

- NESdev Wiki: CPU, PPU, APU, mapper and FDS pages.
- MOS 6502 / Ricoh 2A03 public datasheets and instruction references.
- Public test ROM behaviour (nestest, blargg suites, AccuracyCoin, ppu_vbl_nmi,
  MMC3 tests) as black-box oracles.
- **ares source (ISC)** for sprite evaluation, sprite fetch/latch and OAM data
  behaviour (`ares/fc/ppu/sprite.cpp`, `render.cpp`, `memory.cpp`), and as a
  reference for DMA/DMC scheduling. The adaptation is covered by the ares ISC
  notice in the repository `NOTICE` and `licenses/ares-LICENSE.txt`.
- Our own experiments against reference emulator binaries (observable output
  only).

## Sources deliberately not used or included

- MesenCE source (GPL-3.0)
- RustyNES source (GPL-3.0)
- Dolphin source (GPL-2.0-or-later)
- gopher64 source (GPL-3.0)

No GPL source, table, comment or test asset from those projects is copied,
translated, vendored or linked into this crate. They may only be used as
black-box behaviour oracles or as a separate process outside the distributed
application.

## Affected areas

The ares-derived work is concentrated in:

- `src/ppu.rs` — sprite evaluation counters, OAM data latch, sprite fetch and
  secondary-OAM-driven rendering.
- `src/system.rs` — DMA/DMC scheduling and bus interaction used while
  calibrating against the reference behaviour.
- `src/apu.rs` — DMC/frame-counter timing adjusted against the same reference.

These files remain original Rust implementations, but they are adaptations of
the ISC-licensed algorithms and must retain the ares attribution.

## Verification

The differential harness (`unko trace` / `unko diff`) compares this core
against reference binaries. It records only observable outputs (frames, audio,
state hashes, test results), never source.
