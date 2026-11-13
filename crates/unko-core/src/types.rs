//! System-agnostic types shared by every emulator core and by the host.
//!
//! Nothing in this crate knows about the NES. A core describes what system it
//! emulates, what it can do, and how the host should consume its output.

/// A console family the shell knows how to present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemId {
    /// Nintendo Entertainment System / Famicom.
    Nes,
    /// Super Nintendo / Super Famicom.
    Snes,
    /// Game Boy / Game Boy Color.
    GameBoy,
    /// Game Boy Advance.
    GameBoyAdvance,
    /// Sega Mega Drive / Genesis.
    Genesis,
    /// Sega Master System / Game Gear.
    MasterSystem,
    /// NEC PC Engine / TurboGrafx-16.
    PcEngine,
    /// Bandai WonderSwan.
    WonderSwan,
    /// Sony PlayStation.
    PlayStation,
    /// Arcade boards.
    Arcade,
    /// Anything not listed yet.
    Other(&'static str),
}

impl SystemId {
    /// Stable identifier used in configuration and UI keys.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nes => "nes",
            Self::Snes => "snes",
            Self::GameBoy => "gb",
            Self::GameBoyAdvance => "gba",
            Self::Genesis => "genesis",
            Self::MasterSystem => "sms",
            Self::PcEngine => "pce",
            Self::WonderSwan => "ws",
            Self::PlayStation => "ps1",
            Self::Arcade => "arcade",
            Self::Other(name) => name,
        }
    }
}

/// How closely a core models the original hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accuracy {
    /// Dot/cycle-level scheduling with documented quirks modelled.
    CycleAccurate,
    /// Timing is close but some quirks are approximated.
    High,
    /// Runs the software; timing is not a goal.
    Compatible,
}

/// Optional capabilities a core may advertise.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Features(u32);

impl std::fmt::Debug for Features {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut list = f.debug_list();
        let flags = [
            (Self::SAVE_STATES, "save_states"),
            (Self::BATTERY, "battery"),
            (Self::FOUR_PLAYER, "four_player"),
            (Self::LIGHT_GUN, "light_gun"),
            (Self::EXPANSION_AUDIO, "expansion_audio"),
        ];
        for (flag, name) in flags {
            if self.contains(flag) {
                list.entry(&name);
            }
        }
        list.finish()
    }
}
impl Features {
    /// Save/load state support.
    pub const SAVE_STATES: Self = Self(1 << 0);
    /// Battery-backed cartridge RAM.
    pub const BATTERY: Self = Self(1 << 1);
    /// More than two controller ports.
    pub const FOUR_PLAYER: Self = Self(1 << 2);
    /// Light gun support.
    pub const LIGHT_GUN: Self = Self(1 << 3);
    /// Expansion audio (extra sound chips on the cartridge).
    pub const EXPANSION_AUDIO: Self = Self(1 << 4);

    /// No optional capabilities.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Union of two feature sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether `self` contains every feature in `other`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Static description of a core, used to build menus and choose a backend.
#[derive(Debug, Clone, Copy)]
pub struct CoreInfo {
    /// Stable identifier, unique per core implementation.
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// Core version.
    pub version: &'static str,
    /// Console family this core emulates.
    pub system: SystemId,
    /// Accuracy class.
    pub accuracy: Accuracy,
    /// Advertised optional features.
    pub features: Features,
}

/// Pixel layout of a [`FrameBuffer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 8 bits per channel, red-green-blue-alpha, tightly packed.
    Rgba8,
    /// 8 bits per channel, red-green-blue, tightly packed.
    Rgb8,
}

/// A borrowed view of one finished frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameBuffer<'a> {
    /// Pixel bytes.
    pub data: &'a [u8],
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Distance between the start of two rows, in bytes.
    pub stride: usize,
    /// Pixel layout.
    pub format: PixelFormat,
}

impl FrameBuffer<'_> {
    /// Whether the frame contains no pixels.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// A borrowed view of the audio produced by the last frame.
#[derive(Debug, Clone, Copy)]
pub struct AudioBuffer<'a> {
    /// Interleaved samples in `[-1.0, 1.0]`.
    pub samples: &'a [f32],
    /// Sample rate the samples were produced at.
    pub sample_rate: f32,
    /// 1 for mono, 2 for stereo.
    pub channels: u8,
}

impl AudioBuffer<'_> {
    /// Number of frames (sample sets) in the buffer.
    #[must_use]
    pub const fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }
}

/// What one call to [`crate::Core::run_frame`] produced.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    /// Console frames advanced (a core may run more than one for fast-forward).
    pub frames: u32,
    /// Audio sample frames produced.
    pub samples: usize,
}

/// Region hint for cores that support more than one timing standard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Region {
    /// Detect from the ROM / database.
    #[default]
    Auto,
    /// NTSC (60 Hz family).
    Ntsc,
    /// PAL (50 Hz family).
    Pal,
    /// Dendy (50 Hz PAL variant with NTSC timing ratios).
    Dendy,
}

/// Device attached to a controller port.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PortDevice {
    /// Standard controller.
    #[default]
    Gamepad,
    /// NES Zapper light gun.
    Zapper,
    /// Arkanoid "Vaus" paddle (8-bit position plus fire).
    Paddle,
    /// Power Pad / Family Trainer mat (12 membrane buttons).
    PowerPad,
    /// Family BASIC keyboard matrix (9 rows x 8 columns).
    FamilyKeyboard,
    /// Four Score adapter (four controllers across both ports).
    FourScore,
}

impl PortDevice {
    /// Stable lowercase name used by configuration files and the CLI.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gamepad => "gamepad",
            Self::Zapper => "zapper",
            Self::Paddle => "paddle",
            Self::PowerPad => "powerpad",
            Self::FamilyKeyboard => "keyboard",
            Self::FourScore => "fourscore",
        }
    }

    /// Parses a device name; unknown names fall back to a gamepad.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim() {
            "zapper" => Self::Zapper,
            "paddle" => Self::Paddle,
            "powerpad" => Self::PowerPad,
            "keyboard" => Self::FamilyKeyboard,
            "fourscore" => Self::FourScore,
            _ => Self::Gamepad,
        }
    }
}

/// Display palette applied to the core's RGB output.
///
/// The canonical 2C02 table is the reference default (and what differential
/// traces compare against). The other presets are display preferences derived
/// from it, not hardware captures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Palette {
    /// Canonical 2C02 RGB table used for oracle parity.
    #[default]
    Canonical,
    /// Warmer, slightly softer consumer-TV look.
    Consumer,
    /// Saturated, gamma-corrected RGB-monitor look.
    Rgb,
    /// Luminance-only output.
    Monochrome,
    /// User-supplied 64-entry table (`.pal` / `.palette` import).
    Custom,
}

impl Palette {
    /// Stable lowercase name used by settings and the CLI.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Consumer => "consumer",
            Self::Rgb => "rgb",
            Self::Monochrome => "mono",
            Self::Custom => "custom",
        }
    }

    /// Parses a palette name; unknown names fall back to the canonical table.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim() {
            "consumer" | "warm" => Self::Consumer,
            "rgb" => Self::Rgb,
            "mono" | "monochrome" | "grayscale" => Self::Monochrome,
            "custom" => Self::Custom,
            _ => Self::Canonical,
        }
    }
}

/// Preferred video output style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VideoFilter {
    /// One output pixel per emulated pixel; best for shaders and screenshots.
    #[default]
    Raw,
    /// Core-side composite/NTSC simulation, when the core offers one.
    Composite,
    /// Composite simulation plus stronger scanlines and a slight horizontal
    /// bloom, for a CRT-like presentation.
    Crt,
}

/// Host options applied when a ROM is loaded.
#[derive(Debug, Clone, Copy)]
pub struct LoadOptions {
    /// Timing region override.
    pub region: Region,
    /// Requested audio sample rate.
    pub sample_rate: f32,
    /// Requested video output style.
    pub video_filter: VideoFilter,
    /// Display palette.
    pub palette: Palette,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            region: Region::Auto,
            sample_rate: 48_000.0,
            video_filter: VideoFilter::Raw,
            palette: Palette::Canonical,
        }
    }
}

/// Resampler quality trade-off.
///
/// The core always band-limits its ~1.79 MHz mixer stream; the modes change
/// the FIR cutoff and window rather than selecting an aliasing path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioQuality {
    /// Softer, lower cutoff (more high-frequency roll-off, cheapest kernel).
    Fast,
    /// The default: cutoff 0.46 with a Blackman window.
    #[default]
    Standard,
    /// Sharper cutoff 0.49 with a Blackman-Harris window.
    High,
}

impl AudioQuality {
    /// Stable identifier used by the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Standard => "standard",
            Self::High => "high",
        }
    }

    /// Parses a persisted identifier, falling back to [`Self::Standard`].
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "fast" | "low" => Self::Fast,
            "high" => Self::High,
            _ => Self::Standard,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Standard => "Standard",
            Self::High => "High",
        }
    }
}

/// Analog output filter model applied after resampling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AudioFilterModel {
    /// The front-loader network: 90 Hz + 440 Hz high-pass, 14 kHz low-pass.
    #[default]
    NesRf,
    /// No filtering (flat output; useful for comparing captures).
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

impl AudioFilterModel {
    /// Stable identifier used by the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NesRf => "nes-rf",
            Self::Linear => "linear",
            Self::Crisp => "crisp",
            Self::Warm => "warm",
            Self::Custom { .. } => "custom",
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NesRf => "NES RF",
            Self::Linear => "Linear",
            Self::Crisp => "Crisp (14 kHz low-pass)",
            Self::Warm => "Warm (8 kHz low-pass)",
            Self::Custom { .. } => "Custom",
        }
    }
}
