//! Safe copy/rebuild of matched ROMs into an output folder.

use std::fs;
use std::path::{Path, PathBuf};

use crate::catalog::{MatchReport, MatchStatus};
use crate::hash::hash_bytes;

/// Rebuild behaviour.
#[derive(Debug, Clone)]
pub struct RebuildOptions {
    /// Destination folder.
    pub output: PathBuf,
    /// Put each game's ROMs in a folder named after the game.
    pub per_game: bool,
    /// Replace files that already exist.
    pub overwrite: bool,
    /// Skip games that are not fully matched.
    pub only_complete: bool,
    /// Verify each written file against the DAT entry.
    pub verify: bool,
}

impl Default for RebuildOptions {
    fn default() -> Self {
        Self {
            output: PathBuf::from("rebuilt"),
            per_game: true,
            overwrite: false,
            only_complete: true,
            verify: true,
        }
    }
}

/// Result of a rebuild.
#[derive(Debug, Clone, Default)]
pub struct RebuildReport {
    /// Files written.
    pub written: usize,
    /// Files skipped because they already existed or were not matched.
    pub skipped: usize,
    /// Per-file errors.
    pub errors: Vec<String>,
}

/// Copies the matched ROMs of `selected` games into `options.output`.
#[must_use]
pub fn rebuild(
    report: &MatchReport,
    selected: &[usize],
    options: &RebuildOptions,
) -> RebuildReport {
    let mut result = RebuildReport::default();
    if let Err(err) = fs::create_dir_all(&options.output) {
        result
            .errors
            .push(format!("create {}: {err}", options.output.display()));
        return result;
    }
    for &game_index in selected {
        let Some(game) = report.dat.games.get(game_index) else {
            continue;
        };
        let status = report.games[game_index].status;
        if options.only_complete && status != MatchStatus::Complete {
            result.skipped += game.roms.len();
            continue;
        }
        let directory = if options.per_game {
            options.output.join(sanitize(&game.name))
        } else {
            options.output.clone()
        };
        if let Err(err) = fs::create_dir_all(&directory) {
            result
                .errors
                .push(format!("create {}: {err}", directory.display()));
            continue;
        }
        for (rom_index, rom) in game.roms.iter().enumerate() {
            let Some(flat) = report.flat_index(game_index, rom_index) else {
                result.skipped += 1;
                continue;
            };
            let Some(matched) = report.rom_match.get(flat).and_then(|entry| *entry) else {
                result.skipped += 1;
                continue;
            };
            let Some(scanned) = report.scanned.get(matched.scanned) else {
                result.skipped += 1;
                continue;
            };
            let bytes = match scanned.load_for(matched.headerless) {
                Ok(bytes) => bytes,
                Err(err) => {
                    result.errors.push(err.to_string());
                    continue;
                }
            };
            let destination = directory.join(sanitize(&rom.name));
            if destination.exists() && !options.overwrite {
                result.skipped += 1;
                continue;
            }
            let mut mismatched = None;
            if options.verify {
                let hashes = hash_bytes(&bytes);
                if hashes.size != rom.size {
                    mismatched = Some(format!(
                        "{}: size {} does not match DAT size {}",
                        scanned.origin.display(),
                        hashes.size,
                        rom.size
                    ));
                } else if let Some(expected) = rom.sha1 {
                    if hashes.sha1 != expected {
                        mismatched = Some(format!(
                            "{}: SHA-1 does not match the DAT",
                            scanned.origin.display()
                        ));
                    }
                } else if let Some(expected) = rom.crc {
                    if hashes.crc != expected {
                        mismatched = Some(format!(
                            "{}: CRC-32 does not match the DAT",
                            scanned.origin.display()
                        ));
                    }
                } else if let Some(expected) = rom.md5 {
                    if hashes.md5 != expected {
                        mismatched = Some(format!(
                            "{}: MD5 does not match the DAT",
                            scanned.origin.display()
                        ));
                    }
                }
            }
            if let Some(error) = mismatched {
                result.errors.push(error);
                continue;
            }
            match fs::write(&destination, bytes) {
                Ok(()) => result.written += 1,
                Err(err) => result
                    .errors
                    .push(format!("write {}: {err}", destination.display())),
            }
        }
    }
    result
}

fn sanitize(name: &str) -> String {
    let file_name = Path::new(name)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let mut out: String = file_name
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                ch
            }
        })
        .collect();
    while out.ends_with([' ', '.']) {
        out.pop();
    }
    if out.is_empty() {
        "rom".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::dat::parse_dat_str;
    use crate::hash::hash_bytes;
    use crate::scan::{Origin, ScannedRom};

    #[test]
    fn copies_and_verifies_matched_files() {
        let payload = b"rom bytes";
        let hashes = hash_bytes(payload);
        let dat = parse_dat_str(&format!(
            r#"<datafile><header><name>T</name></header>
              <game name="Game (USA)" id="1"><rom name="Game (USA).bin" size="{}" crc="{:08x}" sha1="{}"/></game>
            </datafile>"#,
            hashes.size,
            hashes.crc,
            hashes.sha1_hex()
        ))
        .expect("dat");
        let output = std::env::temp_dir().join(format!("unko-rebuild-{}", std::process::id()));
        let _ = fs::remove_dir_all(&output);
        fs::create_dir_all(&output).expect("dir");
        let source = output.join("source.bin");
        fs::write(&source, payload).expect("source");
        let report = Catalog::new(dat).match_all(vec![ScannedRom {
            origin: Origin::File(source),
            file_name: "source.bin".into(),
            hashes,
            headerless: None,
            modified: None,
        }]);
        let result = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: true,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(result.written, 1);
        assert!(result.errors.is_empty());
        assert_eq!(
            fs::read(output.join("Game (USA)").join("Game (USA).bin")).expect("read"),
            payload
        );
        let _ = fs::remove_dir_all(&output);
    }
}
