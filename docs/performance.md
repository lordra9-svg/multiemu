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
| Synthetic NROM, 3,000 frames | 784.7 fps (13.06x real-time, 1.274 ms/frame) |
| 7,101-image audit, 20 frames | ~43-45 s wall, parallel |
| Same audit with 8 CPU hogs | ~62 s wall, identical totals |

A non-rendering NROM runs faster still, so the synthetic benchmark enables
rendering: it exercises the CPU bus, the PPU background pipeline and the APU.
`bench` prints the per-frame budget directly (`ms/frame`); the figures above
correspond to about 1.22 ms/frame for the synthetic NROM and 2.4-2.8 ms/frame
for the commercial boards.

Recent changes: the mapper clock/IRQ/CPU-address callbacks are skipped for
boards that do not implement them, expansion-audio mixing is skipped for boards
without audio hardware, and the PPU CHR cache is refreshed only when a mapper
write invalidates it. Pausing on focus loss drops the shell to 4 Hz when the
window is inactive.

Save-state cost (release, synthetic NROM): 417,054-byte state, **172 us** to
serialize, **237 us** to restore, 1.28 ms per frame. A rew/rewind keyframe every
30 frames therefore costs about 0.17 ms every half second, and 4-frame
run-ahead costs two state operations plus three extra frames, about 5.5 ms per
displayed frame; both stay inside the 16.6 ms budget. The GPU shader path adds
one 256x240 RGBA upload and a fullscreen triangle, not a CPU post-process.

## Worst-case load

Frames per second on an idle machine are not the target. The machine has 8
physical cores (16 threads), so the benchmark thread competes with a hog on
every physical core in the measurements below:

| Workload | Idle | 8 `yes` hogs | 15 `yes` hogs |
|---|---|---|---|
| Synthetic NROM, 3,000 frames | 813-819 fps | 419-719 fps | 375 fps |
| MMC2 title, 2,000 frames | 375-416 fps | 277 fps | - |

The spread with 8 hogs is SMT placement: when a hog shares the benchmark
thread's physical core, throughput roughly halves (819 -> 419 fps); when the
hogs land on other cores it barely moves. Even the fully oversubscribed case
stays at 6.2x real-time, so the budget is expressed against that floor:
**synthetic >= 300 fps and the MMC2 title >= 250 fps under contention**.

## Idle shell

The shell requests a repaint every 250 ms (4 Hz) when no game is loaded and
when a game is paused, and the audio stream is paused rather than fed silence.
Under the headless software renderer (llvmpipe, which is what CI and the
capture hosts use) the library window measures 6.4% of one core at that rate,
i.e. about 16 ms of rendering per repaint; this is render-bound, not
busy-waiting, and the core itself does no work while paused. The efficiency
budget is therefore: **no core CPU when paused or unfocused, <= 4 UI repaints
per second, and the per-frame ceilings above**.

## CI budget

`unko bench --synthetic [frames] --min-fps N` runs a self-contained rendering
NROM (no ROM file or copyrighted content) and fails when throughput falls below
the budget. CI requires **300 fps** for 3,000 frames on the shared Linux runner;
the current development machine measures **784.7 fps** (13.06x real-time)
for the same workload. This catches gross per-cycle regressions
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
