//! Host-facing configuration types used by the core.
//!
//! These are defined here rather than imported from a frontend crate so
//! `unko-nes` can be used on its own. The `unko-nes-host` adapter converts
//! between these types and the generic `unko-core` ones.

/// Resampler quality trade-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioQuality {
    /// Softer, lower cutoff (cheapest kernel).
    Fast,
    /// The default: cutoff 0.46 with a Blackman window.
    #[default]
    Standard,
    /// Sharper cutoff 0.49 with a Blackman-Harris window.
    High,
}

/// Analog output filter model applied after resampling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AudioFilterModel {
    /// The front-loader network: 90 Hz + 440 Hz high-pass, 14 kHz low-pass.
    #[default]
    NesRf,
    /// No filtering (flat output).
    Linear,
    /// Only the 14 kHz low-pass.
    Crisp,
    /// The NES high-passes with an 8 kHz low-pass.
    Warm,
    /// One high-pass and one low-pass at the supplied cutoffs; zero (or a
    /// cutoff at/above Nyquist) disables that stage.
    Custom {
        /// High-pass cutoff in Hz; `0` disables it.
        high_pass_hz: f32,
        /// Low-pass cutoff in Hz; `0` or a value at/above Nyquist disables it.
        low_pass_hz: f32,
    },
}

/// Which audio sources reach the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioSource {
    /// The 2A03 and cartridge expansion audio.
    #[default]
    Both,
    /// Only the 2A03 APU.
    Internal,
    /// Only cartridge expansion audio.
    Expansion,
    /// Silence (the machine still runs).
    None,
}

/// Work-RAM contents after a reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RamClear {
    /// Keep the current contents (a soft reset does not clear work RAM).
    Keep,
    /// Fill with `$00`.
    #[default]
    Zero,
    /// Fill with `$FF`.
    Fill,
    /// Fill with a deterministic pseudo-random pattern.
    Random,
}

/// PPU register behavior after a soft reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PpuResetBehavior {
    /// Famicom-style: a soft reset leaves the PPU registers alone.
    #[default]
    Famicom,
    /// NES-style: clear the `$2000`/`$2001` derived registers.
    Nes,
}

/// Debug address space.
#[cfg(feature = "debug")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugSpace {
    /// CPU bus address space (`$0000-$FFFF`).
    Cpu,
    /// PPU address space (`$0000-$3FFF`: CHR, nametables, palette).
    Ppu,
    /// Object attribute memory (256 bytes).
    Oam,
}

/// A cheat applied to the machine.
#[cfg(feature = "cheats")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cheat {
    /// A Game Genie substitution on a PRG address.
    GameGenie {
        /// Original user-entered code, kept for editing/display.
        code: String,
        /// CPU address to intercept.
        address: u16,
        /// Value to return instead of the ROM byte.
        value: u8,
        /// Optional compare byte (8-character codes only).
        compare: Option<u8>,
    },
    /// A raw RAM write applied before each frame.
    Ram {
        /// CPU address (`$0000-$07FF` for work RAM).
        address: u16,
        /// Value to write.
        value: u8,
    },
}
