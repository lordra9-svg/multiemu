# MiSTer NES cores — survey

Research note for Unko's NES feature planning. It records what the MiSTer
FPGA NES core offers, how it compares on accuracy, and which features are
worth borrowing for a software emulator. No HDL or GPL code is used; this is a
feature-level survey.

## The core

- **`MiSTer-devel/NES_MiSTer`** — the main FPGA NES/Famicom core, an
  implementation of the NES in SystemVerilog for the MiSTer platform, based on
  **FPGANES by Ludvig Strigeus** and ported to MiSTer. Licensed GPL-3.0.
- **`agg23`'s NES core** — the open-source NES core for the Analogue Pocket,
  a port/tweak of the MiSTer work; the community fork by Terminator2K2 now
  ships two cores selected by a mapper chip to cover "all" MiSTer mappers, but
  without FDS or NSF support.
- **TriCNES** (`100thCoin/TriCNES`) — a TAS-oriented NES emulator written in
  C# (MIT), useful as a CPU/DMA timing reference for its mapper set
  (NROM/MMC1/2/3/4/7/9/69 and partial FDS). **It produces no audio**, so it is
  not an audio oracle; the wiki describes its DMC state machine as the source
  of one of RustyNES's DMC phase details.

Local checkouts used for comparison live under `refs/mister-nes`,
`refs/openfpga-nes` and `refs/tricnes` (not part of the repository).

## Feature inventory (NES_MiSTer)

- **Systems:** NTSC NES, PAL NES, Dendy; automatic region detection when the
  ROM has a NES 2.0 header (iNES falls back to NTSC with a warning).
- **Media:** NES cartridges, Famicom Disk System (user-supplied `boot0.rom`
  BIOS), NSF player, boot ROM slots (`boot0` BIOS, `boot1` cart, `boot2` disk,
  `boot3` custom palette).
- **Saves/states:** battery saves for most games; save states (mapper support
  table marks which boards have state support).
- **Video:** multiple palettes plus a custom palette file; overscan option.
- **Audio:** APU plus expansion audio for VRC6, VRC7, MMC5, Namco 163 and
  Sunsoft 5B; FDS expansion audio.
- **Peripherals:** standard pads, Zapper, Power Pad, microphone, Miracle
  Piano; **four players**.
- **Cheats:** up to **32 cheat codes**.
- **Accuracy aids:** "extra sprites" option (doubles sprites per line, +8
  sprites, reducing flicker; can break games that rely on the 8-sprite limit,
  e.g. *Castlevania II* swamps).
- **Mappers:** very broad. The repository's table marks support and
  save-state support for the classic set (0–5, 7, 9–11, 13, 15, 16, 18, 19,
  FDS, 21–42, 44–48, 52, 55–82, 85–95, 97, 101, 105, 107, 111–113, 118, 119,
  132, 133, 136–150, 152–156, 158, 159, 162–165, 171–173, 180, 184, 185,
  189–192, 194–197, 200, 205–212, 218, 225, 227, 228, 232, 234, 243, 246,
  255, 268, 413, 547), with a long tail explicitly unsupported (for example
  12, 43, 49–51, 53, 54, 84, 96, 99, 100, 104, 108–110, 114–117, 120,
  122–131, 134, 135, 151, 157, 160, 161, 166–169, 181–183, 186–188, 198,
  199, 201–204, 213–217, 222, 224, 226, 229–231, 233, 235–237, 240–242,
  244, 245, 248–251, 254).
- **Large games:** supports titles such as *Legend of Link* and *Rockman Minus
  Infinity*; NES 2.0 headers preferred for correct sizes/regions.

## Accuracy standing

- Forum measurements (September 2025) put the unstable core at **121/125
  AccuracyCoin tests**, with the note that a real AV Famicom scored 121/125
  as well and that MiSTer passes some tests real hardware fails. The remaining
  failures are described as PPU edge behaviour with little commercial impact.
- A later test-core article reports **128/128** on AccuracyCoin for a reworked
  PPU build — AccuracyCoin's test count changed over time (Unko uses v3.1.0
  with 151 rows, 146 non-DRAW), so the numbers are not directly comparable.
- The core exposes an option to choose NES vs Famicom behaviour for the few
  cases where "passing every test" and "playing the whole library" conflict.

## Lessons for Unko

Worth considering (none of this is implemented today):

1. **Sprite-limit/extra-sprite toggle.** A "disable the 8-sprites-per-line
   limit" or "+8 sprites" presentation option is a common MiSTer/Nestopia/
   Mesen feature; it is not accuracy-preserving, so it belongs behind an
   explicit, off-by-default gameplay option.
2. **Cheat capacity.** MiSTer supports 32 codes; Unko has no cheat engine yet.
   A Game Genie + raw-RAM cheat system with per-game persistence is the
   largest missing player-facing feature.
3. **Custom palette files.** MiSTer boots a palette from `boot3.rom`; Nestopia
   and Mesen read `.pal` files (64 × RGB triplets). Unko currently has four
   built-in presets and no file import.
4. **Peripheral breadth.** Microphone and Miracle Piano are still missing from
   Unko; FDS, Zapper, Power Pad, Four Score, paddle and Family BASIC keyboard
   are present.
5. **Region/header workflow.** MiSTer warns when a NES 2.0 header is absent and
   supports auto-region. Unko reads NES 2.0 timing/region but has no database
   of known bad headers or ROM repair workflow.
6. **Save-state coverage signalling.** MiSTer's mapper table tells the user
   which boards support states. Unko serializes every implemented mapper, so
   this is less of an issue, but a capability badge in the UI is a cheap
   equivalent.
7. **Four-player and NSF** are already covered by Unko.

## Licensing boundary

`NES_MiSTer` is GPL-3.0 and its HDL is not usable in this MIT project. The
survey above is limited to documented, user-visible behaviour and public
feature lists; no code, tables or assets were copied. The same rule applies to
the Analogue Pocket port and TriCNES.

## Sources

- <https://github.com/MiSTer-devel/NES_MiSTer> — feature list, mapper table,
  boot files, extra-sprites and cheat notes.
- <https://misterfpga.org/viewtopic.php?t=9590> — AccuracyCoin measurements and
  the NES/Famicom standards discussion.
- <https://metalgamesolid.com/uncategorized/mister-nes-core-is-it-better-than-the-real-thing/>
  and <https://metalgamesolid.com/fpga/analogue-pocket/analogue-pocket-nes-core-has-all-the-mister-mappers/>
  — test-core AccuracyCoin claim and the Analogue Pocket mapper port.
- <http://nerdlypleasures.blogspot.com/2020/11/fpga-nes-and-famicom-solutions-mapper.html>
  — cross-device mapper support comparison.
