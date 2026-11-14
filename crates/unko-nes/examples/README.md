# Development examples

These binaries are development harnesses for the `unko-nes` core. They are not
part of the public API and may change without notice. Test ROMs are supplied
locally under `test-roms/` (never committed).

| Example | Purpose |
|---|---|
| `testrom` | Run one blargg-style ROM, print its `$6000` text, screen rows, completion bytes and final-frame colors. |
| `inspect` | Boot a ROM for N frames headlessly and print display/scanline diagnostics. |
| `menucursor` | Drive a menu ROM and print cursor/page state. |
| `dumpn` | Dump the CPU RAM window around a selected address. |
| `apudump` | Per-CPU-cycle APU channel trace for one frame; requires `--features apu-trace`. |
| `exectrace` | Per-instruction `(CPU cycle, PC)` trace for one frame; requires `--features apu-trace`. |
| `expwrites` | Per-cycle expansion-port and APU register write trace; requires `--features apu-trace`. |

Typical invocation:

```sh
cargo run -p unko-nes --release --example testrom -- <rom> 1200
```

The AccuracyCoin battery lives in the `unko-nes-dev` crate.
