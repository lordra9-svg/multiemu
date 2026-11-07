# System and core entries

Every system the shell intends to support, the candidate cores, and the
licensing decision. Only cores marked **linkable** can ship inside the MIT
binary; the rest are references, oracles, or separate processes.

Status: `shipped` · `planned` · `deferred` · `reference-only`

## NES / Famicom — shipped, plus an accuracy core

| | |
|---|---|
| Compatibility core | `tetanes-core` 0.17 (Rust, MIT OR Apache-2.0) |
| Accuracy core | Rust, MIT + documented ISC (ares) adaptations — in progress |
| Adapter | `crates/multiemu-nes` (TetaNES) |
| Accuracy bar | reference parity; see `docs/accuracy.md` |
| Scope | licensed library, then FDS/VS/PAL/Dendy/expansion/peripherals/unlicensed/NSF |
| Status | compatibility core shipped; accuracy core at A5 (CPU/PPU 12+19, APU 13, mappers + system + Core ABI; AccuracyCoin 139/151) |

TetaNES remains the default until the accuracy core passes its gates; both will
be selectable by `CoreInfo.id`. The accuracy core is MIT with documented
ISC-licensed adaptations from ares; no GPL source is read, translated or
linked. Provenance is recorded in `crates/multiemu-nes-accuracy/PROVENANCE.md`.

Next: close the remaining A5 failures (DMA/abort timing, NMI hijack, `$2007`
read buffer, scanline-0 sprites), then A6 breadth.

## SNES / Super Famicom — planned

| | |
|---|---|
| Primary | **ares `sfc/`** (C++, ISC) — linkable via a C ABI |
| Oracle | **MesenCE `Core/SNES`** (C++, GPL-3.0) — reference only |
| Size | ares 19.9k LOC; Mesen 35.7k LOC |
| Coverage | 65816, SPC700, S-DSP, dot/scanline PPU, SuperFX, SA-1, SDD1, SPC7110, CX4, DSP1-4, OBC1, ST018, MSU1, Satellaview, Sufami Turbo, BS-X |
| Status | planned after the NES slice is product-quality |

Why ares: it is the only permissively licensed implementation of the two and it
already runs inside the multi-system ares scheduler. Mesen supplies the
compatibility oracle, the coprocessor feature checklist, and the overclock /
SPC-clock / strict-board ideas.

Integration: build `ares/sfc` plus its shared CPU/DSP components as a static
C++ library behind `multiemu_sfc_*`; expose the same `Core` surface. Do not port
Mesen code.

## Nintendo 64 — planned

| | |
|---|---|
| Primary | **ares `n64/`** (C++, ISC) — linkable via a C ABI |
| Reference | **gopher64** (Rust, GPL-3.0) — structural reference only |
| Size | ares 31.9k LOC excluding vendored Vulkan; gopher64 23.9k Rust LOC |
| Renderer | parallel-rdp (MIT) over Vulkan, used by both |
| Coverage | VR4300 CPU + recompiler, RSP + recompiler, TLB/FPU/COP2, VI/AI/PI/SI/RI/MI, RDRAM, PIF/CIC, 64DD, Aleck 64 |
| Status | planned after SNES |

Why ares: ISC, genuinely low-level, recompilers, 64DD/Aleck 64, and the same
framework as the other systems. gopher64 proves the Rust shape but is GPL, so
it is read-only. N64 requires Vulkan; the shell's renderer plan must account
for that before this milestone starts.

## GameCube / Wii — deferred

| | |
|---|---|
| Candidate | **Dolphin** (C++, GPL-2.0-or-later) |
| Size | 476k LOC core; 47k PowerPC/JIT; 95k video backends; 17k DSP |
| Status | deferred; not a registry entry |
| Reason | GPL-2.0+ cannot link into an MIT binary, and Dolphin is an application with no embedding ABI |

If GC/Wii is pursued, the only defensible shape is a **separate GPL process**
launched by the shell, with its own distribution and source offer — a legal and
product decision, not a core adapter. Do not plan it as a `multiemu-core`
implementation.

## Registry policy

1. A system may have more than one core; the host chooses by `CoreInfo.id`.
2. A core enters the registry only if its license permits distribution with
   this project (MIT/ISC/Apache-2.0/0BSD/Zlib/public domain).
3. Reference implementations are never linked, vendored, translated file by
   file, or used to generate tables.
4. Third-party notices and license texts live in `NOTICE` and `licenses/`.

## Order

1. NES — shipped, continue to product quality.
2. SNES — ares backend behind the C ABI.
3. N64 — ares backend + Vulkan renderer.
4. GC/Wii — optional external Dolphin process, only if the product needs it.

## NES backends

| Core id | Crate | Role |
|---|---|---|
| `tetanes` | `multiemu-nes` | compatible default backend (TetaNES) |
| `multiemu-nes-accuracy` | `multiemu-nes-accuracy-core` | reference-parity accuracy backend |

Select with `--core multiemu-nes-accuracy` in the CLI; the desktop app
registers both and defaults to TetaNES.
