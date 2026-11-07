//! The host-facing emulator core contract.

use std::fmt;

use crate::input::InputState;
use crate::types::{AudioBuffer, CoreInfo, FrameBuffer, FrameStats, LoadOptions, VideoFilter};

/// Errors a core may return to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// The ROM format is valid but the core cannot run it.
    Unsupported(String),
    /// The ROM could not be parsed.
    InvalidRom(String),
    /// A save state is malformed or belongs to another ROM/core version.
    State(String),
    /// An I/O operation failed.
    Io(String),
    /// A core invariant was violated.
    Internal(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(msg) => write!(f, "unsupported: {msg}"),
            Self::InvalidRom(msg) => write!(f, "invalid ROM: {msg}"),
            Self::State(msg) => write!(f, "state error: {msg}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// One emulated console instance.
///
/// Implementors own all mutable emulation state. The host owns pacing, video
/// presentation, audio output, input binding and persistent files; the core
/// owns the machine.
///
/// The call sequence is:
///
/// 1. [`Core::load`]
/// 2. any number of [`Core::run_frame`] calls, each followed by reading
///    [`Core::video`] and [`Core::audio`]
/// 3. optional [`Core::save_state`] / [`Core::load_state`] and SRAM access
/// 4. drop
pub trait Core {
    /// Static description of this core.
    fn info(&self) -> CoreInfo;

    /// Loads a ROM and resets the machine.
    fn load(&mut self, rom: &[u8], options: &LoadOptions) -> Result<(), CoreError>;

    /// Hard-resets the machine, keeping the loaded ROM.
    fn reset(&mut self) -> Result<(), CoreError>;

    /// Runs until one display frame is complete.
    fn run_frame(&mut self) -> Result<FrameStats, CoreError>;

    /// The most recently completed frame.
    fn video(&self) -> FrameBuffer<'_>;

    /// Audio produced by the most recent [`Core::run_frame`].
    fn audio(&self) -> AudioBuffer<'_>;

    /// Replaces the state of one controller port.
    fn set_input(&mut self, port: usize, input: InputState);

    /// Changes the output sample rate. May take effect on the next frame.
    fn set_sample_rate(&mut self, sample_rate: f32);

    /// Changes the core-side video filter.
    fn set_video_filter(&mut self, filter: VideoFilter);

    /// Serializes the full machine state into `out`, replacing its contents.
    fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError>;

    /// Restores a state produced by [`Core::save_state`].
    fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError>;

    /// Battery-backed cartridge RAM, if the loaded cartridge has any.
    fn sram(&mut self) -> &[u8] {
        &[]
    }

    /// Replaces battery-backed cartridge RAM.
    fn load_sram(&mut self, _sram: &[u8]) -> Result<(), CoreError> {
        Ok(())
    }
}
