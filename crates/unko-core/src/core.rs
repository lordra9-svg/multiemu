//! The host-facing emulator core contract.

use std::fmt;

use crate::debug::{ApuState, CpuState, DebugSpace, Disassembly, PpuState};
use crate::input::InputState;
use crate::types::{
    AudioBuffer, AudioFilterModel, AudioQuality, CoreInfo, FrameBuffer, FrameStats, LoadOptions,
    Palette, PortDevice, VideoFilter,
};

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

    /// Attaches a device to a controller port (`Gamepad` by default).
    fn set_port_device(&mut self, _port: usize, _device: PortDevice) {}

    /// Aims the Zapper attached to `port` and reports the trigger state.
    fn set_zapper(&mut self, _port: usize, _x: u8, _y: u8, _trigger: bool) {}

    /// Sets a paddle's knob position (`0..=255`) and fire button.
    fn set_paddle(&mut self, _port: usize, _position: u8, _fire: bool) {}

    /// Sets the pressed Power Pad buttons (bit `i` = button `i + 1`).
    fn set_power_pad(&mut self, _port: usize, _buttons: u16) {}

    /// Replaces the Family BASIC keyboard matrix.
    ///
    /// One byte per matrix row: low nibble = column half 0, high nibble =
    /// column half 1, and a set bit means the key is held.
    fn set_family_keyboard(&mut self, _keys: [u8; 9]) {}

    /// Changes the core-side video filter.
    fn set_video_filter(&mut self, filter: VideoFilter);

    /// Changes the display palette.
    fn set_palette(&mut self, _palette: Palette) {}

    /// Supplies the 64-entry RGB table used by [`Palette::Custom`].
    fn set_palette_table(&mut self, _table: [[u8; 3]; 64]) {}

    /// Selects the audio resampler quality. May take effect on the next frame.
    fn set_audio_quality(&mut self, _quality: AudioQuality) {}

    /// Selects the analog output filter model. May take effect on the next
    /// frame.
    fn set_audio_filters(&mut self, _filters: AudioFilterModel) {}

    /// Sets the per-channel mixer gains in
    /// `[pulse1, pulse2, triangle, noise, dmc, expansion]` order.
    ///
    /// A gain of `1.0` is the unmodified hardware mix; the host uses this for
    /// the per-channel volume/mute controls. The values are presentation
    /// state, never part of a save state.
    fn set_channel_gains(&mut self, _gains: [f32; 6]) {}

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

    /// Whether this core implements the debugger surface.
    ///
    /// Frontends check this once instead of probing every accessor.
    fn debug_supported(&self) -> bool {
        false
    }

    /// Snapshot of the CPU registers, or `None` when unsupported.
    fn debug_cpu(&self) -> Option<CpuState> {
        None
    }

    /// Snapshot of the PPU timing and registers, or `None` when unsupported.
    fn debug_ppu(&self) -> Option<PpuState> {
        None
    }

    /// Snapshot of the APU channels, or `None` when unsupported.
    fn debug_apu(&self) -> Option<ApuState> {
        None
    }

    /// Side-effect-free read of one byte in `space`.
    fn debug_peek(&self, _space: DebugSpace, _address: u16) -> Option<u8> {
        None
    }

    /// Writes one byte in `space`; returns whether the write was accepted.
    ///
    /// ROM and register ranges should be rejected rather than silently
    /// ignored, so the frontend can report read-only addresses.
    fn debug_poke(&mut self, _space: DebugSpace, _address: u16, _value: u8) -> bool {
        false
    }

    /// Executes exactly one CPU instruction, advancing the rest of the
    /// machine with it. Returns the cycles consumed.
    fn debug_step(&mut self) -> Result<u32, CoreError> {
        Err(CoreError::Unsupported("debug single-step".into()))
    }

    /// Decodes the instruction at a CPU address without executing it.
    fn debug_disassemble(&self, _address: u16) -> Option<Disassembly> {
        None
    }

    /// The 32 palette entries as RGB, after emphasis, for the debugger.
    fn debug_palette(&self) -> Option<[[u8; 3]; 32]> {
        None
    }
}
