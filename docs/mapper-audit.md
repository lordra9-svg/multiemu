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
unko corpus <library-dir> --frames 20 --skip <n> --limit 2000
unko corpus <library-dir> --mapper 136 --frames 600
```

## Totals

| Metric | Count | Share |
|---|---:|---:|
| Scanned | 7,101 | 100% |
| Loaded (mapper implemented) | 7,059 | 99.4% |
| Rendering | 3,832 | 54.0% |
| Blank | 3,227 | 45.4% |
| Unsupported mapper | 14 | 0.2% |
| Load/run failure | 28 | 0.39% |

Batched totals reproduce the single-pass numbers exactly. Since the previous
audit, 185 boards were added (listed in the completion sweep below). Loaded rose
6,627 → 7,059 (93.3% → 99.4%) and unsupported fell 446 → 14 (6.3% → 0.2%);
4 mapper IDs remain (the ranking prints the top 80): 355/547/594 (4 images
each) and 169 (2).

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
- **MMC3 IRQ clocking (4 and derivatives)** — the system now clocks
  scanline-IRQ mappers at the PPU A12 rise that opens the sprite pattern fetch
  window (dot 260), once per rendered line. Previously `end_cycle` used a
  stale `cycle_before_scanline` field and clocked the counter on nearly every
  CPU cycle, so IRQ latches expired within a few instructions and split-screen
  scroll effects were badly misplaced. Super Mario Bros. 3's curtain intro now
  renders the split at the correct line (frame 200: 7,030 -> 277 differing
  pixels, and the remaining difference is the sprite animation phase). The
  regression test `mmc3_irq_clocks_once_per_rendered_scanline` pins it.
- **Waixing T9552 (253)** — 8 KiB PRG pages at `$8000`/`$A000` (`$8010`/
  `$A010`) with the last two fixed at `$C000`/`$E000`; eight 1 KiB CHR pages
  whose 16-bit bank values are assembled from low/high nibble writes across
  `$B000-$E00C`; four mirroring modes at `$9400`; a CPU-clock IRQ with a /114
  prescaler (`$F000`/`$F004` reload, `$F008` enable); and a 2 KiB CHR-RAM page
  pair selected while a CHR low nibble is 4 or 5 (unless the `$88` signature
  forces CHR-ROM). **6/6 images load, 3/6 render at 600 frames**; Dragon Ball Z
  reaches its title screen correctly.
- **Bonza / Magic Jewelry II (216)** — the 32 KiB PRG page is selected by bit 0
  of the *write address* and the 8 KiB CHR page by bits 1-3 (the data byte is
  ignored); `$5000` reads return 0. **6/6 images load, 4/6 render at 600
  frames**, including the Magic Jewelry II title screen.
- **4-in-1 multicarts (60)** — the selected 16 KiB PRG bank and 8 KiB CHR bank
  advance on every **soft reset**, cycling through four games; there are no
  writable registers. `System::reset` now notifies the mapper through a new
  `Mapper::soft_reset` hook. **6/6 images load and render at 600 frames**, with
  the Battle City title screen confirmed.
- **Dragon Ninja / CTC-31 (222)** — eight 1 KiB CHR banks (`$B000-$E002`),
  two switchable 8 KiB PRG banks (`$8000`/`$A000`) with the last 16 KiB fixed,
  mirroring at `$9000`, and an A12-rise IRQ that counts rendered scanlines and
  fires after 240 (`$F000` reload). **6/6 images load and render at 600
  frames**, including the Dragon Ninja title screen (framebuffer dump).
- **Waixing educational / 164** — a 32 KiB PRG bank selected by two nibble
  writes in `$5000-$51FF` (`$5000` low, `$5100` high) with 8 KiB CHR-RAM; the
  board powers on with the last bank. **6/6 images load, 3/6 render at 600
  frames** (the Koudai Jingling Pokemon Gold Chinese title screen renders
  correctly). The three Final Fantasy pirate titles loop in the reset bank
  writing `$60` to both nibble registers (bank 0x60, out of range for a 16-bank
  512 KiB image), so they are either a different board/submapper or bad dumps.
- **Maxi 15 multicart (234)** — two latches at `$FF80-$FF9F` / `$FFE8-$FFF8`
  with bus conflicts and side-effecting register reads; 32 KiB PRG / 8 KiB CHR
  pages; latch bit 6 selects NINA-03 vs CNROM banking and bit 7 the mirroring.
  The first latch only updates while its bits 0-5 are clear. **6/6 images load
  and render at 600 frames**, with the `MAXI GAME CART RIDGE` title screen
  confirmed by a framebuffer dump.
- **Waixing / Myth Struggle (249)** — MMC3 plus a single `$5000` register: when
  bit 1 is set, the chip permutes the CHR (8-bit) and PRG (5-bit below `$20`,
  8-bit above) page numbers before banking. **6/6 images load and render at 600
  frames** (Shuihu Zhuan, Duo Bao Xiao Yinghao, Fengshenbang), verified by
  framebuffer dumps (Waixing logo / Water Margin intro).
- **Waixing FS309 (199)** — MMC3 plus four extra registers written through
  `$8001` when `$8000` bit 3 is set: they force 8 KiB PRG slots 2/3
  (`$C000`/`$E000`) and 1 KiB CHR slots 1/3, while slots 0/2 use R0/R1. `$A000`
  selects four mirroring modes; the board has 8 KiB CHR-RAM. The general MMC3
  core also gained the Mesen `ResetMmc3` power-on register values and now keeps
  PRG-RAM readable when `$A001` bit 7 gates writes only. Critically, the FS309
  has the same **4 KiB PRG-RAM window at `$5000-$5FFF`** as the FS303 (mapper
  195): the games copy an NMI handler there and jump to `$5C42`, which
  previously executed open bus/zeros. Adding `low_window() = $5000-$5FFF`
  raised the board from 1/6 to **6/6 images rendering at 600 frames**
  (Genghis Khan, Sanguo Waizhuan, Datang Fengyun VI and the Wind Fantasy /
  Tangmu titles), verified by framebuffer dumps.
- **Daou DIS23C01 (156)** — 16 KiB PRG banking (`$C010`, last bank fixed at
  `$C000`), eight 1 KiB CHR banks whose 16-bit registers are split across low
  and high nibble writes (`$C000-$C00F`, Mesen2's slot decode), 8 KiB work RAM
  at `$6000-$7FFF` and runtime vertical/horizontal mirroring (`$C014`,
  power-on single-screen). **6/6 images load, 4/6 render at 600 frames**
  (Metal Force, Koko Adventure, Janggun-ui Adeul and Street Hero); against the
  reference, Metal Force matches on 118/120 video frames.
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
- **Mapper completion sweep (12, 14, 15, 27, 28, 29, 31, 37, 38, 40, 41, 43, 45, 46, 47, 50, 52,
  57, 62, 63, 65, 67, 70, 72, 73, 74, 78, 81, 89, 90, 92, 93, 94, 96,
  97, 103, 104, 105, 106, 107, 117, 119, 120, 122, 123, 125, 127, 133,
  134, 137, 140, 144, 145, 147, 149, 153, 155, 157, 167, 168, 170,
  180, 183, 186, 187, 188, 190, 191, 192, 194, 198, 205, 208, 218,
  219, 221, 225, 228, 229, 238, 243, 244, 245, 246, 250, 252, 254,
  256, 258, 262, 263, 264, 266, 272, 284, 285, 289, 290, 292, 298,
  299, 301, 302, 303, 304, 305, 306, 307, 308, 309, 312, 313, 314,
  326, 330, 332, 334, 346, 347, 351, 359, 368, 374, 380, 389, 394,
  396, 399, 400, 405, 413, 415, 417, 418, 422, 446, 470, 487, 493,
  512, 513, 515, 516, 517, 518, 519, 521, 522, 523, 525, 526, 529,
  530, 533, 534, 535, 538, 539, 540, 544, 548, 549, 551, 552, 553,
  554, 555, 557, 559, 563, 564, 576, 585)** — 185 boards
  added against the RustyNES and Mesen2 references:
  TQROM and the Waixing CHR-RAM overlays share `HybridKind`; the SMB2j
  Whirlwind board exposes PRG bank 6 at `$6000`; the Kaiser and Irem/Sunsoft
  discrete boards add CPU-cycle IRQs; the MesenCE-only group adds the
  MMC3 208/219/258 outer/protection registers, the Drip 284 homebrew board,
  the TF1201 298 and the Kaiser 302/307/346 discrete boards; the next batch
  adds the $4022 board (43), the 1 KiB-CHR IRQ board (106), the $7777
  read-back (170), Street Heroes (262), A65AS (285), SMB2j (304) and
  DreamTech 01 (521). The next batch completes the TEC9719 transform for
  126/422/534 and adds Subor 167, Magic Kid Goo Goo (190), City Fighter IV
  (266) and the BMC 64-in-1 (314). Mapper 292 adds the CPU-RAM protection
  latch and mapper 518 the cart-RAM window. Mapper 183 reuses the VRC4 core
  with its `$6000` PRG window, while 487 and 530 add the Maxivision and
  Ax5705 latches. The next batch adds the Maxivision 493, karaoke 517,
  UNL-BJ-56 526 and Sachen 3013 553 boards. The Caltron 389 and the
  Jncota 523 FS005 variant complete the last batch. The latest additions
  cover Lucky Rabbit 415, Fine Studio 417, Super Soccer 538 and Kid Icarus
  539. The SB-5013 core now serves 359 and its 2 KiB-CHR 540 variant; the
  latest pair adds 8-BIT XMAS 2017 (400) and Waixing FS306 (544), followed
  by TH2131-1 (308), Yung-08 (368), INX_007T (470) and the Techline
  351 MMC3 mode, plus the 380 address-latched multicart, the 413
  BATMAP-SRR-X, the 525 KS7021A VRC4 conversion, the 549 KS7016B latch, the
  515 Noraebang board, the 554 read-triggered conversion, the 374 reset-based
  MMC1 multicart, the 446 SMD172B supervisor, whose `$5000` table maps onto
  the UNROM/CNROM/MMC3/AxROM/GNROM/NINA/VRC1/VRC2-4/VRC6/152/189/H3001
  cores, the 334 32 KiB-bank multicart and the 394 HSK007 256 KiB-slice
  multicart, plus the 272 J-2012-II, 418 NINA-118, 127 Double Dragon pirate,
  186 Study Box, 326 Gryzor, 557 NTDEC, 330 Sangokushi II, 555 NES-EVENT2 and the
  564/576/585 latch boards, and the 559/563 VRC boards with the shared A12
  counter and per-quadrant CHR-ROM nametables, and the 405 UM6578 enhanced
  console model (10 KiB VRAM, word name tables, 4 bpp backgrounds, DMA).
  The 256 OneBus
  core is partial: all five images load and boot, two reach video by 120
  frames. At 120 frames, 16 of the MesenCE group's 18 local images render
  (the two blanks are MesenCE-blank boot screens too); the 20-frame audit
  only catches 208/258 because the rest are still in their boot code; the Sachen
  3009/3018 and JV001 latches
  cover the A8-decoded `$4100-$5FFF` window; Nitra decodes the MMC3 protocol
  from the write address; Datach reuses the Bandai FCG core with the scanner
  window; and the MMC3 clones (123, 187, 198, 245) override PRG/CHR mapping
  while delegating the IRQ. The later additions cover VRC4/27, the
  Irem/Sunsoft/Jaleco discrete boards (65/67/72/78), UNL-117, the Racermate
  periodic IRQ, the KOF '97 value scramble (263), the Yoko paging modes (264),
  the Super 40-in-1 WS latch (332), the Taito X1-017 (552), the reset-advancing
  TXROM multicart (313), the Eh8813A (519), the T230 VRC-like board (529) and
  the BMC GK 6-in-1 (57), the K-1029 100-in-1 (15), the FDS-conversion
  protection board (120) and the Sachen SA-020A on the SA-006 PCB (243). The
  A5 batch adds the NES-EVENT/NWC (105), Cufrom 32 KiB CHR-RAM (29), Oeka
  Kids (96), Bandai LZ93D50+WRAM (153), the Golden Five/BMC-8157 pair
  (104/301), Magic Floor (218), the NTDEC/BMC trio (221/229/289), Waixing
  VRC4 (252), Asder NTD-03 (290), Sachen 9602 (513) and JY043 (122).
  The two Dragon Ball Z Gaiden (529) images poll an undocumented `$5000`
  protection port and remain blank at the title; the other T230 images render.
  Differential results: 246 and 228 are
  pixel-identical to the oracle on sampled frames, 70/74/119/133/140/147/149/
  180/187/191/192/194/225/245/250 match 113-119/120, and 93/107 match exactly
  once the known frame-origin drift is accounted for. 123 and 187 follow the
  Mesen reference because the RustyNES oracle falls back to a different board
  for them.
- **MMC5 (5, partial)** — the CPU-visible register file covers the unsigned
  8×8 multiplier (`$5205`/`$5206`), PRG-RAM protect (`$5102`/`$5103`), the
  `$5113` 8 KiB PRG-RAM bank, the `$5114-$5116` RAM/ROM toggle, ExRAM CPU
  access modes (`$5104`, `$5C00-$5FFF`), `$5130` upper CHR bits and the
  `$5204` IRQ-pending read. A shared ExRAM/CHR state now feeds the PPU:
  `$5105` maps each of the four nametable slots independently to CIRAM page 0,
  CIRAM page 1, ExRAM or fill mode (ExRAM is a nametable only in modes 0/1,
  as on hardware; modes 2/3 read the slot as zeros). Fill mode and the
  1 KiB CHR bank map are consulted per fetch so mid-frame `$5120` writes take
  effect. Vertical split-screen (`$5200-$5202`) substitutes ExRAM
  nametable/attribute data and the `$5202` 4 KiB CHR bank per fetch group, with
  the split fine Y driving CHR A0-A2. Extended-attribute mode (`$5104` = 1)
  takes the palette from ExRAM bits 6-7 and the 4 KiB CHR bank from bits 0-5
  for every background tile, with `$5130` supplying the two upper bank bits.
  Pattern fetches carry a physical byte offset rather than a 16-bit PPU
  address, so CHR above 256 KiB is addressable (tested with a 512 KiB image;
  a 512 KiB CHR title matches the reference on 140/300 frames).
  8x16 sprite mode keeps separate background banks:
  `$5128-$512B` drive background fetches while `$5120-$5127` drive sprites.
  The PCM DAC runs in both modes: `$5011` writes in write mode and
  `$8000-$BFFF` PRG reads in read mode (`$5010` bit 0), and a zero update
  trips the `$5010` IRQ, which is ORed onto the mapper IRQ line.
  11/37 render at 20 frames, 17/37 at 600 (one boot-time title is blank in
  ExRAM modes 2/3 now that the slot correctly reads zeros).
- **Famicom Disk System (20)** — raw `.fds` loader plus a BIOS (both
  user-supplied): the 8 KiB BIOS maps at `$E000-$FFFF`, 32 KiB work RAM at
  `$6000-$DFFF`, and `$4020-$4097` carries the disk timer IRQ, disk/sound
  enables, control/status registers and the FDS audio. Reads are delivered
  from a synthesized wire image (lead-in/inter-block gaps, `$80` start marks,
  per-block CRC-16/KERMIT) at the ~149-cycle byte cadence; the byte-transfer,
  end-of-head and drive-ready flags follow the documented `$4030`/`$4032`
  behaviour, and a motor stop rewinds the head to the start of the side. The
  real BIOS boots a local commercial FDS title to the licence screen and, after
  Start, into the game (Super Mario Bros. title screen renders). The disk write
  path now handles write-mode transfers: each `$4024` byte is landed on the
  wire image at the head and advances it, with the byte-transfer flag/IRQ
  raised as in read mode. Per-block CRC-16 is regenerated after every stored
  byte, and `Cartridge::save_disk_image` flushes modified payloads back into
  the raw `.fds` sides for persistence. Multi-side images are parsed from the fwNES
  header and can be swapped at runtime (`Cartridge::insert_disk_side` /
  `System::insert_disk_side`), which resets the head, re-opens the spin-up
  window and rebuilds the wire image for the new side.
- **VRC6 audio (24/26)** — the two pulse channels (16-step duty generator,
  `$9003` halt/16x/256x frequency scaling) and the sawtooth (14-step
  accumulator reset) feed a linear 6-bit DAC (two 4-bit pulses + the saw's
  high 5 bits). Unit test covers duty peaks, the mode bit, channel summing and
  the saw ladder.
- **MMC5 audio (5)** — two APU-style pulse channels (`$5000-$5007`) using the
  7-bit MMC5 length table, a fixed ~240 Hz envelope/length clock and four duty
  sequences, plus the held 8-bit `$5011` PCM DAC. No sweep unit. Unit test
  covers the pulse peak, both channels summing and length expiration.
- **VRC7 (85)** — 8 KiB PRG banks with the last fixed, eight 1 KiB CHR
  banks, `$E000` mirroring/WRAM-enable (`$80`) and sound-silence (`$40`), and
  the VRC cycle or scanline IRQ. CHR-RAM boards share a banked 8 KiB window
  with the PPU, so Lagrange Point's runtime CHR uploads and 1 KiB switches are
  visible to rendering. Expansion audio is a six-channel, two-operator FM
  engine with the custom patch registers `$00-$07`, `$9010`/`$9030`
  selection, key-on/key-off, octave/frequency, per-channel instrument/volume
  and the exact VRC7 instrument ROM patch table (all 15 built-in patches,
  with the carrier waveform on bit 4 and the modulator waveform on bit 3 of
  `$03` as documented). Audio silence clears all
  state. **6/6** images load; 1/6 renders at 20 frames and 2/6 at 600 (the
  CHR-RAM title now renders; the others are Tiny Toon variants whose board
  lacks the audio resonator and whose boot screens need more frames). The
  CHR-RAM title matches the reference video on 296/300 frames.
- **Sunsoft 5B audio (69)** — YM2149F with three tone channels (CPU/16
  prescaler), the 17-bit/13-tap LFSR noise generator, the 32-step envelope
  generator and the `$07` tone/noise mixer; `$C000` selects an internal
  register and `$E000` writes it. Unit test covers tone toggling, both
  channels summing, constant-volume mixing and noise.
- **Namco 163 audio (19)** — up to eight wavetable channels stored in the
  shared `$40-$7F` chip RAM, updated one channel per 15 CPU cycles with a
  24-bit phase accumulator, 18-bit frequency, waveform length/offset and the
  `$E000` sound-disable bit. `$F800` address-port auto-increment is honoured
  for data writes. Unit test covers a $F sample reaching the expected
  `105/256` output and the disable bit.
- **Namco 163 nametables/IRQ (19)** — `$C000-$D800` map each 1 KiB nametable
  quadrant to CIRAM (`$E0`/`$E1`) or a CHR-ROM page (below `$E0`), with the
  power-on layout following the header mirroring; the shared `N163Nt` state is
  wired into the PPU's nametable reads/writes, and `$E800`-enabled `$E0+` CHR
  banks read the same CIRAM. The `$4800-$5FFF` registers (sound port and IRQ
  counter) are routed to the cartridge in the `$4020-$5FFF` window, which games
  poll. Corpus: **33 of 34 mapper-19 images render at 600 frames**; the sweep
  titles improved to Star Wars 59/60, Hydlide 3 59/60, Kaijuu Monogatari
  58/60, Final Lap 58/60, Youkai Douchuuki 59/60, Mindseeker 58/60. CHR bank
  writes at `$8000-$BFFF` now refresh the PPU's pattern window immediately, so
  raster bands that switch banks mid-frame render correctly; Rolling Thunder
  matches the reference on 275 of 276 frames once the known one-cycle frame
  origin is aligned, leaving only the boundary transition frame (it previously
  rendered the wrong bank set below the split).
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

14 images (0.2%) name a mapper with no implementation across 4 IDs:
355/547/594 (4 images each) and 169 (2). The sections below record the reference material
for each ID and what still blocks it.

### NESdev references

Specs and notes for the mapper IDs covered by this audit. "Partial" means the
board loads and boots but does not render, or renders only some images.

| Mapper | Notes | Status | NESdev reference |
|---:|---|---|---|
| 127 | pirate Double Dragon II | implemented | [INES Mapper 127](https://www.nesdev.org/wiki/INES_Mapper_127) |
| 169 | Yuxing 1 MiB PRG | unimplemented | [INES Mapper 169](https://www.nesdev.org/wiki/INES_Mapper_169) |
| 186 | Fukutake Study Box | implemented | [INES Mapper 186](https://www.nesdev.org/wiki/INES_Mapper_186) |
| 256 | VT02+/VT03 OneBus | partial (2/5 render) | [NES 2.0 Mapper 256](https://www.nesdev.org/wiki/NES_2.0_Mapper_256) |
| 272 | J-2012-II bootleg | implemented | [NES 2.0 Mapper 272](https://www.nesdev.org/wiki/NES_2.0_Mapper_272) |
| 326 | Gryzor/Contra bootleg | implemented | [NES 2.0 Mapper 326](https://www.nesdev.org/wiki/NES_2.0_Mapper_326) |
| 330 | Sangokushi II bootleg (N163) | implemented | [NES 2.0 Mapper 330](https://www.nesdev.org/wiki/NES_2.0_Mapper_330) |
| 334 | 5/20-in-1 1993 multicart | implemented | [NES 2.0 Mapper 334](https://www.nesdev.org/wiki/NES_2.0_Mapper_334) |
| 355 | 3D-BLOCK (PIC16C54) | unimplemented | [NES 2.0 Mapper 355](https://www.nesdev.org/wiki/NES_2.0_Mapper_355) |
| 374 | reset-based MMC1 multicart | implemented | [NES 2.0 Mapper 374](https://www.nesdev.org/wiki/NES_2.0_Mapper_374) |
| 394 | Realtec HSK007 | implemented | [NES 2.0 Mapper 394](https://www.nesdev.org/wiki/NES_2.0_Mapper_394) |
| 405 | UM6578 enhanced console | implemented | [NES 2.0 Mapper 405](https://www.nesdev.org/wiki/NES_2.0_Mapper_405) |
| 418 | 820106-C/821007C (NINA-118) | implemented | [NES 2.0 Mapper 418](https://www.nesdev.org/wiki/NES_2.0_Mapper_418) |
| 446 | Mindkids SMD172B supervisor | partial (1/4 render) | [NES 2.0 Mapper 446](https://www.nesdev.org/wiki/NES_2.0_Mapper_446) |
| 515 | Family Noraebang (YM2413) | implemented | [NES 2.0 Mapper 515](https://www.nesdev.org/wiki/NES_2.0_Mapper_515) |
| 525 | Kaiser KS7021A (VRC4) | implemented | [NES 2.0 Mapper 525](https://www.nesdev.org/wiki/NES_2.0_Mapper_525) |
| 547 | Konami Q-Ta / VRC5 | unimplemented | [NES 2.0 Mapper 547](https://www.nesdev.org/wiki/NES_2.0_Mapper_547) |
| 549 | Kaiser KS7016B | partial (blank) | [NES 2.0 Mapper 549](https://www.nesdev.org/wiki/NES_2.0_Mapper_549) |
| 554 | Kaiser KS-7010 | implemented | [NES 2.0 Mapper 554](https://www.nesdev.org/wiki/NES_2.0_Mapper_554) |
| 555 | Campus Challenge 1991 (NES-EVENT2) | implemented | [NES 2.0 Mapper 555](https://www.nesdev.org/wiki/NES_2.0_Mapper_555) |
| 557 | NTDEC 2718 | implemented | [NES 2.0 Mapper 557](https://www.nesdev.org/wiki/NES_2.0_Mapper_557) |
| 559 | Subor Sango II (VRC2/4) | implemented | [NES 2.0 Mapper 559](https://www.nesdev.org/wiki/NES_2.0_Mapper_559) |
| 563 | J-2020 (VRC2/4) | implemented | [NES 2.0 Mapper 563](https://www.nesdev.org/wiki/NES_2.0_Mapper_563) |
| 564 | bd23 data-latch | implemented | [NES 2.0 Mapper 564](https://www.nesdev.org/wiki/NES_2.0_Mapper_564) |
| 576 | J-2096 address-latch | implemented | [NES 2.0 Mapper 576](https://www.nesdev.org/wiki/NES_2.0_Mapper_576) |
| 585 | FE-01-1 address-latch | implemented | [NES 2.0 Mapper 585](https://www.nesdev.org/wiki/NES_2.0_Mapper_585) |
| 594 | Rinco Talkman (OKI ADPCM) | unimplemented | [NES 2.0 Mapper 594](https://www.nesdev.org/wiki/NES_2.0_Mapper_594) |

### Tail triage (IDs at 6-7 images)

| Mapper | Images | Board / notes | Next step |
|---:|---:|---|---|
| 157 | 7 | Datach Joint ROM (barcode reader) | blocked on the peripheral |
| 168 | 6 | Racermate Challenge II + exercise bike | low value; peripheral |
| 27 | 6 | Mi Hun Che / Super Car (32 KiB PRG, 8 KiB CHR) | undocumented; inspect |
| 85 | 6 | **VRC7** (Lagrange Point, Tiny Toon 2 JP) | implemented (exact instrument ROM patches) |
| 303 | 6 | Kaiser KS-7017 (Armana no Kiseki) | likely related to KS7032 |

Only **172** turned out to be an alias of an implemented board (Sachen JV001
with reversed data bits). The rest are real hardware models; the highest-value
the tail list is now covered except the Datach
board (barcode peripheral).

## Load/run failures

All 28 remaining failures are truncated containers (declared PRG+CHR exceeds
the payload), not emulation issues. The four mapper-173 panics remain fixed.

## Regression tooling

- `unko trace <rom> --frames N --out trace.txt` records per-frame
  video/audio hashes and the final state.
- `unko diff <a.trace> <b.trace>` reports the first divergent frame.
- `unko signatures <dir> [--frames N] [--mapper ID] [--check file]`
  hashes every frame of every ROM in a directory and compares a manifest.
- `cargo run -p nes_accuracy_core --example dump2007` dumps the `$2007`
  stress test's raw `$0500-$0654` array for answer-key comparison.

Manifests and traces are local-only and are never committed.
