# Test-ROM inventory and NESdev suite status

The public test ROMs are not committed. `test-roms/` is a local, gitignored
drop point; the committed tests skip cleanly when it is absent.

## Local drop (`test-roms/`, 72 files)

| Group | Files | Status |
|---|---|---|
| CPU | `nestest`, blargg `instr_basics`/`instr_timing`, `cpu_interrupts_v2` singles | `nestest` 8,980/8,980 lines; 13/13 blargg interrupt singles; timing pass |
| PPU | `ppu_vbl_nmi` 12, `ppu_sprites` 19, `oam_read`, `oam_stress`, `sprite_ram` | 12/12 and 19/19 |
| APU | blargg APU/DMC 13, DMC latency/pitch/status, square/sweep/noise/linear-counter | 13/13; the rest are diagnostic |
| Battery | `AccuracyCoin.nes` + catalog | 146/146 non-DRAW (146/151 total, 5 DRAW, 0 failed) |

See `docs/accuracy-status.md` for the dated evidence.

## NESdev archive (`christopherpow/nes-test-roms`, 263 `.nes`)

Downloaded to a temporary directory and run with:

```sh
cargo run --release -p nes_accuracy_core --example testrom -- <rom> 1200
```

`testrom` polls the cartridge work-RAM text at `$6000` and stops on
`Passed`/`Failed`. Many NESdev tests only report on screen or by audio, so a
missing result is not a failure.

Latest run:

- **17 machine-readable passes**
- **4 machine-readable failures**, in two unique tests:
  - `mmc3_test_2/rom_singles/4-scanline_timing.nes` — test 12, line-239
    `$2000=$10` boundary (duplicated by `mmc3_test/4-scanline_timing.nes`)
  - `mmc3_test_2/rom_singles/6-MMC3_alt.nes` — alternate-revision behavior;
    the ROM has no NES 2.0 submapper, so a revision selector is required
  - `mmc3_test/6-MMC6.nes` — MMC6 IRQ behavior (the combined `mmc3_test`
    suite)
- **242 without a machine-readable result**: visual, audio, input-driven or
  screen-only tests. They load and run without panic; verdicts need a
  screenshot/audio oracle or a controller script.

The MMC3 A12 work (see `docs/accuracy-status.md`) fixed
`1-clocking`, `2-details`, `3-A12_clocking`, `5-MMC3` and the same subtests in
`mmc3_test`. The suite is now 4/6 for the MMC3 subtests and 4/6 including the
MMC6/alternate-revision variants.

## Still needed

### BIOS / firmware

| Asset | Needed for | Notes |
|---|---|---|
| `disksys.rom` (FDS BIOS) | `fdsirqtests`, FDS mirroring/`$4023`/audio/`$4030` tests, every `.fds` title | A local Myrient archive has the BIOS zip; it is **not** in `test-roms/` and the desktop app has no `.fds` loader yet |
| VS. System BIOS/game ROMs + 2C03/2C04/2C05 palette/security data | `characterize-vs`, VS compatibility | Blocked on ROMs and palette dumps |
| PlayChoice-10 BIOS/ROMs | PC10 compatibility | Blocked on assets |
| Miracle Piano firmware | MIDI replies | The transport is implemented, but replies need the piano firmware or a real unit |
| Datach external X24C01 dump (Battle Rush) | external EEPROM | Internal X24C02 is implemented; the external chip is not |

### NESdev suites not yet run to a verdict

- **FDS**: `FdsIrqTests`, `FDS-Mirroring-Test`, `FDS-4023-Test`,
  `FDS-Audio-Registers`, `FDS-4030D1-Addr` (all need the BIOS).
- **Mapper-specific**: `BNTest`, `bxrom_512k_test`, `Holy Mapperel`,
  `mmc3bigchrram`, `mmc3irqtest`, `mmc5ramsize`, `mmc1atest`, `test28`,
  `vrc24test`, `vrc6test`, `serom`, NES 2.0 submapper tests 2/3/7/34,
  `n163_soundram`, `n163_soundram_init`, `fme7acktest`, `fme7ramtest`.
  The archive covers `exram`, `mmc5test`, `mmc5test_v2`, `MMC1_A12` and
  `m22chrbankingtest`; those load but have no machine-readable result.
- **Input**: `allpads`, `dma_sync_test_v2`, `PaddleTest3`, `vaus`,
  `vaus-test`, `read_joy3`, `Zap Ruder`, `spadtest-nes`, `mset`, `mict`,
  `Telling LYs?`, `ctrltest`, `raw pack2`, zapper tests,
  `powerpadgesture`, `POWERPAD.NES`, `d34test`, `kmbtest`, `Hori Tracker`.
  The archive has `PaddleTest3` and `read_joy3`; the rest need a controller
  script or a manual pass.
- **Audio**: `apu_mixer`, `apu_reset`, `apu_test` singles, `dmc_tests`,
  `dmc_dma_during_read4`, `dpcmletterbox`, `volume_tests`, `pal_apu_tests`,
  `nes-audio-tests`. These need an audio capture and a reference recording,
  not just a text result.
- **Visual / timing**: `color_test`, `blargg_ppu_tests_2005`,
  `full_palette`, `nmi_sync`, `ntsc_torture`, `oamtest3`, `palette`,
  `ppu_read_buffer`, `scanline`, `sprdma_and_dmc_dma`, `tvpassfail`,
  `240pee`, `NEStress`, `oc-r1a`.
- **CPU**: `branch_timing_tests`, `cpu_dummy_reads`, `cpu_dummy_writes`,
  `cpu_exec_space`, `cpu_flag_concurrency`, `cpu_reset`,
  `cpu_timing_test6`, `instr_test-v5`, `ram_retain`. Several of these are in
  the archive and boot; their verdicts are screen-only.

## Tool

`crates/nes_accuracy_core/examples/testrom.rs` is the probe used above. It
accepts a ROM and a frame budget and prints the `$6000` text, `$F0`/`$F8`
completion bytes and the number of distinct final-frame colors.
