# AccuracyCoin development harnesses

These binaries drive the AccuracyCoin ROM through the standalone `unko-nes`
core. The ROM and catalog are not committed; supply them under `test-roms/`.

| Example | Purpose |
|---|---|
| `accuracycoin` | Run the AccuracyCoin catalog, either whole or `--isolated`, optionally with `--only <addr>`. This is the canonical battery runner. |
| `accoin_one` | Run a single catalog row and print its result address/value. |
| `accoin_nav` | Drive the AccuracyCoin menu and print the cursor/page state. |
| `accoin_ram` | Dump the CPU RAM window around a selected test result. |
| `accoin_stall` | Find rows that hang or stall (for example at `$F278`). |
| `accoin_trace` | Trace per-row results across a run. |
| `dump2007` | Dump `$2007` behaviour data for a catalog row. |

Typical invocation:

```sh
cargo run -p unko-nes-dev --release --example accuracycoin -- \
  test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated
```
