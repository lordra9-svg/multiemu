# Development examples

These binaries are development harnesses used while bringing the accuracy core
up to reference parity. They are not part of the public API and may change
without notice. All of them expect the test ROMs to be supplied locally under
`test-roms/` (never committed).

| Example | Purpose |
|---|---|
| `accuracycoin` | Run the AccuracyCoin catalog, either whole or `--isolated`, optionally with `--only <addr>`. This is the canonical battery runner. |
| `accoin_one` | Run a single catalog row and print its result address/value. |
| `accoin_nav` | Drive the AccuracyCoin menu and print the cursor/page state; useful when navigation regresses. |
| `accoin_ram` | Dump the CPU RAM window around a selected test result. |
| `accoin_stall` | Find rows that hang or stall (for example at `$F278`). |
| `accoin_trace` | Trace per-row results across a run. |
| `inspect` | Boot a ROM for N frames headlessly and print display/scanline diagnostics. |
| `apudump` | Per-CPU-cycle APU channel trace for one frame; requires `--features apu-trace`. Compares directly with the reference oracle's `APU_TRACE` records. |
| `exectrace` | Per-instruction `(CPU cycle, PC)` trace for one frame; requires `--features apu-trace`. Compares with the oracle's `TRACE_AT_FRAME`/`TRACE_WINDOW` records. |
| `expwrites` | Per-cycle expansion-port and APU register write trace (`$4000-$4017`, `$4020-$FFFF`) for one frame; requires `--features apu-trace`. |

Typical invocation:

```sh
cargo run -p nes_accuracy_core --release --example accuracycoin -- \
  test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated --only 46A
```
