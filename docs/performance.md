# Performance

The reference-parity core is single-threaded and deterministic; performance
work is therefore about the per-cycle bus/PPU/APU path and about not making the
shell wake up more often than necessary.

## Current numbers

Measured on the development machine with the release profile:

| Workload | Throughput |
|---|---|
| MMC2 title, 2,000 frames | ~392-401 fps (6.5-6.7x real-time) |
| MMC5 title, 1,200 frames | ~355-360 fps (5.9-6.0x real-time) |
| 7,101-image audit, 20 frames | ~43-45 s wall, parallel |
| Same audit with 8 CPU hogs | ~62 s wall, identical totals |

Recent changes: the mapper clock/IRQ/CPU-address callbacks are skipped for
boards that do not implement them, expansion-audio mixing is skipped for boards
without audio hardware, and the PPU CHR cache is refreshed only when a mapper
write invalidates it. Pausing on focus loss drops the shell to 4 Hz when the
window is inactive.

## PGO

`scripts/pgo.sh <rom-directory>` builds an instrumented binary, trains it on a
corpus slice, and produces a profile-guided build under `target/pgo`. On the
MMC2 benchmark this is about **10-13% faster** than a plain release build
(~442-448 fps versus ~392-401 fps). The script never copies ROM data into the
repository.

PGO profiles are compiler-version and target specific, so build them on the
machine (or CI runner) that will ship the binary.
