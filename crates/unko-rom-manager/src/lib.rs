//! Standalone ROM manager core: DAT auditing, 1G1R and safe rebuilding.
//!
//! This crate is independent of `unko-nes`, `unko-core`, the Unko app and the
//! CLI. It can be used on its own from any Rust project.
//!
//! ```no_run
//! use unko_rom_manager::{audit, OneGameOneRom, ScanOptions};
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut progress = |_: &unko_rom_manager::ScanProgress| {};
//! let report = audit(
//!     std::path::Path::new("dat/No-Intro.dat"),
//!     std::path::Path::new("roms"),
//!     &ScanOptions::default(),
//!     &mut progress,
//! )?;
//! let selected = OneGameOneRom::default().select(&report.dat);
//! println!("{}", unko_rom_manager::summary_line(&report));
//! println!("1G1R selection: {} games", selected.len());
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod catalog;
pub mod cli;
pub mod dat;
pub mod hash;
pub mod one_g1r;
pub mod rebuild;
pub mod report;
pub mod scan;

use std::fmt;
use std::path::Path;

pub use catalog::{Catalog, GameMatch, Match, MatchReport, MatchStatus, RomRef, Summary};
pub use dat::{parse_dat_path, parse_dat_str, Dat, DatRom, Game};
pub use hash::{hash_bytes, Hashes};
pub use one_g1r::{parse_tags, OneGameOneRom, Tags};
pub use rebuild::{rebuild, RebuildOptions, RebuildReport};
pub use report::{summary_line, write_csv, write_fixdat};
pub use scan::{scan_path, Origin, ScanOptions, ScanProgress, ScannedRom};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Stable feature-core id.
pub const CORE_ID: &str = "unko_rom_manager";

/// Errors raised by the ROM manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Filesystem failure.
    Io(String),
    /// DAT parse or validation failure.
    Dat(String),
    /// ZIP archive failure.
    Zip(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "I/O error: {message}"),
            Self::Dat(message) => write!(formatter, "DAT error: {message}"),
            Self::Zip(message) => write!(formatter, "ZIP error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Loads a DAT, scans a source tree and matches the two.
pub fn audit(
    dat_path: &Path,
    source: &Path,
    options: &ScanOptions,
    progress: &mut dyn FnMut(&ScanProgress),
) -> Result<MatchReport> {
    let dat = parse_dat_path(dat_path)?;
    let (scanned, _) = scan_path(source, options, progress)?;
    Ok(Catalog::new(dat).match_all(scanned))
}
