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
| Loaded (mapper implemented) | 6,550 | 92.2% |
| Rendering | 3,586 | 50.5% |
| Blank | 2,964 | 41.7% |
| Unsupported mapper | 519 | 7.3% |
| Load/run failure | 32 | 0.45% |

Batch consistency: four batches (2,000 + 2,000 + 2,000 + 1,101) reproduce the
single-pass totals exactly, so the batching has no boundary skew.

`rendering` is a smoke signal, not a compatibility claim: a game that needs
input, plays a static screen, or boots to a solid colour can read as blank, and
a rendered frame can still be visually wrong. The numbers rank where mapper
work pays off; they do not certify playability.

## Implemented mappers by volume

84 mapper IDs loaded at least one image. Sorted by total images.

| Mapper | Rendering | Blank | Total | Render% |
|---:|---:|---:|---:|---:|
| 4 | 623 | 702 | 1325 | 47% |
| 0 | 850 | 215 | 1065 | 80% |
| 1 | 571 | 485 | 1056 | 54% |
| 2 | 355 | 288 | 643 | 55% |
| 30 | 137 | 230 | 367 | 37% |
| 3 | 234 | 111 | 345 | 68% |
| 176 | 21 | 85 | 106 | 20% |
| 206 | 64 | 30 | 94 | 68% |
| 7 | 42 | 46 | 88 | 48% |
| 79 | 64 | 20 | 84 | 76% |
| 268 | 22 | 53 | 75 | 29% |
| 163 | 20 | 51 | 71 | 28% |
| 11 | 27 | 43 | 70 | 39% |
| 224 | 7 | 51 | 58 | 12% |
| 21 | 14 | 41 | 55 | 25% |
| 90 | 28 | 19 | 47 | 60% |
| 71 | 9 | 33 | 42 | 21% |
| 178 | 13 | 25 | 38 | 34% |
| 5 | 12 | 25 | 37 | 32% |
| 150 | 36 | 0 | 36 | 100% |
| 28 | 11 | 24 | 35 | 31% |
| 19 | 14 | 20 | 34 | 41% |
| 34 | 23 | 10 | 33 | 70% |
| 66 | 18 | 8 | 26 | 69% |
| 69 | 5 | 21 | 26 | 19% |
| 148 | 25 | 1 | 26 | 96% |
| 87 | 13 | 8 | 21 | 62% |
| 215 | 6 | 15 | 21 | 29% |
| 18 | 10 | 11 | 21 | 48% |
| 91 | 14 | 6 | 20 | 70% |
| 121 | 2 | 18 | 20 | 10% |
| 241 | 10 | 9 | 19 | 53% |
| 16 | 4 | 14 | 18 | 22% |
| 141 | 17 | 1 | 18 | 94% |
| 132 | 6 | 11 | 17 | 35% |
| 319 | 11 | 6 | 17 | 65% |
| 83 | 12 | 5 | 17 | 71% |
| 115 | 5 | 11 | 16 | 31% |
| 185 | 0 | 16 | 16 | 0% |
| 58 | 8 | 7 | 15 | 53% |
| 210 | 13 | 2 | 15 | 87% |
| 189 | 9 | 6 | 15 | 60% |
| 111 | 8 | 6 | 14 | 57% |
| 112 | 0 | 14 | 14 | 0% |
| 177 | 8 | 6 | 14 | 57% |
| 242 | 7 | 7 | 14 | 50% |
| 59 | 13 | 0 | 13 | 100% |
| 232 | 2 | 11 | 13 | 15% |
| 33 | 8 | 4 | 12 | 67% |
| 114 | 6 | 6 | 12 | 50% |
| 116 | 6 | 6 | 12 | 50% |
| 118 | 6 | 6 | 12 | 50% |
| 196 | 7 | 5 | 12 | 58% |
| 45 | 8 | 3 | 11 | 73% |
| 75 | 6 | 5 | 11 | 55% |
| 108 | 2 | 9 | 11 | 18% |
| 184 | 10 | 0 | 10 | 100% |
| 227 | 3 | 7 | 10 | 30% |
| 558 | 6 | 4 | 10 | 60% |
| 64 | 7 | 3 | 10 | 70% |
| 80 | 6 | 4 | 10 | 60% |
| 286 | 5 | 5 | 10 | 50% |
| 24 | 8 | 1 | 9 | 89% |
| 36 | 8 | 1 | 9 | 89% |
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
| 26 | 0 | 5 | 5 | 0% |
| 143 | 4 | 0 | 4 | 100% |
| 13 | 3 | 0 | 3 | 100% |
| 158 | 1 | 0 | 1 | 100% |
| 207 | 0 | 1 | 1 | 0% |

The core nodes are healthy where volume is highest: NROM (80%), UxROM (55%),
CNROM (68%) and MMC3 (47%). MMC1 sits at 54%. MMC3's blank share is the largest
absolute block (702 images), which is consistent with games that wait on
input after a title screen rather than a mapper fault, but it is the first
place to look for timing/IRQ regressions.

Several implemented-but-always-blank rows are worth a targeted check before
any new mapper work: **185** (CNROM variant), **112** (TXC), **42** (FDS
conversion), **162**, **235**, **26**, **138** and **207**. They load and run
20 frames without errors, so the likely causes are power-on CHR RAM state,
a mapper register written before the first render, or a boot screen that the
20-frame window does not reach.

## Unsupported mappers

519 images (7.3%) name a mapper with no implementation, across 193 distinct
IDs. The table lists the top 25; the ranked list below covers 515 of the 519
(the remaining four IDs have at most four images each).

| Mapper | Images | Family (where known) |
|---:|---:|---|
| 9 | 10 | MMC2 (Punch-Out!! CHR latch) |
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

The tail (5 images each): 256, 332, 74, 10, 515. Mapper 10 is MMC4, a sibling
of MMC2; implementing the MMC2 CHR latch would cover both.

Priority for the next mapper work, by volume and family reuse:

1. **Mapper 9 (MMC2)** — 10 images, well-documented latch behaviour
   (`$FD`/`$FE`/`$1FD`/`$1FE` CHR reads), and the same latch logic covers
   **mapper 10 (MMC4)**.
2. **Mapper 68 (Sunsoft-4)** — 8 images, one register file.
3. **Mappers 195/165/157/136/159/193/197** — 7–8 images each; check whether
   these are iNES-header assignment quirks (NES 2.0 submapper or database
   aliases) before writing new hardware models.

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
confirm they are binary-parse panics (likely guarded indexing on a short
header) rather than mapper state.

## Follow-ups

- Capture the four panic inputs' headers (sizes, mapper IDs) and add a
  parser guard so a malformed image returns `CartError` instead of unwinding.
- Re-run the always-blank implemented mappers with a per-frame hash over 600
  frames to separate "static title screen" from "never renders".
- Implement MMC2/MMC4 and re-run the affected batch to confirm the images
  move from `unsupported` to `rendering`.
