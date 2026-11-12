# Performance

The reference-parity core is single-threaded and deterministic; performance
work is therefore about the per-cycle bus/PPU/APU path and about not making the
shell wake up more often than necessary.

## Current numbers

Measured on the development machine with the release profile:

| Workload | Throughput |
|---|---|
| MMC2 title, 2,000 frames | ~375-416 fps (6.2-6.9x real-time) |
| MMC5 title, 1,200 frames | ~355-360 fps (5.9-6.0x real-time) |
| Synthetic NROM, 3,000 frames | ~813-819 fps (13.5-13.6x real-time) |
| 7,101-image audit, 20 frames | ~43-45 s wall, parallel |
| Same audit with 8 CPU hogs | ~62 s wall, identical totals |

A non-rendering NROM runs faster still, so the synthetic benchmark enables
rendering: it exercises the CPU bus, the PPU background pipeline and the APU.

Recent changes: the mapper clock/IRQ/CPU-address callbacks are skipped for
boards that do not implement them, expansion-audio mixing is skipped for boards
without audio hardware, and the PPU CHR cache is refreshed only when a mapper
write invalidates it. Pausing on focus loss drops the shell to 4 Hz when the
window is inactive.

## CI budget

`unko bench --synthetic [frames] --min-fps N` runs a self-contained rendering
NROM (no ROM file or copyrighted content) and fails when throughput falls below
the budget. CI requires **300 fps** for 3,000 frames on the shared Linux runner;
the current development machine measures about **813-819 fps** (13.5x
real-time) for the same workload. This catches gross per-cycle regressions
without making CI dependent on a commercial ROM.

## PGO

`scripts/pgo.sh <rom-directory>` builds an instrumented binary, trains it on a
corpus slice, and produces a profile-guided build under `target/pgo`. Measured
on 2026-11-12 it is **12-13% faster** on the MMC2 benchmark (about 458-463 fps
versus 375-416 fps plain) and **26% faster** on the synthetic NROM (about
1029-1040 fps versus 813-819 fps), i.e. 17.1x versus 13.5x real-time. The
script never copies ROM data into the repository.

PGO profiles are compiler-version and target specific, so build them on the
machine (or CI runner) that will ship the binary.
