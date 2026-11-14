# ROM manager comparison

Comparison of the ROM managers supplied in
`multiemu-local/rom dat/`, plus what Unko's new `unko-rom-manager` core covers.

Versions inspected:

- **igir 5.8.0** (`igir-5.8.0-Linux-amd64.tar.gz`, `igir-5.8.0.zip`)
- **RomVault 3.8.0** (`ROMVault_V3.8.0.zip`)
- **ClrMamePro 4.050** (`cmp4050_64.zip`)
- **RomCenter 4.1.1** (`romcenter32_4.1.1.exe`)
- **Romulus 0.049** (`Romulus049.zip`)

Sources: each tool's shipped README/help, the included No-Intro DATs, the
official igir help/docs, the RomVault wiki comparison, and the vendors' public
feature pages. No source code was copied; this is a behavioural comparison.

Legend: ✅ supported · ⚠ partial/requires an external tool or extra setup ·
❌ not supported · — not applicable/not documented.

## Feature matrix

| Feature | igir 5.8 | RomVault 3.8 | ClrMamePro 4.050 | RomCenter 4.1.1 | Romulus 0.049 |
|---|---|---|---|---|---|
| Primary interface | CLI | Windows GUI | Windows GUI | Windows GUI + `rc.exe` | Windows GUI |
| Other platforms | Linux, macOS, Windows | Linux via Mono; command-line build | Wine | Wine | Wine |
| Logiqx XML DAT | ✅ | ✅ | ✅ | ✅ | ✅ |
| MAME ListXML | ✅ | ✅ (DATVault) | ✅ | ✅ | ✅ |
| Other DAT dialects | Logiqx XML, MAME ListXML, DATs in archives | SuperDAT, DATVault | header XMLs, batch DATs | TOSEC, No-Intro, MAME | Offlinelist, DOSCenter, hashlist, header XML |
| Multiple DATs in one job | ✅ core design | ✅ core design | ⚠ batch/sequential | ✅ database | ✅ profiles/batch |
| Parent/clone metadata | ✅ | ✅ | ✅ | ✅ | ✅ |
| CRC32 matching | ✅ | ✅ | ✅ | ✅ | ✅ |
| MD5 / SHA1 matching | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ |
| SHA256 | ✅ | ⚠ DAT-dependent | ⚠ DAT-dependent | — | — |
| Archive CRC without extraction | ✅ | ✅ optional quick scan | ✅ header scan | ✅ | ✅ cache |
| Persistent hash cache | ✅ | ✅ core design | ❌ | ⚠ database | ✅ |
| Loose files | ✅ | ✅ | ✅ | ✅ | ✅ |
| ZIP | ✅ | ✅ | ✅ | ✅ | ✅ |
| 7z | ✅ native | ✅ native | ⚠ external 7z | ✅ read | ✅ native |
| RAR | ✅ read; extract via external | ❌ | ⚠ external rar | ✅ read-only | ✅ read |
| CHD | ✅ read/verify | ✅ native, multithreaded | ⚠ external chdman | ✅ disk support | ✅ native header |
| Scan/audit | ✅ | ✅ | ✅ | ✅ | ✅ |
| Rebuild/copy/move | ✅ copy/move/link | ✅ fix/rebuild | ✅ rebuild | ✅ fix/rebuild | ✅ scan/rebuild |
| Merge modes | full non-merged, non-merged, split, merged | non-merged, split, merged | ✅ | ✅ split/merge | ✅ set modes |
| 1G1R | ✅ rich preferences | ❌ not documented | ✅ | ✅ region order/keep parent | ⚠ DAT filters/profiles; no dedicated mode documented |
| Region/language filters | ✅ | ⚠ DAT/name based | ✅ | ✅ | ✅ |
| Fixdat | ✅ | ✅ | ✅ | ✅ | ✅ |
| Have/miss report | ✅ CSV + console | ⚠ set metrics/instance locations | ✅ have/miss lists | ✅ reports | ✅ logs/fixdat |
| TorrentZip | ✅ (TorrentZip/RVZSTD output) | ✅ core design | ❌ standard zip | ❌ | ✅ |
| Header detection/removal | ✅ detect/remove, `.nes`/`.fds` | ✅ automatic | ✅ header XMLs | ⚠ NES/SNES hashing | ✅ header XMLs |
| ROM patches | ✅ APS/BPS/IPS/UPS/xdelta/etc. | ❌ | ❌ | ❌ | ❌ |
| BIOS/sample/disk handling | ✅ BIOS filters, disks | ✅ ROM/CHD, no samples | ✅ samples/CHD | ✅ samples/disk/BIOS | ✅ ROM/sample/CHD |
| DAT update/download | ❌ | ✅ DATVault | ⚠ WWW mode | ❌ | ✅ built-in updater |
| GUI | ❌ | ✅ | ✅ | ✅ | ✅ |
| Automation | ✅ full CLI | ✅ command-line version | ⚠ batch mode | ✅ `rc.exe` | ⚠ batch profiles |
| Unattended/parallel | ✅ threads | ✅ | ⚠ | ⚠ | ⚠ |

## What Unko adopted

`unko-rom-manager` is deliberately **headless, generic and independent** of
`unko-nes`, `unko-core`, the app and the CLI. It implements the common core
that matters for a playable library:

- Logiqx XML DAT ingestion (No-Intro/MAME-style `datafile`, `game`, `rom`,
  parent/clone ids, category/status, size/CRC/MD5/SHA1).
- Loose-file and ZIP scanning, with CRC32/MD5/SHA1 hashes.
- NES headerless matching: a `.nes` file is hashed both whole and with its
  16-byte iNES header removed, so a headered library can match a headerless
  DAT and vice versa.
- Match report: complete/partial/missing games, unknown files, duplicates.
- 1G1R selection with region/language/verified/retail/parent/revision
  preferences.
- Fixdat and CSV report output.
- Safe copy/rebuild of matched ROMs into an output folder, with hash
  verification; no delete or in-place move.
- ZIP output for extracted sets is not yet implemented; the current rebuild
  writes loose files. TorrentZip/RVZSTD, 7z/RAR/CHD, patching, DAT download
  and a persistent cache remain future work.

The app exposes this as a **ROM Manager** window; the same library is usable
without the app. The design follows the project rule used for the NES core:
one feature crate, no dependency on the frontend, and a CI guard that keeps
`unko-core`/`unko-nes` out of its dependency tree.
