//! Library facade over `multiemu-rom`.
//!
//! Scanning and archive handling live in the shared crate so the CLI and GUI
//! see exactly the same ROMs, container rules and save ids.

pub use multiemu_rom::{probe_path, scan, RomEntry as GameEntry};
