# Accuracy divergences — NES

This ledger records every known place where `multiemu-nes-accuracy` does not
yet match the chosen reference (real hardware, then ares, then the public test
ROM). It is required by the accuracy charter in `docs/accuracy.md`. Add a dated
entry whenever a divergence is found, and remove it only when a test or a
hardware capture proves the behaviour matches.

Current snapshot: **2026-11-08**, AccuracyCoin v3.1.0, isolated battery
**146/151 passed, 0 failed, 5 skipped, 0 unrun**.

The five skipped rows are the `DRAW` tests, which need a human-drawn reference
image. "ErrorCode N" below is the ROM's `value >> 2` index; the CLI reports
`value >> 1` as `code`.

## Open failures

None. All non-DRAW AccuracyCoin rows pass.

## Recently closed (2026-11-08)

- **`$2007` Stress Test.** The PPU data state machine now loads the read buffer
  four dots after a `$2007` read, from the per-dot background/sprite fetch byte
  (each VRAM read spans two dots). The `v` increment is deferred while
  rendering is enabled so the fetch pipeline is not disturbed, and applied
  immediately under forced blank so DMA dummy reads still advance `v`. The
  dummy tail fetch at dots 337-340 and the first sprite slot's pre-reset
  nametable read are modelled. The raw `$0500-$0654` array matches the test's
  170-byte answer key exactly, cross-checked against both the RustyNES and
  MesenCE reference arrays.

## Intentional approximations

- **DMC `$4015` enable latch.** A re-enable while a byte is shifting is held
  for three CPU cycles and the eventual transfer uses the shorter
  `[halt][put][get]` form. This is calibrated against AccuracyCoin; the exact
  silicon phase for every alignment is not modelled.
- **PPU mask write delay.** `$2001` writes take effect two PPU dots after the
  bus write (chosen to match the test phase), not a full register-latency
  model.
- **Mapper long tail.** Unlicensed mapper behaviour is best-effort with
  save-state round-trip; see the A4 notes in `docs/roadmap.md`.
- **No `Accuracy::CycleAccurate` badge** is exposed until the A5 gate passes.

## Recently closed

- `DMC DMA + OAM DMA`: the CPU-side completion of a fetch that outlives the
  OAM transfer now includes the trailing idle cycle, matching all 32
  answer-key bytes.

- `DMA Landing on Write`: the four-cycle load-DMA halt now also applies
  mid-byte, except inside the timer-rollover implicit-abort window.

- `NMI Overlap BRK` / `NMI Overlap IRQ`: an NMI detected before the status
  push now hijacks the vector (`$FFFA`) while keeping the BRK/IRQ `B` flag in
  the pushed status, and a simultaneous IRQ latch is dropped when the NMI is
  taken. Both answer keys match byte for byte.

- `DMA + $2002 Read`: an idle shift register now gives the load DMA the
  second-following-APU-cycle halt (four cycles after a get-cycle write), so its
  dummy reads hit the `$2002` data-read cycle.

- `Explicit DMA Abort`: the `$4015` stop delay is now 2–3 cycles and a fetch
  that has not halted yet is dropped, giving the ROM's 3-cycle and 1-cycle
  abort durations.
- `DMC DMA + OAM DMA` interleave (partial): halt/dummy cycles overlap the OAM
  transfer, the DMC get takes priority, and an OAM alignment cycle is added.

- `SH*` (`$93`, `$9F`, `$9B`, `$9C`, `$9E`): the `H+1` AND term is dropped when
  a DMA drives the bus on the cycle before the store.
- `Delta Modulation Channel` and `Implicit DMA Abort`: request-level enable
  latch and held-fetch handling.
- `Stale Sprite Shift Regs`: per-slot sprite latch moved to the middle of its
  fetch window, and dot 339 arms only occupied sprite slots.
- `Sprites On Scanline 0`: the pre-render sprite fetch treats the first
  non-empty stale secondary-OAM slot as sprite zero, so a Y=0 sprite is drawn
  from tile row 5 on line 0 when rendering is enabled after dot 66.
- `APU Register Activation`, `$2004 Stress`, `Implied Dummy Reads`,
  `Arbitrary Sprite zero`, `$2002 Flag Timing`, `Controller Strobing`.

## How to add an entry

1. Record the suite, test name, ROM version, emulator build/commit, and date.
2. Record the observed value and the expected value (answer key or capture).
3. State the likely responsible module and any experiment already tried.
4. Link the fixing commit when the entry is removed.
