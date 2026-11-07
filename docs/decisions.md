# Decisions

Short ADRs for choices that are expensive to reverse.

## ADR-0001 — MIT license

The project is MIT. Consequences:

- **GPL-3.0 cores cannot be linked or vendored.** MesenCE and RustyNES are
  therefore **read-only references**: behaviour, test suites and design ideas
  may be studied, but no code, tables or assets are copied into this tree.
- **ares' NES core (ISC) is license-compatible** if a port or FFI binding is
  ever wanted; keep its notice if that happens.
- Cores must be MIT/Apache-2.0/ISC/0BSD/Zlib or public domain.

## ADR-0002 — Rust

Rust 2021, MSRV 1.88 (driven by the current NES backend). The core API is a Rust
trait; a C ABI can be added later for plugins without changing the trait.

## ADR-0003 — core ABI is a trait, not libretro (yet)

A trait keeps the host type-safe and lets cores expose richer metadata than
libretro's C structs. If third-party cores or RetroArch distribution matter later,
add a `multiemu-libretro` adapter crate; do not make libretro the internal model.

## ADR-0004 — NES backend: tetanes-core

Chosen because it is MIT OR Apache-2.0, actively maintained, Rust, has save
states/run-ahead/filters/zapper, and covers 30+ mappers and >90% of the licensed
library. It sits behind `multiemu-nes`, so it is replaceable.

Rejected for now:

- **RustyNES** — GPL-3.0, NES-only, larger surface.
- **MesenCE** — GPL-3.0, C++/C# application, no library ABI.
- **ares** — ISC and excellent, but C++ and coroutine/global based; useful as a
  correctness oracle and as a possible future port, not the first backend.

## ADR-0005 — scope order

1. Licensed NES library first.
2. Then unlicensed/Homebrew boards, FDS, Vs. System, NSF.
3. A second core for the same system only when the first one has a concrete gap
   (e.g. a mapper or accuracy mode).
4. A second system only after the NES slice is product-quality, so the ABI is
   proven before it is copied.

## ADR-0006 — provenance firewall

- No GPL code, generated tables or test assets from MesenCE/RustyNES enter this
  repository.
- Public test ROMs are used as oracles only where their licenses allow it.
- Every third-party component is listed in `NOTICE` with its license text under
  `licenses/`.
- No ROMs are committed. Test fixtures must be synthetic or freely licensed.

## ADR-0007 — frontend blueprint: Dolphin's UX, original implementation

The desktop shell follows Dolphin's **information architecture and interaction
model**: menu bar, toolbar, game library as the home screen, a single settings
window with a category rail, per-game properties, and an always-visible status
bar. See `docs/frontend.md` for the mapping.

Dolphin is GPL-2.0-or-later. Therefore:

- no Dolphin source, icons, theme assets, translations or database files are
  copied into this repository;
- the UI is implemented from scratch with `egui`/`eframe`;
- feature parity is a design target, not a code-reuse route.

This keeps the shell MIT while borrowing the proven UX structure.

## ADR-0008 — reference-parity accuracy core; TetaNES becomes the fallback

The NES target is **reference parity**, the license stays MIT, the core is pure
Rust, and there is no deadline. The consequences:

- A second NES core is written from public hardware documentation and
  ISC-licensed ares source (`multiemu-nes-accuracy`), independent of any GPL
  source. ares adaptations are attributed in `PROVENANCE.md` and `NOTICE`.
- TetaNES remains the shipped compatibility backend and the default until the
  accuracy core passes its gates.
- Both cores implement `multiemu_core::Core`; selection is by `CoreInfo.id`.
- GPL emulators (MesenCE, RustyNES, Dolphin) are run as black-box oracles only
  and are never copied or linked. ares is ISC, so documented adaptations are
  allowed with attribution.
- The bar, oracle hierarchy and milestones are in `docs/accuracy.md`.

This is additive: the shell, ABI and frontend do not change.
