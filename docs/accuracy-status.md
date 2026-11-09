# Accuracy status

Measured on the reference-parity core (`multiemu-nes-accuracy`, MIT with
documented ISC adaptations from ares). These are local runs
against public test ROMs placed in `test-roms/` (gitignored).

## A1 — 6502

| Test | Result | Evidence |
|---|---|---|
| `nestest` golden log | **8980 / 8980 lines**, registers + PC + SP + P + CYC all match | `cargo test -p multiemu-nes-accuracy --test nestest -- --nocapture` |
| blargg `instr_timing` | **Passed** (official, NOP/SBC, unofficial) | `cargo test -p multiemu-nes-accuracy --test blargg blargg_instr_timing -- --nocapture` |
| blargg `instr_basics` | **Passed** | `cargo test -p multiemu-nes-accuracy --test blargg blargg_instr_basics -- --nocapture` |
| blargg interrupt suites | see A2/A3 | requires PPU NMI + APU/DMA timing; tests are `#[ignore]` |

The CPU implementation in `crates/multiemu-nes-accuracy/src/cpu.rs` is
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

Remaining A3 follow-up: mixer-level scoring against the reference and the
DMC DMA cycle-stealing tests (`dmc_latency`, `dmc_dma_*`), which need the
system bus rather than the APU-only harness.


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
| MMC3 (4) | PRG/CHR banking, mirroring, scanline IRQ |
| AxROM (7) | 32 KiB banking, single-screen mirroring, bus conflicts |
| VRC2/4 (21/22/23/25) | PRG/CHR banking, mirroring, scanline IRQ; audio missing |
| Color Dreams (11) | 32 KiB PRG / 8 KiB CHR banking, bus conflicts |
| CPROM (13) | 16 KiB CHR-RAM, switchable upper 4 KiB |
| GxROM (66) | 32 KiB PRG / 8 KiB CHR banking |
| Camerica (71) | 16 KiB banking + fixed last, mirroring |
| UNROM-512 (30) | 16 KiB switchable + fixed last 16 KiB, 8 KiB CHR-RAM, one-screen/vertical/horizontal |
| BNROM/NINA-001 (34) | 32 KiB PRG banking (BNROM bus conflicts) or NINA-001 32 KiB PRG + 4 KiB CHR |
| Sachen SA-015 (150) | indexed register pair `$4100`/`$4101`: 32 KiB PRG, 8 KiB CHR, mirroring |
| Sachen SA-008-A (148) | mapper 79 latch moved to `$8000-$FFFF`, bus conflicts |
| Jaleco SS88006 (18) | VRC4-like 8 KiB PRG / 1 KiB CHR nibble-pair banking, CPU-cycle IRQ |
| NINA-03/06 (79/146) | 32 KiB PRG + 8 KiB CHR via a single control register |
| DxROM (206/76/88/95/154) | MMC3-style 8 KiB PRG / 1 KiB CHR banking, no IRQ; `$C000`/`$E000` fixed |
| Nanjing FC-001 (163) | 32 KiB PRG banking via `$5000`/`$5200`/`$5300`, CHR-RAM, feedback register |
| Sunsoft FME-7 (69) | 8 KiB PRG + 1 KiB CHR banking, mirroring, CPU-cycle IRQ |
| Namco 129/163 (19) | 8 KiB PRG / 1 KiB CHR banking, NT-as-CHR, 15-bit CPU-cycle IRQ, 128-byte chip RAM (audio missing) |
| Action 53 (28) | reference inner/outer PRG bank formula, 32 KiB CHR-RAM window, H/V/1-screen mirroring |
| J.Y. ASIC (35/90/209/211) | 8/16/32 KiB PRG, 1/2/4/8 KiB CHR, mirroring, multiplier, prescaled M2/A12 IRQ; ROM nametables missing |
| Waixing FS305 (178) | 8 KiB CHR-RAM, NROM-256/UNROM/NROM-128/UNROM-fixed PRG modes, banked 8 KiB PRG-RAM |
| J87 (87/101) | 32 KiB fixed PRG, 8 KiB CHR latch at `$6000-$7FFF`; mapper 101 reverses CHR bit order |
| J.Y. clone (91) | two 8 KiB PRG banks + fixed last, four 2 KiB CHR banks, outer 128 KiB PRG/512 KiB CHR, 64-rise A12 IRQ |
| Bandai FCG (16) | 16 KiB PRG, eight 1 KiB CHR, mirroring, 16-bit CPU-cycle IRQ latch/counter |
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
| Konami VRC7 (85) | 8 KiB PRG, eight 1 KiB CHR banks, shared banked CHR-RAM window, mirroring/WRAM-enable, VRC cycle/scanline IRQ, six-channel two-operator FM audio (approximate built-in patches) |
| Irem G-101 (32) | two 8 KiB PRG banks with switchable/fixed pair, eight 1 KiB CHR banks, mirroring/PRG-mode |
| Taito TC0690 (48) | TC0190-style PRG/CHR banking plus inverted-reload MMC3-style IRQ |
| Jaleco JF-13 (86) | GNROM-style 32 KiB PRG / 8 KiB CHR latch at `$6000-$6FFF` (speech chip not modelled) |
| TXC 01-22000 (36) | 2-bit latch/inverter PRG select plus an 8 KiB CHR latch at `$4200` |
| FDS conversions (42) | 8 KiB bank at `$6000-$7FFF`, fixed last 32 KiB, CHR bank, 24576-cycle IRQ |
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
| Benshieng BMC-BS-5 (286) | address-latched 8 KiB PRG and 2 KiB CHR windows (partial; menus still blank) |
| MMC5 (5) | PRG modes/CHR banking/mirroring/IRQ, multiplier, ExRAM nametables/fill, split screen, extended attributes and two-pulse + PCM audio; per-page CIRAM and >256 KiB CHR remain |

NES 2.0 PRG (16 KiB units) and CHR (8 KiB units) sizes are parsed correctly.

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

New crate `multiemu-nes-accuracy-core` implements `multiemu_core::Core`
(id `multiemu-nes-accuracy`) and is registered beside TetaNES. The CLI can
select it with `--core multiemu-nes-accuracy`.

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
6,627 loaded (93.3%), 3,554 rendering (50.0%), 3,073 blank (43.3%), 446
unsupported (6.3%) and 28 load/run failures (0.39%). The batched totals
reproduce the single-pass numbers exactly. Top unsupported family is now
157 (Datach) at 7 images, then a six-image tail (156/164/168/199/216/222/234/
249/253/27/60/85/303). See `docs/mapper-audit.md` for the per-mapper table,
ranked gaps and the tail triage.

Since the previous audit, Sunsoft-4 (68) and the VRC2/VRC4 rewrite
(21/22/23/25) landed and the mapper-173 16 KiB-PRG panic was fixed. Sunsoft-4
renders 5/8 local images; the VRC family went from 8 rendering / 47 blank to
34 rendering / 21 blank at the same 20-frame window (39/55 at 600 frames);
the four formerly-panicking mapper-173 images now render. The VRC `number()`
also reported every board as mapper 21 — the audit now separates 21/22/23/25.
A new `multiemu signatures` batch harness records per-frame hashes for
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
error). All 15 mapper-9/10 images in the local library render at 600 frames;
the 20-frame audit window still shows them as blank because they are booting.
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
isolated battery is now **139/151**.

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
  the two-pulse + PCM audio are implemented. Accurate per-page CIRAM mapping
  and the >256 KiB CHR path remain.
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
  registers and key-on/key-off/silence handling; its built-in instrument table
  is an approximation of the YM2413 patch ROM. The VRC7 CHR-RAM board shares
  its banked window with the PPU, and its local title matches the reference
  video on 296/300 frames. FDS audio is still missing.
- IRQ counters are clocked once per rendered scanline rather than from exact
  PPU A12 rises, so fine-grained split timing is approximate.
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

Current measured result (v3.1.0 ROM):

```text
Full-battery mode:   84/151 pass, 44 fail, 5 skip, 18 unrun (stall at $F278)
Isolated mode:       139/151 pass,  7 fail, 5 skip,  0 unrun (199320 frames)
```

The 5 skipped rows are the DRAW tests. The 3 remaining unrun all hang even with
8000 frames and need emulator fixes:

```text
[PPU VBlank Timing]  NMI disabled at VBlank   @ 0456
[Sprite Evaluation]  OAM Corruption           @ 047B
[PPU Misc.]          $2007 Stress Test        @ 048E
```

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
- **CLI debugger**: `multiemu run <rom> --dump-ppu` prints one CPU/PPU/APU state
  line per frame; `--break <hex>` steps instructions until the PC matches and
  prints the state.

### Performance track (started 2026-11-07)

Baseline for the accuracy core, headless (`multiemu run <rom> --frames 1200
--core multiemu-nes-accuracy` on AccuracyCoin, release build): **~535 fps**
(about 9x real-time). The default `multiemu-nes` core runs at ~1200 fps, so the
accuracy core has clear headroom.

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
