//! DAT indexing and match reports.

use std::collections::HashMap;

use crate::dat::{Dat, DatRom, Game};
use crate::hash::Hashes;
use crate::scan::ScannedRom;

/// A ROM location inside a DAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RomRef {
    /// Game index.
    pub game: usize,
    /// ROM index inside the game.
    pub rom: usize,
}

/// How a scanned file matched a DAT ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    /// Index into the scanned list.
    pub scanned: usize,
    /// The DAT entry was matched after removing the iNES header.
    pub headerless: bool,
}

/// Per-game match state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStatus {
    /// Every ROM matched.
    Complete,
    /// Some but not all ROMs matched.
    Partial,
    /// No ROM matched.
    Missing,
}

impl MatchStatus {
    /// Short label for reports and UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Missing => "missing",
        }
    }
}

/// Match state for one game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameMatch {
    /// Number of matched ROMs.
    pub matched: usize,
    /// Number of expected ROMs.
    pub total: usize,
    /// Aggregate status.
    pub status: MatchStatus,
}

/// Summary counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Games in the DAT.
    pub games: usize,
    /// Fully matched games.
    pub complete: usize,
    /// Partially matched games.
    pub partial: usize,
    /// Games with no matched ROMs.
    pub missing: usize,
    /// ROMs in the DAT.
    pub roms: usize,
    /// Matched ROMs.
    pub matched_roms: usize,
    /// Scanned files with no DAT match.
    pub unknown: usize,
    /// Extra scanned files matching an already matched ROM.
    pub duplicates: usize,
}

/// A DAT plus its match report.
pub struct MatchReport {
    /// The DAT that was audited.
    pub dat: Dat,
    /// Every scanned candidate, in scan order.
    pub scanned: Vec<ScannedRom>,
    /// Flat ROM references, aligned with `rom_match`.
    pub roms: Vec<RomRef>,
    /// Match for each flat ROM.
    pub rom_match: Vec<Option<Match>>,
    /// Scanned entries that matched no DAT ROM.
    pub unknown: Vec<usize>,
    /// Scanned entries that duplicated an earlier match.
    pub duplicates: Vec<usize>,
    /// Per-game status, in DAT order.
    pub games: Vec<GameMatch>,
}

impl MatchReport {
    /// The scan summary.
    #[must_use]
    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            games: self.games.len(),
            roms: self.roms.len(),
            unknown: self.unknown.len(),
            duplicates: self.duplicates.len(),
            ..Summary::default()
        };
        for game in &self.games {
            summary.matched_roms += game.matched;
            match game.status {
                MatchStatus::Complete => summary.complete += 1,
                MatchStatus::Partial => summary.partial += 1,
                MatchStatus::Missing => summary.missing += 1,
            }
        }
        summary
    }

    /// The match for one game.
    #[must_use]
    pub fn game_status(&self, game: usize) -> Option<GameMatch> {
        self.games.get(game).copied()
    }

    /// The scanned source for a flat ROM.
    #[must_use]
    pub fn source(&self, flat: usize) -> Option<&ScannedRom> {
        let matched = self.rom_match.get(flat)?.as_ref()?;
        self.scanned.get(matched.scanned)
    }

    /// Flat index of a game/rom pair.
    #[must_use]
    pub fn flat_index(&self, game: usize, rom: usize) -> Option<usize> {
        self.roms
            .iter()
            .position(|reference| reference.game == game && reference.rom == rom)
    }

    /// The DAT ROM for a flat index.
    #[must_use]
    pub fn dat_rom(&self, flat: usize) -> Option<(&Game, &DatRom)> {
        let reference = *self.roms.get(flat)?;
        Some((
            self.dat.games.get(reference.game)?,
            self.dat.games[reference.game].roms.get(reference.rom)?,
        ))
    }
}

/// A DAT indexed for fast matching.
pub struct Catalog {
    dat: Dat,
    roms: Vec<RomRef>,
    by_sha1: HashMap<[u8; 20], Vec<usize>>,
    by_md5: HashMap<[u8; 16], Vec<usize>>,
    by_crc: HashMap<(u64, u32), Vec<usize>>,
}

impl Catalog {
    /// Builds the indexes for `dat`.
    #[must_use]
    pub fn new(dat: Dat) -> Self {
        let mut roms = Vec::new();
        let mut by_sha1: HashMap<[u8; 20], Vec<usize>> = HashMap::new();
        let mut by_md5: HashMap<[u8; 16], Vec<usize>> = HashMap::new();
        let mut by_crc: HashMap<(u64, u32), Vec<usize>> = HashMap::new();
        for (game_index, game) in dat.games.iter().enumerate() {
            for (rom_index, rom) in game.roms.iter().enumerate() {
                let flat = roms.len();
                roms.push(RomRef {
                    game: game_index,
                    rom: rom_index,
                });
                if let Some(sha1) = rom.sha1 {
                    by_sha1.entry(sha1).or_default().push(flat);
                }
                if let Some(md5) = rom.md5 {
                    by_md5.entry(md5).or_default().push(flat);
                }
                if let Some(crc) = rom.crc {
                    by_crc.entry((rom.size, crc)).or_default().push(flat);
                }
            }
        }
        Self {
            dat,
            roms,
            by_sha1,
            by_md5,
            by_crc,
        }
    }

    /// The indexed DAT.
    #[must_use]
    pub fn dat(&self) -> &Dat {
        &self.dat
    }

    /// Matches scanned files against this DAT.
    #[must_use]
    pub fn match_all(&self, scanned: Vec<ScannedRom>) -> MatchReport {
        let mut rom_match = vec![None; self.roms.len()];
        let mut unknown = Vec::new();
        let mut duplicates = Vec::new();
        for (index, rom) in scanned.iter().enumerate() {
            let mut matched = self.lookup(&rom.hashes).map(|_| Match {
                scanned: index,
                headerless: false,
            });
            if matched.is_none() {
                if let Some(headerless) = rom.headerless {
                    matched = self.lookup(&headerless).map(|_| Match {
                        scanned: index,
                        headerless: true,
                    });
                }
            }
            let Some(matched) = matched else {
                unknown.push(index);
                continue;
            };
            let flat = if matched.headerless {
                self.lookup(&rom.headerless.expect("headerless hash"))
                    .expect("lookup")
            } else {
                self.lookup(&rom.hashes).expect("lookup")
            };
            if rom_match[flat].is_some() {
                duplicates.push(index);
            } else {
                rom_match[flat] = Some(matched);
            }
        }

        let mut games = Vec::with_capacity(self.dat.games.len());
        for (game_index, game) in self.dat.games.iter().enumerate() {
            let total = game.roms.len();
            let matched = self
                .roms
                .iter()
                .enumerate()
                .filter(|(flat, reference)| {
                    reference.game == game_index && rom_match[*flat].is_some()
                })
                .count();
            let status = if total > 0 && matched == total {
                MatchStatus::Complete
            } else if matched > 0 {
                MatchStatus::Partial
            } else {
                MatchStatus::Missing
            };
            games.push(GameMatch {
                matched,
                total,
                status,
            });
        }

        MatchReport {
            dat: self.dat.clone(),
            scanned,
            roms: self.roms.clone(),
            rom_match,
            unknown,
            duplicates,
            games,
        }
    }

    fn lookup(&self, hashes: &Hashes) -> Option<usize> {
        if let Some(matches) = self.by_sha1.get(&hashes.sha1) {
            return matches.first().copied();
        }
        if let Some(matches) = self.by_md5.get(&hashes.md5) {
            return matches.first().copied();
        }
        self.by_crc
            .get(&(hashes.size, hashes.crc))
            .and_then(|matches| matches.first().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::parse_dat_str;
    use crate::hash::hash_bytes;
    use crate::scan::Origin;

    const DAT: &str = r#"<datafile><header><name>T</name></header>
      <game name="Parent (USA)" id="1">
        <rom name="Parent (USA).nes" size="23" crc="00000000" md5="00000000000000000000000000000000" sha1="0000000000000000000000000000000000000000"/>
      </game>
      <game name="Clone (Japan)" id="2" cloneofid="1">
        <rom name="Clone (Japan).nes" size="7" crc="00000000" md5="00000000000000000000000000000000" sha1="0000000000000000000000000000000000000000"/>
      </game>
    </datafile>"#;

    fn scanned(name: &str, bytes: &[u8]) -> ScannedRom {
        let mut full = vec![0u8; 16];
        full[..4].copy_from_slice(b"NES\x1a");
        full.extend_from_slice(bytes);
        let mut rom = ScannedRom {
            origin: Origin::File(name.into()),
            file_name: name.into(),
            hashes: hash_bytes(&full),
            headerless: Some(hash_bytes(bytes)),
            modified: None,
        };
        rom.file_name = name.into();
        rom
    }

    #[test]
    fn headerless_files_match_headerless_dat_entries() {
        let dat = parse_dat_str(DAT).expect("dat");
        // Patch the DAT hashes to the real payload hashes.
        let mut dat = dat;
        let payload = b"payload";
        let hashes = hash_bytes(payload);
        dat.games[1].roms[0].sha1 = Some(hashes.sha1);
        dat.games[1].roms[0].crc = Some(hashes.crc);
        let catalog = Catalog::new(dat);
        let report = catalog.match_all(vec![scanned("Clone (Japan).nes", payload)]);
        let summary = report.summary();
        assert_eq!(summary.complete, 1);
        assert_eq!(summary.missing, 1); // parent has no source
        assert_eq!(summary.unknown, 0);
        assert!(report.rom_match[1].expect("clone match").headerless);
    }
}
