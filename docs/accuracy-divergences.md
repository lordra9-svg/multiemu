# Accuracy divergences — NES

This ledger records every known place where `nes_accuracy_core` does not
yet match the chosen reference (real hardware, then ares, then the public test
ROM). It is required by the accuracy charter in `docs/accuracy.md`. Add a dated
entry whenever a divergence is found, and remove it only when a test or a
hardware capture proves the behaviour matches.

Current snapshot: **2026-11-13**, AccuracyCoin v3.1.0, isolated battery
**146/146 non-DRAW rows passed** (146/151 total, 0 failed, 5 DRAW skips,
0 unrun).

The five skipped rows are the `DRAW` tests, which need a human-drawn reference
image. "ErrorCode N" below is the ROM's `value >> 2` index; the CLI reports
`value >> 1` as `code`.

## Open failures

None in AccuracyCoin. All non-DRAW rows pass.

### NESdev screen-suite failures (2026-11-13)

- `cpu_interrupts_v2/5-branch_delays_irq` — the branch-delay/IRQ table CRC
  differs from the reference; the other four interrupt singles pass.
- `dmc_dma_during_read4/dma_4016_read` — controller bit reads during DMC DMA
  are corrupted (the other four DMC/read tests pass).
- `pal_apu_tests` 04-08 and 10-11 — PAL APU/PPU phase; the length-counter,
  length-table and IRQ-flag tests pass in PAL mode.
- The `read_joy3/thorough_test` failure is input-dependent (no controller
  script is supplied).

### MMC3 A12 timing (2026-11-13)

The new PPU-A12 clocking is 5/6 in `mmc3_test_2` with the alternate-revision
selector:

- `4-scanline_timing` — test 12 (`Scanline 239 IRQ should occur later when
  $2000=$10`) is one dot late/early at the line-239 boundary. Tests for
  scanlines 0 and 1 in both `$2000` modes pass, as do all `$2000=$08` cases.
- `6-MMC3_alt` and the combined suite's `6-MMC6` pass when the alternate
  MMC3A/NEC revision is selected; the default remains normal MMC3B/C.

AccuracyCoin, the library audit and the existing mapper unit tests are
unaffected.

## Expansion-audio parity (2026-11-12)

A per-cycle differential probe (`apudump` with the expansion-channel columns;
oracle `APU_TRACE` `RD` records) compared the summed expansion output against
RustyNES:

| Board | Title / frame | Result |
|---|---|---|
| MMC5 | *Just Breed*, frame 300 | all five APU channels, expansion mix and total match on 29,780/29,780 cycles |
| Namco 163 | *Erika to Satoru no Yume Bouken*, frame 400 | exact on 29,781/29,781 cycles |
| VRC7 | *Lagrange Point*, frame 400 | exact on 29,780/29,780 cycles |
| VRC6 | *Akumajou Densetsu*, frame 400 | APU pulse 1/triangle/noise/DMC exact; pulse 2 differs on ~12% of cycles and the expansion mix by one raw unit |
| Sunsoft 5B | *Gimmick!* frames 20 and 400 | frame 20 exact on every APU channel and the expansion mix; later frames diverge because of the oracle errors listed below |

As a control, **Batman (Japan)** (FME-7 without expansion audio) matches all
five APU channels, the mix and the total at frame 400, so the DMC and mapper
changes do not depend on the 5B.

The probe found and fixed a real mapper bug: **Sunsoft FME-7 (mapper 69)**
command 8 was decoded as a PRG bank instead of the `$6000` window's
RAM-enable/RAM-select/bank bits, nametable arrangement 0/1 was swapped, and
the cycle IRQ ignored the counter-enable bit, used a fictitious reload value
and asserted without the enable bit. After the fix *Gimmick!* boots and its
video matches RustyNES on 118/120 frames; before, it stayed on a blank
backdrop.

The VRC6 sawtooth difference is recorded rather than matched: RustyNES adds
the accumulator on seven steps per 14-step sweep, while the NESdev wiki and
Mesen add on six (even) steps and reset on the 14th. Unko follows the
wiki/Mesen schedule.

### 5B mixer and noise: the oracle is wrong

The same probe exposed three independent RustyNES errors in its Sunsoft 5B
model. Unko follows the AY-3-8910/YM2149 datasheet, the NESdev wiki and the
MiSTer `Sunsoft.sv` HDL, which agree:

1. **Mixer `$07` bit layout.** The register is `--CB Acba`: tone A/B/C in bits
   0/1/2 and noise A/B/C in bits 3/4/5, active low. RustyNES decodes it as
   interleaved tone bits 0/2/4 and noise bits 1/3/5. *Gimmick!* writes `$F8`
   (all three tones on, all noise off); RustyNES therefore enables noise A and
   its expansion output flickers at the noise rate, while Unko's three-tone
   output is steady. The project's earlier code used the correct layout; it
   was only briefly changed while comparing against the oracle and is back on
   the documented mapping.
2. **Noise clock.** A 5B tone toggles every `16 * period` CPU cycles and the
   noise LFSR shifts every `32 * period` cycles. RustyNES shifts the noise LFSR
   every `16 * period`, twice as fast. Unko matches the wiki/HDL/MAME.
3. **Noise LFSR reset.** The AY/YM noise register powers up at 1 (MAME
   `ay8910_reset_ym`, MiSTer HDL `17'h1`). Unko now uses 1 and the hardware
   shift input, bit 0 XOR bit 3 of the 17-bit register.

The envelope shape state machine now also follows the wiki's ten shapes:
`continue=0` holds at the end of its single ramp, `hold` freezes at the reached
level, and `alternate+hold` flips to the opposite end before holding (`$0B`
ends at 31, `$0F` at 0). The only commercial 5B game leaves the envelope off,
so this matters for homebrew work rather than *Gimmick!*.

Unit tests pin the tone half-period, the 32-cycle noise shift, the LFSR reset
value, the `--CB Acba` mixer mapping, the envelope step interval and the
hold/alternate shapes.

### DMC DAC mixing

The DMC's 7-bit counter output always reaches the mixer, "regardless the
channel is enabled or not"; `$4011` also updates it while automatic playback
is off (NESdev APU DMC; Mesen's `AddOutput` path does the same). Unko muted the
DAC when `$4015` bit 4 was clear or the buffer was empty, so *Gimmick!*'s
`$4011 = 2` produced 0 instead of 2. After the fix the whole of frame 20
(pulse 1/2, triangle, noise, DMC, expansion mix and total) matches the oracle
cycle-for-cycle.

## Known residual differences (no test failures)

These do not fail any suite and are documented exceptions under the charter.

- **Frame-origin phase / APU pulse phase.** The frame-complete point can land
  one CPU cycle (three PPU dots) away from the reference capture, alternating
  with frame parity. Video is unaffected (the per-dot PPU phase matches), but
  it moves the audio drain slice and the APU clock phase seen by a register
  write. A per-cycle channel trace shows triangle, noise and DMC outputs
  bit-identical and both pulse channels exactly one CPU cycle out of phase
  once continuous music starts (for example *The Legend of Zelda* from frame
  39: 4-16 transition cycles per frame, mixed-output delta <= 0.05, inaudible
  at 1.79 MHz). Controlled differential probe ROMs - enable, duty, timer and
  length writes, mid-note timer writes, duty resets and `$4017` writes at every
  cycle parity - are bit-identical, as is every frame before the music starts.
  Per-frame drained sample counts differ by at most one sample in 36 of 120
  frames, with the same total (87,304) over the run.

  Earlier sessions ported the reference's master-clock bus and a coupled
  CPU/PPU scheduler; both were measured and regressed video/audio parity, so
  the integer dot scheduler is kept. Closing this difference needs a hardware
  capture of the exact frame wrap, which is not available locally.

- **Gimmick! DMC reload-chain phase (4 cycles).** At the start of a sample
  Unko's second output update lands four CPU cycles before RustyNES's
  (`$4015` enable at frame 23, rate index C: gaps of 731 vs 735 cycles, then
  identical 106-cycle intervals and identical DAC values). RustyNES models the
  TriCNES detail that a load DMA's get completes three cycles before the byte
  timer boundary, so the boundary consumes the buffer and arms a four-cycle
  reload DMA; Unko does not insert that stall. The shift is 2.2 microseconds
  and inaudible, and no test ROM covers it; closing it needs a hardware
  capture or a direct port of the TriCNES DMC state machine, not a guess.
- **Gimmick! CPU wait-loop phase.** The sound driver waits on `$F0` in a
  two-instruction loop and its mapper-IRQ return address alternates between
  the loop head and the branch target. Unko and RustyNES take the interrupt
  after different loop instructions on alternating frames (a one-instruction,
  2-3 cycle phase difference), although the handler entry cycles and frame
  boundaries match. This changes when later APU register writes land relative
  to the frame, so the 5B and DMC traces drift after frame 22 even though the
  per-cycle channel model agrees at frame 20. The FME-7 counter itself follows
  the wiki (decrement per CPU cycle, `$0000 -> $FFFF` assert, write to `$D`
  acknowledges); the remaining difference is the CPU's interrupt-poll position
  inside a taken/not-taken branch, which needs a hardware trace to resolve.

## Recently closed (2026-11-12)

- **PPU Read Buffer.** A read from palette RAM returned the palette entry but
  the deferred `$2007` buffer fill then read `$3F00` itself instead of the
  nametable underneath it (`$2F00`), so test 7 saw zero instead of `$5A`.
  The buffer fill now masks palette addresses with `$2FFF`, matching the
  immediate palette-read path. Unit test
  `palette_read_loads_the_buffer_from_the_nametable_underneath`.
- **Palette RAM Quirks.** Passes once its PPU Read Buffer prerequisite passes.

## Multi-oracle note

Raw RGBA hashes are palette-sensitive, so the trace format now carries an
optional fourth field: a palette-independent geometry hash that re-maps each
distinct colour to its first-appearance index before hashing. Two frames with
identical pixels but different RGB palette tables have equal geometry hashes;
`unko diff` reports such a pair as "palette only". This separates "the RGB
tables differ" from "the emulators render different pixels" and makes the
ares/MesenCE captures usable even though their palettes differ from RustyNES
(see `docs/emulator-tooling.md`).

Verified with that hash: *Star Wars* (Namco, mapper 19) and *Juvei Quest*
(Namco, mapper 19) at frame 120 are pixel-identical to **both** ares and
MesenCE (0 of 61,440 pixels differ after canonicalising the palette), while
RustyNES reports a blank or divergent frame. RustyNES is therefore a false
negative for these titles; the oracle hierarchy must prefer ares/MesenCE for
geometry and behaviour whenever the primary oracle's palette or boot model is
suspect. (An earlier entry compared our frame 119 against the reference's
frame 120 and reported four differing colour pairs; that was a frame-index
offset in the comparison, not a rendering difference.)

A 20-frame sweep of the whole 7,073-image library with the geometry hash
(6,727 images comparable, 346 oracle load failures) gives video 84.3%,
palette-independent geometry 85.9%, and 573 images that need a frame shift.
Short 20-frame windows are dominated by boot and fade frames, so the number is
a regression tripwire rather than an accuracy grade. The mappers where
geometry is far above video parity (35: 35.0% -> 95.0%, 69: 41.0% -> 78.8%,
12: 45.0% -> 85.0%, 513: 0.0% -> 90.0%) differ only in output RGB. Sampling
*Batman - Return of the Joker* (mapper 69) shows why: at frame 300 all cores
render a uniform backdrop, ours in NES index `$00` (grey) and ares in black
(`$0F`), because palette RAM is uninitialised on real hardware and each core
picks a different power-on value. That is a display choice, not a rendering
defect, and the geometry hash correctly ignores it.

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
