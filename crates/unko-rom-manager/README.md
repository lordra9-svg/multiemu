# unko-rom-manager

A standalone DAT auditing and 1G1R library manager. It has no dependency on
`unko-nes`, `unko-core`, the Unko app or the CLI, so it can be embedded in any
Rust project or used through the `unko-app` ROM Manager window.

## Features

- Logiqx XML DAT parsing (No-Intro/MAME-style `datafile`, `game`, `rom`,
  parent/clone ids, categories, size/CRC/MD5/SHA1/status).
- Loose-file and ZIP scanning with CRC-32, MD5 and SHA-1.
- iNES headerless matching: a `.nes` file is hashed whole and with its 16-byte
  header removed.
- Complete/partial/missing game status, unknown files and duplicates.
- 1G1R selection with region, language, verified, retail, parent and revision
  preferences.
- Fixdat and CSV reports.
- Safe copy/rebuild with per-file hash verification.

## Usage

```rust
use unko_rom_manager::{audit, OneGameOneRom, RebuildOptions, ScanOptions, rebuild};

let mut progress = |_: &unko_rom_manager::ScanProgress| {};
let report = audit(
    std::path::Path::new("dat/No-Intro.dat"),
    std::path::Path::new("roms"),
    &ScanOptions::default(),
    &mut progress,
)?;
let selected = OneGameOneRom::default().select(&report.dat);
let result = rebuild(
    &report,
    &selected,
    &RebuildOptions { output: "rebuilt".into(), ..RebuildOptions::default() },
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The crate also ships `unko-rom-manager`, a small headless CLI:

```sh
cargo run -p unko-rom-manager -- \
  --dat "No-Intro.dat" --input roms --1g1r \
  --fixdat missing.xml --csv report.csv --copy rebuilt
```

## Current limits

ZIP is the only archive format read today. 7z/RAR/CHD, TorrentZip/RVZSTD output,
ROM patching, DAT download/update and a persistent hash cache are future work.
The rebuild operation only copies; it never deletes or moves source files.
