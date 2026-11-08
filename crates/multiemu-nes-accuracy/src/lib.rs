//! Clean-room NES core targeting reference parity.
//!
//! This crate is intentionally independent of `multiemu-nes` (the TetaNES
//! adapter) and of every reference emulator. It is built from public hardware
//! documentation; see `PROVENANCE.md` and `docs/accuracy.md`.
//!
//! Current state: A3 complete (CPU, PPU, APU) and A4 started (iNES cartridge
//! loading with NROM and MMC1 mappers).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod accuracycoin;
pub mod apu;
pub mod bus;
pub mod cart;
pub mod clock;
pub mod cpu;
pub mod nsf;
pub mod ppu;
pub mod system;

pub use accuracycoin::{
    run_accuracycoin, run_accuracycoin_isolated, AccuracyCoinReport, CatalogEntry,
};
pub use apu::Apu;
pub use bus::{CpuBus, RamBus};
pub use cart::{CartError, Cartridge, Header, Mapper, Mirroring};
pub use clock::{Region, Timing};
pub use cpu::{Cpu, Registers, Status};
pub use nsf::{Nsf, NsfError, NsfHeader};
pub use ppu::Ppu;
pub use system::System;
