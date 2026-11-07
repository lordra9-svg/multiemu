# Accuracy divergences — NES

This ledger records every known place where `multiemu-nes-accuracy` does not
yet match the chosen reference (real hardware, then ares, then the public test
ROM). It is required by the accuracy charter in `docs/accuracy.md`. Add a dated
entry whenever a divergence is found, and remove it only when a test or a
hardware capture proves the behaviour matches.

Current snapshot: **2026-11-07**, AccuracyCoin v3.1.0, isolated battery
**138/151 passed, 8 failed, 5 skipped, 0 unrun**.

The five skipped rows are the `DRAW` tests, which need a human-drawn reference
image. "ErrorCode N" below is the ROM's `value >> 2` index; the CLI reports
`value >> 1` as `code`.

## Open failures

| Suite | Test | CLI code | Failing sub-test | Divergence |
|---|---|---|---|---|
| CPU Interrupts | NMI Overlap BRK | 5 | ErrorCode 2 | The status byte pushed when an NMI hijacks a `BRK` does not match the ROM's answer key; NMI polling/hijack ordering needs work. |
| CPU Interrupts | NMI Overlap IRQ | 3 | ErrorCode 1 | Same hijack path for a hardware IRQ; the pushed status is wrong. |
| APU Registers and DMA | DMA + `$2002` Read | 5 | ErrorCode 2 | A load DMA intended to land on the `LDA $2002` data-read cycle starts one cycle early (on the operand fetch), so the dummy reads do not clear VBlank before the CPU read. |
| APU Registers and DMA | DMC DMA + OAM DMA | 5 | ErrorCode 2 | DMC and OAM transfers cannot interleave: DMC has priority on get cycles, OAM keeps running on halt/put cycles, and OAM needs an alignment cycle after a DMC get. |
| APU Registers and DMA | Explicit DMA Abort | 5 | ErrorCode 2 | The DMC transfer is atomic inside one CPU cycle, so a `$4015` disable cannot produce the ROM's partial 3-cycle and 1-cycle abort durations. |
| CPU Behavior 2 | DMA Landing on Write | 13 | ErrorCode 6 | The 4-cycle load-DMA variant is not produced; sub-tests 1–5 (including the 3-cycle get-start case) pass. |
| PPU Misc. | `$2007` Stress Test | 5 | ErrorCode 2 | The `$2007` read buffer during a visible scanline does not match per dot. |
| Advanced Sprite Evaluation | Sprites On Scanline 0 | 5 | ErrorCode 2 | The documented pre-render "treated as scanline 5" stale-secondary-OAM path does not yet latch a sprite-zero sprite for scanline 0. |

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

- `SH*` (`$93`, `$9F`, `$9B`, `$9C`, `$9E`): the `H+1` AND term is dropped when
  a DMA drives the bus on the cycle before the store.
- `Delta Modulation Channel` and `Implicit DMA Abort`: request-level enable
  latch and held-fetch handling.
- `Stale Sprite Shift Regs`: per-slot sprite latch moved to the middle of its
  fetch window, and dot 339 arms only occupied sprite slots.
- `APU Register Activation`, `$2004 Stress`, `Implied Dummy Reads`,
  `Arbitrary Sprite zero`, `$2002 Flag Timing`, `Controller Strobing`.

## How to add an entry

1. Record the suite, test name, ROM version, emulator build/commit, and date.
2. Record the observed value and the expected value (answer key or capture).
3. State the likely responsible module and any experiment already tried.
4. Link the fixing commit when the entry is removed.
