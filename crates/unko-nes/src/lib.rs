//! Standalone, reference-parity NES core.
//!
//! This crate has no dependency on the Unko frontend or on
//! [`unko_core`](https://docs.rs/unko-core); it can be embedded directly in
//! other Rust projects. The `unko-nes-host` crate provides the optional
//! `unko_core::Core` adapter used by the Unko app and CLI.
//!
//! The emulator is built from public hardware documentation; see
//! `PROVENANCE.md` and `docs/accuracy.md` in the repository.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod apu;
pub mod bus;
pub mod cart;
#[cfg(feature = "cheats")]
pub mod cheats;
pub mod clock;
pub mod cpu;
pub mod nsf;
pub(crate) mod palette;
pub mod ppu;
pub mod resampler;
pub mod system;
pub mod types;

pub use apu::Apu;
pub use bus::{CpuBus, RamBus};
pub use cart::{CartError, Cartridge, Header, Mapper, Mirroring};
#[cfg(feature = "cheats")]
pub use cheats::{GameGenie, GenieError};
pub use clock::{Region, Timing};
pub use cpu::{Cpu, Registers, Status};
pub use nsf::{Nsf, NsfError, NsfHeader};
pub use ppu::{PaletteVariant, Ppu};
pub use resampler::Resampler;
pub use system::{PortDevice, System};
#[cfg(feature = "cheats")]
pub use types::Cheat;
#[cfg(feature = "debug")]
pub use types::DebugSpace;
pub use types::{AudioFilterModel, AudioQuality, AudioSource, PpuResetBehavior, RamClear};
