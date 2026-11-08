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

The scan is split into 2,000-entry batches, or filtered with `--mapper`:

```
multiemu corpus <library-dir> --frames 20 --skip <n> --limit 2000
multiemu corpus <library-dir> --mapper 68 --frames 600
```

## Totals

| Metric | Count | Share |
|---|---:|---:|
| Scanned | 7,101 | 100% |
| Loaded (mapper implemented) | 6,577 | 92.6% |
| Rendering | 3,503 | 49.3% |
| Blank | 3,074 | 43.3% |
| Unsupported mapper | 496 | 7.0% |
| Load/run failure | 28 | 0.39% |

The batched totals reproduce the single-pass numbers exactly, so batching adds
no skew. Since the previous audit: MMC2/MMC4, Sunsoft-4 and the VRC2/VRC4
rewrite landed, and the four mapper-173 panics were fixed. Loaded rose
6,565 → 6,577, unsupported fell 504 → 496, failures fell 32 → 28, and the
panic count fell 4 → 0.

`rendering` is a smoke signal, not a compatibility claim: a game that needs
input, plays a static screen, or boots to a solid colour can read as blank, and
a rendered frame can still be visually wrong. The 20-frame window is
deliberately short; several families need hundreds of frames to leave their
boot screen (MMC2 Punch-Out!!, Sunsoft-4, Nanjing 163, Sunsoft FME-7).

## Recent mapper and PPU work

- **MMC2/MMC4 (9/10)** — 8/16 KiB PRG banking, four CHR registers and the
  `$FD`/`$FE` fetch latches driven by the PPU mid-frame. All 15 local images
  render at 600 frames.
- **Sunsoft-4 (68)** — 2 KiB CHR banks, 16 KiB switchable PRG, WRAM enable and
  `$E000` mirroring. 5/8 local images render; the three blanks are the
  combined Nantettatte!! Baseball dumps and the US After Burner prototype,
  which rely on ROM nametables or missing sub-cartridges.
- **VRC2/VRC4 (21/22/23/25)** — rewritten with the correct fixed PRG layout
  (`$C000` second-to-last, `$E000` last), eight 1 KiB CHR register pairs,
  per-board address-line decoding, VRC4 swap/WRAM mode and a CPU-cycle IRQ.
  At the same 20-frame window the family went from 8 rendering / 47 blank to
  34 rendering / 21 blank, and at 600 frames 39/55 render. The old
  `number()` also reported every VRC board as mapper 21; they are now
  reported separately.
- **Mapper 173 panic** — 32 KiB-window mappers indexed past a 16 KiB PRG
  buffer (`len 16384, index 32764`). The index now wraps, covering the four
  images that previously panicked; they render.
- **PPU backdrop** — the backdrop double-indirected the palette, so
  backdrop-only screens used palette entry 1's colour. Fixed and covered by
  `backdrop_uses_palette_index_zero_not_double_indirection`.

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
| 90 | 25 | 22 | 47 | 53% |
| 71 | 9 | 33 | 42 | 21% |
| 178 | 0 | 38 | 38 | 0% |
| 5 | 12 | 25 | 37 | 32% |
| 150 | 36 | 0 | 36 | 100% |
| 28 | 11 | 24 | 35 | 31% |
| 19 | 14 | 20 | 34 | 41% |
| 34 | 23 | 10 | 33 | 70% |
| 23 | 18 | 12 | 30 | 60% |
| 66 | 16 | 10 | 26 | 62% |
| 69 | 6 | 20 | 26 | 23% |
| 148 | 26 | 0 | 26 | 100% |
| 18 | 9 | 12 | 21 | 43% |
| 87 | 13 | 8 | 21 | 62% |
| 215 | 4 | 17 | 21 | 19% |
| 91 | 4 | 16 | 20 | 20% |
| 121 | 0 | 20 | 20 | 0% |
| 241 | 10 | 9 | 19 | 53% |
| 16 | 3 | 15 | 18 | 17% |
| 25 | 9 | 9 | 18 | 50% |
| 141 | 18 | 0 | 18 | 100% |
| 83 | 12 | 5 | 17 | 71% |
| 132 | 6 | 11 | 17 | 35% |
| 319 | 11 | 6 | 17 | 65% |
| 115 | 5 | 11 | 16 | 31% |
| 185 | 0 | 16 | 16 | 0% |
| 58 | 4 | 11 | 15 | 27% |
| 189 | 9 | 6 | 15 | 60% |
| 210 | 13 | 2 | 15 | 87% |
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
| 9 | 0 | 10 | 10 | 0% |
| 64 | 7 | 3 | 10 | 70% |
| 80 | 7 | 3 | 10 | 70% |
| 173 | 8 | 2 | 10 | 80% |
| 184 | 10 | 0 | 10 | 100% |
| 227 | 3 | 7 | 10 | 30% |
| 286 | 0 | 10 | 10 | 0% |
| 558 | 6 | 4 | 10 | 60% |
| 24 | 8 | 1 | 9 | 89% |
| 36 | 6 | 3 | 9 | 67% |
| 42 | 0 | 9 | 9 | 0% |
| 86 | 9 | 0 | 9 | 100% |
| 162 | 0 | 9 | 9 | 0% |
| 226 | 6 | 3 | 9 | 67% |
| 32 | 4 | 4 | 8 | 50% |
| 48 | 2 | 6 | 8 | 25% |
| 68 | 0 | 8 | 8 | 0% |
| 113 | 8 | 0 | 8 | 100% |
| 139 | 6 | 2 | 8 | 75% |
| 142 | 0 | 8 | 8 | 0% |
| 152 | 6 | 2 | 8 | 75% |
| 235 | 0 | 8 | 8 | 0% |
| 240 | 3 | 5 | 8 | 38% |
| 212 | 5 | 2 | 7 | 71% |
| 10 | 2 | 3 | 5 | 40% |
| 21 | 5 | 0 | 5 | 100% |
| 26 | 0 | 5 | 5 | 0% |
| 143 | 2 | 2 | 4 | 50% |
| 13 | 0 | 3 | 3 | 0% |
| 22 | 2 | 0 | 2 | 100% |
| 138 | 0 | 2 | 2 | 0% |
| 158 | 1 | 0 | 1 | 100% |
| 207 | 0 | 1 | 1 | 0% |

The core nodes are healthy where volume is highest: NROM (80%), CNROM (68%),
UxROM (55%), MMC1 (54%) and MMC3 (47%). MMC3's blank share is the largest
absolute block, consistent with titles waiting on input, but it remains the
first place to look for timing/IRQ regressions.

Rows at 0% in the 20-frame window (163, 178, 121, 185, 112, 42, 162, 235, 138,
26, 207) load and run without errors. Some simply need a longer boot window
(mapper 163 renders at 600 frames); a sampled mapper 176 and mapper 21 title
render in a reference emulator while staying blank here, which is a
mapper/PPU follow-up rather than a window problem. Use
`multiemu signatures <dir> --frames 600 --check <manifest>` to separate the
two across a whole mapper.

## Unsupported mappers

496 images (7.0%) name a mapper with no implementation. The table lists the
top 25; the printed ranked list is capped at 80 IDs and covers 342 of the 496.
Five-image tail: 256, 332, 40, 515, 551, 70, 74, 117, 123, 140, 149, 192, 228, 250, 263, 264, 313, 355, 446, 519, 529, 547, 552, 57, 594, 93, 144, 147, 15, 188, 194, 198, 208, 246, 284, 29, 31, 394, 396, 534, 65, 67, 72, 78, 103, 107, 12, 120, 125, 134, 137, 14, 145, 155, 169.

| Mapper | Images | Family (where known) |
|---:|---:|---|
| 165 | 8 | — |
| 195 | 8 | — |
| 136 | 7 | — |
| 157 | 7 | — |
| 159 | 7 | — |
| 193 | 7 | — |
| 197 | 7 | — |
| 156 | 6 | — |
| 164 | 6 | — |
| 168 | 6 | — |
| 172 | 6 | — |
| 199 | 6 | — |
| 216 | 6 | — |
| 222 | 6 | — |
| 234 | 6 | — |
| 249 | 6 | — |
| 253 | 6 | — |
| 27 | 6 | — |
| 303 | 6 | — |
| 60 | 6 | — |
| 85 | 6 | — |
| 119 | 5 | — |
| 133 | 5 | — |
| 187 | 5 | — |
| 225 | 5 | — |

Priority for the next mapper work, by volume:

1. **Mappers 195/165 (8 each)** then **136/157/159/193/197 (7 each)** — check
   whether these are iNES-header assignment quirks (NES 2.0 submapper or
   database aliases) before writing new hardware models.
2. **Sunsoft-4 ROM nametables** — three remaining Sunsoft-4 images use CHR ROM
   as nametables; the mapper records the mode but the PPU still renders CIRAM.
3. The NES 2.0 high-bit aliases (**521/522/266**) around the MMC2/MMC4 family
   are header variants, not new hardware; they are left unsupported
   deliberately rather than guessed at.

## Load/run failures

| Count | Cause |
|---:|---|
| 20 | image truncated: expected 40,976 bytes, got 16,400 |
| 4 | image truncated: expected 172,048 bytes, got 49,168 |
| 3 | image truncated: expected 32,784 bytes, got 8,208 |
| 1 | image truncated: expected 81,936 bytes, got 18,448 |

All remaining failures are containers whose declared PRG+CHR size exceeds the
payload — an archive/ROM extraction issue, not an emulation issue. The four
mapper-173 panics are gone.

## Regression tooling

- `multiemu trace <rom> --frames N --out trace.txt` records per-frame
  video/audio hashes and the final state.
- `multiemu diff <a.trace> <b.trace>` reports the first divergent frame.
- `multiemu signatures <dir> [--frames N] [--mapper ID] [--check file]`
  hashes every frame of every ROM in a directory and compares against a
  manifest, reporting changed/new/missing titles. Manifests are local-only
  (they derive from game content and are never committed).

## Follow-ups

- Add Sunsoft-4 ROM-nametable support in the PPU (three images).
- Investigate the sampled mapper 176/21 titles that render in a reference
  emulator but stay blank here.
- Re-run the full audit at 600 frames for the always-blank shortlist; the
  20-frame window cannot distinguish a slow boot from a broken mapper.
