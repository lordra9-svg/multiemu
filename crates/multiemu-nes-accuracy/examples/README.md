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

Typical invocation:

```sh
cargo run -p multiemu-nes-accuracy --release --example accuracycoin -- \
  test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated --only 46A
```
