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
    /// Famicom controller microphone (`$4016` D2).
    Microphone,
    /// Family BASIC data recorder (cassette loopback on `$4016`).
    Tape,
    /// Miracle Piano MIDI transport (`$4016`, controller port 1).
    MiraclePiano,
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
            Self::Microphone => "microphone",
            Self::Tape => "tape",
            Self::MiraclePiano => "piano",
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
            "microphone" | "mic" => Self::Microphone,
            "tape" | "recorder" => Self::Tape,
            "piano" | "miraclepiano" | "miracle-piano" => Self::MiraclePiano,
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
    /// Generated high-saturation preset.
    Vivid,
    /// Generated low-saturation, warm preset.
    Soft,
    /// Generated PAL-like hue/level preset.
    Pal,
    /// Generated neutral studio-monitor preset.
    Studio,
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
            Self::Vivid => "vivid",
            Self::Soft => "soft",
            Self::Pal => "pal",
            Self::Studio => "studio",
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
            "vivid" => Self::Vivid,
            "soft" => Self::Soft,
            "pal" => Self::Pal,
            "studio" => Self::Studio,
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

/// Power-on / soft-reset RAM fill policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RamClear {
    /// Keep the current contents (a soft reset does not clear work RAM).
    Keep,
    /// Fill with `$00` (the previous emulator default).
    #[default]
    Zero,
    /// Fill with `$FF`.
    Fill,
    /// Fill with a deterministic pseudo-random pattern.
    Random,
}

impl RamClear {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Zero => "zero",
            Self::Fill => "fill",
            Self::Random => "random",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "keep" | "none" => Self::Keep,
            "fill" | "ff" => Self::Fill,
            "random" => Self::Random,
            _ => Self::Zero,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Keep => "Keep contents",
            Self::Zero => "Fill $00",
            Self::Fill => "Fill $FF",
            Self::Random => "Random",
        }
    }
}

/// PPU register behaviour on a soft reset (the MiSTer "PPU Reset Behavior"
/// option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PpuResetBehavior {
    /// Famicom-style: a soft reset leaves the PPU registers alone.
    #[default]
    Famicom,
    /// NES-style: clear the `$2000`/`$2001` derived registers.
    Nes,
}

impl PpuResetBehavior {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Famicom => "famicom",
            Self::Nes => "nes",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "nes" => Self::Nes,
            _ => Self::Famicom,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Famicom => "Famicom (keep registers)",
            Self::Nes => "NES (clear registers)",
        }
    }
}

/// Which audio sources reach the mixer (the MiSTer "Audio Enable" option).
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

impl AudioSource {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Internal => "internal",
            Self::Expansion => "expansion",
            Self::None => "none",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "internal" | "apu" => Self::Internal,
            "expansion" | "cart" => Self::Expansion,
            "none" | "off" => Self::None,
            _ => Self::Both,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Both => "Both",
            Self::Internal => "Internal (2A03)",
            Self::Expansion => "Cartridge expansion",
            Self::None => "None",
        }
    }
}

/// Analog overscan crop (the MiSTer "Overscan" option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overscan {
    /// All 256x240 pixels.
    #[default]
    Normal,
    /// Hide 8 scanlines top and bottom.
    Vertical,
    /// Hide 8 pixels left and right.
    Borders,
    /// Hide 8 pixels on every side.
    Everything,
}

impl Overscan {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Vertical => "vertical",
            Self::Borders => "borders",
            Self::Everything => "everything",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "vertical" => Self::Vertical,
            "borders" => Self::Borders,
            "everything" | "all" => Self::Everything,
            _ => Self::Normal,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Vertical => "Vertical overscan",
            Self::Borders => "Borders",
            Self::Everything => "Everything",
        }
    }

    /// Visible source rectangle in pixels: `(x, y, width, height)`.
    #[must_use]
    pub const fn visible(self) -> (u16, u16, u16, u16) {
        match self {
            Self::Normal => (0, 0, 256, 240),
            Self::Vertical => (0, 8, 256, 224),
            Self::Borders => (8, 0, 240, 240),
            Self::Everything => (8, 8, 240, 224),
        }
    }
}

/// Display mask over the left/right software-mask columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskEdges {
    /// Show every column.
    #[default]
    Off,
    /// Mask the leftmost 8 pixels.
    Left,
    /// Mask both 8-pixel edges.
    Both,
    /// Mask the left edge unless the game already clips it.
    Auto,
}

impl MaskEdges {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Left => "left",
            Self::Both => "both",
            Self::Auto => "auto",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "left" => Self::Left,
            "both" => Self::Both,
            "auto" => Self::Auto,
            _ => Self::Off,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Left => "Left",
            Self::Both => "Both",
            Self::Auto => "Auto",
        }
    }
}
