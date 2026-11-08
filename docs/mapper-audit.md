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
multiemu corpus <library-dir> --mapper 136 --frames 600
```

## Totals

| Metric | Count | Share |
|---|---:|---:|
| Scanned | 7,101 | 100% |
| Loaded (mapper implemented) | 6,627 | 93.3% |
| Rendering | 3,554 | 50.0% |
| Blank | 3,073 | 43.3% |
| Unsupported mapper | 446 | 6.3% |
| Load/run failure | 28 | 0.39% |

Batched totals reproduce the single-pass numbers exactly. Since the previous
audit, mappers 136 (Sachen JV001), 159 (Bandai LZ93D50/EEPROM), 197
(MMC3 512 KiB CHR wiring), 195 (Waixing FS303), 165 (MMC2/MMC3 hybrid),
193 (NTDEC TC-112) and 172 (Super Mega JV001) were added, and the `$2007` PPU
data state machine was corrected. Loaded rose 6,577 → 6,627 and unsupported
fell 496 → 446.

`rendering` is a smoke signal, not a compatibility claim. The 20-frame window
is deliberately short; several families need hundreds of frames to leave their
boot screen (MMC2, Sunsoft-4, MMC3-8025, Waixing 195).

## Mapper and PPU work

- **MMC2/MMC4 (9/10)** — 8/16 KiB PRG banking, four CHR registers and the
  `$FD`/`$FE` fetch latches driven by the PPU mid-frame. 15/15 local images
  render at 600 frames.
- **Sunsoft-4 (68)** — 2 KiB CHR banks, 16 KiB switchable PRG, WRAM enable,
  `$E000` mirroring and CHR-ROM nametables. 6/8 images render; the two blanks
  are combined Nantettatte!! Baseball cassette dumps.
- **MMC3-8025 (176)** — FS005 submapper-2 RAM configuration/protection,
  32 KiB banked WRAM and the `$5000-$5FFF` outer-register window. 80/106
  images render at 600 frames.
- **VRC2/VRC4 (21/22/23/25)** — corrected fixed PRG banks, eight 1 KiB CHR
  register pairs, per-board address decoding, swap/WRAM mode and CPU-cycle
  IRQ. 39/55 images render at 600 frames.
- **Sachen JV001 (136)** — six-bit latch with adder/inverter; CHR from output
  bits 0-2, 32 KiB PRG from bit 4. **7/7** images render at 60 frames.
- **Super Mega JV001 (172)** — the same latch with reversed CPU data bits and
  software mirroring; CHR from output bits 4-5. **4/6** render; the two
  *1991 Du Ma Racing* images never enable rendering (game-specific follow-up).
- **NTDEC TC-112 (193)** — 4 KiB CHR at `$6000`, two 2 KiB CHR banks at
  `$6001`/`$6002`, 8 KiB PRG banks at `$6003` with the last three fixed.
  **7/7** render at 60 and 600 frames.
- **Bandai LZ93D50 (159)** — mapper 16 register set with the 128-byte EEPROM
  window; **7/7** render at 600 frames (EEPROM persistence is a follow-up).
- **MMC3 512 KiB CHR (197)** — per-submapper CHR wiring translates the MMC3's
  1 KiB banks into 2/4 KiB ROM banks; **7/7** render at 60 frames.
- **Waixing FS303 (195)** — MMC3 plus 4 KiB PRG-RAM at `$5000`, and a shared
  hybrid CHR map: PPU `$2007` writes are forwarded to the mapper, the CHR-RAM
  window is addressed raw, and the CHRB value written through CHR bank numbers
  adds the remapped ROM-bank ranges. The System bus now exposes the mapper's
  `$5000-$5FFF` PRG-RAM window to the CPU, which was the last piece: the
  protection code these games copy there executed as open-bus zeroes before.
  **8/8 render**; Tiexue Lianmeng sits on a black title screen at 600 frames
  without input and renders as soon as Start is pressed.
- **Hybrid MMC2/MMC3 (165)** — 4 KiB CHR pages selected by the MMC2-style
  latches (page 0 is CHR-RAM), toggled by `$FD`/`$FE` pattern fetches and by
  `$2007` reads. The PPU reads and writes through the shared map. 8/8 render at
  600 frames.
- **MMC5 (5, partial)** — the CPU-visible register file covers the unsigned
  8×8 multiplier (`$5205`/`$5206`), PRG-RAM protect (`$5102`/`$5103`), the
  `$5113` 8 KiB PRG-RAM bank, the `$5114-$5116` RAM/ROM toggle, ExRAM CPU
  access modes (`$5104`, `$5C00-$5FFF`), `$5130` upper CHR bits and the
  `$5204` IRQ-pending read. A shared ExRAM/CHR state now feeds the PPU:
  ExRAM-backed nametables (modes 2/3) and fill mode are rendered, and the
  1 KiB CHR bank map is consulted per fetch so mid-frame `$5120` writes take
  effect (plain CIRAM nametables keep the working vertical/horizontal
  approximation). Vertical split-screen (`$5200-$5202`) substitutes ExRAM
  nametable/attribute data and the `$5202` 4 KiB CHR bank per fetch group, with
  the split fine Y driving CHR A0-A2. 12/37 render at 20 frames, 17/37 at 600.
  Still missing: extended attributes, accurate per-page CIRAM mapping, and
  expansion audio.
- **PPU** — the backdrop no longer double-indirects the palette; all five
  mirroring modes are modelled with 4 KiB CIRAM; the `$2007` read buffer is
  fed by the per-dot background/sprite fetch with a four-dot data state
  machine (see `docs/accuracy-divergences.md`).

## Implemented mappers by volume

95 mapper IDs loaded at least one image. Sorted by total images.

| Mapper | Rendering | Blank | Total | Render% |
|---:|---:|---:|---:|---:|
| 4 | 596 | 729 | 1325 | 45% |
| 0 | 843 | 222 | 1065 | 79% |
| 1 | 567 | 489 | 1056 | 54% |
| 2 | 356 | 287 | 643 | 55% |
| 30 | 128 | 239 | 367 | 35% |
| 3 | 231 | 114 | 345 | 67% |
| 176 | 39 | 67 | 106 | 37% |
| 206 | 60 | 34 | 94 | 64% |
| 7 | 51 | 37 | 88 | 58% |
| 79 | 56 | 28 | 84 | 67% |
| 268 | 18 | 57 | 75 | 24% |
| 163 | 0 | 71 | 71 | 0% |
| 11 | 28 | 42 | 70 | 40% |
| 224 | 6 | 52 | 58 | 10% |
| 90 | 25 | 22 | 47 | 53% |
| 71 | 9 | 33 | 42 | 21% |
| 178 | 0 | 38 | 38 | 0% |
| 5 | 12 | 25 | 37 | 32% |
| 150 | 36 | 0 | 36 | 100% |
| 28 | 11 | 24 | 35 | 31% |
| 19 | 14 | 20 | 34 | 41% |
| 34 | 23 | 10 | 33 | 70% |
| 23 | 24 | 6 | 30 | 80% |
| 66 | 16 | 10 | 26 | 62% |
| 69 | 6 | 20 | 26 | 23% |
| 148 | 26 | 0 | 26 | 100% |
| 18 | 9 | 12 | 21 | 43% |
| 87 | 15 | 6 | 21 | 71% |
| 215 | 4 | 17 | 21 | 19% |
| 91 | 4 | 16 | 20 | 20% |
| 121 | 0 | 20 | 20 | 0% |
| 241 | 10 | 9 | 19 | 53% |
| 16 | 4 | 14 | 18 | 22% |
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
| 68 | 3 | 5 | 8 | 38% |
| 113 | 8 | 0 | 8 | 100% |
| 139 | 6 | 2 | 8 | 75% |
| 142 | 0 | 8 | 8 | 0% |
| 152 | 6 | 2 | 8 | 75% |
| 165 | 5 | 3 | 8 | 62% |
| 195 | 5 | 3 | 8 | 62% |
| 235 | 0 | 8 | 8 | 0% |
| 240 | 3 | 5 | 8 | 38% |
| 136 | 7 | 0 | 7 | 100% |
| 159 | 2 | 5 | 7 | 29% |
| 197 | 5 | 2 | 7 | 71% |
| 212 | 5 | 2 | 7 | 71% |
| 193 | 4 | 3 | 7 | 57% |
| 172 | 4 | 2 | 6 | 67% |
| 10 | 2 | 3 | 5 | 40% |
| 21 | 5 | 0 | 5 | 100% |
| 26 | 0 | 5 | 5 | 0% |
| 143 | 2 | 2 | 4 | 50% |
| 13 | 0 | 3 | 3 | 0% |
| 22 | 2 | 0 | 2 | 100% |
| 138 | 0 | 2 | 2 | 0% |
| 158 | 1 | 0 | 1 | 100% |
| 207 | 0 | 1 | 1 | 0% |

The core nodes are healthy where volume is highest: NROM (79%), CNROM (68%),
UxROM (55%), MMC1 (54%) and MMC3 (46%).

## Unsupported mappers

446 images (6.3%) name a mapper with no implementation. The printed ranked
list is capped at 80 IDs and covers 302 of the 446. Top families: **157
(Datach, needs the barcode peripheral)** and a tail of six-image boards.

### Tail triage (IDs at 6-7 images)

| Mapper | Images | Board / notes | Next step |
|---:|---:|---|---|
| 157 | 7 | Datach Joint ROM (barcode reader) | blocked on the peripheral |
| 156 | 6 | Daou DIS23C01 (Korean) | distinct board; small |
| 164 | 6 | Dongda PEC-9588 educational computer, CHR RAM | small/medium |
| 168 | 6 | Racermate Challenge II + exercise bike | low value; peripheral |
| 199 | 6 | Waixing FS309, MMC3-like + CHR RAM + IRQ | medium; shares MMC3 work |
| 216 | 6 | Bonza / Magic Jewelry II (Dendy) | small; Dendy timing |
| 222 | 6 | Dragon Ninja (CTC-31) / Super Bros. 8 | medium; two submappers |
| 234 | 6 | Maxi 15 multicart, discrete logic + bus conflicts | small |
| 249 | 6 | Waixing T9552 board | medium |
| 253 | 6 | Waixing VRC4-based (Dragon Ball Z) | medium; reuses VRC4 |
| 27 | 6 | Mi Hun Che / Super Car (32 KiB PRG, 8 KiB CHR) | undocumented; inspect |
| 60 | 6 | 4-in-1 multicarts | small latch |
| 85 | 6 | **VRC7** (Lagrange Point, Tiny Toon 2 JP) | expansion audio (OPLL) |
| 303 | 6 | Kaiser KS-7017 (Armana no Kiseki) | likely related to KS7032 |

Only **172** turned out to be an alias of an implemented board (Sachen JV001
with reversed data bits). The rest are real hardware models; the highest-value
next targets are **85 (VRC7 + expansion audio)**, **199/253/249** (Waixing,
MMC3/VRC4 reuse), and the small discrete-logic boards 234/60.

## Load/run failures

All 28 remaining failures are truncated containers (declared PRG+CHR exceeds
the payload), not emulation issues. The four mapper-173 panics remain fixed.

## Regression tooling

- `multiemu trace <rom> --frames N --out trace.txt` records per-frame
  video/audio hashes and the final state.
- `multiemu diff <a.trace> <b.trace>` reports the first divergent frame.
- `multiemu signatures <dir> [--frames N] [--mapper ID] [--check file]`
  hashes every frame of every ROM in a directory and compares a manifest.
- `cargo run -p multiemu-nes-accuracy --example dump2007` dumps the `$2007`
  stress test's raw `$0500-$0654` array for answer-key comparison.

Manifests and traces are local-only and are never committed.
