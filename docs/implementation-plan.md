# Implementation plan — oracle gap closure

Dated 2026-11-13. This is the execution queue for the three oracle tiers,
ordered **easy → hard** and scoped by the project owner's decisions.

Scope decisions from this pass:

- One commit per item, easiest first; later items can be revisited when ROMs
  or hardware captures arrive.
- Cheats: Game Genie + raw RAM, 32 slots, per-game persistence.
- Rewind: configurable ring plus 1-frame run-ahead (0-4 configurable).
- Speed: 25/50/100/150/200/300%, hold-to-fast-forward, frame advance.
- Header repair and IPS/BPS patching: **decision deferred**, not in this queue.
- Video: GPU shader pipeline first, then the NTSC presets on top of it.
- Extra sprites and overclock: **not added** (deliberately not
  accuracy-preserving).
- VS. System / PlayChoice-10: **blocked** on VS ROMs and 2C03/2C04/2C05
  palette/security dumps.
- Peripherals: quick wins, then FDS, then the protocol-ROM items.
- Tier 3 is now in scope following the latest "all three tiers" directive;
  its items are ordered after the remaining Tier 1/2 work, easiest first.

## Queue

| # | Item | Tier | Difficulty | Depends on | Status |
|---|---|---|---|---|---|
| 1 | 25-300% speed presets, hold-FF (Tab), frame advance | 1 | Easy | – | **done** |
| 2 | Cheats: Game Genie + raw RAM, 32 slots, per-game persistence, Cheats window | 1 | Easy-medium | – | **done** |
| 3 | Quick peripherals: Family Trainer, Subor keyboard, Famicom 3D glasses, mouse | 2 | Easy-medium | protocol notes | **partial (Family Trainer + Subor done; 3D glasses/mouse need an expansion-port/output model or test hardware)** |
| 4 | Rewind ring (configurable 64-256 MB) + 1-4 frame run-ahead | 1 | Medium-hard | deterministic states | **done** |
| 5 | GPU shader pipeline (glow/wgpu) and built-in presets | 1 | Hard | renderer abstraction | **done (glow; iOS keeps the overlay fallback)** |
| 6 | NTSC Budget/Premium + brightness/sharpness as shader presets | 1 | Hard | #5 | **done** |
| 7 | FDS disk speed and automatic side swap | 2 | Hard | app FDS loader + local `disksys.rom` | **blocked** (the core/CLI can boot FDS with a user BIOS, but the library/app has no `.fds` entry path, side menu or disk-write persistence yet; auto-swap also needs real-game validation) |
| 8 | Miracle Piano MIDI replies and Datach EEPROM (X24C02) | 2 | Hard | real test ROMs | **partial (X24C02 done and persisted; MIDI replies still need firmware)** |
| 9 | Input movies/TAS recording and playback | 3 | Medium-hard | rewind input log | **done** |
| 10 | Lua scripting (sandboxed 5.4) | 3 | Hard | scripting host API | **deferred** (needs an embedded interpreter dependency, a sandbox policy and API surface; out of this pass) |
| 11 | RetroAchievements | 3 | Hard | network + memory hashes | **blocked** (requires RA credentials/network and achievement hashing; no offline validation) |
| 12 | HD packs | 3 | Very hard | graphics pack loader | **deferred** (Mesen-style pack format and tile replacement pipeline) |
| 13 | Netplay (rollback, 2-4 players) | 3 | Very hard | deterministic states + transport | **deferred** (rollback loop and NAT/transport are a separate subsystem) |

Items 7 and 8 stay queued until the required ROMs/hardware captures exist;
the transports and scanner are already synthetic-tested (see
`docs/accuracy-status.md`).

## Deferred / out of scope

- NES 2.0 header editor, repair database and IPS/BPS soft-patching (owner
  deferred the decision). The warning for missing NES 2.0 headers is in.
- Extra sprites / sprite-limit toggle and CPU/PPU overclock.
- VS. System / PlayChoice-10, including DIPs, coins and RGB PPU palettes.
- Movies/TAS, Lua, RetroAchievements, HD packs, netplay/run-ahead rollback.

## Gates per item

Each item must keep `cargo test --workspace`, clippy `-D warnings`, fmt and
the AccuracyCoin isolated battery green, and add focused unit/integration
tests for the new behavior. Display-only changes must leave the canonical
palette + Raw filter trace identical.
