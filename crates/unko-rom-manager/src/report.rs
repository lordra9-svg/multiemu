//! Fixdat, CSV and text reports.

use std::fmt::Write as _;
use std::path::Path;

use crate::catalog::{MatchReport, MatchStatus, RomRef};
use crate::one_g1r::parse_tags;
use crate::Error;

/// Writes a Logiqx fixdat containing every missing ROM of incomplete games.
pub fn write_fixdat(report: &MatchReport, path: &Path) -> Result<(), Error> {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\"?>\n<datafile>\n  <header>\n");
    let _ = writeln!(
        xml,
        "    <name>{}</name>",
        escape(&format!("Unko fixdat for {}", report.dat.name))
    );
    let _ = writeln!(
        xml,
        "    <description>{}</description>",
        escape(&format!("Missing ROMs for {}", report.dat.description))
    );
    let _ = writeln!(
        xml,
        "    <version>{}</version>",
        escape(&report.dat.version)
    );
    xml.push_str("  </header>\n");
    for (game_index, game) in report.dat.games.iter().enumerate() {
        let status = report.games[game_index].status;
        if status == MatchStatus::Complete {
            continue;
        }
        xml.push_str("  <game");
        let _ = write!(xml, " name=\"{}\"", escape(&game.name));
        if let Some(id) = &game.id {
            let _ = write!(xml, " id=\"{}\"", escape(id));
        }
        if let Some(parent) = &game.clone_of {
            let _ = write!(xml, " cloneofid=\"{}\"", escape(parent));
        }
        xml.push_str(">\n");
        let _ = writeln!(
            xml,
            "    <description>{}</description>",
            escape(game.description.as_deref().unwrap_or(&game.name))
        );
        for (rom_index, rom) in game.roms.iter().enumerate() {
            let flat = RomRef {
                game: game_index,
                rom: rom_index,
            };
            let matched = report
                .roms
                .iter()
                .position(|reference| *reference == flat)
                .is_some_and(|flat| report.rom_match[flat].is_some());
            if matched {
                continue;
            }
            xml.push_str("    <rom");
            let _ = write!(xml, " name=\"{}\" size=\"{}\"", escape(&rom.name), rom.size);
            if let Some(crc) = rom.crc {
                let _ = write!(xml, " crc=\"{crc:08x}\"");
            }
            if let Some(md5) = rom.md5 {
                let _ = write!(xml, " md5=\"{}\"", crate::hash::hex(&md5));
            }
            if let Some(sha1) = rom.sha1 {
                let _ = write!(xml, " sha1=\"{}\"", crate::hash::hex(&sha1));
            }
            if let Some(status) = &rom.status {
                let _ = write!(xml, " status=\"{}\"", escape(status));
            }
            xml.push_str("/>\n");
        }
        xml.push_str("  </game>\n");
    }
    xml.push_str("</datafile>\n");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, xml).map_err(|err| Error::Io(format!("{}: {err}", path.display())))
}

/// Writes a per-game CSV report.
pub fn write_csv(report: &MatchReport, path: &Path) -> Result<(), Error> {
    let mut csv = String::from("game,status,matched,total,region,language,source\n");
    for (index, game) in report.dat.games.iter().enumerate() {
        let status = report.games[index];
        let tags = parse_tags(&game.name);
        let source = game
            .roms
            .iter()
            .enumerate()
            .find_map(|(rom, _)| {
                report
                    .flat_index(index, rom)
                    .and_then(|flat| report.source(flat))
                    .map(|rom| rom.origin.display())
            })
            .unwrap_or_default();
        let _ = writeln!(
            csv,
            "{},{},{},{},{},{},{}",
            csv_escape(&game.name),
            status.status.label(),
            status.matched,
            status.total,
            csv_escape(&tags.regions.join(";")),
            csv_escape(&tags.languages.join(";")),
            csv_escape(&source)
        );
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, csv).map_err(|err| Error::Io(format!("{}: {err}", path.display())))
}

/// One-line human summary.
#[must_use]
pub fn summary_line(report: &MatchReport) -> String {
    let summary = report.summary();
    format!(
        "{} games: {} complete, {} partial, {} missing; {}/{} ROMs matched; {} unknown, {} duplicate files",
        summary.games,
        summary.complete,
        summary.partial,
        summary.missing,
        summary.matched_roms,
        summary.roms,
        summary.unknown,
        summary.duplicates
    )
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Catalog, MatchStatus};
    use crate::dat::parse_dat_str;

    const DAT: &str = r#"<datafile><header><name>T</name><version>1</version></header>
      <game name="Game &amp; Watch (USA)" id="1"><rom name="a.nes" size="1" crc="00000001"/></game>
      <game name="Missing (USA)" id="2"><rom name="b.nes" size="2" crc="00000002"/></game>
    </datafile>"#;

    #[test]
    fn fixdat_contains_only_missing_games_and_is_escaped() {
        let dat = parse_dat_str(DAT).expect("dat");
        let report = Catalog::new(dat).match_all(Vec::new());
        let path = std::env::temp_dir().join(format!("unko-fixdat-{}.xml", std::process::id()));
        write_fixdat(&report, &path).expect("write");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("<game name=\"Game &amp; Watch (USA)\""));
        assert!(text.contains("<game name=\"Missing (USA)\""));
        assert_eq!(report.games[0].status, MatchStatus::Missing);
        assert!(summary_line(&report).contains("0/2 ROMs"));
        let _ = std::fs::remove_file(path);
    }
}
