//! Per-game cheat list (Game Genie and raw RAM).
//!
//! Stored beside the other per-game files as `<stem>.cheats`; states, SRAM and
//! recordings are untouched. The core receives only enabled, decoded entries.

use std::fs;
use std::path::Path;

use nes_accuracy_core::GameGenie;
use unko_core::Cheat;

/// One editable cheat row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheatEntry {
    /// Decoded cheat passed to the core when enabled.
    pub cheat: Cheat,
    /// Whether the cheat is active.
    pub enabled: bool,
}

impl CheatEntry {
    /// Human-readable row label.
    #[must_use]
    pub fn label(&self) -> String {
        match &self.cheat {
            Cheat::GameGenie {
                code,
                address,
                value,
                compare,
            } => match compare {
                Some(compare) => {
                    format!("Genie {code}  ${address:04X} = ${value:02X} (if ${compare:02X})")
                }
                None => format!("Genie {code}  ${address:04X} = ${value:02X}"),
            },
            Cheat::Ram { address, value } => {
                format!("RAM ${address:04X} = ${value:02X}")
            }
        }
    }
}

/// Loads a cheat file, skipping malformed lines.
#[must_use]
pub fn load(path: &Path) -> Vec<CheatEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("G") => {
                let Some(code) = parts.next() else { continue };
                let enabled = parts.next().is_none_or(|value| value != "0");
                let Ok(decoded) = GameGenie::parse(code) else {
                    continue;
                };
                entries.push(CheatEntry {
                    cheat: Cheat::GameGenie {
                        code: decoded.code().to_string(),
                        address: decoded.address(),
                        value: decoded.value(),
                        compare: decoded.compare(),
                    },
                    enabled,
                });
            }
            Some("R") => {
                let (Some(address), Some(value), enabled) = (
                    parts.next(),
                    parts.next(),
                    parts.next().is_none_or(|value| value != "0"),
                ) else {
                    continue;
                };
                let Ok(address) = u16::from_str_radix(address.trim_start_matches("0x"), 16) else {
                    continue;
                };
                let Ok(value) = u8::from_str_radix(value.trim_start_matches("0x"), 16) else {
                    continue;
                };
                entries.push(CheatEntry {
                    cheat: Cheat::Ram { address, value },
                    enabled,
                });
            }
            _ => {}
        }
        if entries.len() == 32 {
            break;
        }
    }
    entries
}

/// Writes the cheat list; returns an I/O error only for filesystem failures.
pub fn save(path: &Path, entries: &[CheatEntry]) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        if !directory.as_os_str().is_empty() {
            let _ = fs::create_dir_all(directory);
        }
    }
    let mut text = String::from("# Unko cheats\n");
    for entry in entries.iter().take(32) {
        match &entry.cheat {
            Cheat::GameGenie {
                code,
                address: _,
                value: _,
                compare: _,
            } => {
                text.push_str(&format!("G {code} {}\n", u8::from(entry.enabled)));
            }
            Cheat::Ram { address, value } => {
                text.push_str(&format!(
                    "R {address:04X} {value:02X} {}\n",
                    u8::from(entry.enabled)
                ));
            }
        }
    }
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_genie_and_ram_entries() {
        let path = std::env::temp_dir().join("unko-test.cheats");
        let entries = vec![
            CheatEntry {
                cheat: Cheat::GameGenie {
                    code: "SXIOPO".into(),
                    address: 0x91D9,
                    value: 0xAD,
                    compare: None,
                },
                enabled: true,
            },
            CheatEntry {
                cheat: Cheat::Ram {
                    address: 0x0010,
                    value: 0x99,
                },
                enabled: false,
            },
        ];
        save(&path, &entries).expect("save");
        let loaded = load(&path);
        assert_eq!(loaded, entries);
        assert!(loaded[0].label().contains("SXIOPO"));
        let _ = fs::remove_file(path);
    }
}
