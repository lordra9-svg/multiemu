# NES oracle emulators — feature matrix

A feature-level survey of the NES emulators Unko uses as oracles or peers,
compared with Unko's current shell. It is input for prioritisation, not a
commitment: the "gaps" section lists candidates, and the project owner picks
what to build.

Method: public documentation and release notes, plus the locally checked-out
reference sources (`refs/ares`, `refs/mesence`, `refs/rustynes`) for
behaviour that the docs do not state. No GPL code is copied; only
user-visible features and documented behaviour are recorded.

Legend: **Yes** / **Partial** / **No**; "—" means not part of that
emulator's scope.

## At a glance — emulation

| | ares | MesenCE | FCEUX | Nestopia UE | Nintendulator/NRS | puNES | RustyNES | **Unko** |
|---|---|---|---|---|---|---|---|---|
| Accuracy model | cycle-level, per-dot PPU | cycle-level, per-dot | older cycle-ish core, TAS-oriented | cycle-accurate CPU, dot PPU | cycle-accurate, quirk-focused | cycle-accurate (2nd to Mesen on test-ROM rankings) | one master clock, per-half-cycle | dot-locked CPU/PPU, per-cycle bus |
| AccuracyCoin | — (not claimed) | 100% claimed (v2.x) | not claimed | not claimed | test-ROM reference testbed | ranked high | 146/146 | **146/151 (5 DRAW)** |
| Mappers | many boards; NES core subset | 290+ | broad | broad + NstDatabase | NRS: all NES 2.0 mappers | broad + UNIF | 208 families | **245 implementations incl. 446** |
| FDS | Yes | Yes | Yes | Yes | Yes | Yes (QD too) | Yes (real BIOS) | **Yes (real BIOS)** |
| VS. System / PlayChoice | VS | VS | VS (fixed list) | VS + PlayChoice-10 | VS | VS Uni + Dual | VS + PlayChoice-10 | **No** |
| PAL / Dendy | region models | Yes | Yes (NTSC-PAL hybrid too) | Yes | Yes | Yes | Yes | **Yes** |
| Expansion audio | VRC6/7, MMC5, N163, 5B, FDS, EPSM | VRC6/7, MMC5, N163, 5B, FDS, EPSM | VRC6/7, MMC5, N163, 5B, FDS | VRC6/7, MMC5, N163, 5B, FDS | VRC7, MMC5, N163, 5B, FDS (DLLs) | VRC6/7, MMC5, N163, 5B, FDS | VRC6/7, MMC5, N163, 5B, FDS | **VRC6/7, MMC5, N163, 5B, FDS** |
| Devices | gamepad, Zapper, Family BASIC keyboard, data recorder | pad, Zapper, Power Pad, Four Score, paddle, keyboard, microphone, more | pad, Four Score, Zapper, Power Pad, Arkanoid, Space Shadow, Family Keyboard | pad, Zapper, Power Pad, Four Score, exotic peripherals | pad, Zapper, Four Score, Game Genie | pad, Zapper, Power Pad, paddle, Family/Subor keyboard, tape, microphone | Four Score, Zapper, paddle, Power Pad, keyboards, mouse | **pad, Zapper, Paddle, Power Pad + Family Trainer, Four Score, Family BASIC keyboard, Subor keyboard, microphone, tape loopback, Miracle Piano transport, Datach scanner** |
| Overclocking | No | Yes | No | 2x option | No | PPU overclock | speed control | No |
| Sprite-limit toggle | No | Yes | No (sprite limit disable via CLI) | Yes | No | No | Yes (enhancement) | No |

## At a glance — player and creator features

| | ares | MesenCE | FCEUX | Nestopia UE | Nintendulator | puNES | RustyNES | **Unko** |
|---|---|---|---|---|---|---|---|---|
| Save states | Yes | Yes | Yes | Yes | Yes | Yes | Yes | **Yes** |
| Rewind | Yes | Yes | Yes | frontend (libretro) | No | Yes (timeline) | Yes | No |
| Fast-forward / speed control | Partial | Yes | Yes | Yes | Yes | Yes (up to 64x) | 25–300% + fast-forward | No (debugger step only) |
| Frame advance | debugger | debugger | Yes | Yes | debugger | Yes | Yes | debugger only |
| Run-ahead | No | No | No | No | No | No | Yes | No |
| Game Genie / cheats | cheats editor | Yes | Yes (GG + RAM) | cheat manager | GG (3 codes) | Yes | GG database + RAM | No |
| Netplay | No | Yes | experimental (2.7) | Yes | No | Yes | rollback, 2–4p | No |
| Movies / TAS | trace | movie recording | rich TAS editor + movies | movie recording | movie recording | No | TAStudio + movie import | No (WAV/PNG/GIF only) |
| Debugger | memory editor, tracer | full debugger suite | full (RAM search, trace, hex, PPU/nametable) | no | full (breakpoints, viewers, palette) | no | conditional bp, watchpoints, hex, callstack | **CPU/PPU/APU + hex + PC breakpoints** |
| Video filters / shaders | slang shaders | NTSC, xBRZ, HD packs | NTSC filters, palettes | Blargg NTSC, HQx, scanlines | palette editor, scanlines | many (NTSC, CRT, xBRZ, hqNx) | NTSC + CRT shader stack | **raw/composite/CRT + scanline/LCD/CRT output presets** |
| Palettes | several | many + custom | many + `.pal` | many + `.pal`, palette editor | palette editor + custom | many + editor + Raw | `.pal` + generated NTSC | **8 presets + `.pal`/`.palette` import** |
| HD packs | No | Yes | No | No | No | No | Yes | No |
| Lua / scripting | No | debugger scripting | Yes (Lua 5.1) | No | No | No | Yes (sandboxed Lua 5.4) | No |
| RetroAchievements | No | Yes (libretro) | No | No | No | No | Yes | No |
| Recording | WAV/streams | video+audio | AVI + WAV, screenshots | AVI/WAV, screenshots | AVI | FFmpeg video/audio | A/V recording | **WAV/PNG/GIF** |
| Archive load | No | Yes | Yes | ZIP/7z | No | Yes | Yes | **ZIP/7Z/RAR** |
| IPS/BPS patching | No | IPS/BPS | IPS | IPS | No | Yes | soft-patching | No |
| Platforms | Win/mac/Linux | Win/mac/Linux, libretro | Win/Linux/mac (Qt) | Win/mac/Linux/BSD | Windows | Win/Linux/BSD/mac | Win/mac/Linux/web/Android/iOS | **Linux/Win/mac/iOS/Android** |
| License | ISC | GPL-3.0 | GPL-2.0 | GPL-2.0 | GPL-2.0 | GPL-2.0 | GPL-3.0 | MIT (+ ISC adaptations) |

## Notes per emulator

### ares (ISC)
Multi-system accuracy project (higan/bsnes lineage). Its Famicom core has a
per-dot PPU, an FDS implementation, VS support, expansion audio including
EPSM, and expansion-port devices: the Family BASIC keyboard, Famicom data
recorder, and EPSM. Controller ports support the standard pad and Zapper.
Desktop features include save states, a rewind buffer, a cheat editor, a
memory editor, a tracer, tape support, and slang shader presets. It has no
netplay, run-ahead, HD packs or achievements. Source: local `refs/ares`
(`ares/fc/expansion`, `desktop-ui/tools`, `desktop-ui/program/rewind.cpp`).

### MesenCE (GPL-3.0)
The reference for feature breadth: 290+ mappers, FDS, VS, Dendy, a
per-dot PPU with the documented quirks, HD packs, NTSC and xBRZ filters,
per-channel audio with stereo/reverb, overclocking, rewind, netplay, cheats,
movie recording, and a deep debugger with memory search, palette/nametable
viewers and scripting. Libretro exposes states/rewind/netplay/cheats/
achievements. Source: libretro docs, emulation wiki, local `refs/mesence`.

### FCEUX (GPL-2.0)
The TAS and ROM-hacking workbench: powerful movie/TAS editor, RAM
search/watch, hex editor, trace logger, code/data logger, Lua 5.1 scripting,
Game Genie encode/decode, palette editor, AVI capture, and a broad device set
(Four Score, Zapper, Power Pad, Arkanoid, Space Shadow, Famicom 4-player,
Family Keyboard). Netplay is basic/experimental in 2.7. Accuracy is not its
selling point; it is useful as a feature benchmark and as a movie/TAS format
source.

### Nestopia UE (GPL-2.0)
Cycle-accurate CPU and dot PPU, NstDatabase.xml for region/mapper/4-player
detection, VS and PlayChoice-10, FDS, exotic peripherals (Karaoke Studio,
Barcode Battler, Datach, data recorder, Turbo File), `.pal` palettes and a
palette editor, Blargg NTSC filters, HQx/scanline shaders, cheat manager,
movies, IPS patching, and a 4-player adapter option. Rewind is provided by
libretro, not the standalone app. Source: libretro docs, project readme,
Nestopia UE site.

### Nintendulator / NintendulatorNRS (GPL-2.0)
The quirk-focused test bed. NRS supports every documented NES 2.0 mapper,
famiclone chips (VT02/VT03/VT09/VT32), VS DualSystem and per-device key
configuration. Features: Game Genie (3 codes), 4-player, movies, AVI, a
full debugger (read/write/execute/opcode/interrupt breakpoints, CPU regs,
PPU/sprite/palette memory, scanline breakpoints), palette editor and header
editor. No netplay, shaders, HD packs or rewind. Windows-first.

### puNES (GPL-2.0)
Ranked second to Mesen on public test-ROM rankings. Broad mappers + UNIF, FDS
(with QD), VS Uni/Dual, NSF/NSF2/NSFe, rewind timeline with 1–64x scrubbing,
netplay, GLSL shaders plus NTSC/CRT filters, palette editor + "Raw" palette
for shader decoding, cheats, per-game PPU overclock, FFmpeg video/audio
recording, Family/Subor keyboard, tape and microphone support.

### RustyNES (GPL-3.0)
The closest feature peer to Mesen and Unko's primary differential oracle:
208 mapper families, real-BIOS FDS, VS/PlayChoice, rollback netplay,
RetroAchievements, TAStudio, HD packs, CRT shader stack, `.pal` palettes,
save states, rewind, run-ahead, 25–300% speed, Game Genie database, raw RAM
cheats, Lua 5.4, movie formats, and a debugger with conditional breakpoints,
watchpoints, RAM search, hex editor and callstack. Also ships web/mobile/
libretro builds. Source: local `refs/rustynes/README.md`.

## Remaining gaps (2026-11-13)

This is the current answer to "what is still missing from the oracles". The
matrix above is the comparison; this list is the work queue.

**Implemented since the first survey**
- Per-channel audio mixer (5 APU channels + expansion), latency target,
  resampler quality, NES RF/Crisp/Warm/Linear/custom filters.
- `.pal`/`.palette` import and eight display presets (four generated from the
  ares ISC colorimetry).
- Output scaling/aspect/supersampling, window presets and scanline/LCD/CRT
  output presets.
- Microphone, Family BASIC tape loopback, Miracle Piano MIDI transport and
  Datach barcode scanner (synthetic-tested; real-ROM validation pending).
- MiSTer accuracy options: PPU reset behaviour, RAM clear, audio source,
  overscan, edge masking, and the NES 2.0 missing-header warning.
- Per-system library roots (NES scanned), grid/list columns, detachable
  windows, Devices matrix and reset-all.

**Still needed — player-facing**
1. **Cheats**: Game Genie (6/8-character with compare) and raw RAM, up to 32
   entries per game, enable/disable and per-game persistence are implemented.
2. **Speed and rewind**: fast-forward (hold), 25-300% presets and frame
   advance are implemented. The rewind ring over the deterministic
   `UNKOST06` states is still queued (states are 0.7–1.5 MB, so the ring needs
   a memory budget or delta compression).
3. **GPU shader pass**: built-in output presets exist, but arbitrary
   GLSL/WGSL/slang shaders and MiSTer-style HQ2x/CRT-25/50/75 scanline
   scaling are not implemented.
4. **NTSC filter fidelity**: the core Composite/CRT filters are a 1-2-1
   blend + scanlines. MiSTer offers Budget TV/Premium TV, brightness and
   sharpness controls; Mesen/FCEUX/Nestopia have full NTSC models.
5. **IPS/BPS soft-patching and a NES 2.0 header editor/repair database**
   (MiSTer warns and points at an external repair tool; Unko now warns but
   has no editor).

**Still needed — hardware breadth**
6. **VS. System / PlayChoice-10**: mapper 99/VS input, coin/service/DIP
   switches, and 2C03/2C04-xxxx/2C05 palettes with the 2C05 security code.
   Blocked on VS ROMs and palette/security dumps; no local evidence.
7. **Miracle Piano MIDI replies**, **Datach EEPROM (X24C02)**, **FDS disk
   speed / automatic side swap**, **Karaoke Studio**, **Family Trainer
   variant**, **Subor keyboard**, **mouse** and **Famicom 3D glasses**.
   The transports/scanners/microphone/tape are in; these need protocol
   ROMs or hardware captures.
8. **Optional accuracy knobs**: sprite-per-line limit / extra sprites,
   CPU/PPU overclock. Deliberately not added (not accuracy-preserving);
   the NES/Famicom PPU-reset and RAM-clear knobs are in.

**Still needed — creator/community**
9. **Movies/TAS** (input recording/playback, TAStudio).
10. **Netplay** (rollback 2–4 players).
11. **Lua scripting**, **RetroAchievements**, **HD packs**, **run-ahead**.

## Licensing and provenance

ares is ISC; FCEUX, Nestopia UE, Nintendulator, puNES are GPL-2.0; MesenCE
and RustyNES are GPL-3.0. None of their code, assets or databases may be
ported into this MIT project. The observations above are limited to
documented, user-visible features and behaviour. Palette tables are factual
colour data, but any imported set will be recorded with its source and kept
in a data file rather than copied from a GPL implementation unless the
license permits it.

## Sources

- ares: <https://ares-emu.net>, local `refs/ares` (`ares/fc`, `desktop-ui`).
- MesenCE: <https://docs.libretro.com/library/mesen2/>,
  <https://emulation.gametechwiki.com/index.php/Mesen>,
  local `refs/mesence`.
- FCEUX: <https://fceux.com/web/fceux-sdl-docs.html>,
  <https://fceux.com/web/help/LuaScripting.html>,
  <https://fceux.com/web/home.html>.
- Nestopia UE: <https://docs.libretro.com/library/nestopia/>,
  <https://github.com/0ldsk00l/nestopia/blob/master/readme.html>,
  <https://nestopiaue.com/>.
- Nintendulator: <https://emulation.gametechwiki.com/index.php/Nintendulator>,
  <https://forums.nesdev.org/viewtopic.php?t=18250>.
- puNES: <https://emulation.gametechwiki.com/index.php/PuNES>,
  <https://github.com/punesemu/puNES>.
- RustyNES: local `refs/rustynes/README.md`.
