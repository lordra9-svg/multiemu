//! System-agnostic emulator core interface.
//!
//! `multiemu-core` deliberately contains no emulation code and no console
//! knowledge beyond a [`SystemId`] tag. Frontends and cores meet here.
//!
//! ```no_run
//! use multiemu_core::{Core, LoadOptions, Registry};
//! # fn run(registry: &Registry, rom: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! let mut core = registry
//!     .create_default(multiemu_core::SystemId::Nes)
//!     .ok_or("no NES core")?;
//! core.load(rom, &LoadOptions::default())?;
//! core.run_frame()?;
//! let frame = core.video();
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod core;
pub mod input;
pub mod registry;
pub mod types;

pub use core::{Core, CoreError};
pub use input::{Button, InputState};
pub use registry::{CoreRegistration, Factory, Registry};
pub use types::{
    Accuracy, AudioBuffer, CoreInfo, Features, FrameBuffer, FrameStats, LoadOptions, PixelFormat,
    Region, SystemId, VideoFilter,
};
