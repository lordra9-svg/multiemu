//! Library facade over `unko-rom`.
//!
//! Scanning and archive handling live in the shared crate so the CLI and GUI
//! see exactly the same ROMs, container rules and save ids.

pub use unko_rom::{probe_path, scan, RomEntry as GameEntry};
