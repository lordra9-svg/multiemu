//! Logiqx-style XML DAT parsing.

use std::collections::HashMap;
use std::path::Path;

use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::hash::{parse_byte_string, parse_crc, parse_digest};
use crate::{Error, Result};

/// A parsed DAT file.
#[derive(Debug, Clone, Default)]
pub struct Dat {
    /// DAT name from `<header><name>`.
    pub name: String,
    /// DAT description from `<header><description>`.
    pub description: String,
    /// DAT version/date from `<header><version>`.
    pub version: String,
    /// Games in file order.
    pub games: Vec<Game>,
}

impl Dat {
    /// Number of ROM entries across every game.
    #[must_use]
    pub fn rom_count(&self) -> usize {
        self.games.iter().map(|game| game.roms.len()).sum()
    }

    /// Index of every game id, used to resolve `cloneofid`.
    #[must_use]
    pub fn id_index(&self) -> HashMap<&str, usize> {
        self.games
            .iter()
            .enumerate()
            .filter_map(|(index, game)| game.id.as_deref().map(|id| (id, index)))
            .collect()
    }
}

/// One game/machine in a DAT.
#[derive(Debug, Clone, Default)]
pub struct Game {
    /// Full game name.
    pub name: String,
    /// Description (usually equal to the name).
    pub description: Option<String>,
    /// DAT id, used by clone relationships.
    pub id: Option<String>,
    /// Parent game id for a clone.
    pub clone_of: Option<String>,
    /// DAT category.
    pub category: Option<String>,
    /// ROMs in this game.
    pub roms: Vec<DatRom>,
}

impl Game {
    /// Parent key for 1G1R: the parent id or the game's own id.
    #[must_use]
    pub fn parent_key(&self) -> &str {
        self.clone_of
            .as_deref()
            .or(self.id.as_deref())
            .unwrap_or(self.name.as_str())
    }

    /// Whether this is a clone.
    #[must_use]
    pub fn is_clone(&self) -> bool {
        self.clone_of.is_some()
    }

    /// Whether every ROM is marked verified by the DAT.
    #[must_use]
    pub fn verified(&self) -> bool {
        !self.roms.is_empty()
            && self
                .roms
                .iter()
                .all(|rom| rom.status.as_deref() == Some("verified"))
    }
}

/// One ROM entry in a game.
#[derive(Debug, Clone, Default)]
pub struct DatRom {
    /// Expected file name.
    pub name: String,
    /// Expected byte length.
    pub size: u64,
    /// CRC-32.
    pub crc: Option<u32>,
    /// MD5.
    pub md5: Option<[u8; 16]>,
    /// SHA-1.
    pub sha1: Option<[u8; 20]>,
    /// `status="verified"` and similar.
    pub status: Option<String>,
    /// Optional iNES header bytes declared by the DAT.
    pub header: Option<Vec<u8>>,
}

/// Parses a DAT from XML text.
pub fn parse_dat_str(text: &str) -> Result<Dat> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut dat = Dat::default();
    let mut in_header = false;
    let mut game: Option<Game> = None;
    let mut capture: Option<Capture> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match local_name(event.name().as_ref()) {
                b"header" => in_header = true,
                b"game" | b"machine" => {
                    game = Some(game_from(&event)?);
                    in_header = false;
                }
                b"rom" => {
                    if let Some(game) = game.as_mut() {
                        if let Some(rom) = rom_from(&event)? {
                            game.roms.push(rom);
                        }
                    }
                }
                b"name" if in_header => capture = Some(Capture::HeaderName),
                b"description" if in_header => capture = Some(Capture::HeaderDescription),
                b"version" if in_header => capture = Some(Capture::HeaderVersion),
                b"description" if game.is_some() => capture = Some(Capture::GameDescription),
                b"category" if game.is_some() => capture = Some(Capture::GameCategory),
                _ => {}
            },
            Ok(Event::Empty(event)) => {
                if local_name(event.name().as_ref()) == b"rom" {
                    if let Some(game) = game.as_mut() {
                        if let Some(rom) = rom_from(&event)? {
                            game.roms.push(rom);
                        }
                    }
                }
            }
            Ok(Event::Text(text)) => {
                if let Some(capture) = capture {
                    let raw = text
                        .decode()
                        .map_err(|err| Error::Dat(format!("text decode: {err}")))?;
                    let value = unescape(&raw)
                        .map_err(|err| Error::Dat(format!("text escape: {err}")))?
                        .into_owned();
                    let value = value.trim().to_string();
                    if value.is_empty() {
                        continue;
                    }
                    match capture {
                        Capture::HeaderName => dat.name = value,
                        Capture::HeaderDescription => dat.description = value,
                        Capture::HeaderVersion => dat.version = value,
                        Capture::GameDescription => {
                            if let Some(game) = game.as_mut() {
                                game.description = Some(value);
                            }
                        }
                        Capture::GameCategory => {
                            if let Some(game) = game.as_mut() {
                                game.category = Some(value);
                            }
                        }
                    }
                }
            }
            Ok(Event::CData(text)) => {
                if let Some(capture) = capture {
                    let value = String::from_utf8_lossy(&text).trim().to_string();
                    match capture {
                        Capture::HeaderName => dat.name = value,
                        Capture::HeaderDescription => dat.description = value,
                        Capture::HeaderVersion => dat.version = value,
                        Capture::GameDescription => {
                            if let Some(game) = game.as_mut() {
                                game.description = Some(value);
                            }
                        }
                        Capture::GameCategory => {
                            if let Some(game) = game.as_mut() {
                                game.category = Some(value);
                            }
                        }
                    }
                }
            }
            Ok(Event::End(event)) => match local_name(event.name().as_ref()) {
                b"header" => in_header = false,
                b"game" | b"machine" => {
                    if let Some(game) = game.take() {
                        dat.games.push(game);
                    }
                }
                b"name" | b"description" | b"version" | b"category" => capture = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(err) => {
                return Err(Error::Dat(format!(
                    "XML at {}: {err}",
                    reader.buffer_position()
                )))
            }
        }
    }

    if game.is_some() {
        return Err(Error::Dat("unterminated <game>".into()));
    }
    if dat.name.is_empty() {
        dat.name = "Unnamed DAT".into();
    }
    if dat.description.is_empty() {
        dat.description = dat.name.clone();
    }
    Ok(dat)
}

/// Parses a DAT from a file.
pub fn parse_dat_path(path: &Path) -> Result<Dat> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| Error::Io(format!("{}: {err}", path.display())))?;
    parse_dat_str(&text)
}

#[derive(Clone, Copy)]
enum Capture {
    HeaderName,
    HeaderDescription,
    HeaderVersion,
    GameDescription,
    GameCategory,
}

fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|byte| *byte == b':') {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

fn attr(event: &BytesStart, key: &str) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|attribute| local_name(attribute.key.as_ref()) == key.as_bytes())
        .and_then(|attribute| attribute.normalized_value(XmlVersion::Explicit1_0).ok())
        .map(|value| value.into_owned())
}

fn game_from(event: &BytesStart) -> Result<Game> {
    let name = attr(event, "name").unwrap_or_default();
    if name.is_empty() {
        return Err(Error::Dat("game without a name".into()));
    }
    Ok(Game {
        name,
        description: None,
        id: attr(event, "id"),
        clone_of: attr(event, "cloneofid").or_else(|| attr(event, "cloneof")),
        category: None,
        roms: Vec::new(),
    })
}

fn rom_from(event: &BytesStart) -> Result<Option<DatRom>> {
    let name = attr(event, "name").unwrap_or_default();
    let Some(size) = attr(event, "size").and_then(|value| value.parse::<u64>().ok()) else {
        return Ok(None);
    };
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(DatRom {
        name,
        size,
        crc: attr(event, "crc").and_then(|value| parse_crc(&value)),
        md5: attr(event, "md5").and_then(|value| parse_digest::<16>(&value)),
        sha1: attr(event, "sha1").and_then(|value| parse_digest::<20>(&value)),
        status: attr(event, "status"),
        header: attr(event, "header").and_then(|value| parse_byte_string(&value)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<datafile>
  <header>
    <name>Sample DAT</name>
    <description>Sample DAT description</description>
    <version>1.0</version>
  </header>
  <game name="Parent (USA)" id="1">
    <description>Parent (USA)</description>
    <category>Games</category>
    <rom name="Parent (USA).nes" size="40976" crc="cbd55de3" md5="d0e2b142a11e6f66d96932b7a6ba43ce" sha1="629d5de4897216411e4b5d6477da5d780aef3b2a" status="verified"/>
  </game>
  <game name="Clone (Japan)" id="2" cloneofid="1">
    <description>Clone (Japan)</description>
    <rom name="Clone (Japan).nes" size="40976" crc="a15bf934" md5="2b13f366f8ccd85d060af13ae29efb05" sha1="3b24541f616b78fabdaa0563d5b40fc45e2b270a"/>
  </game>
</datafile>"#;

    #[test]
    fn parses_header_games_and_roms() {
        let dat = parse_dat_str(SAMPLE).expect("parse");
        assert_eq!(dat.name, "Sample DAT");
        assert_eq!(dat.version, "1.0");
        assert_eq!(dat.games.len(), 2);
        assert_eq!(dat.rom_count(), 2);
        assert_eq!(dat.games[0].category.as_deref(), Some("Games"));
        assert!(dat.games[0].verified());
        assert_eq!(dat.games[1].clone_of.as_deref(), Some("1"));
        assert_eq!(dat.games[1].parent_key(), "1");
        assert_eq!(dat.games[0].roms[0].crc, Some(0xcbd5_5de3));
    }
}
