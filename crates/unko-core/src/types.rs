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
}

/// Preferred video output style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VideoFilter {
    /// One output pixel per emulated pixel; best for shaders and screenshots.
    #[default]
    Raw,
    /// Core-side composite/NTSC simulation, when the core offers one.
    Composite,
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
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            region: Region::Auto,
            sample_rate: 48_000.0,
            video_filter: VideoFilter::Raw,
        }
    }
}
