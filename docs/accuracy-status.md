# Accuracy status

Measured on the reference-parity core (`nes_accuracy_core`, MIT with
documented ISC adaptations from ares). These are local runs
against public test ROMs placed in `test-roms/` (gitignored).

## A1 — 6502

| Test | Result | Evidence |
|---|---|---|
| `nestest` golden log | **8980 / 8980 lines**, registers + PC + SP + P + CYC all match | `cargo test -p nes_accuracy_core --test nestest -- --nocapture` |
| blargg `instr_timing` | **Passed** (official, NOP/SBC, unofficial) | `cargo test -p nes_accuracy_core --test blargg blargg_instr_timing -- --nocapture` |
| blargg `instr_basics` | **Passed** | `cargo test -p nes_accuracy_core --test blargg blargg_instr_basics -- --nocapture` |
| blargg interrupt suites | see A2/A3 | requires PPU NMI + APU/DMA timing; tests are `#[ignore]` |

The CPU implementation in `crates/nes_accuracy_core/src/cpu.rs` is
cycle-stepped: every cycle is a bus access or an explicit internal cycle, and
the nestest comparison includes the absolute cycle counter, not just registers.

The `instr_timing` runner uses a minimal APU frame/length-counter stub because
one calibration check times against the APU length period. The stub lives in the
test harness, not in the core.

## A2 — PPU

| Test group | Result |
|---|---|
| `vbl_nmi` suite | **12 / 12 Passed** |
| `ppu_sprites` (19 ROMs) | **19 / 19 passing** |

Sprite passing: all 11 `spr_hit_*`, `spr_overflow_basics/details/obscure/emulator/timing`,
`oam_read`, `oam_stress`, `sprite_ram`.

The PPU has registers, 2 KiB VRAM, palette mirroring, OAM, dot timing,
vblank/NMI, suppression, odd-frame skip, CHR-RAM, background rendering and a
first sprite-evaluation/sprite-zero/overflow implementation.

Sprite evaluation now runs during dots 65–256, one sprite every two dots, latches
the next line's sprites at dot 0, applies the OAM `Y = top - 1` rule, and
implements the overflow byte-scan bug. OAM DMA (`$4014`) is handled with its
513-cycle stall. 14/19 sprite ROMs pass; the remaining work is overflow timing,
the obscure scan, and OAM corruption/decay.

The NMI path now polls the 6502 penultimate-cycle latch, which fixed the last
two `vbl_nmi` ROMs (`nmi_suppression`, `nmi_timing`).

## Not yet covered

- NMI/IRQ edge and branch-delay semantics (`int_branch_delays_irq`,
  `int_nmi_and_irq`, `int_cli_latency`) — A2/A3.
- OAM/DMC DMA cycle stealing — A2/A3.
- PPU, APU, mappers, FDS/VS/PAL/Dendy, expansion audio, peripherals, NSF —
  A2+.


## A3 — APU (complete)

Implemented a full APU core (`apu.rs`): pulse 1/2 (duty, timer, envelope,
sweep), triangle (linear counter), noise (LFSR), DMC (buffer, shifter, DMA
requests, loop, IRQ), frame counter (4/5-step, write delay, immediate clock),
length counters with deferred reload/halt, mixer, and IRQ line.

| ROM | Result |
|---|---|
| `01.len_ctr` | pass |
| `02.len_table` | pass |
| `03.irq_flag` | pass |
| `04.clock_jitter` | pass |
| `05.len_timing_mode0` | pass |
| `06.len_timing_mode1` | pass |
| `07.irq_flag_timing` | pass |
| `08.irq_timing` | pass |
| `09.reset_timing` | pass |
| `10.len_halt_timing` | pass |
| `11.len_reload_timing` | pass |
| `dmc_basics` | pass |
| `dmc_rates` | pass |

Remaining A3 follow-up: mixer-level scoring against the reference. DMC DMA
cycle stealing is now covered by a system-level differential benchmark: a
synthetic NROM loops a 16-bit counter while the DMC loops a one-byte sample,
and the reference (RustyNES) and this core steal exactly the same 10,126 loop
iterations over 300 frames (64,388 without DMC vs 54,262 with DMC here,
64,390 vs 54,264 in the reference; the constant two-iteration start-up offset
is also present with DMC disabled). The `$4015` abort and OAM/DMC interleave
edge cases remain.


## A4 — mappers (started)

`cart.rs` now loads iNES / NES 2.0 images and exposes a `Mapper` trait with:

| Mapper | Status |
|---|---|
| NROM (0) | implemented (16 KiB mirror / 32 KiB), CHR-ROM and CHR-RAM |
| MMC1 (1) | implemented (5-bit serial, PRG/CHR modes, mirroring, PRG-RAM) |
| others | rejected with `UnsupportedMapper` |

Unit tests cover header parsing, NROM PRG mapping, 16 KiB mirroring, CHR-RAM
writes, and MMC1 bank switching.

## A4 — mappers, system bus, Core ABI (in progress)

### Mappers

| Mapper | Status |
|---|---|
| NROM (0) | complete (16/32 KiB PRG, CHR-ROM/RAM) |
| MMC1 (1) | complete (serial banking, PRG/CHR modes, mirroring, PRG-RAM) |
| UxROM (2) | complete (16 KiB switchable + fixed last) |
| CNROM (3) | complete (CHR banking) |
| MMC3 (4) | PRG/CHR banking, mirroring, scanline IRQ clocked at the PPU A12 rise (dot 260) |
| AxROM (7) | 32 KiB banking, single-screen mirroring, bus conflicts |
| VRC2/4 (21/22/23/25) | PRG/CHR banking, mirroring, scanline IRQ; audio missing |
| Color Dreams (11) | 32 KiB PRG / 8 KiB CHR banking, bus conflicts |
| CPROM (13) | 16 KiB CHR-RAM, switchable upper 4 KiB |
| GxROM (66) | 32 KiB PRG / 8 KiB CHR banking |
| Camerica (71) | 16 KiB banking + fixed last, mirroring |
| UNROM-512 (30) | 16 KiB switchable + fixed last 16 KiB, 8 KiB CHR-RAM, one-screen/vertical/horizontal |
| BNROM/NINA-001 (34) | 32 KiB PRG banking (BNROM bus conflicts) or NINA-001 32 KiB PRG + 4 KiB CHR |
| Sachen SA-015 (150) | indexed register pair `$4100`/`$4101`: 32 KiB PRG, 8 KiB CHR, mirroring |
| Daou DIS23C01 (156) | 16 KiB PRG banking, eight split-register 1 KiB CHR banks, 8 KiB WRAM, runtime H/V mirroring |
| Waixing FS309 (199) | MMC3 + four extra PRG/CHR bank registers, four mirroring modes, 8 KiB CHR-RAM and a 4 KiB `$5000-$5FFF` PRG-RAM window (6/6 render) |
| Waixing T9552 (253) | 8 KiB PRG + split-nibble 1 KiB CHR banking, four mirroring modes, /114 CPU-clock IRQ, 2 KiB CHR-RAM page pair |
| Waixing / Myth Struggle (249) | MMC3 + a `$5000` page-scramble register for CHR/PRG banking (6/6 render) |
| Maxi 15 multicart (234) | two bus-conflict latches, CNROM/NINA-03 banking, 32 KiB PRG / 8 KiB CHR, register reads (6/6 render) |
| Waixing educational (164) | 32 KiB PRG bank from `$5000`/`$5100` nibble writes, 8 KiB CHR-RAM (3/6 render) |
| Dragon Ninja CTC-31 (222) | eight 1 KiB CHR banks, two switchable 8 KiB PRG banks + fixed last 16 KiB, A12-rise IRQ (6/6 render) |
| 4-in-1 multicart (60) | soft-reset-selected 16 KiB PRG / 8 KiB CHR bank, four games (6/6 render) |
| Bonza / Magic Jewelry II (216) | write-address-selected 32 KiB PRG / 8 KiB CHR page, `$5000` register read (4/6 render) |
| Sachen SA-008-A (148) | mapper 79 latch moved to `$8000-$FFFF`, bus conflicts |
| Gouder SL-1632 (14) | Huang-1 dual ASIC: VRC2b at power-on or MMC3, selected by the `$A131` PAL supervisor, which also adds CHR A18 for three PPU windows |
| Jaleco SS88006 (18) | VRC4-like 8 KiB PRG / 1 KiB CHR nibble-pair banking, CPU-cycle IRQ |
| NINA-03/06 (79/146) | 32 KiB PRG + 8 KiB CHR via a single control register |
| DxROM (206/76/88/95/154) | MMC3-style 8 KiB PRG / 1 KiB CHR banking, no IRQ; `$C000`/`$E000` fixed |
| Nanjing FC-001 (163) | 32 KiB PRG banking via `$5000`/`$5200`/`$5300`, CHR-RAM, feedback register |
| Sunsoft FME-7 (69) | 8 KiB PRG + 1 KiB CHR banking, mirroring, CPU-cycle IRQ |
| Namco 129/163 (19) | 8 KiB PRG / 1 KiB CHR banking, NT-as-CHR, 15-bit CPU-cycle IRQ, 128-byte chip RAM (audio missing) |
| Action 53 (28) | reference inner/outer PRG bank formula, 32 KiB CHR-RAM window, H/V/1-screen mirroring |
| J.Y. ASIC (35/90/209/211) | 8/16/32 KiB PRG, 1/2/4/8 KiB CHR, multiplier, prescaled M2/A12 IRQ, per-quadrant nametables mixing CIRAM pages with 1 KiB CHR-ROM pages (`$B000-$B007`, `$D000` bit 5, `$D001` extended mirroring) |
| Waixing FS305 (178) | 8 KiB CHR-RAM, NROM-256/UNROM/NROM-128/UNROM-fixed PRG modes, banked 8 KiB PRG-RAM |
| J87 (87/101) | 32 KiB fixed PRG, 8 KiB CHR latch at `$6000-$7FFF` (87 swaps register bits 0/1); mapper 101 uses the raw value and reverses CHR bit order |
| J.Y. clone (91) | two 8 KiB PRG banks + fixed last, four 2 KiB CHR banks, outer 128 KiB PRG/512 KiB CHR, 64-rise A12 IRQ |
| Bandai FCG (16) | 16 KiB PRG, eight 1 KiB CHR, mirroring, 16-bit CPU-cycle IRQ latch/counter |
| Bandai FCG 153 | `$8000-$8007` CHR registers double as the outer PRG bank, sampled as `chr[PPU addr>>10 & 3]` on PPU fetches; 8 KiB WRAM |
| NTDEC 2291 (63) | address latch: 8-bit `addr>>2` PRG bank, `addr&2` 32 KiB mode, `addr&1` mirroring, `addr&0x400` CHR-RAM write protect, open bus past the ROM |
| TXC 112 | scrambled MMC3-like 8 KiB PRG + 2/1 KiB CHR banking, mirroring |
| BxROM+WRAM (241) | 32 KiB PRG banking without bus conflicts plus battery PRG-RAM |
| NROM/CNROM multicart (58/213) | PRG/CHR bank, NROM-128/256 and mirroring latch at `$8000-$FFFF` |
| TXC latch (132/173) | three-bit shift/invert latch with `$4100` readback; 132 banks PRG and CHR, 173 banks CHR |
| Hengge BxROM (177) | 32 KiB PRG banking, mirroring and 8 KiB banked PRG-RAM |
| CNROM-185 | 8 KiB CHR CNROM with `$8000` latch and bus conflicts |
| MMC3 32 KiB (189) | MMC3 CHR/IRQ/mirroring plus a 32 KiB PRG latch at `$4100-$5FFF` |
| CoolBoy/MINDKIDS (224/268) | oversize MMC3 clone with outer PRG masks, 2/1 KiB CHR banking, IRQ |
| Enhanced MMC3 8025 (176) | standard MMC3 plus PRG/CHR base registers, NROM-128/256 and UNROM compatibility modes |
| Sugar Softec (215) | scrambled MMC3 registers (8 address/data patterns), outer PRG/CHR banks, NROM override |
| Kasheng MMC3 (115) | MMC3 clone with NROM-mode PRG override and 256 KiB outer CHR bank |
| Camerica Quattro (232) | mapper 71 plus a 64 KiB outer block register (BF9096) |
| Namco 175/340 (210) | 8 KiB PRG + 1 KiB CHR banking, last-bank fixed, mirroring select on 340 |
| Kasheng protected MMC3 (121) | MMC3 plus read-back protection array, bit-reversed bank latch and outer 256 KiB bank (partial; some hacks still fail) |
| GTROM / Cheapocabra (111) | 32 KiB PRG pages via `$5000-$5FFF` / `$7000-$7FFF` (32 KiB PPU-RAM banking not yet modelled) |
| Cony/Yoko ASIC (83) | PRG base + mode, 4x8 KiB PRG windows, 8x1 KiB CHR, scratch RAM, up/down IRQ |
| MMC3 variant (45) | standard MMC3 banking with the 1 MiB PRG board's mapper number |
| Taito TC0190 (33) | two 8 KiB PRG banks, six CHR windows (2x2 KiB + 4x1 KiB), mirroring |
| Konami VRC1 (75) | three 8 KiB PRG banks + fixed last, two 4 KiB CHR banks with high bits |
| Taito X1-005 (80) | three 8 KiB PRG banks, six CHR windows, 128 B internal RAM, RAM-enable register |
| Address-latch multicart (242) | write address selects outer/inner PRG, mirroring and NROM/UNROM modes |
| Tengen RAMBO-1 (64) | 16-entry register file, CHR A12 inversion, 1 KiB CHR mode, scanline/CPU-cycle IRQ |
| Sunsoft-1 (184) | 32 KiB fixed PRG plus two 4 KiB CHR banks from a `$6000-$7FFF` latch |
| TxSROM (118) | MMC3 with single-screen nametable selected by the CHR bank bit |
| Address-latch multicart (227) | mapper 242's latch with a third outer PRG bit (A19) |
| Address-latch multicart (59) | 8 KiB CHR, mirroring, 16 KiB PRG and NROM-128/256 mode from the write address |
| RAMBO-1 mirroring (158) | mapper 64 with single-screen mirroring from the CHR bank bit |
| X1-005 single-screen (207) | mapper 80 with hardwired single-screen nametables |
| MRCM MMC3 clone (196) | MMC3 with the CPU A0 input remapped to A1/A2/A3 |
| HP-898F (319) | two 16 KiB PRG windows, mirroring and 8 KiB CHR bank from `$6000`/`$6004` |
| FDS conversions (108) | 8 KiB bank at `$6000-$7FFF`, fixed last 32 KiB, CHR-RAM/ROM variant |
| Konami VRC6 (24/26) | 16 KiB + 8 KiB PRG, eight 1 KiB CHR banks, PPU banking/mirroring, VRC cycle/scanline IRQ, two-pulse + sawtooth expansion audio |
| Konami VRC7 (85) | 8 KiB PRG, eight 1 KiB CHR banks, shared banked CHR-RAM window, mirroring/WRAM-enable, VRC cycle/scanline IRQ, six-channel two-operator FM audio with the documented VRC7 instrument ROM patches |
| Irem G-101 (32) | two 8 KiB PRG banks with switchable/fixed pair, eight 1 KiB CHR banks, mirroring/PRG-mode |
| Taito TC0690 (48) | TC0190-style PRG/CHR banking plus inverted-reload MMC3-style IRQ |
| Jaleco JF-13 (86) | GNROM-style 32 KiB PRG / 8 KiB CHR latch at `$6000-$6FFF`; NES 2.0 submapper 1 bootlegs run a software-clocked HC-55516 CVSD decoder over the miscellaneous ROM (`$5000` rate, `$7000` message), submapper 0 boards have no speech data in the dump |
| TXC 01-22000 (36) | 2-bit latch/inverter PRG select plus an 8 KiB CHR latch at `$4200` |
| CK-001 4-in-1 (301) | mapper-242 address latch plus the ET-113 512 KiB / 128 KiB chip select; 16 KiB switchable + fixed-bank windows, NROM modes, power-on last 32 KiB page |
| 09-034A / LE10 (304) | 8 KiB PRG bank 4/5 at `$6000` from `$4027` bit 0, fixed first 32 KiB at `$8000-$FFFF`, direct `/IRQ` from `$4068` bit 0; the two loadable 48 KiB LE08/LF11 images render 2/2 at 120/240, while the SMB2 LE10 dumps carry corrupt 864 KiB headers and fail to load |
| KS-7031 (305) | 32 KiB PRG at `$8000-$FFFF` with its 16 x 2 KiB pages reversed, four switchable 2 KiB `$6000` windows |
| NTDEC 2131 (308) | VRC2b PRG/CHR banking, 8 KiB WRAM at `$6000`, and the NTDEC level IRQ (bit-12 half period plus a high-nibble down-counter) |
| Whirlwind LH51 (309) | 8 KiB WRAM at `$6000`, switchable `$8000` bank, fixed last three banks, `$F000` mirroring |
| SA-9602B (513) | MMC3 core whose outer PRG bits come from R0 and apply to `$8000`/`$A000` only |
| N-32 SMB2j conversion (50) | `$6000`=bank 15, `$8000`/`$A000`=8/9, `$C000` from the scrambled `$4020` register, `$E000`=11; 13-bit CPU-cycle IRQ with inverted line polarity |
| Nintendo NES-QJ (47) | MMC3 with a `$6000` outer register whose high nibble is added to every PRG slot (`syncPRG(0x0F, reg<<4)`) and CHR A17-A18; renders by 120 frames |
| ING005 (52) | MMC3 with `prgAND = reg&8?0x0F:0x1F`, `prgOR = (reg<<4)&0x70`, `chrAND = reg&0x40?0x7F:0xFF`, `chrOR = (reg<<3)&0x180 | (reg<<7)&0x200`, locked by reg bit 7 |
| FDS conversions (42) | `$6000` switchable 8 KiB PRG-ROM bank (`$E000`), fixed last 32 KiB at `$8000-$FFFF`, `$8000` CHR bank, `$E001` mirroring, and the free-running 15-bit CPU-cycle timer enabled by `$E002 = $02`; the patched BIOS acknowledges it through `$4030`. All nine Ai Senshi Nicol images render by 120 frames. |
| SOMARI-P (116) | power-on MMC3 mode of the 3-mode VRC2/MMC3/MMC1 ASIC |
| Waixing FS304 (162) | 32 KiB PRG banking from `$5000`/`$5100`/`$5200`/`$5300`, 8 KiB CHR-RAM |
| HES NTD-8 (113) | 32 KiB PRG / 8 KiB CHR / mirroring control register at `$4100-$5FFF` |
| SuperGame MMC3 (114) | scrambled MMC3 address/index map, NROM PRG override, 256 KiB outer CHR |
| Bus-conflict 152 | 16 KiB PRG / 8 KiB CHR / one-screen mirroring latch at `$8000-$FFFF` |
| Multicart 226 | 7-bit PRG latch at `$8000`/`$8001`, 32/16 KiB mode, mirroring |
| GNROM 240 | 32 KiB PRG / 8 KiB CHR latch at `$4020-$5FFF` plus PRG-RAM |
| Kaiser KS7032 (142) | four 8 KiB PRG windows, 8 KiB CHR-RAM, VRC3-like 16-bit cycle IRQ (partial; Kaiser FDS ports still blank) |
| Yancheng YC-03-09 (558) | 32 KiB PRG banking from `$5000`/`$5100`/`$5300`, 8 KiB CHR-RAM, D0/D1 swap |
| Multicart 212 | write-address latch: 16/32 KiB PRG, 8 KiB CHR, mirroring, DIP-switch read |
| Protected NROM (143) | NROM plus a `$4100-$5FFF` protection read of inverted low address bits |
| Golden Game 235 | write-address latch: 32 KiB page, 16 KiB half, ROM chip, 1/2-screen mirroring |
| Benshieng BMC-BS-5 (286) | address latches: 8-bit PRG pages (DIP-gated address bit 4), 2 KiB CHR pages, `$C000` mirroring; all 10 images render |
| MMC5 (5) | PRG modes/CHR banking/IRQ, multiplier, per-slot `$5105` nametable mapping (CIRAM page 0/1, ExRAM, fill), split screen, extended attributes with >256 KiB CHR, two-pulse + PCM audio (write/read DAC modes with the `$5010` IRQ) |
| Konami Q-Ta (547) | VRC5-style 8 KiB PRG windows, 4 KiB work RAM at `$6000`/`$7000`, cycle IRQ, JIS glyph lookup, and QTRAM: shadow 2 KiB nametables whose tile byte selects CHR-ROM pages or 4 KiB CHR-RAM banks, with the shadow writable through `$D010` |
| Rinco FSG2 (594) | MMC3 core with outer 256 KiB PRG/CHR banks, 8 KiB PRG-ROM at `$6000`, and a 1024-byte MSM6585 ADPCM FIFO at `$5000` (rate at `$5001`) mixed into the audio stream |
| UMC UM6578 (405) | eight readable 4 KiB PRG windows, timer IRQ, CPU/PPU DMA (A15-preserving targets); enhanced video: 2/4-bpp background with 32-byte VT03 planar tiles, 16-bit word name tables, 64-entry palette, 2-bpp sprites with `$2008` pattern page, 10 KiB internal plus 32 KiB external VRAM |

NES 2.0 PRG (16 KiB units) and CHR (8 KiB units) sizes are parsed correctly.

Educational-computer mappers are modelled at the board level: mapper 169
implements the MMC3 game mode, the 32 KiB PRG/CHR-RAM windows, the 1-bpp and
nametable-split graphics modes and the keyboard matrix, but the two available
dumps are BIOS images with no FDS media, so their lessons cannot be exercised
without the missing disk images.

The UM6578 enhanced video path is verified against `colortest-sh6578` from
the SH6578 documentation project: the 16-colour background test, all four
background palettes and the sprite palette swatches render with the expected
text and colours. The international `City Patrolman` dump boots and its DMA
runs, but its screen is not coherent (MAME also marks the game as not
working), so it is recorded as a title-specific failure rather than a
missing mapper feature.

### System bus

`system.rs` implements `CpuBus` over cartridge + CPU + PPU + APU:

- full NES memory map, OAM DMA (`$4014`) with cycle stall
- DMC sample fetches from CPU memory with a stall
- controller shift registers (`$4016`/`$4017`)
- PPU/APU advanced three dots per CPU cycle
- CHR-ROM synced from the mapper per frame; CHR-RAM left to the PPU so
  runtime tile uploads survive
- frame rendering and downsampled audio capture

### Core ABI

New crate `nes_accuracy_core` implements `unko_core::Core`
(id `nes_accuracy_core`), the only registered NES core. The CLI can select it
explicitly with `--core nes_accuracy_core`.

### Commercial verification

Rendered with the accuracy core (300 frames, 256x240):

| Title | Mapper | Result |
|---|---|---|
| Super Mario Bros. | NROM | title screen |
| Contra | UxROM | title screen |
| Castlevania | UxROM | title screen |
| Mega Man 2 | MMC1 + CHR-RAM | Capcom screen |
| Super Mario Bros. 3 | MMC3 | title screen (correct, with scanline IRQ) |
| Castlevania III (US) | MMC5 | boots and runs the intro (mode 2 mapping fixed) |

Corpus smoke run (`corpus --frames 25 --limit 1000`):

```text
908 loaded, 594 rendering, 314 blank, 86 unsupported mapper,
6 load/run failures (1000 scanned, 25 frames each)
mapper   0: 106 rendering, 27 blank
mapper   1:  98 rendering, 73 blank
mapper   2: 106 rendering, 20 blank
mapper   3:  34 rendering,  6 blank
mapper   4:  81 rendering, 45 blank
mapper   5:   1 rendering,  2 blank
mapper   7:  13 rendering,  5 blank
mapper  11:   3 rendering,  8 blank
mapper  21:   2 rendering,  3 blank
mapper  30:  22 rendering, 39 blank
mapper  34:  18 rendering,  0 blank
mapper  66:   2 rendering,  0 blank
mapper  71:   8 rendering,  0 blank
mapper  79:   6 rendering,  8 blank
mapper  69:   1 rendering,  7 blank   (Batman title renders by ~600 frames)
mapper 150:   0 rendering,  0 blank   (Sachen titles render; none in this slice)
mapper 163:   0 rendering,  2 blank   (initialises past 25 frames)
mapper 206:   5 rendering,  0 blank
mapper  19:   1 rendering,  0 blank
mapper  28:   8 rendering,  4 blank
mapper  90:   5 rendering,  0 blank
mapper  16:   1 rendering,  0 blank
mapper  58:   2 rendering,  8 blank
```

Adding mappers 34/79/146/206 raised loaded ROMs 643→680 and rendering
470→480; UNROM-512 (30) and Nanjing 163 raised them to 743/505; adding Sunsoft
FME-7 (69) and Sachen SA-015 (150) reached 753/508; Jaleco SS88006 (18) and
Sachen SA-008-A (148) reached 763 loaded / 516 rendering; Namco 163 (19),
Action 53 (28), the J.Y. ASIC (90/209/211) and Waixing FS305 (178) reached
789 loaded / 530 rendering; J87 (87/101), the J.Y. clone (91), Bandai FCG (16),
TXC 112 and BxROM+WRAM (241) reached 793 loaded / 532 rendering; the
NROM/CNROM multicart (58/213), TXC latch (132/173), Hengge BxROM (177) and
CNROM-185 reached 809 loaded / 536 rendering, with unsupported mappers
falling 351→314→251→241→231→205→201→185→181→175→172→167→166→160→155→150→146→141→140→138→133→132→127→124→121→112→111→107→95→92→86.

The SHA/SHX/SHY/SHS unstable stores now also corrupt the store address high
byte on a page crossing (`high &= stored value`), which moved the AccuracyCoin
SH* failures from the address subtest to the DMA-collision subtest tied to the
known DMC timing gap. MMC3-32K (189) and the
CoolBoy/MINDKIDS ASIC (224/268) are dispatched and boot their multicart
menus; the enhanced MMC3 8025 (176) followed, taking the sample to
819 loaded / 536 rendering; the Sugar Softec scrambled MMC3 (215) reached
822 loaded / 537 rendering; the Kasheng MMC3 (115), Camerica Quattro (232),
Namco 175/340 (210), Kasheng protected MMC3 (121), GTROM (111), the Cony/Yoko
ASIC (83) and MMC3-45 reached 839 loaded / 548 rendering; the Taito TC0190
(33) and Konami VRC1 (75) reached 844 loaded / 552 rendering; the Taito
X1-005 (80) and the address-latch multicart (242) reached 848 loaded /
556 rendering; Tengen RAMBO-1 (64) and Sunsoft-1 (184) reached 853 loaded /
561 rendering; TxSROM (118) reached 854 loaded / 562 rendering, and the
larger address-latch multicart (227) reached 856 loaded / 564 rendering; the
mapper-59 multicart latch reached 861 loaded / 569 rendering; the RAMBO-1
mirroring (158) and X1-005 single-screen (207) variants reached 862 loaded /
570 rendering; HP-898F (319) and the FDS-conversion board (108) reached
867 loaded / 571 rendering; Konami VRC6 (24/26) reached 870 loaded /
571 rendering; Irem G-101 (32), Taito TC0690 (48) and Jaleco JF-13 (86)
reached 873 loaded / 573 rendering; TXC 01-22000 (36), FDS conversions (42)
and the SOMARI-P board (116) reached 882 loaded / 576 rendering; Waixing
FS304 (162) and HES NTD-8 (113) reached 883 loaded / 577 rendering; the
SuperGame MMC3 clone (114) reached 887 loaded / 579 rendering; mappers
152, 226 and 240 reached 899 loaded / 588 rendering; Kaiser KS7032 (142)
reached 902 loaded / 588 rendering (its FDS ports still boot blank); the
mapper-212 multicart latch reached 908 loaded / 594 rendering. Many multicart titles fall in the 25-frame blank
window, so the rendering count lags the loaded count. Mapper 163 and 69 titles need
more than the 25-frame smoke budget to draw (verified at ~600 frames: Diablo,
Chrono Trigger and Batman render their title screens; Auto-Upturn and Chess
Academy render at 300 frames).

Full-library mapper audit (7,101 images, 20 frames, four 2,000-entry batches):
7,073 loaded (99.6%), 3,896 rendering (54.3%), 3,177 blank (45.3%), 0
unsupported (0.0%) and 28 load/run failures (0.39%). The batched totals
reproduce the single-pass numbers exactly. No mapper IDs remain unsupported.
A 120-frame pass renders 6,490 with 583 blank; a 240-frame pass renders
6,420 with 653 blank. The 240 total is slightly lower because several
hundred games sit on a fade or transition frame at that exact sample;
across both passes every loaded image renders except mapper 308 (NTDEC
Batman), which waits for Start. The 120-frame pass attributes 2,556 of the 20-frame blanks to
slow boot (MMC3 +654, MMC1 +424, UxROM +254, mapper 163 +69).
The follow-up work closed the original 18-ID never-rendering set:
121/303/13/12/14/50/301/305/309/513 render by 120 frames, 42 renders
9/9 at 120 after its `$6000` window and `$4030` timer acknowledge, 47
renders by 120 after its outer-bank fix, 52 at 20, and 63/153 by 120
after the NTDEC 2291 latch and PPU-driven FCG outer-bank fixes. MMC2
(9, 10 Punch-Out!! images) needed its full 5-bit PRG register and
renders 10/10 by 240 (~frame 210). Mapper 512 fades through a blank
frame at 120 and renders at 240; 330 renders by 240. The only images
with 0 rendering at both 120 and 240 is 308, which waits for Start (6
colors / 7,760 px at 120 and 10 / 16,638 at 240 with input). Mapper 252
now renders headless from 120 (3 colors / 2,789 px) after the `$2006`
warm-up fix.
The 28 load failures are all `image truncated`, in four groups:
expected 40,976 got 16,400 (20 images), expected 172,048 got 49,168 (4),
expected 32,784 got 8,208 (3) and expected 81,936 got 18,448 (1). No
failure is an unsupported mapper.
Mapper 303 (6 images) was
fixed next: the KS-7017 model was missing its `$6000` work RAM and the
`$5100` commit for the `$4A00` bank latch; all sampled Armana no Kiseki
images now render by 120 frames. Mapper 13 (CPROM, 3 images) followed:
its 16 KiB CHR-RAM with a 4 KiB high-window switch now reaches the PPU
through a shared CHR state, and all three images render by 120 frames.
Mapper 12 (SL-5020B, 2 Dragon Ball Z 5 images) was next: it uses a NEC
MMC3A, whose single-IRQ-at-zero revision was missing (the Sharp-B-style
IRQ storm starved the main loop); both images now render at 20 frames.
Mapper 14 (2 Wushi Hun images) was reimplemented as the Gouder SL-1632
(VRC2b/MMC3 dual ASIC with the `$A131` supervisor); both render by 120
frames (5 colors by 240). Mapper 50 (N-32 SMB2j conversions, 2 images)
followed: its `$4020`/`$4120` decode compared the masked address exactly,
so the game's `$4022` PRG writes never latched; decoding by bit tests and
inverting the 13-bit IRQ line (asserted while bit 12 is clear) makes both
images render from frame ~40. Mapper 301 (CK-001 4-in-1) followed: it uses
the mapper-242 address latch with the ET-113 512 KiB / 128 KiB chip select,
and power-on banks the last 32 KiB page (both images render at 20 frames).
The never-rendering set was then cleared out: 305 (KS-7031, reversed 2 KiB
windows), 309 (LH51, missing `$6000` WRAM), 513 (SA-9602B, R0 outer bits)
and 12 (NEC IRQ) all render by 120 frames; 308 (NTDEC 2131) also needed its
`$6000` WRAM and level IRQ and renders by 120 once Start is pressed (the
audit runs headless, so it reports the wait screen as blank). 304 (LE10)
was remodelled from NRS; the two loadable 48 KiB LE08/LF11 images render
2/2 at 120/240, and the SMB2 LE10 dumps have corrupt headers (864 KiB) and
fall in the truncated-load set rather than being a mapper failure. All
four such LE10 dumps, given corrected 48 KiB headers, still spin in an
FDS-BIOS-like loop, so they are recorded as data-blocked fragments.
The sweep added the MesenCE-only 208/219/258/284/298/302/307/346 group,
the 43/106/170/
262/285/304/521 batch, the TEC9719/167/190/266/314 batch and the
292/518 pair, the 183/487/530 batch, the 493/517/526/553 group, the
389/523 pair, the 415/417/538/539 group, the SB-5013 359/540 core and the
400/544 pair, the 308/368/470 group, the Techline 351 MMC3 mode and mapper
380, mapper 413, the 525 KS7021A VRC4 conversion, the 549 KS7016B latch
(now rendering; it boots with the PPU off for ~0.5 s),
the 515/554 pair, the 256 OneBus core (its middle-bank merge no longer
shifts, COMR6 swaps only the even slots, the submapper-1 CHR register order
is applied, and CHR-less images read CHR from the PRG space: all five local
images render by 120 frames, three at 20), the 374 reset-based MMC1 multicart and
the 446 SMD172B supervisor are now implemented. The supervisor decodes the
full `$5000` board table onto the UNROM/CNROM/MMC3/AxROM/GNROM/NINA/VRC1/
VRC2-4/VRC6/152/189/H3001 cores. It now uses NRS's fixed unlocked mapping
(`$8000/$A000` from `$5001`/`$5002`, `$C000/$E000` fixed to `$3E/$3F`, or
`$1E/$1F` for submapper 3) instead of an MMC1 stand-in, with the `$5003`
inversion and the sub-0/2/3 fuse tables; locked writes are ignored as
`applyMode` does. MMC1-family fuses now apply the supervisor's 8 KiB mask to
the MMC1 16 KiB page (`page & (mask >> 1) | (outer >> 1)`), which fixes the
`JMP ($FFFC)` entry after m446-65 locks to SNROM. All four local images render
by 240 frames (m446-25 and m446-49 at 120, m446-42 and m446-65 at 240); the
20-frame audit is blank because the multicart loader needs 1-2 s. The outer
base/mask is applied to the MMC1, MMC3, VRC2/4, VRC6, VRC3, UNROM, CNROM,
ANROM, GNROM, BNROM, Bandai 152 and 189 inner boards; untranslated fuses fall
back to the direct ASIC-number table (e.g. fuse 3 locks CNROM rather than
SKROM).
See `docs/mapper-audit.md` for the per-mapper table, ranked gaps and the
tail triage.

Since the previous audit, Sunsoft-4 (68) and the VRC2/VRC4 rewrite
(21/22/23/25) landed and the mapper-173 16 KiB-PRG panic was fixed. Sunsoft-4
renders 5/8 local images; the VRC family went from 8 rendering / 47 blank to
34 rendering / 21 blank at the same 20-frame window (39/55 at 600 frames);
the four formerly-panicking mapper-173 images now render. The VRC `number()`
also reported every board as mapper 21 — the audit now separates 21/22/23/25.
A new `unko signatures` batch harness records per-frame hashes for
directory-level regression checks.

Further A4 work: the PPU now models vertical, horizontal, single-lower,
single-upper and four-screen mirroring (4 KiB CIRAM), Sunsoft-4 drives
nametables from CHR ROM, and MMC3-8025 implements the FS005 submapper-2 RAM
configuration/protection path. Sunsoft-4 reaches 6/8 images and MMC3-8025
80/106 at 600 frames. Subsequent work added Sachen JV001 (136), Bandai
LZ93D50 (159), MMC3 512 KiB CHR wiring (197), Waixing FS303 (195), the
MMC2/MMC3 hybrid (165), Super Mega JV001 (172) and NTDEC TC-112 (193); the
remaining tail is triaged as real boards in `docs/mapper-audit.md` (157 needs
the Datach peripheral; 85 is VRC7).

MMC2 (9) and MMC4 (10) are implemented, including the two 4 KiB CHR fetch
latches that switch on tile `$FD`/`$FE` during rendering and the 5-bit bank
register wrap for smaller CHR ROMs. MMC2's top 24 KiB is fixed to the last
three 8 KiB banks (a real Punch-Out!! image caught the initial `N-2` mapping
error). The `$A000` PRG register is five bits wide; a four-bit mask wrapped
every bank above 15 into the first 128 KiB, so the ten 256 KiB Punch-Out!!
images spun on a blank screen. With the full mask they render 10/10 by 240
frames (the title appears around frame 210, so the 20- and 120-frame
snapshots are still blank).
The audit also exposed two display bugs now fixed and unit-tested: the
backdrop path double-indirected the palette, and CHR bank registers were not
wrapped to the ROM size.

The PPU now powers on at dot 40 instead of dot 0. The power-on dot is
undefined on hardware; dot 0 made some Nanjing titles lock onto the `$2002`
vblank-suppression dot in their reset poll loop. After the shift both the
blargg `vbl_nmi`/sprite suites and the Nanjing titles pass.

### Accuracy notes

The APU frame counter now follows the hardware phase model: an explicit
`odd` (get/put) phase toggles each CPU cycle, `$4017` writes set a delayed
reload via two delay flags, and the `$4015` IRQ flag is cleared on the next
GET cycle rather than immediately. This keeps all 13 blargg APU/DMC ROMs
green and moves the AccuracyCoin frame-counter subtests past the reset timing;
the remaining frame-counter failures are the read/write cycle alignment.
OAM DMA now starts at the current OAMADDR and wraps (latched at the start of
the transfer), fixing the misaligned-OAM-DMA behaviour.

The system now owns a global CPU get/put phase that toggles once per CPU cycle
and is handed to the APU frame counter, replacing its private toggle. This is
the shared-phase groundwork for the remaining DMA/controller/frame-IRQ tests;
its polarity is currently chosen to preserve the existing pass set. A
phase-gated controller strobe was trialled and reverted because it broke
normal controller reads in the full battery.

OAM DMA is now phase-driven: it reads on get cycles and writes on put cycles
using the global CPU phase, inserting one alignment cycle when it starts on a
put cycle (513 vs 514 cycles). This re-aligned the CPU/APU after the test
ROM's DMA sync and fixed `Frame Counter IRQ`, `4-step` and `5-step`.

Two phase-driven DMC DMA variants were trialled: a cycle-spanning state
machine (regressed five passing DMA tests because the CPU's triggering read
must happen after the complete transfer) and an inline phase-driven loop
(regressed six tests in the full battery). Both were reverted; the DMC cluster
still uses the inline fixed 4-cycle model, which is what the passing DMA/open-
bus tests are calibrated to. The DMC get/put rewrite needs a combined
DMC+OAM scheduler that keeps the inline read ordering.

A combined/spanning DMC+OAM scheduler was also trialled: the DMC spans CPU
cycles and `Cpu::read` drains the just-started transfer before its bus access.
It makes `DMC DMA + OAM DMA`, `Explicit DMA Abort` and `Implicit DMA Abort`
pass in isolation, but in the full battery it regresses five calibrated
DMA-read/open-bus tests and nets 107/151 (vs 109), so it was reverted. The
missing piece is reproducing the exact dummy-read/timebase of the inline
model while still allowing the transfer to span and abort.

The PPU now renders through a real background fetch/shift-register pipeline:
each visible dot composes a pixel from 16-bit pattern/attribute shift
registers, fetches nametable/attribute/pattern bytes on the hardware cadence,
loads the fetched tile at the end of each 8-dot group, prefetches two tiles on
dots 321-336, and only shifts while rendering is enabled. Sprite-zero hits are
now derived from the composed background and sprite pixels. This fixed
`PPU Misc. | Rendering Flag Behavior` without regressing the blargg sprite
suite. The high pattern plane shifts in a constant 1 (the PPU serial-in
input), matching the reference shifter.

The sprite path is now stateful too: patterns and X positions are latched during
the dots 257-320 fetch window, X counters are armed at dot 339 only when
rendering is enabled (the stale-shift quirk), and sprite shift registers only
advance while rendering is enabled. Sprite zero is the first in-range sprite
evaluated at dot 66 rather than necessarily OAM index 0. This fixed
`Advanced BG | Stale BG Shift Registers` and `Sprite Eval | Suddenly Resize
Sprite`.

`$2004` reads during dots 321-340 (rendering enabled) now return secondary-OAM
index 0, matching the documented idle-HBlank behaviour; this advanced
`Misaligned OAM2 Address` without regressions.

Gating DMA APU-register access on the halted CPU address bus (so an OAM DMA
whose CPU address is not in $4000-$401F cannot touch APU registers) was
trialled twice. It makes `APU Register Activation` run into the ROM's explicit
`JMP $80DD` trap: that test synthesises an instruction from PPU/open bus and
the CPU's next fetch address in our model is `$3FFE`, outside the APU-active
window, while the test expects the APU registers to be active. Distinguishing
the two page-$40 OAM transfers needs the PPU register-write delay and the true
open-bus fetch address, so the gating was reverted again.

The CPU/PPU cycle phase was corrected: each CPU bus access now happens two PPU
dots into the three-dot cycle, with the final dot completed after the access
(`CpuBus::end_cycle`). Cart/APU clocking and the get/put phase stay at the cycle
start as before; NMI sampling happens at the cycle end. This removed the
one-cycle-late NMI and fixed `NMI Timing`, `NMI Suppression` and `NMI disabled
at VBlank` without regressing the blargg NMI suite. (Moving the NMI edge to
dot 0 had fixed the same three tests but broke blargg; the phase split is the
principled fix.)

The DMC DMA now follows the reference (ares, ISC oracle) structure: the APU is
ticked before the CPU samples the request; the halt cycle performs the dummy
read of the current CPU address; the transfer then alternates put (alignment /
dummy) and get (real sample) cycles, and a trailing idle cycle gives the
`DMC DMA Bus Conflicts` was traced: the captured `$500` array now matches the
expected 00/FF pattern, including the controller conflicts at $16/$17 and
$36/$37. The `$4015`-mirror sample reads at $15/$35 were fixed by separating
the internal 2A03 data bus (which drives `$4015` bit 5) from the external open
bus, and by returning the sample ROM byte for those reads while still
acknowledging the frame IRQ. The remaining controller-bit mismatch was the external open-bus bit 6: the
conflict combines the sample high bits with the external open bus (bit 6) and
the controller bits 0-4. With that, `DMC DMA Bus Conflicts` passes.

A global two-dot earlier CPU/PPU access phase (bus at the start of the
three-dot window) was measured: it did not fix `Misaligned OAM2 Address` or
`$2002 flag timing` and broke the three NMI timing tests, so the two-dot
before/one-after phase remains the best compromise.

The reference (`ares`) sprite-evaluation algorithm was ported for the `$2004`
OAMADDR glitch (main/temp counters, timing bits, overflow skip and 4-byte
advance). A CPU bus trace of the bundled ROM now works (the earlier zero capture was a
stale/silent patch: the RAM write arm already contained a trace, so the
replacement was skipped; `touch` + a startup marker fixed it). It shows
`$2004 Stress` writing its result to $48C = $0A (ErrorCode 2, failing its third
subtest), and `$2004` reads can be logged with line/dot/value. NMI hijacking was implemented for BRK and IRQ vector selection (use $FFFA and
clear the NMI latch when an NMI is pending at the vector fetch). It did not
change `NMI Overlap BRK/IRQ` error codes and was reverted.

The bundled ROM was identified exactly: its SHA-256 (`f5e6596e...`) matches the
committed `AccuracyCoin.nes` at upstream commit
`f5f41dc240aedd46467518dfbe7643ca2e433a5f`. That source revision was downloaded
to `/tmp/opencode/AccuracyCoin` and is now the reference for labels, tables and
test flow (the earlier main-branch source differed).

The bundled ROM's answer-key tables were located directly in the image:
key1 at PRG $987B and key2 at $99D0, byte-identical to the published source.
Disassembly identified the fail routine (`$A038`: LDA $10/ASL/ASL/ORA #2/RTS)
and the evaluation helper `$97F2` (which reads the collected `$0500` array and
the key through an inline pointer). The first evaluation (key1) passes, the
second (key2) fails; the evaluation reads key2 through a pointer path that does
not directly touch `$99D0`. A dot-0 `$2004` read now returns the previous scanline's OAM2[0] (the key's
first byte, $7F), reducing the key1 diff from 117 to 116 entries with no
battery regressions.

The ares sprite-evaluation port was made to integrate with the calibrated
`$2004` path by giving the legacy `$2004` machine its own OAM data latch
(`legacy_data`) instead of sharing `oam_data` with the ares evaluation state.
That fixed the target cluster (`Arbitrary Sprite zero`, `Misaligned OAM
behavior`, `Sprite overflow behavior`, `Frozen OAM2 Increment`, `$2004 Stress`,
`$2002 flag timing`, `Misaligned OAM2 Address`) and kept the full blargg sprite
suite green. The remaining `INC $4014` regression was the sticky OAM2-freeze
flag: it was being re-raised on every temporary-counter wrap after the
dot-255 clear. Raising it only on the first wrap of an evaluation restores the
page-06 OAM-DMA sprite-zero hit without changing the frozen-OAM2 tests. The
isolated battery was then **139/151**.

`Stale Sprite Shift Regs` now passes. Two sprite-fetch details were wrong:
the per-slot shifter/position latch ran on the first dot of each slot's fetch
window instead of the middle (so disabling rendering just after dot 257 still
reloaded the shifter before HBlank), and dot 339 re-armed all eight sprite
counters rather than only the slots that actually received a sprite. Loading
each slot at dot `257 + 8*n + 4` and arming only non-empty slots gives the
documented stale-shifter behaviour for both the blanked-dot-339 case and the
blanked-HBlank case.

The `SH*` cluster (`$93`, `$9F`, `$9B`, `$9C`, `$9E`) now passes. The CPU bus
reports whether a DMA drove the bus on the bus cycle that just ran, and the
unstable stores drop the `H+1` AND term when it did: the DMA's fetch replaces
the high-byte read, so the stored value is only the register term (`A & X` for
SHA, `X` for SHX, `Y` for SHY, `SP` for SHS). Non-DMA stores keep the normal
constant, so the five instruction variants all pass without affecting the
non-DMA sub-tests.

The DMC `$4015` enable latch was then modelled at the request level.
Disabling the DMC clears the byte counter but preserves the in-flight bit
counter and sample buffer, so the last shifted bit still queues a fetch. A
re-enable while a byte is in flight takes three CPU cycles to become effective:
the queued request is held for one cycle (the CPU's access is redirected but no
transfer starts), then serviced with the shorter `[halt][put][get]` transfer
instead of the usual four-cycle form. The held transfer compensates the DMC
timer by the one cycle spent waiting so the next fetch keeps its 432-cycle
phase. This fixes both `Delta Modulation Channel` and `Implicit DMA Abort`
without changing `DMC Reload Timing`, `APU Register Activation` or `INC $4014`.

A secondary-OAM-driven renderer was also prototyped: evaluation stayed on the
calibrated baseline path (so overflow behaviour was bit-identical), while a
parallel 4-byte copy filled a shadow secondary OAM used only by `latch_sprite`.
That made `Arbitrary Sprite zero` pass and kept the entire blargg sprite suite,
`Sprite overflow behavior`, `$2004 Stress` and `$2002 flag timing` green, but
`Misaligned OAM behavior` test 6 still failed even though the evaluation path and
the final `PPUSTATUS` value were unchanged from the baseline build. The failure
therefore depends on a render/evaluation coupling not visible in the current
status model; resolving it needs a bus-level reference trace of that test.

A follow-up experiment kept the overflow scan untouched and only registered the
misaligned dot-66 sprite with its raw OAM address. That fixes `Arbitrary Sprite
zero`, but the sprite-zero hit then occurs at X=8 instead of the legacy X=0
path, which shifts the CPU/PPU get-put phase before `Misaligned OAM behavior`
test 6 and makes that test fail. Rendering the misaligned sprite at X=0 keeps
the phase but no longer produces the required hit. The remaining coupling is
therefore the `$2003` write latency / get-put phase, not the sprite-evaluation
byte counter alone.

Sprite evaluation still uses the legacy `lo == 4` byte-boundary shortcut. A
prototype proper per-sprite byte counter (X-position in-range check with
`+1 & $FC` re-alignment) made both `Arbitrary Sprite zero` and `Misaligned OAM
behavior` pass, but it changed the overflow scan semantics: `Sprite overflow
behavior`, `$2004 Stress`, `$2002 flag timing`, `Frozen OAM2 Increment` and the
blargg `spr_overflow_*`/`spr_hit_*` ROMs regressed. It was reverted rather than
trading calibrated tests. Resolving this requires modelling the hardware
overflow scan as a separate state from the normal in-range byte copy, not just
one shared counter.

The `APU Register Activation` test now passes. OAM DMA now observes the 6502
address bus: when the CPU program counter is in `$40xx`, every OAM DMA read
selects the APU register/mirror at `$4000 | (address & $1F)`; otherwise reads in
`$4000-$401F` are open bus. This covers both the inactive and active test cases
without changing ordinary OAM DMA pages.

The `Implied Dummy Reads` test now passes. Implied and accumulator
instructions perform their second-cycle dummy read from `PC` instead of an
internal-only idle; `PLA`/`PLP` additionally dummy-read the stack pointer
before pulling, and `PHA`/`PHP` dummy-read `PC` before pushing. This matches
the bus side effects the test observes without changing instruction timing.

The `Controller Strobing` test now passes. Normal writes to `$4016` always
strobe, while an RMW dummy write only reaches the controller when it lands on a
put cycle; this reproduces the one-cycle strobe behavior of `DEC $4016`.

The `Interrupt flag latency` test now passes. CLI, SEI and PLP update the I flag
on their final cycle, after the interrupt poll has already sampled the old
value, so the pending IRQ is not taken until the following instruction.

The `$2002 Flag Timing` test now passes. A `$2002` read latches the vblank bit at
the start of the cycle but samples sprite-zero/overflow at the end; the read that
ends on the pre-render clear dot therefore returns `$80` (vblank set, sprite
flags cleared). `$2004` is likewise sampled on the final PPU dot of the read
cycle, which fixed the one-dot alignment of the `$2004 Stress` result array.

The `$2004 Stress Test` now passes both answer keys. During rendering, `$2004`
returns the PPU's internal OAM data latch rather than `OAM[OAMADDR]`. The latch
follows the real sprite-evaluation pipeline: OAM2 is cleared to `$FF` during
dots 1-64, primary OAM bytes are fetched on odd dots, secondary-OAM bytes are
returned on even dots after the eight-sprite overflow, HBlank returns the
sprite-loading sequence, and the post-wrap primary address alternates between
the primary fetch and OAM2[0]. OAM DMA now masks the three unimplemented
attribute bits on writes, and the HBlank `$2004` read keeps the calibrated
misaligned-OAM2-address behaviour.

Dumping the collected `$0500-$0654` array at the key1 read and diffing against
the bundled key1 gives 117/341 mismatches: index 0 is FF instead of 7F and from
index 66 the values lead the key by two entries. A trial two-byte read
compensation made it worse (213 mismatches), confirming the mismatch is in the
OAMADDR stepping model, not a constant offset.

A `$60/$61` (key pointer) trace shows the key1
pointer ($987B) being installed, but the key2 pointer is never installed: the
test aborts between the two evaluations, before key2 is read.

Tracing the `$48C = $0A` store gives PC $FA7E (the fail routine's
`STA ($1E),Y`), with a 32-entry PC ring ending `... $97C6 -> $A038 (copy
routine) -> $FA7C`. The failure is reached through the test engine's
memory-copy/return path, not a visible PPU check, and cannot be resolved from
the public source.

A read trace during `$2004 Stress` shows two accesses in the key range, but the
PC trace shows they come from the engine's indirect pointer copies, not the
stress comparator;
the test then fails before reading the rest of the table, so the failure is in
the pre-comparison/collection phase or an earlier subtest, not in the key data.

The captured sequence was diffed against the published `AnswerKey2`: after
mapping reads by their 21-dot cadence, the best alignment matches only 33/341
bytes and the leading values differ (`FF FF FF DB 53 AF...` vs `7F FF FF...`).
The bundled ROM's expected sequence or collection cadence therefore differs from
the published source, so source keys cannot drive the OAMADDR state machine.
A matching ROM version or a reference trace is required.

A shadow OAMADDR state machine (reference stepping used only for `$2004` reads,
leaving sprite rendering untouched) was implemented for the `$2004` stress test.
It preserved all passing sprite tests but did not flip `$2004 Stress`. Capturing
its 341-byte result showed the bundled ROM does not write the source's documented
`$500` result area at all, so the public answer keys cannot be used to calibrate
it. The shadow model was reverted.

While debugging the port against `Sprite 0 Hit`, a corrected trace showed the
OAM data arrives normally via `$4014` page 2 (values EF/FC/00/08, 02/C0/00/13,
00/FC/00/FE, ...). The earlier "no setup" conclusion was a trace-filter artifact:
the ROM writes the page through a RAM mirror (address & $07FF), not the literal
$0200-$0203 range. So the bundled ROM does follow the documented setup, and the
port failure must be debugged per-frame against which OAM DMA is current.

A second integration pass added the 5-bit temp-counter wrap, in-range sprite
count tracking, the dot-66 sprite-zero id, and the frozen flag on OAM2 fill.
It still regressed the basic sprite cluster (`Sprite 0 Hit` code 25, overflow,
`Frozen OAM2`, `Misaligned OAM2`) and did not flip `$2004 Stress`, so it was
reverted again. The port needs stepwise validation against the basic sprite
tests before it can be kept.

For `$2004 Stress` and `Arbitrary Sprite zero`, sprite evaluation was reworked
to use a byte-phase counter (0=Y, 1=tile, 2=attr, 3=X) instead of the OAMADDR
low bits, and the sprite fetch read secondary-OAM bytes. It did not change
either test and destabilised the blargg sprite suite, so it was reverted. The
`$2004` stress answer key requires the full glitchy OAMADDR increment state
machine, not just the phase/alignment fix.

`$2004` idle-HBlank reads now use an OAM2 fetch-address counter: it resets at
dot 257, increments every two dots while rendering is enabled, and freezes
while rendering is disabled. The idle read (dots 321-340) returns the next byte
of that counter (the step at the transition into idle), and wraps at 32 for a
full fetch. This reproduces the misaligned-OAM2 case ($18 -> value $06) and
fixed `Misaligned OAM2 Address`.

OAM2 fetch-address and OAM2-driven sprite fetch were re-measured on the
persistent-OAM2 code. Instrumentation shows the misaligned test's `$2001`
disable/enable apply at dots 269/287, while its expected OAM2 count requires
283/285 (a two-dot earlier global phase). The address counter therefore ends at
23 (value $A3) instead of 24 ($06). Driving the sprite fetch from secondary-OAM
bytes also left `Arbitrary Sprite zero` unchanged. Both were reverted.

`$2002` flag timing was implemented as a split read: vblank latched at the
start of the cycle and sprite flags sampled at the bus access (~2 dots later),
with a fallback for direct PPU reads in the test harness. It advanced the
AccuracyCoin subtest (error code 1 -> 2) but regressed eight tests in the
NMI/vblank cluster (`NMI Timing`, `NMI Suppression`, `NMI disabled at VBlank`,
`NMI at VBlank end`, `VBlank beginning`, `BG Serial In`, `Frozen OAM2`,
`Suddenly Resize Sprite`), so it was reverted.

The APU-tick ordering was also exercised: ticking the APU after the CPU bus
access (rather than before it) is required by the `$4015`-abort window in
principle, but it breaks the calibrated DMA, instruction-timing and
frame-counter tests. The current before-access tick is therefore load-bearing.

The DMC trailing cycle was exercised in three forms: (a) full clock counted as
a stall cycle (current), (b) PPU-only advance, (c) full APU/PPU clock not
counted in the stall. Form (b) makes `DMC DMA + OAM DMA` and both abort tests
pass but breaks six calibrated DMA/APU tests; form (c) is identical to (a) on
the battery (122/151). This isolates the abort answer keys to the APU tick in
the trailing cycle, which the calibrated `$4015`/reload/bus-conflict tests also
require — they cannot both be satisfied without the reference abort schedule.

The reference DMC start-delay rule (delay = 2 when the timer counter is odd,
else 3) was tried in place of the global-cycle parity; it did not change any DMC
cluster outcome and was reverted.

Enabling sprite evaluation on the pre-render line (as line 5) for
`Sprites On Scanline 0` was retried after the OAM2 work; it still did not fix
that test and regressed `Stale Sprite Shift Regs` and `Suddenly Resize Sprite`,
so it was reverted.

A nested DMC lane inside the spanning OAM transfer (DMC get priority, OAM put
priority, shared halt) was also tried; it did not change the overlap/abort
outcomes and was reverted.

A fully unified OAM+DMC transfer (one reference-style loop owning both lanes,
get/put priority, abort hooks) was implemented next. It kept the calibrated
DMA-read tests plus `DMA + $4015 Read` and `DMC Reload Timing`, but changing
OAM from the spanning to the inline loop regressed `BG Serial In`,
`Frozen OAM2 Increment` and the three frame-counter tests, so it was reverted.
The remaining overlap/abort tests need the OAM lane's exact alignment offsets
without disturbing those calibrations.

This fixed
`DMA + $4015 Read` and `DMC Reload Timing`; `DMA + $2002 Read` passes in
isolation but fails in the full-battery sample (a clock-alignment boundary),
and the overlap/abort tests still need the nested OAM lane.

Measured `PEND`/`START` events confirm the DMC request is already pending when the
instruction's operand fetch begins; one- and two-stage request pipelines change
when `dma_ready` is visible but the next read in the contiguous burst is still
stolen, so the DMA still lands on `$BED6` rather than `$4015`. The remaining
unknown is the absolute phase of the free-running DMC timer (set when the
sample was first enabled), not the request delay.

The DMC request pipeline was delayed by one and two APU ticks to try to move the
steal from the operand-fetch cycle to the following `$4015` read in
`DMA + $4015 Read`. It did not change the outcome: the request is already
pending before the operand fetch, so the first read cycle consumes it. The
remaining fix is the DMC timer-reload phase that asserts RDY after that fetch,
i.e. the cycle-exact DMC timer/DMA state machine, not a request delay.

DMC start-delay parity was swept (2/3, 3/4, 1/2, 3/2, 2/1) with no effect on
`DMA + $4015 Read`, so the remaining DMC failures are in the DMA execution
phase rather than the initial fetch delay. Penultimate-cycle I-flag polling was
retried on the clean CPU; `Interrupt flag latency` still loops the IRQ handler
(unrun), so it was reverted.

`DMC Reload Timing` was investigated: its result array is all `$00`, meaning the
inline DMC DMA model lands on both open-bus read windows instead of only the
fast or slow one. That requires the APU DMC timer/DMA phase to match the CPU
cycle exactly, which the inline fixed 4-cycle model does not provide, so it was
left for the combined-scheduler work.

The dummy write cycle of a read-modify-write instruction no longer starts a
second `$4014` OAM transfer; only the real write's page is used. This fixed
`PPU Misc. | INC $4014` with no regressions. (An earlier trial appeared to
trade it against `NMI Control`; that was a stale `nmi_latched` polling variant
left in the CPU, now restored to the calibrated `prev_latched` polling.)

OAM2 now persists while rendering is disabled: the dots 1-64 clear phase and
the per-line OAM2/next-sprite reset only run when rendering is enabled. The
OAM2-overflow latch is raised during evaluation when eight sprites fill
secondary OAM and at dot 320 of a rendered line; it is cleared at dots
63/255/339 when rendering is enabled. While latched, every sprite-fetch field
re-reads secondary-OAM byte 0 (the frozen address), which fixed
`Advanced Sprite Evaluation | Frozen OAM2 Increment`. The dot-320 raise is
gated on rendering so the false-positive subtest still sees a clear flag.

The OAM2 fetch-address counter for `$2004` reads in dots 321-340 was retried
with the persistent-OAM2 model; it moved `Misaligned OAM2 Address` backwards
(error code 4 to 3), so it was reverted and reads keep returning OAM2 index 0.

Polling IRQs with the penultimate-cycle I flag was trialled for `Interrupt flag
latency`; it made that test hang (unrun), so it was reverted.

An OAM2-overflow/frozen-fetch model (re-read index 0 once eight sprites are in
range, clear at dots 63/255/339) was trialled for `Frozen OAM2 Increment` and
`Misaligned OAM2 Address`; it did not flip either test and was reverted.

A `$2007` read during rendering now corrupts the next background low-plane fetch
(the ALE/read octal-latch feedback): the shared address/data pins substitute
`$FF` for the low address byte, so the fetch reads from the top of the pattern
row. This fixed `Advanced BG | ALE + Read` without regressing the blargg sprite
suite.

A mid-fetch `$2006` write now produces the documented hybrid address: the next
background nametable read takes its high byte from the new `v` while its low
byte comes from the address bus latched just before the write (advanced one
fetch step, matching the documented `$2C19 -> $2F19` example). This fixed
`Advanced BG | Hybrid Addresses` without regressing the blargg sprite suite.

`$2001` writes now model the PPU register latency (applied two dots after the
bus write) on rendered lines, while pre-render writes stay immediate so the
dot-339 odd-frame-skip sampling keeps its calibrated phase. This fixed
`Advanced BG | BG Serial In` without regressing the blargg NMI/sprite suites.

A matching `$2000` delay was tried but broke the blargg NMI suite without
flipping any AccuracyCoin test, so it was reverted.

An OAM2 fetch-address counter (incremented every two dots during dots 257-320,
frozen while rendering is disabled) was trialled for `$2004` reads in dots
321-340, both before and after modelling `$2001` latency. The expected value
still does not match, so it was reverted; the simpler OAM2-index-0 read is
retained.

Enabling sprite evaluation on the pre-render line (as line 5) to reach
`Sprites On Scanline 0` regressed `Stale Sprite Shift Regs` and the blargg
sprite suite, so it was reverted.

An attempt to service a PPU-latched `nmi_pending` edge from the CPU (so a
`$2000` disable a fraction of a cycle after vblank edge would not erase it)
fixed `NMI disabled at VBlank` but broke `NMI Control` and the blargg
`vbl_nmi_suite`, so it was reverted; NMI edge latching remains load-bearing.

DMC sample reads whose low five address bits alias `$4015` now perform the
`$4015` side effect (clearing the frame IRQ flag) while leaving the data bus
as open bus, matching the documented DMA bus conflict. This fixed
`CPU Behavior 2 | Internal Data Bus`.

OAM corruption is now modelled: disabling rendering during a rendered line
seeds the secondary-OAM address, and the next rendered dot copies OAM row 0
into that row. The secondary-OAM address also advances every two dots during
the dots 1-64 clear phase, which provides the seed.

### Known A4 gaps

- MMC5 is near-complete: Castlevania III (US) boots and runs its intro, and
  ExRAM nametables/fill, split screen, extended attributes, the multiplier and
  the two-pulse + PCM audio are implemented. `$5105` now maps each nametable
  slot independently to CIRAM page 0/1, ExRAM or fill, with ExRAM usable as a
  nametable only in `$5104` modes 0/1 (modes 2/3 read zeros), matching Mesen
  and the wiki. The PCM channel now models both DAC modes: `$5011` writes in
  write mode and `$8000-$BFFF` PRG reads in read mode (`$5010` bit 0), with a
  zero update tripping the `$5010` IRQ that joins the mapper IRQ line. The
  Extended-attribute fetches use the full `$5130` upper bank bits, so CHR
  beyond 256 KiB is addressable (the PPU pattern path carries a physical
  byte offset rather than a 16-bit PPU address); a 512 KiB CHR title
  (Metal Slader Glory) matches the reference video on 140/300 frames.
- The differential oracle harness now compares per-frame RGBA8 hashes against
  a RustyNES capture (`trace --no-state` plus `diff`). Local results: the
  AccuracyCoin test image matches 49/60 frames; a commercial NROM title
  matches 297/300, differing only on frames 0/9/27 (a few first-scanline
  backdrop pixels during start-up). The harness found two real PPU defects:
  the completed-`$2007` read masked the address with `$2FFF` (breaking CHR
  reads above `$0FFF`, which truncated a title-screen nametable upload), and
  8x8 sprites selected their pattern table with PPUCTRL bit 4 instead of
  bit 3 (garbling sprites when the background and sprite tables differ). The
  palette is now the canonical 2C02 table and palette RAM powers on to index
  0.
- A long-standing IRQ bug is fixed: `System::end_cycle` compared against a
  `cycle_before_scanline` field that was never updated, so after the first
  scanline it saw a change on *every* CPU cycle and clocked the mapper ~94
  times per line. MMC3/VRC/FME-7 IRQ counters therefore expired almost
  immediately, which broke split-screen scroll effects completely. With the
  fix and A12-window clocking, Super Mario Bros. 3's split-screen intro is
  pixel-correct outside the sprite animation phase (frame 200: 7,030 -> 277
  differing pixels; frame 299 matches the reference's next frame to within 8
  pixels). The regression test `mmc3_irq_clocks_once_per_rendered_scanline`
  pins the ~17-scanline latency of a latch of 16.
- The output audio chain now matches the reference architecture: the mixer
  formula was aligned (95.52/8128 pulse, 163.67/24329 `3*tri+2*noise+dmc`),
  and samples pass through a 256-phase/32-tap windowed-sinc BLEP decimator to
  44.1 kHz followed by the 90 Hz/440 Hz high-pass and 14 kHz low-pass stages
  (new `resampler` module with unit tests). Audio is no longer a DC-shifted
  box average. The pulse duty sequencers clock at the APU rate, the triangle
  DAC always outputs the current sequencer step (the length/linear counters
  only freeze stepping) and freezes on periods below 2, and the system models
  the console power-on phase (PPU pre-render/dot 340) plus the one-frame
  post-reset warm-up window that ignores `$2000`/`$2001`/`$2005`/`$2006`
  writes. The system leaves reset on CPU cycle 8 like the reference, and
  `run_frame` reports the reset's power-on frame without executing, so frame
  indices match reference captures and every frame start is within one cycle
  of the reference (previously up to five). The per-cycle channel outputs and
  mixed value are now bit-identical on compared frames; audio samples are
  bit-identical for the first 38 frames of a local commercial title, and the
  NROM title reaches 226/300 identical audio hashes (299/300 video). The
  remaining divergence follows the last one-cycle frame-boundary difference,
  which moves the audio drain point. A per-cycle trace at the first divergent
  frame shows every channel matching except one pulse-2 duty transition two
  cycles late; the preceding `$4007` write lands at CPU cycle 1,154,224 here
  and 1,154,223 in the reference, and that one-cycle absolute offset flips the
  APU clock phase at the write. Fractional sub-cycle scheduling experiments
  (the reference's (5,7)/(7,5) master-clock split around the bus access, and a
  1-dot/2-dot pre/post split) did not improve frame alignment — they drifted
  +1..+5 cycles from frame 3 — so the shipped 2-dot/1-dot split (frame starts
  within ±1) is retained. Closing the gap requires the frame boundary to land
  on the same CPU instruction as the reference, i.e., a master-clock CPU/PPU
  scheduler rather than the current integer-dot split. Further master-clock
  tracing narrows this: with the reference's reset pre-advance (one CPU
  divider of master clock before reset) and its (5,7)/(7,5) read/write split,
  both the frame-start cycle and the PPU `(scanline, dot)` match the reference
  exactly through frame 11 (frame 11: cycle 297,808, dot 5 in both). At frame
  12 our boundary lands one CPU cycle later (dot 4 vs dot 1) while the CPU
  instruction streams remain identical (10,276 PCs match); the reference
  executes one additional instruction (`$8D26`) at the boundary. So the
  frame-11 PPU boundary is reached three dots (one CPU cycle) later here — a
  sub-cycle frame-detection difference, not CPU instruction timing. Draining
  audio at the exact PPU frame dot was also implemented and measured
  identical to the instruction-boundary drain, confirming the residual
  divergence is not a slice-boundary artefact. A per-dot PPU state trace of
  the reference pins the exact wrap cycles: over frames 1-12 the reference
  wraps at CPU cycles 29,779 / 59,560 / 89,340 / 119,121 / ... while this core
  wraps 2-3 cycles later, alternating (the two `29,780`-vs-`29,781`-cycle
  frame-length patterns are phase-shifted by one frame, so the offset does not
  accumulate). Advancing the PPU by seven dots before reset reproduces the
  reference's wrap cycles exactly (12/12 frames, including the odd-skip
  pattern), but it shifts the CPU/PPU relative phase and *regresses* oracle
  parity: SMB video 299 -> 273, audio 226 -> 194 (and every other tested
  pre-advance of 1-6 dots is worse still). The current reset phase
  (APU cycle 8, PPU line 0 dot 23) is therefore the best measured
  configuration, pinned by the `reset_phase_matches_the_reference_calibration`
  test.
- Differential parity re-measured at 600 frames on *Super Mario Bros.* (NROM):
  **599/600 identical video hashes and 504/600 identical audio hashes**
  against the RustyNES capture (`trace --no-state`, summarized by the new
  `unko parity` command). *The Legend of Zelda* (MMC1, 600 frames) gives
  598/600 video but 28/600 audio: its continuously playing music makes the
  per-frame sample hash sensitive to the residual drain-point phase, while
  SMB's sparser track tolerates it. The nonlinear mixer tables (95.52/8128
  pulse, 163.67/24329 `3*tri+2*noise+dmc`) and the 90 Hz/440 Hz high-pass plus
  14 kHz low-pass chain match the reference formulas bit-for-bit. DMC cycle
  stealing stays pinned by `dmc_dma_stealing_matches_the_reference_benchmark`
  (10,126 stolen loop iterations over 300 frames).
- The remaining frame-boundary gap is a three-dot (one CPU cycle) origin
  difference. It is visible as frame 0's first-scanline backdrop prefix: 22
  dots here versus 25 in the reference capture, while every later SMB frame is
  identical (599/600 video hashes). The reference also ends a frame at the
  first CPU-instruction boundary after its PPU frame-complete flag, and both
  run three PPU dots per CPU cycle, so the integer-dot split already matches
  the reference at dot granularity. The only local knob -- pre-advancing the
  PPU by 1-7 dots before reset -- reproduces the reference wrap cycles but was
  measured to regress SMB video 299 -> 273 and audio 226 -> 194. Closing the
  last dots therefore needs the reference's master-clock bus for frame
  detection, not another split tweak; that is a dedicated port, not a bounded
  change.
- A quantitative differential corpus mode (`corpus --oracle PATH`, now
  `corpus --parity` with `UNKO_ORACLE` and a 120-frame default, gateable
  with `--min-video/--min-audio`) was added.
  A 120-image / 120-frame sweep (109 images with oracle output) measured
  overall **video 81.1% and audio 70.1%** identical frame hashes and exposed
  the real outliers: mapper 58 (117-in-1) 1/120, 115 (1997 Super HIK) 2/120,
  132/172 (Du Ma Racing) 4/120, 226 (22 Games) 8/120, 242 multicarts 9/120,
  176 (100-in-1) 13/120, 215 (2-in-1) 37/120 and 142 (Jinpai Mali) 40/120.
  The harness ranks these by title instead of reporting only render/blank.
  The first fixes from that ranking, each re-measured with the same mode:
  mapper **58** was decoding the register from the data byte instead of the
  address (video 1/120 -> **117/120** on the sampled 117-in-1, **96.8%**
  across all 15 board images); mapper **132** used a guessed
  p/r/s latch instead of the reference accumulator/inverter chip (48.3% ->
  **70.3%** over 17 images, Qi Wang 4 -> 116/120); mapper **172** wrongly
  banked PRG and took CHR from output bits 4-5 (fixed PRG and output bits
  0-1 now, Venice Beach 117/120); mapper **226** used the wrong bank/mode bits
  (6.7% -> **58.0%** over 9 images, five titles at 118/120); mapper **242**
  (Waixing address-latched multicart) used the 227-family latch instead of
  the reference A2-A6 decode (8.3% -> **92.4%** over 14 images); mapper
  **115** (Kasheng clone) wrote a data-decoded outer register instead of the
  reference address-decoded `$4100-$7FFF` override plus `$5000` protection
  byte (sampled title 2/120 -> **118/120**; 54.1% over 16 images, ten titles
  at 118/120); mapper **176** was ported to the reference FK23C logic (PRG
  modes 0-4, CHR mode via `$5000.6`, FS005 base bits, `$A001` RAM config,
  `$E003` decode and the 6/7 register swap) for 70.1% over 106 images
  (from 53.1%), then routing the FS005 CHR-RAM overlay through the PPU
  (`HybridKind::Fk23c`, so `$2007` writes reach the first 8 KiB and banks
  >= 8 fall back to ROM) reached **80.1%**; mapper **215** had the NROM/enable and 128 KiB substitution
  bits shifted by one and lacked the submapper-1 CHR bit, now **92.1%** video
  and 89.6% audio over 21 images; mapper **142** (KS7032) read open bus at
  `$6000-$7FFF` because the CPU bus forwarded that window only when PRG-RAM
  existed (Jinpai Mali then executed the wrong bytes and stalled at `$EDC4`),
  and matching the reference up-counter IRQ took it from 17.8% to **89.8%**
  video and 41.6% to **84.6%** audio over 8 images (Jinpai Mali 40 -> 107/120);
  mapper **178** (FS305) decoded the PRG mode from `$4800` bits 2-3 instead of
  1-2 and banked work RAM that the board does not bank, giving **86.9%** video
  over 38 images (the sampled 48-in-1 was 0/120).
- The next ranked boards: **45** (T-GA23C) was wired as a plain MMC3 with no
  outer-register layer, so falling back to the reference's four rotating
  `$6000` registers (lock/reset, PRG-AND/OR and CHR-OR algebra) took *8-Bit
  Slasher* from 1/120 to **118/120** and the board to **89.2%** over 11
  images; **28** (Action 53) used a different bank formula and aliased its
  32 KiB CHR-RAM into 8 KiB, now **89.5%** over 35 images; the **JY ASIC**
  (90/209/211/35) used the wrong PRG outer composition, CHR low/high block
  handling, `$D001` mirroring encoding and a CHR-write gate, now matching the
  reference decode — CHR-ROM *45-in-1* went from 7/120 to **114/120**. The
  register blocks mirror across their `$x000-$xFFF` pages (`addr & 0xF007`;
  this board writes `$D500`), and the mapper id now comes from the header
  instead of always reporting 90.
- Two header/timing bugs then accounted for most of the remaining JY and
  FS005 gap. (1) NES 2.0 size fields used the exponent-multiplier form for
  *any* non-zero MSB nibble instead of only `$0F`, so large images were
  truncated (JY-120A's 2 MiB CHR-ROM was read as 16 KiB); with the 12-bit
  form it went from 7/120 to **114/120** and mapper 176 rose from 80.1% to
  **83.4%** video and 11.8% to **65.9%** audio. (2) The TV-system byte
  (NES 2.0 byte 12, plus the legacy iNES PAL bit) was ignored, so 577 local
  Dendy and 495 PAL images ran at NTSC speed; European MMC1 titles now match
  at 115-118/120. Mapper 45 reached **97.9%** and mapper 90 59.5%
  (62.1% best-aligned) after both fixes. The same 252-image / 120-frame
  sample moved from 87.0% to **90.3%** video (91.9% best-aligned) and from
  53.3% to 54.9% audio, while the number of frame-shifted images fell from
  32 to 19. The 226 mirroring, MMC1 PRG-RAM and SUROM fixes contributed the
  last two points (89.2% -> 91.1%). The MMC3 CHR-RAM map then took the same
  252-image sample to **92.0%** video and **93.6%** best-aligned; the MMC1
  cycle-spacing rule then took it to **93.0%** video and **94.6%**
  best-aligned.
- The next differential fixes: **226** (Waixing multicart) had mirroring
  fixed from the header instead of `reg0` bit 6, so *22 Games* sat at 9/120;
  with the register-driven mirroring the board measures **98.3%** video over
  nine images. **MMC1** now honours the `$E000` bit-4 PRG-RAM deselect
  (reads float, writes drop), matching the reference; the full 398-image
  mapper-1 board measures **91.0%** video (93.1% best-aligned) once the SUROM
  wiring is included: 512 KiB boards reuse CHR register bit 4 as PRG A18,
  which our mapper previously ignored, so the *11-in-1 NES Musical Pack* ran
  the wrong banks at 3/120 and now matches **118/120**. The one low MMC1
  title left is *Hillsfar* (4/120), whose 24C02 EEPROM probe the reference
  does not emulate either; it falls into an anti-piracy trap in our run.
- CHR-RAM MMC3 boards (mapper 4 homebrew such as the Atari Flashback
  *Adventure*) bank 1 KiB pattern slots through the MMC3 registers, but the
  PPU wrote and read its 8 KiB buffer flat, so uploaded tiles landed in the
  wrong slots. Exposing the resolved map to the PPU (the machinery built for
  the JY ASIC) took *Adventure* from 9/120 to **117/120**; the mapper-4
  board measures **92.4%** video (95.5% best-aligned). The remaining two
  low titles from that ranking are PPU-timing differences, not decode:
  *Pugsley's Scavenger Hunt* differs only in an 8-pixel-wide band on the
  left edge (a mid-frame scroll/write-timing effect) and *8-BIT XMAS 2015*
  only in a central sprite region with matching CHR/VRAM/palette.
- The last low MMC1 title, *Hillsfar*, was not an EEPROM probe after all: the
  game resets the MMC1 with the `INC $FFD7` trick, whose dummy `$FF` write
  resets the chip and whose real `$00` write must be **ignored because it
  lands on the next CPU cycle** (the shift register needs a cycle to settle).
  With the consecutive-write rule — a documented MMC1 behaviour the reference
  models — Hillsfar went from 4/120 (anti-piracy trap) to **116/120** and the
  mapper-1 board from 91.0% to **92.1%** video (94.2% best-aligned).
- The first alias batch from the mapper index: **31** (INL eight 4 KiB PRG
  slots; slot 7 powers on at the last bank) reached **98.9%** video over 3
  images; **92** (Jaleco JF-19, sharing the mapper-72 strobe latch with the
  fixed-first/switchable-second layout) **97.9%** over 2; and the later
  **Kaiser** boards **305/306/312** (2 KiB windows, address-decoded 8 KiB
  window, switchable 16 KiB half) **99.2/98.3/98.3%** over 2 images each.
  **37** (SMB + Tetris + NWC) is an MMC3 plus a 74HC161 register whose three
  bits feed the NAND equations for PRG A16/A17 and CHR A17: **98.3%** video
  over 2 images.
- The A5 mapper batch added **17** more boards from the RustyNES reference:
  **29** (Cufrom, 32 KiB banked CHR-RAM), **50** (FDS-conversion SMB2j),
  **104/301** (Golden Five / BMC-8157), **105** (NES-EVENT/NWC),
  **134/205/238** (MMC3 clones with outer banks, block select and the
  `$4020` security LUT), **153** (Bandai LZ93D50 with outer bank and
  battery WRAM), **218** (Magic Floor flags-6 single-screen wiring),
  **221/229/289** (NTDEC N625092, BMC 31-in-1 and BMC-60311C), **252**
  (Waixing VRC4 with RAM pages), **290** (Asder NTD-03), **513** (Sachen
  9602) and **96** (Oeka Kids, whose inner CHR bank is latched from A9/A8 on
  the PPU nametable edge). Scores over 2-3 images each:
  104/105/134/205/238/289/50 **98.3%**, 153/301/513 **99.2%**, 221
  **96.7%**, 218 **97.5%**, 29 **80.8% best-aligned**. Four remain low for
  timing or PPU-latch reasons rather than decode: **229** and **290** sample
  a scrolling/animation step one frame away from the oracle (15.0% and 35.8%
  raw, 47.5% and 65.0% best-aligned), **252**'s nametable upload used to be
  dropped by the post-reset mask (**30.0%**; `$2006` is now exempt, so the
  upload lands), and **96** draws the title but misses one
  nametable-row CHR bank in the centre logo (**40.0%**). The NES 2.0
  PRG-RAM/NVRAM size decoder was also corrected to `64 << shift` bytes (it
  had treated the nibbles as 8 KiB units).
- The first documented orphan boards from the NESdev mapper pages, all
  reference-blind (the RustyNES oracle does not implement them): **122**
  (JY043, separate even/odd 4 KiB CHR windows), **516** (AX5202P MMC3 clone
  with the address-as-data `$8010` 128 KiB outer register), **533** (Sachen
  3014 latch readback through the `$E000` bus conflict), **551** (the Jncota
  variant of 178 with a `$4803` CHR bank and hard-wired mirroring), **548**
  (Co Tung CTC-15), **535** (Whirlwind Manu LH53) and **399** (BATMAP-000).
  Unit tests pin the register behaviour; the local images load, and 548/535
  render by 120 frames while 399 reaches its title by 600. **554**
  (KS-7010) is deferred: its read-trigger mask is not documented, and the
  best-effort port still halts in the wrong bank.

The second-reference hosts are now functional outside the repository:
`refs/refcap/mesen-refcap` drives the rebuilt `MesenCore.so` capture exports
(256x240 RGBA8, mono f32 audio, FNV hashes) and `refs/refcap/ares-refcap`
runs the patched ares desktop-ui under cage's headless wlroots backend,
cropping the Famicom 283x242 canvas to 256x240. Both emit the same trace
contract as the RustyNES oracle.
- PPU colour emphasis was then implemented from the documented 2C02
  composite model (levels, phase table, YUV decode, per-colour RGB delta).
  The previously ignored PPUMASK bits 5-7 were why games such as *Du Ma
  Racing* rendered with the wrong colours: its video parity rose from
  **4/120 to 114/120 (95%)**, while SMB stayed at 599/600. The same pass
  declared the `$4100-$5FFF` and FK23C `$5000-$5FFF` read windows so
  protection reads reach the mapper instead of open bus. The refreshed
  audit is **7,073 loaded, 3,896 rendering, 3,177 blank, 0 unsupported
  (0 IDs), 28 failures**. A 252-image / 120-frame sample after the FK23C
  work measured **video 83.6% and audio 52.3%** identical frame hashes, up
  from the 120-image sweep's 81.1%.
  A fresh 260-image / 120-frame sweep after the A6 batch measured **video
  92.9% (94.6% best-aligned) and audio 55.5%**; the 16 oracle failures and
  the tiny 212-image/`--limit 252` variant are the same selection effect,
  not a regression.
- A `boundary` diagnostic prints the frame-completion CPU cycle and PPU dot.
  On SMB the completed frames land at cycles 29,781 / 59,566 / 89,343 /
  119,122 / ... against the reference capture's documented 29,779 / 59,560 /
  89,340 / 119,121 / ..., i.e. **+1 to +6 cycles, non-accumulating**. The
  independent second-emulator cross-check remains unavailable: MesenCE ships
  only `MesenCore.so` (a renderer-backed library with no headless frame-hash
  entry point) and ares' headless build has no frame-dump CLI, so neither can
  produce a comparable per-frame hash stream here.
- Two faithful local ports of the reference's arming were implemented and
  measured, and both were rejected. Moving the odd-frame skip wrap from dot
  340 to the reference's dot 339 left every SMB/SMB3 hash unchanged (inert).
  Porting the reference's read/write master-clock split (5/7 for reads = one
  PPU dot before the access, 7/5 for writes = two before) was also inert for
  SMB and SMB3 but regressed Zelda video from 598/600 to 38/600 same-index
  (best shift -1 -> 588/600). Both were reverted. The remaining difference is
  therefore in the reference's master-clock bus/frame-complete plumbing
  (`master_clock`/`ppu_clock`, DMA and interrupt ordering), not in the dot
  split or the skip position.
- The reference-style master-clock bus was then ported as an isolated,
  default-off experiment: `master_clock`/`ppu_clock` counters, `run_ppu_to`
  ticking whole PPU dots with the reference's one-master-clock sample lag, the
  5/7 read and 7/5 write splits, and the same path for DMA/reset cycles. On
  SMB (300 frames) it produced **video 297-298/300 and audio 5/300**, against
  **299/300 and 226/300** for the shipped scheduler; a sweep of the initial
  PPU dot (334-340) moved video only between 273 and 298. It was reverted.
  The remaining gap is therefore coupled to the reference's DMA, interrupt and
  APU ordering inside the same bus; porting the clock arithmetic alone makes
  both video and audio worse.
- The ordering was then ported too -- pre-access PPU dots run before the
  APU/mapper tick (the reference's `start_cycle`), with DMA compensation for
  the already-spent portion of the cycle -- and the one-clock sample lag was
  swept 0/1/2. At **lag 0** the port reproduces the shipped scheduler on SMB
  (299/300 video, 226/300 audio) and SMB3 (299/300 video, 300/300 audio), but
  still regresses Zelda (598 -> 38/600 same-index; best shift -1 = 588/600).
  At the reference's **lag 1**, SMB falls to 297/300 video and 226/300 audio.
  Reverted again: the shipped scheduler is already dot-equivalent once the
  ordering matches, and the read/write split is what moves Zelda the wrong
  way.
- A per-instruction `(pc, cycle)` comparison against the reference capture
  finally rules out CPU bus/IRQ timing as the frame-39 cause. Over the window
  around the seeding `$4007` write (cycle ~1,154,2xx) the PC streams are
  identical and **every instruction is spaced identically**, with our cycle
  exactly **+1** for the whole window. So there is no missing/misaligned bus or
  IRQ cycle: the divergence is a constant one-cycle offset in the frame origin,
  introduced at a frame boundary, which changes where the audio drain slices
  fall relative to the APU write. This is consistent with the wrap-cycle
  measurements above and confirms that further CPU-timing work cannot close the
  gap; only exact frame-boundary phase alignment can. An early frame-complete
  latch (raise a completion epoch N dots before the pre-render wrap while
  leaving the rendering dot phase untouched) was also swept over N=1..8:
  every lead is worse than N=0 (SMB video 298 vs 299 and audio 206-224 vs 226;
  Zelda audio 24-28 vs 28), so taking completion at the actual wrap remains
  optimal. Closing the gap requires a coupled master-clock CPU/PPU scheduler
  that matches both the wrap cycle and the per-instruction access sub-cycle,
  not a reset-phase or latch adjustment. The coupled master-clock scheduler was also built
  and measured directly: installing the reference's per-bus-access split
  (reads 5/7, writes 7/5 master clocks) and sweeping the PPU master-clock
  offset over 0-3 gives SMB 297-298/300 video and 204-216/300 audio, versus
  the current integer 2-dot/1-dot split's 299/300 video and 226/300 audio.
  The additional fidelity of the master-clock model does not translate into
  better oracle agreement here, so it was reverted rather than shipped as a
  regression.
- Namco 163 nametable/IRQ audit (mapper 19) — two concrete bugs found by
  sweeping 10 titles against the oracle at 60 frames (Star Wars 59/60,
  Hydlide 3 59/60, Dragon Ninja 58/60, Erika 57/60, Pac-Man CE 55/60,
  Mappy Kids 60/60 were already good; Final Lap 4/60, Youkai Douchuuki 9/60,
  Mindseeker 28/60, Kaijuu Monogatari 3/60 were low):
  1. **`$5000-$5FFF` reads never reached the cartridge.** The system answered
     open bus for the whole `$4020-$5FFF` window except `$5204`. The N163 IRQ
     counter lives at `$5000`/`$5800`, and games poll it: Kaijuu Monogatari
     spins in `LDA $5000 / CMP #$BF / BCC` waiting for the counter's low byte,
     then writes the scroll registers. With open bus it looped forever, leaving
     a stale `$2000 = $91` (nametable 1) in a vertical-mirrored layout, so the
     visible quadrant was a blank CIRAM page. Exposing a `$4800-$5FFF` low
     window for mapper 19 fixed it: Kaijuu 3 -> 58/60, Mindseeker 28 -> 58/60.
  2. **Nametable quadrant registers were modelled incorrectly.** `$C000-$D800`
     select, per quadrant, either the console CIRAM (`$E0`/`$E1` pages A/B) or
     a CHR-ROM page (values below `$E0`); the power-on layout follows the header
     mirroring. The old code used a separate RAM buffer with zeroed power-on
     registers and no CHR-ROM page path, which broke titles that write the
     nametable before programming the registers. The state now lives in a
     shared `N163Nt` (CIRAM + a CHR-ROM copy, header-mirroring power-on) that
     the PPU reads/writes directly, and the mapper's pattern-table path uses the
     same CIRAM for `$E800`-enabled `$E0+` CHR values. Final Lap 4 -> 58/60,
     Youkai Douchuuki 9 -> 59/60.
  Net: the mapper-19 corpus goes **32 -> 33 of 34 rendering** at 600 frames
  (the remaining blank title never enables rendering), with no change to the
  other titles' parity (SMB 299/226, SMB3 70/228, CV3 188/8, MSG 298/238,
  Zelda 299/28, VRC6 298/192, FME7 298/202; N163 298/234).
  Rolling Thunder (21/60 video, 240/300 audio) remains the one low mapper-19
  title and is the next diff target.
- Mid-frame CHR bank switching is now visible to `$2007` readback: the CHR-ROM
  window is refreshed on `$2006` writes and `$2007` reads (previously it was
  copied only at the start of each frame). Some mappers self-test their banking
  by switching a CHR bank and reading it back through `$2007`; with a stale
  window that check fails and the game parks in a `JMP $FF66` hang. Namco 163
  (mapper 19) was affected: Mappy Kids hung at the CHR-bank self-test. With the
  refresh its video goes **8 -> 298/300** and audio 205 -> 234/300, and the
  mapper-19 corpus is **32/34 rendering at 600 frames**. The refresh is cheap
  (only on `$2006`/`$2007` accesses) and leaves every other title's parity
  unchanged (SMB 299/226, SMB3 70/228, CV3 188/8, MSG 298/238, Zelda 299/28,
  VRC6 298/192, FME7 298/202).
- The full reference master-clock CPU/PPU scheduler port (replacing the integer
  2-dot/1-dot split with `(5,7)`/`(7,5)` master-clock access splits, a global
  `master_clock`/`ppu_clock` with `run_ppu_to(master-1)`, and the 108-master-
  clock reset pre-add) was implemented on a throwaway branch and measured. It
  is decisively worse and does **not** fix the frame origin or the SMB3
  artefact: SMB 297/300 video / 218/300 audio, Zelda 37/300 video at 300
  frames, SMB3 15/300, MSG 298/230, and SMB3 frame 200 still differs by the
  same 8 pixels. The branch was discarded and `main` (integer split, 299/226
  SMB) remains the best measured configuration. This closes the "full port"
  route empirically; the residual is not merely an incomplete scheduler.
- Same-frame framebuffer diffs at frames 11/30/60/90/100/120/150/180/200/220/
  250/280 classify the remaining video gaps:
  - **SMB3** — most frames differ by exactly **8 pixels at (x0-7, y194)**, one
    scanline of the first background column. The nametable/attribute VRAM is
    byte-identical to the reference and OAM holds no sprite in rows 185-205, so
    it is a first-column background-prefetch artefact on a split/CHR-boundary
    line, not sprite-0 or phase. Because it touches every such frame, it is what
    holds SMB3 at 70/300 despite otherwise-identical frames (at frames 180 the
    frame is bit-identical; at 60/200/299 only these 8 pixels differ).
  - **CV3** — 3 white pixels at (31,0), (223,0), (31,1) from frame ~150 onward,
    the top corners of the playfield/status region; also a background artefact,
    not phase. CV3 is otherwise frame-identical (188/300 comes mostly from the
    one-frame game-logic phase, where our frame N matches the reference N+1).
  - **MSG** — now pixel-identical after the MMC5 CHR-mode fix.
  An MMC3 IRQ-clock-dot sweep (256-264) leaves SMB3's 8-pixel artefact
  unchanged, so it is not the scanline-IRQ split timing; it needs a background
  prefetch/shift-register trace at the split line.
- MMC5 CHR banking now follows the documented region-top register decoding:
  mode 0 (8 KiB) uses `$5127`, mode 1 (4 KiB) uses `$5123`/`$5127`, mode 2
  (2 KiB) uses `$5121`/`$5123`/`$5125`/`$5127` with the low bits ignored, and
  mode 3 (1 KiB) uses `$5120-$5127`. In 8x16 sprite mode, background tiles use
  the `$5128-$512B` set with the same per-mode granularity (so a 4 KiB
  background uses the single `$512B` page for both halves). Previously modes
  0-2 used the *first* register of each region and the 8x16 background used
  per-slot 1 KiB registers, which garbled every tile on boards that used those
  modes. Effect on the 512 KiB CHR MMC5 title (Metal Slader Glory): video
  **142 -> 298/300** and audio **1 -> 238/300**; CV3 (1 KiB mode) is
  unaffected, confirming the fix targets modes 0-2. `Metal Slader Glory` is now
  pixel-identical at frames 60, 200 and 299.
- Expansion-audio levels now use the documented per-chip scales instead of
  ad-hoc normalizations, so the per-channel mixing matches the reference:
  VRC6 is a centred linear 6-bit sum scaled so one full pulse equals a 2A03
  pulse (`(p1+p2+saw-30) * 650 / 65536`), the Sunsoft 5B uses the YM2149
  logarithmic DAC tables (16-step fixed volume / 32-step envelope) scaled by
  `2549/138`, the Namco 163 sums its active wave channels, divides by the
  channel count and scales by `261`, and MMC5 pulses use the 2A03 DAC gain
  (`650` per pulse unit) with the 7-bit PCM at `40` per unit. Measured effect
  on a local VRC6 title: 1/300 -> 192/300 identical audio hashes; MMC5
  (Castlevania III) 1/300 -> 8/300.
- VRC6 (24/26) expansion audio is implemented (two pulses + sawtooth, with
  `$9003` halt/16x/256x scaling), as is the Sunsoft 5B (mapper 69) YM2149
  audio (three tone channels, 17-bit LFSR noise, 32-step envelope and the
  `$07` tone/noise mixer). The Namco 163 (mapper 19) wave channels are also
  implemented: up to eight channels cycle every 15 CPU cycles from the shared
  `$40-$7F` sound RAM, with 24-bit phase accumulation, waveform length/offset
  and the `$E000` sound-disable bit. MMC5 audio (mapper 5) is also
  implemented: two APU-style pulses with the 7-bit length table, envelope and
  the fixed ~240 Hz frame clock, plus the held `$5011` PCM DAC. The VRC7
  (mapper 85) six-channel FM engine is implemented with the custom patch
  registers, key-on/key-off/silence handling and the documented VRC7
  instrument ROM patches (15 built-ins; the modulator waveform is bit 3 and
  the carrier waveform bit 4 of `$03`). The VRC7 CHR-RAM board shares its
  banked window with the PPU, and its local title now matches the reference
  on 299/300 video frames (243/300 audio hashes). The FDS wavetable/modulation audio device is also
  implemented (`FdsAudio`: wave RAM, volume/mod envelopes, modulation table,
  `$4023` enable/reset) and unit-tested. A raw loader (`System::from_fds`)
  and a complete mapper 20 board map the 8 KiB BIOS at `$E000-$FFFF`, 32 KiB
  work RAM, and the `$4020-$4097` window. The disk model synthesizes the wire
  image (lead-in/inter-block gaps, `$80` start marks, per-block CRC-16/KERMIT)
  and delivers bytes to `$4031` every ~149 CPU cycles while a read transfer is
  armed; `$4030` reports the byte-transfer/end-of-head/timer flags, `$4032`
  reports drive readiness, and the `$4020-$4022` timer IRQ and `$4023`
  disk/sound enables are implemented. The real BIOS boots a local FDS title
  to its licence screen, and pressing Start loads and starts the game: the
  Super Mario Bros. FDS title screen renders correctly. Against the reference
  capture the 900-frame run matches 735/900 frames directly and 877/899 when
  allowing a single-frame phase offset (our boot completes one frame early).
  Multi-side images are parsed from the fwNES header and can be swapped at
  runtime (`Cartridge::insert_disk_side`, `System::insert_disk_side`), which
  resets the head, re-opens the spin-up window and rebuilds the wire image;
  ejecting reports the drive as not ready. Write-mode transfers also work:
  `$4024` bytes are stored into the wire image at the head and advance it,
  raising the byte-transfer flag/IRQ. Per-block CRC-16 is regenerated after
  each stored byte (the 2C33 does this in hardware, and the BIOS never writes
  CRCs itself), and `Cartridge::save_disk_image` / `System::save_disk_image`
  flush modified block payloads back into the raw `.fds` sides so a host can
  persist them.
- IRQ counters are clocked once per rendered scanline at the sprite-fetch A12
  window (dot 260) rather than from every individual A12 transition, which is
  sufficient for the standard split-screen case but still approximate for
  games that toggle CHR tables mid-line.
- AxROM titles that depend on exact PPU timing (e.g. Battletoads) do not boot yet.
- The accuracy core does not implement save states and supports NTSC only.

## A5 — AccuracyCoin (runner complete, accuracy work ongoing)

- `src/accuracycoin.rs` implements a real battery runner:
  - parses `SOURCE_CATALOG.tsv` (`suite<TAB>name<TAB>0xaddr`)
  - boots the ROM, moves the cursor to the page index, presses Start (the
    documented "run all tests" trigger)
  - runs a frame budget, then reads every per-test result byte from CPU RAM
  - decodes `value & 1 == 1` as pass, `value >> 1` as the failure code, `$FF`
    as skipped, and addresses below `$0400` as the print-only DRAW rows
- `tests/accuracycoin.rs` covers boot/menu rendering and runner parsing.
- `examples/accuracycoin.rs` prints totals, a per-suite breakdown and failures.

### The `$F278` stall (diagnosed)

The full-battery loop saves the current test index on the stack around each
test. On the Sprite Evaluation page a failing test corrupts that saved index:
the cursor jumps from row 5 back to row 1, the same tests re-run, and
`PostAllTestTally` climbs past the 151-test total. The engine then computes a
test-name pointer of `$8EC6`, which is code, and the text scanner at
`$F277-$F284` (`INY / LDA ($00),Y / BPL`) loops forever because its 256-byte
window contains no `$FF` terminator.

### Isolated runner (workaround, writes every row)

`run_accuracycoin_isolated` boots a fresh console for each catalog row and
navigates with closed-loop feedback from the ROM's own cursor variables
(`$14` page, `$16` row), then presses A and reads that row's result address.
It is slower but immune to cross-test corruption.

Current measured result (v3.1.0 ROM, 2026-11-12):

```text
Isolated mode:       146/151 pass,  0 fail, 5 skip,  0 unrun (199320 frames)
```

The 5 skipped rows are the DRAW tests. The former hangs (NMI disabled at
VBlank, OAM Corruption and the `$2007` Stress Test) are resolved.

Recent accuracy fixes from the failure clusters:

- **CPU open bus**: the data-bus latch is tracked across reads/writes, so
  unmapped `$4020-$5FFF`, `$4018-$401F` and cartridges without PRG-RAM return
  the last bus value; `$4015` deliberately does not drive the bus; controller
  reads expose open-bus upper bits.
- **PPU I/O bus**: every PPU register write latches the bus; write-only
  registers return it; `$2002` drives bits 7-5 and updates the latch; palette
  reads expose its upper bits; the latch decays to zero after ~16 frames.

Measured effect: `Open Bus`, `Dummy write cycles`, `PPU Register Open Bus` and
`PPU Read Buffer` now pass, and `CPU Behavior` is 9/9.

A further PPU fix came out of tracing the read-buffer test: the background
fetch pipeline was updating `v` on vblank scanlines. It now updates `v` only on
visible lines and the pre-render line, so `$2002` polling during vblank no
longer scrambles `$2006/$2007` sequences. That fixed `PPU Read Buffer`,
`Palette RAM Quirks` (its prerequisite) and several CPU/PPU tests.

Further fixes:

- **Palette RAM greyscale reads**: with `$2001` bit 0 set, palette reads zero
  the low four bits (`value & $30`), per the hardware quirk. `Palette RAM
  Quirks` now passes and **PPU Behavior is 5/5**.
- **SH*/AHX/TAS unstable stores**: the stored value now uses the computed high
  byte on a page crossing and the literal high byte + 1 otherwise, per the
  documented hardware behavior. The AccuracyCoin SH* rows still report code 3
  (their harness has additional unstable-address expectations), but the CPU
  behavior is now hardware-correct.
- **Branch dummy reads**: taken branches now perform the real cycle-3 read of
  the next opcode address and the cycle-4 read at the old high byte with the
  new low byte, instead of idle ticks. `Branch Dummy Reads` now passes.
- **DMA cycle accounting**: DMC fetches now run as a 4-cycle DMA whose first
  two cycles are dummy reads at the CPU's *pending* address, followed by the
  sample get/put. Those dummy reads have real bus side effects, so a DMA can
  clear `$2002`, step `$2007`, clear `$4015` or clock `$4016`. OAM DMA performs
  one halt plus 256 interleaved get/put cycles (513 cycles), reads the source
  through the bus, and both report stolen cycles to the CPU via `take_stall`.
  `DMA + Open Bus`, `DMA + $2002 Read`, `DMA + $2007 Read` and
  `DMA + $2007 Write` pass; the APU DMA suite is 5/10. A DMA cannot start on a
  CPU put (write) cycle; it waits for the next get cycle. A pending DMC DMA is
  serviced on the OAM DMA get cycle. The NES controller double-read quirk is
  modelled: only one of the DMA's two dummy reads clocks `$4016`/`$4017`, which
  makes `DMA + $4016 Read` pass. DMC reads whose sample address aliases
  `$4016`/`$4017` now produce the documented bus conflict (bits 5-7 from ROM,
  bits 0-4 from the controller). DMC looping also reloads its address/length
  immediately at end-of-sample instead of reusing the initial `$4015` enable
  delay, keeping the loop cadence at 432 CPU cycles for a 1-byte sample.

### DMA state machine (groundwork)

`System` now owns an explicit `Dma` state (`kind`, `total`, `remaining`,
`cpu_address`, `page`) and the bus contract gained `begin_cycle` /
`dma_active`. The CPU calls `begin_cycle` once per cycle and, through
`wait_for_dma`, stalls for as many cycles as the engine needs, so an OAM
transfer is a real 513-cycle spanning transfer rather than a synchronous loop.
DMC transfers keep their synchronous 4-phase shape (halt, align, get, put)
because that is the byte-fetch alignment the hardware tests are calibrated to;
the `$4015`-disable abort path is wired into the state machine but the
alignment/abort answer keys are not matched yet.

Measured explicit-abort durations (`$50..$5F`) versus the ROM answer key:

```text
ours: 05 05 05 05 05 05 01 01 01 01 01 01 01 01 01 01
key:  04 04 04 04 04 04 03 04 01 01 00 00 00 00 00 00
```

The normal DMA is one cycle long and the 3-cycle/aborted phases are missing,
which is the remaining DMA work. The battery total is 123/151 with
the DMA suite at 5/10, and all A1-A3 suites remain green.

Remaining failure clusters: APU Registers and DMA (10), CPU Behavior 2 (5),
APU Tests (6), unofficial SH* (5), PPU Misc. (5), Advanced Background (5),
Sprite Evaluation (4), Advanced Sprite (4), CPU Interrupts (3), PPU VBlank (2).

Failures still cluster in DMA/open-bus, CPU interrupt latency, unofficial SH*,
PPU read-buffer/misc and sprite-evaluation tests, which is the A7 backlog.

```text
test accuracycoin_boots_and_renders_menu ... ok
AccuracyCoin: 84/151 pass, 44 fail, 5 skip, 18 unrun
test accuracycoin_runner_parses_results ... ok
isolated: 5/5 pass, 0 fail, 0 unrun
test accuracycoin_isolated_runner_writes_every_row ... ok
```

## Current status (2026-11-07)

### 2026-11-07 — A6/M2 depth: PAL/Dendy, NSF, Zapper, composite filter

- **PAL/Dendy timing** is wired: `System::set_region(Region)` selects the
  region's 312-line PPU geometry, the fractional CPU/PPU dot ratio (3.0 NTSC,
  3.2 PAL/Dendy) through a master-clock credit, and suppresses the NTSC
  odd-frame dot skip. `clock.rs` already carried the tables; verification is
  unit-test math (`region_selects_timing_geometry`), since no PAL capture is
  available here.
- **NSF/NSFE**: `nsf.rs` parses the `NESM\x1A` fixed header and the NSFE
  `INFO`/`DATA` chunks, maps the payload into a synthetic 32 KiB NROM image
  with a player stub, and returns a running `System`; INIT runs once and PLAY
  from the NMI handler. Unit tests cover header fields, image/vector layout,
  NSFE chunks and an end-to-end player run (INIT write, PLAY counter, audio).
- **Zapper**: per-port device type plus `$4016`/`$4017` bit 4 trigger and
  bit 3 light (active-low), with the sensor sampling the 8x8 area as the beam
  passes. The desktop app aims/fires from the mouse and has a Controllers-tab
  toggle; a unit test covers the register layout and gamepad fallback.
- **Composite/display filter**: `VideoFilter::Composite` applies a display-only
  3-tap horizontal blend plus scanline attenuation in the core adapter. The
  adapter test `composite_filter_changes_pixels_and_keeps_geometry` loads a ROM,
  runs 20 frames with Raw and Composite, and asserts the frames differ on a
  rendered screen while dimensions and alpha are unchanged, plus that a live
  `set_video_filter` call affects the next frame. The app exposes nearest/linear
  texture filtering, integer scaling and a native folder dialog (`rfd`). Saves
  are keyed by a content hash (scan CRC plus FNV-1a 64) and state slots write a
  64x60 RGBA thumbnail.
- **CLI debugger**: `unko run <rom> --dump-ppu` prints one CPU/PPU/APU state
  line per frame; `--break <hex>` steps instructions until the PC matches and
  prints the state.

### Performance track (started 2026-11-07)

Baseline for the accuracy core, headless (`unko run <rom> --frames 1200
--core nes_accuracy_core` on AccuracyCoin, release build): **~535 fps**
(about 9x real-time). The retired TetaNES adapter ran at ~1200 fps; the accuracy core still has
clear 60 Hz headroom.

Profiling by subsystem (temporary A/B builds): the per-pixel background/sprite
composition was ~27% of frame time, the APU mixer ~4%, and the rest is the
CPU/PPU/APU dot loop.

First optimization: the pixel composition looped over all eight sprite slots
every pixel. It now keeps a per-line bitmask of the slots that hold a sprite
(`spr_active`, rebuilt at dot 0) and walks only those with
`trailing_zeros`/`clear-lowest`, with the win/lose pick kept in plain locals
instead of an `Option`. The shift-register advance and counter semantics are
unchanged (the sprite-mask-off case still shifts the registers).

Measured effect (5 runs, 1200 frames): **~535 → ~620-640 fps (+16-18%)**. All
A1-A3 suites and the isolated AccuracyCoin battery are unchanged.

Follow-up profiling (A/B builds, 2400 frames): after the sprite loop, the pixel
composition is still ~15% of frame time, sprite evaluation ~3-4%, the APU tick
and the sprite-fetch bus reads negligible. Two micro-optimizations were
evaluated: a backdrop fast path for rendering-disabled lines (no measurable
gain on this ROM, reverted) and a palette RGB cache (`palette_rgb`, rebuilt on
palette writes, one lookup per pixel instead of two). The cache is kept as a
strict reduction in per-pixel work (gain within run-to-run noise on a loaded
desktop).

Next optimization: an 8-dot background span composer. At each fetch-group
boundary the eight background pixels of the group are precomputed from the
shift registers into RGB (`span_px`) and the per-dot path only writes them,
falling back to the full per-dot composer when any sprite is active, when BG
rendering is off, or after a `$2001`/palette write invalidates the span. The
shift registers and fetch cadence are untouched, so sprite-zero timing and all
sprite tests are unchanged. Measured ~630 → ~640 fps max-of-7 on the loaded
desktop (small but positive; the span only applies on sprite-free background
lines).

Re-measured 2026-11-10 on the same AccuracyCoin / 1200-frame basis
(`unko bench`, release): **1130-1159 fps** (three runs, 18.8-19.3x
real-time), versus the 640 fps max-of-7 recorded when the pass was written.
The accumulated span/table/cache work covers the change; the host was also
less loaded than during the original measurements.

**A5 gate met (2026-11-08):** the isolated battery is at **146/151 passed,
0 failed, 5 skipped, 0 unrun** — every non-DRAW test passes. The former
`$2007 Stress Test` failure was fixed with the PPU data state machine:
the read buffer is loaded four dots after a `$2007` read from the per-dot
background/sprite fetch byte (reads span two dots), the `v` increment is
deferred while rendering and immediate under forced blank, and the dummy
tail fetch at dots 337-340 is modelled. The raw `$0500-$0654` array now
matches the ROM's 170-byte answer key exactly. Two independent references
confirmed the array: RustyNES's `ppu-misc-2007-stress` sub-test and MesenCE's
headless `--testRunner` (after installing its shipped `settings.json` and
passing an absolute ROM path) both match **170/170** stable bytes.

ares was investigated further: it does run the sub-test headless once its
`mia/Database` directory is linked into `~/.local/share/ares/Database`, and
it opens its GDB server (`GDB listening 127.0.0.1:27960` in the status bar).
However, the Famicom system registers no GDB memory hooks: `m` returns an
empty reply even after `?` returns `T05`, and `nall::gdb/server.cpp` gates
memory reads on `hooks.read`, which only the N64 backend sets. The GUI
state-save path is also closed here: ares has no state-save CLI, and xdotool
input injection does not move the pointer under this Wayland session, so the
Save State menu/hotkey cannot be triggered. ares therefore cannot supply a
capture for this test without a manual GUI session.

**Decision (2026-11-08):** RustyNES + MesenCE are accepted as the reference
pair for this gate; ares is waived because its Famicom backend exposes no GDB
memory hooks and no scriptable state save. Both accepted references match the
ROM's 170-byte answer key exactly. The earlier
`wip/ppu-bus` experiment and its 159/170 plateau are superseded.

AccuracyCoin v3.1.0, isolated mode:

```text
AccuracyCoin: 146/151 passed, 0 failed, 5 skipped, 0 unrun (199320 frames)
```

All A1–A3 public suites remain green: `nestest` 0 diff, `instr_timing`,
`instr_basics`, `vbl_nmi` 12/12, `ppu_sprites` 19/19, and all 13 blargg
APU/DMC ROMs. `docs/accuracy-divergences.md` now has no open failures.

### 2026-11-07 — DMC/OAM carry trailing cycle

`DMC DMA + OAM DMA` now matches all 32 answer-key bytes. When the OAM transfer
ends while a DMC fetch is still in flight, the fetch is finished on the CPU
side (halt, dummy, alignment, get), and the get now costs the same trailing
idle cycle as a normal inline fetch. Without it the CPU resumed one cycle early
for the alignments where the DMC halt lands in the last four OAM cycles, so
loop 2 X=5..8 measured one cycle short (and X=5 wrapped a whole 576-cycle DMC
period, storing `$2C`). With the trailing cycle the measured array is
byte-identical to the key:

```
04 03 04 03 04 03 02 01 02 01 02 01 02 01 02 01
02 01 02 01 02 00 01 02 03 03 04 03 04 03 04 03
```

### 2026-11-07 — load-DMA phase covers mid-byte writes

`DMA Landing on Write` now passes. The four-cycle load-DMA halt (the get cycle
of the second following APU cycle) is now used whenever the enable write lands
on a get, not only when the shift register is idle. The one exception is the
implicit-abort window (`bits_remaining == 1` with the DMC timer at 0 or 1),
where the fetched byte is being handed to the shift register as the timer rolls
over; that window and put-cycle writes keep the timer-parity 2/3-cycle start
delay. This satisfies `DMA Landing on Write` sub-test 6 as well as `DMA +
$2002 Read` while leaving `Delta Modulation Channel`, `Implicit DMA Abort`,
`Explicit DMA Abort`, `DMC Reload Timing` and the DMA-read tests green.

### 2026-11-07 — NMI interrupt hijacking

`NMI Overlap BRK` and `NMI Overlap IRQ` now pass. An NMI detected before the
status push of an interrupt sequence replaces the vector with `$FFFA` while the
pushed status keeps the BRK/IRQ `B` flag, so the handler reads a BRK-shaped
status byte from a hijacked `BRK` and an IRQ-shaped one from a hijacked IRQ.
The hijack is applied both in `BRK` and in the shared `interrupt()` sequence
(which covers the hardware IRQ and the NMI service path). Taking the NMI now
clears a simultaneously latched IRQ, because the interrupt sequence sets `I`
and the IRQ must not be taken on top of the NMI. An NMI edge that arrives
after the `BRK` hijack sample is left pending for the following instruction's
penultimate-cycle poll, matching the hardware poll cadence.

The answer keys now match byte for byte: `$500` = `$A4`×10, `$36`×15, `$26`×7
and `$520` = `$36`×10, `$00`×15, `$36`×7 for `NMI Overlap BRK`, and the
corresponding 64-entry arrays for `NMI Overlap IRQ`.

### 2026-11-07 — load-DMA start phase

`DMA + $2002 Read` now passes. A load DMA raised by a `$4015` DMC-enable write
halts on the get cycle of the second following APU cycle when the shift
register is idle. With the CPU's get/put phase available at the write, that is
four cycles after a write that lands on a get (previously two), which lets the
DMA's dummy reads hit the `$2002` data-read cycle and clear VBlank before the
CPU's own read. Writes on a put cycle, and writes while a byte is still
shifting, keep the timer-parity 2/3-cycle start delay, which is what the
`Delta Modulation Channel`, abort and DMC-reload tests are calibrated to. This
also matches the ROM's own note that `DMA Landing on Write` sub-test 6 (the
4-cycle load-DMA variant) only passes on CPUs that take the extra APU cycle.

### 2026-11-07 — Explicit DMA Abort and DMC/OAM interleave

`Explicit DMA Abort` now passes. A `$4015` write that clears the DMC enable bit
arms a 2–3 cycle stop delay; when the stop lands between a queued fetch and its
halt cycle the fetch is dropped, and when it lands after the halt it costs the
single halted cycle and aborts, reproducing the ROM's 3-cycle and 1-cycle abort
durations. The byte counter is only cleared when the delay expires, so a fetch
started inside the window is cancelled instead of completing.

`DMC DMA + OAM DMA` is now modelled as a real interleave rather than a
synchronous 4-cycle fetch: the OAM transfer keeps running through the DMC halt
and dummy cycles, the DMC get takes priority on a get cycle and pauses the OAM
read, the OAM transfer then adds one alignment cycle, and a fetch still in
flight when the OAM transfer ends is carried into the CPU's next cycles. The
result array now matches 30 of 32 answer-key bytes; only loop 2, X=5 differs
(`2C` instead of `00`). That entry is the alignment where the DMC fetch lands on
the final OAM cycle; the measured duration wraps one 576-cycle DMC period
(`44` is `556 & $FF`), so the first post-OAM fetch is about 20 cycles off the
ROM's phase. The interleave is kept because it also fixes `Explicit DMA Abort`
and keeps the rest of the DMA cluster green.

### 2026-11-07 — pre-render sprites; `$2007` buffer investigation

`Sprites On Scanline 0` now passes. Evaluation does not run on the pre-render
line, so its sprite fetch works from whatever secondary OAM still holds. The
fetch now treats the first non-empty stale slot as sprite zero; combined with
the existing `(261 - Y) & 7` row calculation, a Y=0 sprite is drawn from tile
row 5 on line 0, which is the documented behaviour.

`$2007 Stress Test` was investigated but remains open. Feeding the
background-fetch reads into `read_buffer` made attribute/pattern values appear,
but the sequence is still misaligned and the pattern reads use the `v` that the
`$2007` reads themselves advanced. A correct model needs the PPU data state
machine (address latch, idle, read) with the shared octal latch, not an
immediate buffer update; that experiment was reverted.

### 2026-11-12 — accuracy-core save states; TetaNES retired

The accuracy core now owns the host save-state contract. `UNKOST03` states
carry a ROM fingerprint, the mapper state and the CPU/PPU/APU/system state
(including the resampler). `unko verify` resumes byte-identically, and state
files are core-tagged (`<rom>.<core>.slotN.state`) so a state from another
implementation cannot be loaded by accident.

All 245 mapper implementations serialize and restore, including the mapper
446 supervisor, whose inner board is tagged and restored through the same
contract. A state from a commercial
MMC5 title is about 1.1 MB because mapper serialization currently includes the
ROM copy; skipping immutable ROM in the state is a later size optimization.

The TetaNES adapter, `tetanes-core` dependency, workspace member and licence
notice were removed. The header-correction database that shipped with TetaNES
was not carried over (it contains commercial title/CRC metadata); iNES/NES 2.0
headers are used directly, and DAT-based dump detection/header repair remains
the planned ROM-management feature.

### 2026-11-12 — PPU palette `$2007` buffer; multi-oracle gate note

A palette read (`$3F00-$3FFF`) returns the palette entry immediately, but the
deferred four-dot `$2007` buffer fill was reading the palette address itself
instead of the nametable underneath it. The completion path now masks palette
addresses with `$2FFF`, matching the immediate path. AccuracyCoin's PPU Read
Buffer test goes from code 15 to PASS, and Palette RAM Quirks follows once its
prerequisite passes. The isolated battery is **146/151 pass, 0 fail, 5 DRAW
skips, 0 unrun**.

Oracle note: raw hashes are palette-sensitive. For *Star Wars* (Namco,
mapper 19), RustyNES reports a blank 120-frame sample while ares and MesenCE
both render the title; at frame 120 our RGBA frame differs from ares in only
four palette colour pairs and has identical geometry (24,134/61,440 pixels
byte-identical). Use ares/MesenCE for geometry when the primary oracle's
boot/palette model disagrees.

### 2026-11-12 — palette-independent differential harness and cycle tests

The differential oracle harness now hashes a palette-independent view of each
frame: distinct RGBA colours are re-mapped to their first-appearance index in
scanline order and the indices are hashed. `trace` writes it as an optional
fourth field, so references that emit only video/audio still compare, and
`diff`/`parity`/`corpus --parity` report geometry separately from the RGBA
hash. A video mismatch with an equal geometry hash is reported as palette-only.

This resolved the mapper-19 audit: *Star Wars* and *Juvei Quest* at frame 120
are pixel-identical (0/61,440) to both ares and MesenCE once the palette is
canonicalised, while RustyNES renders a blank or different frame. The
remaining 20-frame library sweep (6,727 images) measures geometry 85.9% vs
video 84.3%; mappers 12/35/69/513 gain most of their parity back from the
geometry hash, which traces to uninitialised palette RAM (our index `$00`
backdrop vs ares's `$0F`), not to rendering.

New regression tests pin the timing invariants that the harness relies on:

- `frame_cycle_accounting_holds_over_a_long_run` runs 600 frames of a
  non-rendering NROM and asserts the total CPU cycles equal
  `frames x (341 x 262 / 3)` within an instruction overrun.
- `odd_frames_skip_one_pre_render_dot_only_while_rendering` checks the
  even/odd frame length difference and that disabling rendering removes it.
- `ppu_data_read_while_rendering_uses_the_fetch_byte` checks that a `$2007`
  read during rendering fills the buffer from the fetch pipeline (not VRAM)
  and applies the deferred, glitching increment (`$2000 -> $3001`).

### 2026-11-12 — per-cycle APU differential trace

Added a feature-gated per-CPU-cycle APU trace (`apudump`, `--features
apu-trace`) and a raw-audio dump on `unko trace` (`--audio-dir`), then compared
against the reference oracle's `APU_TRACE` records.

Results for *The Legend of Zelda* (USA) and *Super Mario Bros.*:

- Frames 1-38 are audio-bit-identical; the first difference is the first
  continuous music note (Zelda frame 39, sample 567 of that frame).
- From then on triangle, noise and DMC are bit-identical every cycle, and both
  pulse channels are exactly one CPU cycle out of phase (4-16 transition
  cycles per frame, mixed delta <= 0.05).
- Per-frame sample counts differ by at most one in 36 of 120 frames; the run
  total is identical (87,304 samples).
- Twelve controlled probe ROMs (pulse enable, duty, timer low/high, mid-note
  timer writes, duty resets, `$4017` writes, and every write parity) are
  bit-identical, so the APU register model itself matches the reference.
- The one-cycle offset is the known frame-origin phase difference already
  documented above (per-instruction streams are identical but shifted by one
  cycle from frame ~12). It only becomes audible when the pulse channels are
  gated on, which is why the earlier calibrations stopped at frame 38.

No scheduler change is shipped: the master-clock and coupled CPU/PPU
schedulers were ported and measured earlier and both regress parity. The
residual is recorded as a documented exception in
`docs/accuracy-divergences.md`; closing it needs a hardware capture of the
frame wrap.

The new `pulse_timer_period_matches_the_reference_probe` unit test pins the
sequencer period that the differential probes verify.

### 2026-11-12 — controller-port devices and state format 02

Added four optional port devices alongside the gamepad and Zapper, each with
unit-tested serial behavior taken from the NESdev documentation:

- **Four Score**: a 24-read chain per port (players 1+3 on `$4016`, 2+4 on
  `$4017`) ending in the adapter signature (`0x08` / `0x04`, LSB-first).
- **Arkanoid paddle**: 8-bit position inverted and MSB-first on bit 4, fire
  on bit 3, latched by the `$4016` strobe.
- **Power Pad**: 12 buttons shifted out two bits per read on bits 3 and 4 in
  the documented order.
- **Family BASIC keyboard**: 9x8 matrix, `$4016` column/clock/enable, keys
  active-low on `$4017` bits 1-4.

`set_controller` now accepts players 1-4. The peripheral state and the
four-controller array are part of the snapshot, so the save-state format was
bumped (`UNKOST02` at this point, `UNKOST03` when the palette preset was
added); old states are rejected with a clear format error. Arkanoid
(USA) boots into gameplay with the paddle attached (the frame changes with the
knob position in the demo), and Family Trainer 1 boots with the Power Pad.
AccuracyCoin remains **146/151 (0 failures)**.

### 2026-11-12 — display palettes, CRT filter and state format 03

The PPU now supports four display palettes: the canonical 2C02 table (default,
and still the table every differential trace uses), a warm Consumer TV preset,
a saturated RGB-monitor preset and Monochrome. The presets are documented
transforms of the canonical table, not hardware captures, so they are a
display preference and do not change emulated values; `refresh_palette_rgb`
applies the chosen table to every emphasis-adjusted entry.

The video post-processor gained a CRT mode next to Raw and Composite: the
1-2-1 horizontal blend stays, with stronger scanlines and a two-pixel
horizontal bloom. The filters are CPU passes for now (256x240), applied after
the frame is composed; a GPU shader pass remains future work.

The palette preset lives in the PPU state so a restored state keeps its
appearance; the save-state magic moves to `UNKOST03`. With the canonical
palette and raw filter (the defaults) output is byte-identical, and the
AccuracyCoin battery is unchanged at 146/151 with 0 failures.

### 2026-11-12 — expansion-audio parity probes and an FME-7 boot fix

Added per-cycle expansion-channel columns to the APU debug trace and compared
the summed expansion output with RustyNES (`APU_TRACE`):

- MMC5 (*Just Breed*, frame 300): exact on all 29,780 cycles.
- Namco 163 (*Erika*, frame 400): exact on 29,781 cycles.
- VRC7 (*Lagrange Point*, frame 400): exact on 29,780 cycles.
- VRC6 (*Akumajou Densetsu*, frame 400): APU pulse 1/triangle/noise/DMC exact;
  pulse 2 and the expansion mix differ (see the ledger; the oracle's sawtooth
  adds one extra time per sweep versus the wiki/Mesen schedule Unko follows).
- Sunsoft 5B (*Gimmick!*, frame 400): still under investigation.

The probe exposed a real **Sunsoft FME-7 (mapper 69)** bug: `$8000` command 8
was treated as a PRG bank rather than the `$6000` window's RAM enable/select
and bank; the nametable arrangement bits 0/1 were swapped; and the cycle IRQ
used the wrong enable bit, a fictitious reload value and asserted without the
assert-enable bit. Fixing all three makes *Gimmick!* boot and match the
oracle's video on 118/120 frames (previously a blank backdrop). Unit tests
cover FME-7 banking/mirroring/IRQ and the VRC6 sawtooth sweep.

Gates: fmt, clippy (with and without `apu-trace`), 433 tests. The 20-frame
library audit improved to **3,904 rendering / 3,169 blank** (8 more images
draw, all mapper 69); AccuracyCoin remains **146/151 with 0 failures**.

### 2026-11-12 — 5B/MAME/HDL alignment and DMC DAC mixing

Continued the *Gimmick!* audio investigation with the MiSTer Sunsoft HDL
(`refs/mister-nes/rtl/mappers/Sunsoft.sv`), the NESdev wiki, MAME's
`ay8910.cpp` and RustyNES as cross-checks.

- **DMC DAC (real fix).** The 7-bit counter output always feeds the mixer,
  regardless of `$4015` bit 4 or an empty sample buffer (NESdev APU DMC;
  Mesen's `AddOutput` path). Unko muted it, so *Gimmick!*'s `$4011 = 2` read as
  0. With the fix the entire frame 20 matches the oracle after the documented
  frame-origin alignment (pulse 1/2, triangle, noise, DMC, expansion mix and
  total), and *Batman (Japan)*, an FME-7 title without expansion audio,
  matches all channels and the mix at frame 400.
- **5B mixer `$07` (oracle bug, ours correct).** The datasheet/wiki/HDL layout
  is `--CB Acba` (tone bits 0/1/2, noise bits 3/4/5). RustyNES uses
  interleaved tone 0/2/4 and noise 1/3/5, so it enables noise A from
  *Gimmick!*'s `$F8` write and its expansion output flickers at the noise
  rate. Unko keeps the documented layout; the project's earlier brief change
  to the oracle's layout was reverted.
- **5B noise (oracle bug, ours correct).** The wiki, MiSTer HDL and MAME agree
  that the 5B noise LFSR shifts every `32 * period` CPU cycles (half the AY
  speed); RustyNES uses `16 * period`. Unko already used 32; the phase
  compare-after-increment now makes the first shift land at cycle 32 and the
  LFSR powers up at 1 with the hardware bit-0 XOR bit-3 input, matching MAME
  and the HDL.
- **5B tone/envelope timing.** The phase counters now compare after
  incrementing, so a half-period is exactly `16 * period` CPU cycles with no
  `+1`; the envelope steps on the same `16 * period` cadence. The envelope
  shape state machine now follows the wiki's ten shapes: `continue=0` holds at
  the end of its single ramp, `hold` freezes at the reached level, and
  `alternate+hold` flips to the opposite end before holding ($0B ends at 31,
  $0F at 0). New unit tests pin the tone half-period, the 32-cycle noise shift
  and reset value, the mixer bit layout, the envelope step interval and the
  hold/alternate shapes.
- **Still open:** the DMC reload chain starts four CPU cycles before RustyNES
  (the reference models a three-cycles-before-boundary get plus a four-cycle
  reload DMA), and *Gimmick!*'s wait-loop/IRQ phase moves later APU register
  writes by one instruction on alternating frames. Both are recorded in
  `docs/accuracy-divergences.md`; neither is audible and neither fails a test
  ROM.

New differential tooling in `crates/nes_accuracy_core/examples`: `exectrace`
(per-instruction CPU cycle/PC) and `expwrites` (per-cycle APU and expansion
writes), both behind `--features apu-trace`.

Gates: fmt, clippy (with and without `apu-trace`), **439 tests**, and
AccuracyCoin remains **146/151 with 0 failures** (199,320 frames) after the
DMC DAC and 5B changes.

### 2026-11-12 — generated display palettes and the UI batch state boundary

Added four generated display models (Vivid, Soft, PAL-like, Studio) as
parameter variations of the ares NTSC waveform colorimetry (ISC; see
`NOTICE`), alongside the canonical/consumer/RGB/monochrome presets and the
custom `.pal`/`.palette` import. They are display models, not hardware
captures: the canonical table still drives oracle traces, and the PPU applies
emphasis on top of whichever base table is selected.

The custom palette table lives in the PPU (64 RGB triples), so the UI batch
adds a serialized field after the audio work. The save-state magic moves
again to **`UNKOST05`**; `UNKOST04` states are rejected with the normal
bad-magic error rather than a decode failure.

### 2026-11-13 — microphone and Family BASIC tape recorder

Added two controller-port devices with synthetic tests:

- **Famicom microphone** — the immediate `$4016` D2 report documented on the
  NESdev controller page. The standard controller bits stay available on bits
  0/1, and the Devices window has a hold-to-talk checkbox.
- **Family BASIC data recorder** — `$4016` bit 0 is the cassette DAC/readback
  bit and bit 1 forces readback to 0; writes are queued and reads pop the same
  bit stream (the wiki's recommended emulator model). The queue is capped at
  1 Mbit.

Both are save-state fields. Miracle Piano and Datach remain queued until
protocol test ROMs are available; VS. System/PlayChoice-10 remains blocked on
ROM and palette/security data. Tests: microphone bit 2, tape loopback and
readback-disable, device settings round-trip.

### 2026-11-13 — Miracle Piano and Datach transports

- **Miracle Piano**: port 1 MIDI transport from the published interface — a
  short `OUT0` pulse requests a status bit followed by eight inverted data
  bits, and a long pulse selects the eight-bit transmit phase, with each
  `$4016` read clocking one bit. Received bytes are injected through
  `push_piano_rx` because the piano firmware is not reproduced; transmitted
  bytes are logged. Synthetic test covers a poll of `$5A` and a send of
  `$A5`.
- **Datach**: mapper 157 gains the barcode scanner register at `$6000` bit 3,
  advancing one pixel every 1000 CPU cycles with the 61/95/61 white framing,
  plus UPC-A/EAN-13 module generation (checksum recomputed). The EEPROM data
  line is stubbed high like MAME's default build. The Devices window accepts a
  barcode and the test covers the generated guards, scan timing, invalid
  codes and non-Datach boards.

Both are synthetic-validated; real Miracle Piano and Datach ROMs are still
needed before claiming parity.

### 2026-11-13 — accuracy and performance regression pass

- AccuracyCoin isolated battery: **146/151 passed, 0 failed, 5 DRAW skips,
  0 unrun** (199,320 frames).
- 20-frame library audit: **3,904 rendering / 3,169 blank**, 0 unsupported
  mapper, 28 load/run failures out of 7,101 scanned — identical to the last
  run, so the audio, UI and palette work did not change video output.
- CI budget: `unko bench --synthetic 3000 --min-fps 300` measured **786.2 fps**
  (13.08x real time, 1.272 ms/frame), well above the 300 fps floor.
- Determinism: `unko verify` on *Gimmick!* (60 frames) resumed from a
  900,441-byte state and produced the same tail hash.
- The canonical palette and Raw filter remain the default and the 120-frame
  canonical trace is unchanged by the generated display palettes.
- The microphone/tape/piano/Datach additions landed after `UNKOST05` and
  changed the serialized peripheral layout, so the state magic is bumped to
  **`UNKOST06`**; older states are rejected by the normal bad-magic check.

### 2026-11-13 — MiSTer accuracy options

Ported the accuracy-relevant NES_MiSTer OSD options. Defaults preserve the
previous behaviour:

- **PPU reset behaviour** (Famicom / NES): a soft reset keeps the PPU
  `$2000`/`$2001` registers on Famicom, or clears the derived register state
  on the NES setting.
- **RAM clear** (Keep / `$00` / `$FF` / Random): applied by the next soft
  reset, matching MiSTer; the random fill is deterministic per reset loop.
- **Audio source** (Both / Internal / Expansion / None): gates the 2A03 and
  cartridge expansion contributions at the mixer while the machine keeps
  running.
- **Overscan** (Normal / Vertical / Borders / Everything) and **Mask edges**
  (Off / Left / Both / Auto): display cropping and the 8-pixel software-mask
  columns, in the Graphics page.
- **NES 2.0 warning**: loading an iNES image with Auto region reports "No
  NES 2.0 header; defaulting to NTSC", as MiSTer does. Bulk header repair
  remains an external tool.
- Advanced -> soft-reset controls explain that power-on starts from zeroed
  work RAM and the options apply on the next reset.

Still deferred because they need ROM/hardware evidence or are outside the
accuracy charter: VS./PlayChoice-10, FDS disk speed and auto side swap (no
local BIOS), Miracle Piano MIDI replies, Datach EEPROM, extra sprites,
overclocking, and the 32-slot cheat engine.

AccuracyCoin remains **146/151 with 0 failures** with the new defaults
(`RamClear::Zero`, Famicom PPU reset, audio both sources), and 457 workspace
tests pass.

### 2026-11-13 — host speed controls

Added 25/50/100/150/200/300% playback presets, hold-to-fast-forward (Tab,
300% cap) and frame advance (Period or Emulation -> Frame advance). The
core is still driven one frame at a time and all accuracy gates are
unchanged; audio is paused whenever the speed is not 100% so the output
cannot drift or pitch-shift, and it flushes on return to normal speed.

### 2026-11-13 — Game Genie and raw RAM cheats

Added a per-game cheat engine: Game Genie 6-character substitutions and
8-character substitutions with a compare byte, plus raw work-RAM writes.
Game Genie patches are applied on both the CPU and DMA PRG read paths; raw
RAM cheats are written before each frame and immediately on edit. Up to 32
entries are stored as `<stem>.cheats`, with enable/disable and a detachable
Cheats window. Cheats are host presentation state and are never serialized
into a save state; the canonical/no-cheat output is unchanged.

The decoder is pinned by the published `SXIOPO` (`$91D9 = $AD`) and
`YYKPOYZZ` (`$9F41 = $77` if `$22`) examples. Tests cover decoding, compare
gating, immediate/frame RAM writes and the cheat-file round trip.

### 2026-11-13 — Family Trainer

Added `PortDevice::FamilyTrainer` as the Japanese Power Pad variant. It uses
the same 12-button two-bit serial matrix and strobe, so the existing mat
overlay and `set_power_pad` path drive it; only the device label and the
strobe reload condition differed. A test reuses the documented Power Pad
button order through the Family Trainer device.

### 2026-11-13 — Subor keyboard

Added the Subor/Dendy clone keyboard protocol: `$4016` bit 2 enables the
keyboard, bit 1 selects the lower/upper nibble (a 1 -> 0 edge advances one of
13 rows) and bit 0 resets the row counter; `$4017` bits 1-4 return the
inverted nibble. The app maps the documented FCEUX/PlayPower matrix onto host
keys (letters, digits, arrows, F1-F12, editing keys, punctuation, Shift and
Ctrl) and sends the 13-row matrix every frame while the device is attached.
Test covers nibble reads, row advance and reset.

### 2026-11-13 — rewind

Added a rewind ring built on the deterministic state format: a full keyframe
every 30 frames plus the per-frame input log, within a 64/128/256 MiB budget
(default 64). Holding Backspace while paused restores the newest keyframe at
or before the target and replays the recorded inputs; a test compares the
replayed state at frame 45 byte-for-byte against the live state. The buffer is
suspended while recording or when the speed is not 100%, cleared on reset and
manual state load, and the status bar shows the available rewind depth.
Run-ahead is queued next.

### 2026-11-13 — input movies (TAS)

Added the `UNKOMOV1` input movie format: an 8-byte magic, the save stem, the
full `UNKOST06` state at the start of the recording together with the 4-port
`u16` input words for every frame. The player pauses/suspends the rewind ring
while recording or playing back, captures `Player::last_input` (the same
authoritative value rewind uses), reloads the stored state before playback,
and stops with the machine paused at the final frame. The Emulation -> Movie
menu exposes record, stop, play (file picker rooted in
`<save dir>/recordings`) and stop playback; the status bar shows the capture
length and playback progress. Round-trip and truncation tests cover the file
format.

### 2026-11-13 — run-ahead

Added 0-4 frame run-ahead. For each committed frame the player saves the
post-frame state, emulates the remaining prediction frames for display, then
restores that state so the machine still advances exactly one frame per
update; audio, the recorder and the rewind log only see the committed frame.
It is suspended at non-100% speeds, during movie recording/playback and with
live debugger breakpoints. A test runs two cores side by side, applies 4-frame
run-ahead to one, and asserts the committed states stay byte-identical every
10 frames. Release measurements on the synthetic ROM: save 172 us, load
237 us, frame 1.28 ms, so 4-frame run-ahead remains inside the 16.6 ms frame
budget.
