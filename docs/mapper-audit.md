# Commercial ROM mapper audit

Smoke audit of a local, licensed NES library (7,101 images) against the
accuracy core, recorded as aggregate results only. The library and its catalog
metadata live outside this repository and are never committed; see
`docs/emulator-tooling.md`.

## Method

Each image is loaded, run for 20 frames, and its final frame sampled:

- **rendering** — the frame has more than one distinct pixel (something drew).
- **blank** — the frame is a single uniform colour after 20 frames.
- **unsupported** — the iNES/NES 2.0 mapper ID has no implementation.
- **failed** — the image could not be loaded (truncated/short container).

The scan is split into 2,000-entry batches:

```
multiemu corpus <library-dir> --frames 20 --skip <n> --limit 2000
```

## Totals

| Metric | Count | Share |
|---|---:|---:|
| Scanned | 7,101 | 100% |
| Loaded (mapper implemented) | 6,565 | 92.4% |
| Rendering | 3,473 | 48.9% |
| Blank | 3,092 | 43.5% |
| Unsupported mapper | 504 | 7.1% |
| Load/run failure | 32 | 0.45% |

The batched totals reproduce the single-pass numbers exactly, so batching adds
no skew. Since the previous audit, MMC2 (9) and MMC4 (10) landed: loaded rose
6,550 → 6,565 and unsupported fell 519 → 504. The rendering count also fell
3,586 → 3,473 because the PPU backdrop fix (below) removed frames that were
only non-uniform due to the wrong backdrop colour; a uniform frame is not
necessarily a broken one.

`rendering` is a smoke signal, not a compatibility claim: a game that needs
input, plays a static screen, or boots to a solid colour can read as blank, and
a rendered frame can still be visually wrong. The 20-frame window is
deliberately short; several mapper families need hundreds of frames to leave
their boot screen (MMC2's Punch-Out!!, Nanjing 163, Sunsoft FME-7).

## MMC2 / MMC4

Mappers 9 and 10 are implemented:

- **MMC2** — 8 KiB switchable PRG at `$8000`; the top 24 KiB is fixed to the
  last three 8 KiB banks (`$A000`/`$C000`/`$E000`). The reset code sits in
  `N-3` and the vectors at the end of `N-1`, which the first draft (fixed
  `$A000` to `N-2`) got wrong; a real Punch-Out!! image caught it.
- **MMC4** — 16 KiB switchable PRG at `$8000`, last 16 KiB fixed at `$C000`.
- Both share the four CHR registers at `$B000`–`$E000` and mirroring at
  `$F000`, plus two 4 KiB CHR latches that switch when the PPU fetches tile
  `$FD` or `$FE`. The PPU owns the latch state and reads the full CHR ROM, so
  the switch happens during rendering rather than once per frame.
- CHR bank registers are 5 bits wide; the PPU wraps them to the ROM size, so a
  64 KiB CHR game that writes bank 24 reads bank 8 (high address lines are
  unconnected).

Verification:

- `mmc2_maps_prg_and_chr_registers`, `mmc4_switches_16k_prg_banks` (register
  and PRG/CHR mapping).
- `mmc2_latch_switches_pattern_banks_on_fd_fe_fetches` (independent latches,
  `$FD`/`$FE` switching, live bank-register updates).
- `mmc2_chr_registers_wrap_to_the_rom_size` (64 KiB CHR wraparound).
- `mmc2_chr_banks_reach_the_ppu` (system-level wiring).
- Local library slice: all **15/15** mapper-9/10 images render at 600 frames
  (10 Punch-Out!! variants and 5 Fire Emblem / Famicom Wars images). At the
  20-frame audit window they are still booting (mapper 9: 0/10 rendering;
  mapper 10: 2/5), which is why the 20-frame row below looks blank.

## Implemented mappers by volume

84 mapper IDs loaded at least one image. Sorted by total images.

| Mapper | Rendering | Blank | Total | Render% |
|---:|---:|---:|---:|---:|
| 4 | 604 | 721 | 1325 | 46% |
| 0 | 857 | 208 | 1065 | 80% |
| 1 | 568 | 488 | 1056 | 54% |
| 2 | 356 | 287 | 643 | 55% |
| 30 | 128 | 239 | 367 | 35% |
| 3 | 236 | 109 | 345 | 68% |
| 176 | 10 | 96 | 106 | 9% |
| 206 | 60 | 34 | 94 | 64% |
| 7 | 44 | 44 | 88 | 50% |
| 79 | 58 | 26 | 84 | 69% |
| 268 | 18 | 57 | 75 | 24% |
| 163 | 0 | 71 | 71 | 0% |
| 11 | 27 | 43 | 70 | 39% |
| 224 | 6 | 52 | 58 | 10% |
| 21 | 8 | 47 | 55 | 15% |
| 90 | 25 | 22 | 47 | 53% |
| 71 | 9 | 33 | 42 | 21% |
| 178 | 0 | 38 | 38 | 0% |
| 5 | 12 | 25 | 37 | 32% |
| 150 | 36 | 0 | 36 | 100% |
| 28 | 11 | 24 | 35 | 31% |
| 19 | 14 | 20 | 34 | 41% |
| 34 | 23 | 10 | 33 | 70% |
| 66 | 16 | 10 | 26 | 62% |
| 69 | 6 | 20 | 26 | 23% |
| 148 | 26 | 0 | 26 | 100% |
| 87 | 13 | 8 | 21 | 62% |
| 215 | 4 | 17 | 21 | 19% |
| 18 | 9 | 12 | 21 | 43% |
| 91 | 4 | 16 | 20 | 20% |
| 121 | 0 | 20 | 20 | 0% |
| 241 | 10 | 9 | 19 | 53% |
| 16 | 3 | 15 | 18 | 17% |
| 141 | 18 | 0 | 18 | 100% |
| 132 | 6 | 11 | 17 | 35% |
| 319 | 11 | 6 | 17 | 65% |
| 83 | 12 | 5 | 17 | 71% |
| 115 | 5 | 11 | 16 | 31% |
| 185 | 0 | 16 | 16 | 0% |
| 58 | 4 | 11 | 15 | 27% |
| 210 | 13 | 2 | 15 | 87% |
| 189 | 9 | 6 | 15 | 60% |
| 111 | 9 | 5 | 14 | 64% |
| 112 | 0 | 14 | 14 | 0% |
| 177 | 8 | 6 | 14 | 57% |
| 242 | 7 | 7 | 14 | 50% |
| 59 | 13 | 0 | 13 | 100% |
| 232 | 2 | 11 | 13 | 15% |
| 33 | 8 | 4 | 12 | 67% |
| 114 | 6 | 6 | 12 | 50% |
| 116 | 4 | 8 | 12 | 33% |
| 118 | 9 | 3 | 12 | 75% |
| 196 | 7 | 5 | 12 | 58% |
| 45 | 8 | 3 | 11 | 73% |
| 75 | 6 | 5 | 11 | 55% |
| 108 | 2 | 9 | 11 | 18% |
| 184 | 10 | 0 | 10 | 100% |
| 227 | 3 | 7 | 10 | 30% |
| 558 | 6 | 4 | 10 | 60% |
| 9 | 0 | 10 | 10 | 0% |
| 64 | 7 | 3 | 10 | 70% |
| 80 | 7 | 3 | 10 | 70% |
| 286 | 0 | 10 | 10 | 0% |
| 24 | 8 | 1 | 9 | 89% |
| 36 | 6 | 3 | 9 | 67% |
| 42 | 0 | 9 | 9 | 0% |
| 86 | 9 | 0 | 9 | 100% |
| 226 | 6 | 3 | 9 | 67% |
| 162 | 0 | 9 | 9 | 0% |
| 32 | 4 | 4 | 8 | 50% |
| 48 | 2 | 6 | 8 | 25% |
| 113 | 8 | 0 | 8 | 100% |
| 142 | 0 | 8 | 8 | 0% |
| 152 | 6 | 2 | 8 | 75% |
| 139 | 6 | 2 | 8 | 75% |
| 235 | 0 | 8 | 8 | 0% |
| 240 | 3 | 5 | 8 | 38% |
| 212 | 5 | 2 | 7 | 71% |
| 173 | 4 | 2 | 6 | 67% |
| 10 | 2 | 3 | 5 | 40% |
| 26 | 0 | 5 | 5 | 0% |
| 143 | 2 | 2 | 4 | 50% |
| 13 | 0 | 3 | 3 | 0% |
| 138 | 0 | 2 | 2 | 0% |
| 158 | 1 | 0 | 1 | 100% |
| 207 | 0 | 1 | 1 | 0% |

The core nodes are healthy where volume is highest: NROM (80%), CNROM (68%),
UxROM (55%), MMC1 (54%) and MMC3 (46%). MMC3's blank share is the largest
absolute block (721 images), which is consistent with games that wait on input
after a title screen rather than a mapper fault, but it remains the first place
to look for timing/IRQ regressions.

Several implemented rows stay at 0% in the 20-frame window: **163, 178, 121,
185, 112, 42, 162, 235, 138, 26, 207** (plus 176/224/21 at ≤15%). These load
and run without errors. Some need a longer boot window, not a fix: mapper 163
(a Chinese RPG sample) renders at 600 frames. Others are genuinely blank at
1,800 frames in this core while a reference emulator renders them (sampled
mapper 176 and 21 titles), which is a mapper/PPU follow-up rather than a
window problem. A per-frame hash over 600 frames would separate the two.

## Unsupported mappers

504 images (7.1%) name a mapper with no implementation, across 191
distinct IDs. The table lists the top 25; the ranked list covers 500 of
the 504. Five-image tail: 332, 74, 515, 70, 119, 187, 551, 40, 228, 313, 355, 529, 57, 123, 263, 519, 594, 140, 446, 552, 117, 264, 93, 192, 547, 250, 149, 144, 284, 396, 147, 15, 194, 31, 394, 188, 246, 29, 208, 67, 78, 72, 198, 12, 169, 180, 244, 272, 290, 301, 305, 309, 374, 516, 533, 548, 553, 554, 134, 191, 289, 399, 107, 137, 243, 298, 302, 306, 307, 312, 418, 513, 522, 525, 549, 218, 258, 534, 65, 125, 145, 37, 43, 50, 535, 557, 92, 96, 155, 103, 120, 14, 219, 245, 304, 346, 347, 512, 127, 186, 205, 221, 229, 238, 266, 292, 299, 308, 314, 326, 334, 351, 38, 380, 389, 400, 405, 41, 417, 422, 470, 153, 170, 190, 285, 415, 487, 493, 517, 521, 523, 539, 540, 97, 104, 105, 106, 167, 183, 252, 254, 262, 330, 368, 46, 52, 526, 530, 555, 559, 56, 564, 585, 62, 63, 73, 77, 81, 94, 122, 413, 47, 518, 538, 563, 89.

| Mapper | Images | Family (where known) |
|---:|---:|---|
| 68 | 8 | Sunsoft-4 |
| 195 | 8 | — |
| 165 | 8 | — |
| 157 | 7 | — |
| 136 | 7 | — |
| 159 | 7 | — |
| 193 | 7 | — |
| 197 | 7 | — |
| 303 | 6 | — |
| 60 | 6 | — |
| 156 | 6 | — |
| 172 | 6 | — |
| 199 | 6 | — |
| 216 | 6 | — |
| 222 | 6 | — |
| 249 | 6 | — |
| 164 | 6 | — |
| 234 | 6 | — |
| 27 | 6 | — |
| 85 | 6 | — |
| 168 | 6 | — |
| 253 | 6 | — |
| 225 | 5 | — |
| 133 | 5 | — |
| 256 | 5 | — |

Priority for the next mapper work, by volume and family reuse:

1. **Mapper 68 (Sunsoft-4)** — 8 images, a documented register file with
   CHR/PRG banking and a scanline IRQ.
2. **Mappers 195/165/157/136/159/193/197** — 7–8 images each; check whether
   these are iNES-header assignment quirks (NES 2.0 submapper or database
   aliases) before writing new hardware models.
3. The NES 2.0 high-bit aliases (**521/522/266**) that appear around the
   MMC2/MMC4 family are header variants, not new hardware; they are left
   unsupported deliberately rather than guessed at.

## Load/run failures

| Count | Cause |
|---:|---|
| 20 | image truncated: expected 40,976 bytes, got 16,400 |
| 4 | panic |
| 4 | image truncated: expected 172,048 bytes, got 49,168 |
| 3 | image truncated: expected 32,784 bytes, got 8,208 |
| 1 | image truncated: expected 81,936 bytes, got 18,448 |

The truncated cases are containers whose declared PRG+CHR size exceeds the
payload — an archive/ROM extraction issue, not an emulation issue. The four
panics are worth a backtrace with `RUST_BACKTRACE=1` on an isolated batch to
confirm they are binary-parse panics rather than mapper state.

## PPU fixes found by the audit

- **Backdrop colour**: the backdrop path double-indirected the palette
  (`palette_rgb[palette[0]]` instead of `palette_rgb[0]`), painting
  backdrop-only screens with the colour of palette entry 1. Covered by
  `backdrop_uses_palette_index_zero_not_double_indirection`.
- **CHR bank wrap**: see the MMC2/MMC4 section, covered by
  `mmc2_chr_registers_wrap_to_the_rom_size`.

Both are display-only and leave the AccuracyCoin battery at 145/151.

## Follow-ups

- Capture the four panic inputs' headers (sizes, mapper IDs) and add a parser
  guard so a malformed image returns `CartError` instead of unwinding.
- Re-run the always-blank implemented mappers with a per-frame hash over 600
  frames to separate "static title screen" from "never renders"; the sampled
  176/21 titles render in a reference emulator and are the first targets.
- Add Sunsoft-4 (68), then re-run the affected batch.
