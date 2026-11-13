//! Persisted frontend settings.
//!
//! Small, explicit key/value file rather than a serialization dependency; the
//! set is stable enough that a human can edit it when diagnosing a problem.

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use unko_core::{
    AudioFilterModel, AudioQuality, AudioSource, MaskEdges, Overscan, Palette, PortDevice,
    PpuResetBehavior, RamClear, Region, VideoFilter,
};

use crate::gamepad::{button_name, NesButton, PadMapping};

/// Actions with user-editable keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    /// Pause/resume.
    Pause,
    /// Save state.
    Save,
    /// Load state.
    Load,
    /// Toggle fullscreen.
    Fullscreen,
    /// Take a screenshot.
    Screenshot,
    /// Reset the console.
    Reset,
    /// Hold to fast-forward.
    FastForward,
    /// Advance one frame while paused.
    FrameAdvance,
    /// Step one frame back while paused.
    Rewind,
}

impl HotkeyAction {
    /// Every action, in display order.
    pub const ALL: [Self; 9] = [
        Self::Pause,
        Self::Save,
        Self::Load,
        Self::Fullscreen,
        Self::Screenshot,
        Self::Reset,
        Self::FastForward,
        Self::FrameAdvance,
        Self::Rewind,
    ];

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pause => "Play / Pause",
            Self::Save => "Save state",
            Self::Load => "Load state",
            Self::Fullscreen => "Fullscreen",
            Self::Screenshot => "Screenshot",
            Self::Reset => "Reset",
            Self::FastForward => "Hold to fast-forward",
            Self::FrameAdvance => "Frame advance",
            Self::Rewind => "Rewind",
        }
    }

    /// Stable setting key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Pause => "hotkey_pause",
            Self::Save => "hotkey_save",
            Self::Load => "hotkey_load",
            Self::Fullscreen => "hotkey_fullscreen",
            Self::Screenshot => "hotkey_screenshot",
            Self::Reset => "hotkey_reset",
            Self::FastForward => "hotkey_fast_forward",
            Self::FrameAdvance => "hotkey_frame_advance",
            Self::Rewind => "hotkey_rewind",
        }
    }
}

/// Keyboard shortcut table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkeys {
    /// Pause/resume key.
    pub pause: egui::Key,
    /// Save-state key.
    pub save: egui::Key,
    /// Load-state key.
    pub load: egui::Key,
    /// Fullscreen key.
    pub fullscreen: egui::Key,
    /// Screenshot key.
    pub screenshot: egui::Key,
    /// Reset key.
    pub reset: egui::Key,
    /// Hold-to-fast-forward key.
    pub fast_forward: egui::Key,
    /// Frame-advance key.
    pub frame_advance: egui::Key,
    /// Rewind key.
    pub rewind: egui::Key,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            pause: egui::Key::Space,
            save: egui::Key::F5,
            load: egui::Key::F8,
            fullscreen: egui::Key::F11,
            screenshot: egui::Key::F12,
            reset: egui::Key::F2,
            fast_forward: egui::Key::Tab,
            frame_advance: egui::Key::Period,
            rewind: egui::Key::Backspace,
        }
    }
}

impl Hotkeys {
    /// Returns the key bound to `action`.
    #[must_use]
    pub const fn get(self, action: HotkeyAction) -> egui::Key {
        match action {
            HotkeyAction::Pause => self.pause,
            HotkeyAction::Save => self.save,
            HotkeyAction::Load => self.load,
            HotkeyAction::Fullscreen => self.fullscreen,
            HotkeyAction::Screenshot => self.screenshot,
            HotkeyAction::Reset => self.reset,
            HotkeyAction::FastForward => self.fast_forward,
            HotkeyAction::FrameAdvance => self.frame_advance,
            HotkeyAction::Rewind => self.rewind,
        }
    }

    /// Binds `action` to `key`.
    pub fn set(&mut self, action: HotkeyAction, key: egui::Key) {
        match action {
            HotkeyAction::Pause => self.pause = key,
            HotkeyAction::Save => self.save = key,
            HotkeyAction::Load => self.load = key,
            HotkeyAction::Fullscreen => self.fullscreen = key,
            HotkeyAction::Screenshot => self.screenshot = key,
            HotkeyAction::Reset => self.reset = key,
            HotkeyAction::FastForward => self.fast_forward = key,
            HotkeyAction::FrameAdvance => self.frame_advance = key,
            HotkeyAction::Rewind => self.rewind = key,
        }
    }

    /// Parses a persisted key name.
    #[must_use]
    pub fn parse(self, action: HotkeyAction, value: &str) -> egui::Key {
        egui::Key::from_name(value).unwrap_or_else(|| self.get(action))
    }
}

/// Display aspect handling when no fixed integer scale is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aspect {
    /// Stretch to the available window area.
    Fit,
    /// Contained 4:3 (television).
    Tv43,
    /// Contained 8:7 pixel aspect (square-ish NES pixels).
    Pixel87,
}

impl Aspect {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Tv43 => "4:3",
            Self::Pixel87 => "8:7",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "4:3" | "tv" => Self::Tv43,
            "8:7" | "pixel" => Self::Pixel87,
            _ => Self::Fit,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fit => "Fit window",
            Self::Tv43 => "4:3 TV",
            Self::Pixel87 => "8:7 pixel",
        }
    }

    /// Display width divided by height, or `None` for a stretched fit.
    #[must_use]
    pub fn ratio(self) -> Option<f32> {
        match self {
            Self::Fit => None,
            Self::Tv43 => Some(4.0 / 3.0),
            Self::Pixel87 => Some(256.0 * (8.0 / 7.0) / 240.0),
        }
    }
}

/// Output post-process preset (applied after the core video filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderPreset {
    /// No additional effect.
    Off,
    /// Dark horizontal lines over odd rows.
    Scanlines,
    /// Aperture-grille style vertical/horizontal grid.
    Lcd,
    /// Scanlines plus a soft vignette.
    Crt,
}

impl ShaderPreset {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Scanlines => "scanlines",
            Self::Lcd => "lcd",
            Self::Crt => "crt",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "scanlines" => Self::Scanlines,
            "lcd" => Self::Lcd,
            "crt" => Self::Crt,
            _ => Self::Off,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Scanlines => "Scanlines",
            Self::Lcd => "LCD grid",
            Self::Crt => "CRT (scanlines + vignette)",
        }
    }
}

/// One library root tagged with the system it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
    /// System identifier (`nes` for now; SNES/N64 placeholders later).
    pub system: String,
    /// Folder scanned for that system.
    pub path: PathBuf,
}

/// UI colour theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Dark.
    Dark,
    /// Light.
    Light,
}

/// Everything the shell remembers between runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Folder scanned for ROMs.
    pub rom_dir: PathBuf,
    /// Folder for states, SRAM and screenshots.
    pub save_dir: PathBuf,
    /// Timing region.
    pub region: Region,
    /// Core-side video filter.
    pub filter: VideoFilter,
    /// Display palette preset.
    pub palette: Palette,
    /// User palette file used by [`Palette::Custom`] (`.pal`/`.palette`).
    pub custom_palette: PathBuf,
    /// Playback speed in percent (25, 50, 100, 150, 200 or 300).
    pub speed_percent: u32,
    /// Rewind ring budget in MiB (64, 128 or 256).
    pub rewind_mb: u32,
    /// Requested audio sample rate.
    pub sample_rate: f32,
    /// Target output latency in milliseconds (15..=250).
    pub audio_latency_ms: u32,
    /// Resampler kernel quality.
    pub audio_quality: AudioQuality,
    /// Analog output filter model (Custom carries the slider values).
    pub audio_filter: AudioFilterModel,
    /// Last custom high-pass cutoff in Hz.
    pub audio_high_pass: f32,
    /// Last custom low-pass cutoff in Hz.
    pub audio_low_pass: f32,
    /// Per-channel mixer gains: pulse1, pulse2, triangle, noise, DMC,
    /// expansion.
    pub channel_gains: [f32; 6],
    /// Per-channel mute switches, preserving the gain while muted.
    pub channel_mutes: [bool; 6],
    /// Which audio sources reach the mixer.
    pub audio_source: AudioSource,
    /// Analog overscan crop.
    pub overscan: Overscan,
    /// Display mask over the left/right software-mask columns.
    pub mask_edges: MaskEdges,
    /// Work-RAM fill applied by the next soft reset.
    pub ram_clear: RamClear,
    /// PPU register behaviour on a soft reset.
    pub ppu_reset: PpuResetBehavior,
    /// Keep integer pixel scaling when possible.
    pub integer_scale: bool,
    /// Use linear texture filtering (smooth scaling) instead of nearest.
    pub linear_filter: bool,
    /// Attach a Zapper light gun to port 2 (kept in sync with
    /// `port2_device` for older config files).
    pub zapper_port2: bool,
    /// Device attached to controller port 1.
    pub port1_device: PortDevice,
    /// Device attached to controller port 2.
    pub port2_device: PortDevice,
    /// Colour theme.
    pub theme: Theme,
    /// Pause when the window loses focus.
    pub pause_on_focus_loss: bool,
    /// Ask before stopping a running game.
    pub confirm_stop: bool,
    /// Show the bottom status bar.
    pub show_status_bar: bool,
    /// Library starts in grid mode.
    pub grid_view: bool,
    /// Output volume, 0.0..=1.0 (playback lands with the audio backend).
    pub volume: f32,
    /// Mute output.
    pub muted: bool,
    /// Enable SDL3 gamepad input.
    pub gamepad_enabled: bool,
    /// Gamepad-to-NES binding table.
    pub pad_mapping: PadMapping,
    /// Analog-stick dead zone.
    pub pad_deadzone: i16,
    /// Enable auto-fire for gamepad A/B.
    pub turbo_enabled: bool,
    /// Auto-fire frequency in Hz.
    pub turbo_hz: f32,
    /// Show the input display overlay.
    pub show_input_display: bool,
    /// Show the on-screen touch controller (mobile, also testable on desktop).
    pub show_touch_controls: bool,
    /// Keyboard shortcuts.
    pub hotkeys: Hotkeys,
    /// Keep gamepad bindings per game instead of globally.
    pub per_game_input: bool,
    /// Let Settings/Debugger/Devices open as native OS windows on desktop.
    pub detachable_windows: bool,
    /// Disable the two physical NES controller ports.
    pub port_disabled: [bool; 2],
    /// Disable individual physical gamepads.
    pub pad_disabled: [bool; 4],
    /// Physical gamepad slot -> NES port assignment (`0..=3`).
    pub pad_ports: [u8; 4],
    /// Number of columns in the library grid (1..=8).
    pub grid_columns: u32,
    /// Bitmask of visible list columns: 1 system, 2 file, 4 size, 8 region,
    /// 16 mapper, 32 modified (title is always visible).
    pub list_columns: u8,
    /// Additional per-system library roots (stub for multi-system support).
    pub library_roots: Vec<LibraryRoot>,
    /// Display aspect when no fixed integer scale is selected.
    pub aspect: Aspect,
    /// Fixed integer scale factor (0 = automatic).
    pub scale: u32,
    /// Output supersampling factor (1 = off, 2 or 3).
    pub supersample: u32,
    /// Output post-process preset.
    pub shader: ShaderPreset,
    /// Remembered Settings OS-window geometry `(x, y, width, height)`.
    pub layout_settings: Option<[f32; 4]>,
    /// Remembered Devices OS-window geometry.
    pub layout_devices: Option<[f32; 4]>,
    /// Remembered Debugger OS-window geometry.
    pub layout_debugger: Option<[f32; 4]>,
    /// Remembered Cheats OS-window geometry.
    pub layout_cheats: Option<[f32; 4]>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rom_dir: default_data_root().join("roms"),
            save_dir: default_data_root().join("saves"),
            region: Region::Auto,
            filter: VideoFilter::Raw,
            palette: Palette::Canonical,
            custom_palette: PathBuf::new(),
            speed_percent: 100,
            rewind_mb: 64,
            sample_rate: 48_000.0,
            audio_latency_ms: 60,
            audio_quality: AudioQuality::Standard,
            audio_filter: AudioFilterModel::NesRf,
            audio_high_pass: 90.0,
            audio_low_pass: 14_000.0,
            channel_gains: [1.0; 6],
            channel_mutes: [false; 6],
            audio_source: AudioSource::Both,
            overscan: Overscan::Normal,
            mask_edges: MaskEdges::Off,
            ram_clear: RamClear::Zero,
            ppu_reset: PpuResetBehavior::Famicom,
            integer_scale: true,
            linear_filter: false,
            zapper_port2: false,
            port1_device: PortDevice::Gamepad,
            port2_device: PortDevice::Gamepad,
            theme: Theme::Dark,
            pause_on_focus_loss: true,
            confirm_stop: false,
            show_status_bar: true,
            grid_view: false,
            volume: 0.8,
            muted: false,
            gamepad_enabled: true,
            pad_mapping: PadMapping::default(),
            pad_deadzone: 12_000,
            turbo_enabled: false,
            turbo_hz: 12.0,
            show_input_display: false,
            // Touch is the primary controller on phones and tablets.
            show_touch_controls: cfg!(any(target_os = "android", target_os = "ios")),
            hotkeys: Hotkeys::default(),
            per_game_input: false,
            detachable_windows: !cfg!(any(target_os = "android", target_os = "ios")),
            port_disabled: [false; 2],
            pad_disabled: [false; 4],
            pad_ports: [0, 1, 2, 3],
            grid_columns: 5,
            list_columns: 0b11_1111,
            library_roots: Vec::new(),
            aspect: Aspect::Fit,
            scale: 0,
            supersample: 1,
            shader: ShaderPreset::Off,
            layout_settings: None,
            layout_devices: None,
            layout_debugger: None,
            layout_cheats: None,
        }
    }
}

impl Settings {
    /// Default config path.
    ///
    /// Desktop keeps Dolphin-style portable mode (`unko.conf` beside the
    /// executable); mobile uses the platform configuration directory.
    pub fn path() -> PathBuf {
        if cfg!(any(target_os = "ios", target_os = "android")) {
            if let Some(dirs) = ProjectDirs::from("", "", "Unko") {
                return dirs.config_dir().join("unko.conf");
            }
        }
        PathBuf::from("unko.conf")
    }

    /// Loads settings, falling back to defaults for missing/unknown keys.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(text) = fs::read_to_string(path) else {
            return settings;
        };
        let mut explicit_port2 = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "rom_dir" => settings.rom_dir = value.into(),
                "save_dir" => settings.save_dir = value.into(),
                "region" => settings.region = parse_region(value),
                "filter" => settings.filter = parse_filter(value),
                "palette" => settings.palette = Palette::from_name(value),
                "custom_palette" => settings.custom_palette = value.into(),
                "speed_percent" => {
                    settings.speed_percent = match value.parse::<u32>().unwrap_or(100) {
                        25 | 50 | 150 | 200 | 300 => value.parse().unwrap_or(100),
                        _ => 100,
                    }
                }
                "rewind_mb" => {
                    settings.rewind_mb = match value.parse::<u32>().unwrap_or(64) {
                        128 | 256 => value.parse().unwrap_or(64),
                        _ => 64,
                    }
                }
                "sample_rate" => settings.sample_rate = value.parse().unwrap_or(48_000.0),
                "audio_latency_ms" => {
                    settings.audio_latency_ms = value.parse::<u32>().unwrap_or(60).clamp(15, 250)
                }
                "audio_quality" => settings.audio_quality = AudioQuality::from_name(value),
                "audio_filter" => settings.audio_filter = parse_audio_filter(value),
                "audio_high_pass" => {
                    settings.audio_high_pass =
                        value.parse::<f32>().unwrap_or(90.0).clamp(0.0, 2_000.0)
                }
                "audio_low_pass" => {
                    settings.audio_low_pass = value
                        .parse::<f32>()
                        .unwrap_or(14_000.0)
                        .clamp(0.0, 24_000.0)
                }
                "channel_gains" => {
                    let mut gains = [1.0f32; 6];
                    for (slot, text) in gains.iter_mut().zip(value.split(',')) {
                        *slot = text.trim().parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);
                    }
                    settings.channel_gains = gains;
                }
                "channel_mutes" => {
                    let mut mutes = [false; 6];
                    for (slot, text) in mutes.iter_mut().zip(value.split(',')) {
                        *slot = matches!(text.trim(), "1" | "true" | "on");
                    }
                    settings.channel_mutes = mutes;
                }
                "audio_source" => settings.audio_source = AudioSource::from_name(value),
                "overscan" => settings.overscan = Overscan::from_name(value),
                "mask_edges" => settings.mask_edges = MaskEdges::from_name(value),
                "ram_clear" => settings.ram_clear = RamClear::from_name(value),
                "ppu_reset" => settings.ppu_reset = PpuResetBehavior::from_name(value),
                "integer_scale" => settings.integer_scale = parse_bool(value),
                "linear_filter" => settings.linear_filter = parse_bool(value),
                "zapper_port2" => settings.zapper_port2 = parse_bool(value),
                "port1_device" => settings.port1_device = PortDevice::from_name(value),
                "port2_device" => {
                    settings.port2_device = PortDevice::from_name(value);
                    explicit_port2 = true;
                }
                "theme" => {
                    settings.theme = if value == "light" {
                        Theme::Light
                    } else {
                        Theme::Dark
                    }
                }
                "pause_on_focus_loss" => settings.pause_on_focus_loss = parse_bool(value),
                "confirm_stop" => settings.confirm_stop = parse_bool(value),
                "show_status_bar" => settings.show_status_bar = parse_bool(value),
                "grid_view" => settings.grid_view = parse_bool(value),
                "volume" => settings.volume = value.parse().unwrap_or(0.8),
                "muted" => settings.muted = parse_bool(value),
                "gamepad" => settings.gamepad_enabled = parse_bool(value),
                "pad_deadzone" => {
                    settings.pad_deadzone = value.parse().unwrap_or(12_000).clamp(0, 32_767)
                }
                "turbo" => settings.turbo_enabled = parse_bool(value),
                "turbo_hz" => {
                    settings.turbo_hz = value.parse::<f32>().unwrap_or(12.0).clamp(1.0, 60.0)
                }
                "input_display" => settings.show_input_display = parse_bool(value),
                "touch_controls" => settings.show_touch_controls = parse_bool(value),
                "per_game_input" => settings.per_game_input = parse_bool(value),
                "detachable_windows" => settings.detachable_windows = parse_bool(value),
                "port_disabled" => {
                    for (slot, text) in settings.port_disabled.iter_mut().zip(value.split(',')) {
                        *slot = matches!(text.trim(), "1" | "true" | "on");
                    }
                }
                "pad_disabled" => {
                    for (slot, text) in settings.pad_disabled.iter_mut().zip(value.split(',')) {
                        *slot = matches!(text.trim(), "1" | "true" | "on");
                    }
                }
                "pad_ports" => {
                    for (slot, text) in settings.pad_ports.iter_mut().zip(value.split(',')) {
                        *slot = text.trim().parse::<u8>().unwrap_or(0).min(3);
                    }
                }
                "grid_columns" => {
                    settings.grid_columns = value.parse::<u32>().unwrap_or(5).clamp(1, 8)
                }
                "list_columns" => {
                    settings.list_columns = value.parse::<u8>().unwrap_or(0b11_1111) & 0x3F
                }
                "aspect" => settings.aspect = Aspect::from_name(value),
                "scale" => settings.scale = value.parse::<u32>().unwrap_or(0).min(6),
                "supersample" => {
                    settings.supersample = value.parse::<u32>().unwrap_or(1).clamp(1, 3)
                }
                "shader" => settings.shader = ShaderPreset::from_name(value),
                "layout_settings" => settings.layout_settings = parse_layout(value),
                "layout_devices" => settings.layout_devices = parse_layout(value),
                "layout_debugger" => settings.layout_debugger = parse_layout(value),
                "layout_cheats" => settings.layout_cheats = parse_layout(value),
                "library_root" => {
                    if let Some((system, path)) = value.split_once('|') {
                        settings.library_roots.push(LibraryRoot {
                            system: system.trim().to_string(),
                            path: PathBuf::from(path),
                        });
                    }
                }
                key if key.starts_with("hotkey_") => {
                    if let Some(action) =
                        HotkeyAction::ALL.iter().find(|action| action.key() == key)
                    {
                        let bound = settings.hotkeys.parse(*action, value);
                        settings.hotkeys.set(*action, bound);
                    }
                }
                key if key.starts_with("pad_") => {
                    let name = &key[4..];
                    if let Some(button) = NesButton::ALL.iter().find(|b| b.key() == name) {
                        let bound = settings.pad_mapping.parse(*button, value);
                        settings.pad_mapping.set(*button, bound);
                    }
                }
                _ => {}
            }
        }
        if !explicit_port2 && settings.zapper_port2 {
            settings.port2_device = PortDevice::Zapper;
        }
        if matches!(settings.audio_filter, AudioFilterModel::Custom { .. }) {
            settings.audio_filter = settings.custom_audio_filter();
        }
        settings
    }

    /// The filter model with any custom slider values filled in.
    #[must_use]
    pub fn custom_audio_filter(&self) -> AudioFilterModel {
        AudioFilterModel::Custom {
            high_pass_hz: self.audio_high_pass,
            low_pass_hz: self.audio_low_pass,
        }
    }

    /// The per-channel gains after applying the mute switches.
    #[must_use]
    pub fn effective_channel_gains(&self) -> [f32; 6] {
        let mut gains = self.channel_gains;
        for (gain, muted) in gains.iter_mut().zip(self.channel_mutes) {
            if muted {
                *gain = 0.0;
            }
        }
        gains
    }

    /// The filter model to send to the core.
    #[must_use]
    pub fn audio_filter_model(&self) -> AudioFilterModel {
        match self.audio_filter {
            AudioFilterModel::Custom { .. } => self.custom_audio_filter(),
            other => other,
        }
    }

    /// Writes the settings file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                let _ = fs::create_dir_all(dir);
            }
        }
        let mut text = format!(
            "\
# Unko settings
rom_dir={}
save_dir={}
speed_percent={}
region={}
filter={}
sample_rate={}
audio_latency_ms={}
audio_quality={}
audio_filter={}
audio_high_pass={}
audio_low_pass={}
channel_gains={}
channel_mutes={}
audio_source={}
overscan={}
mask_edges={}
ram_clear={}
ppu_reset={}
integer_scale={}
linear_filter={}
palette={}
custom_palette={}
port1_device={}
port2_device={}
zapper_port2={}
theme={}
pause_on_focus_loss={}
confirm_stop={}
show_status_bar={}
grid_view={}
volume={}
muted={}
gamepad={}
pad_deadzone={}
turbo={}
turbo_hz={}
input_display={}
touch_controls={}
per_game_input={}
detachable_windows={}
port_disabled={}
pad_disabled={}
pad_ports={}
grid_columns={}
list_columns={}
aspect={}
scale={}
supersample={}
shader={}
layout_settings={}
layout_devices={}
layout_debugger={}
layout_cheats={}
",
            self.rom_dir.display(),
            self.save_dir.display(),
            self.speed_percent,
            region_name(self.region),
            filter_name(self.filter),
            self.sample_rate,
            self.audio_latency_ms,
            self.audio_quality.name(),
            self.audio_filter.name(),
            self.audio_high_pass,
            self.audio_low_pass,
            self.channel_gains
                .iter()
                .map(|gain| gain.to_string())
                .collect::<Vec<_>>()
                .join(","),
            self.channel_mutes
                .iter()
                .map(|muted| u8::from(*muted).to_string())
                .collect::<Vec<_>>()
                .join(","),
            self.audio_source.name(),
            self.overscan.name(),
            self.mask_edges.name(),
            self.ram_clear.name(),
            self.ppu_reset.name(),
            self.integer_scale,
            self.linear_filter,
            self.palette.name(),
            self.custom_palette.display(),
            self.port1_device.name(),
            self.port2_device.name(),
            self.port2_device == PortDevice::Zapper,
            match self.theme {
                Theme::Dark => "dark",
                Theme::Light => "light",
            },
            self.pause_on_focus_loss,
            self.confirm_stop,
            self.show_status_bar,
            self.grid_view,
            self.volume,
            self.muted,
            self.gamepad_enabled,
            self.pad_deadzone,
            self.turbo_enabled,
            self.turbo_hz,
            self.show_input_display,
            self.show_touch_controls,
            self.per_game_input,
            self.detachable_windows,
            self.port_disabled
                .iter()
                .map(|disabled| u8::from(*disabled).to_string())
                .collect::<Vec<_>>()
                .join(","),
            self.pad_disabled
                .iter()
                .map(|disabled| u8::from(*disabled).to_string())
                .collect::<Vec<_>>()
                .join(","),
            self.pad_ports
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(","),
            self.grid_columns,
            self.list_columns,
            self.aspect.name(),
            self.scale,
            self.supersample,
            self.shader.name(),
            layout_text(self.layout_settings),
            layout_text(self.layout_devices),
            layout_text(self.layout_debugger),
            layout_text(self.layout_cheats),
        );
        for root in &self.library_roots {
            text.push_str(&format!(
                "library_root={}|{}
",
                root.system,
                root.path.display()
            ));
        }
        for action in HotkeyAction::ALL {
            text.push_str(&format!(
                "{}={}\n",
                action.key(),
                self.hotkeys.get(action).name()
            ));
        }
        for button in NesButton::ALL {
            text.push_str(&format!(
                "pad_{}={}\n",
                button.key(),
                button_name(self.pad_mapping.get(button))
            ));
        }
        fs::write(path, text)
    }
}

fn default_data_root() -> PathBuf {
    if cfg!(any(target_os = "ios", target_os = "android")) {
        if let Some(dirs) = ProjectDirs::from("", "", "Unko") {
            return dirs.data_dir().to_path_buf();
        }
    }
    PathBuf::new()
}

/// Loads a 64-entry RGB palette from a binary `.pal` (192 bytes) or a text
/// file with 64 rows of three decimal/hex values.
#[must_use]
pub fn load_palette(path: &Path) -> Option<[[u8; 3]; 64]> {
    let bytes = fs::read(path).ok()?;
    let mut table = [[0u8; 3]; 64];
    // Binary `.pal` files are 64 RGB triplets (192 bytes) or 64 RGBA quads
    // (768 bytes, as written by some tools).
    let stride = match bytes.len() {
        768 => Some(4),
        192 => Some(3),
        _ if bytes.len() >= 768 && std::str::from_utf8(&bytes).is_err() => Some(4),
        _ => None,
    };
    if let Some(stride) = stride {
        for (index, entry) in table.iter_mut().enumerate() {
            let start = index * stride;
            entry.copy_from_slice(&bytes[start..start + 3]);
        }
        return Some(table);
    }
    let text = String::from_utf8(bytes).ok()?;
    let values: Vec<u8> = text
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|token| !token.is_empty() && !token.starts_with(';') && !token.starts_with('#'))
        .filter_map(|token| {
            let token = token.trim();
            token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
                .map_or_else(
                    || token.parse::<u8>().ok(),
                    |hex| u8::from_str_radix(hex, 16).ok(),
                )
        })
        .collect();
    if values.len() < 192 {
        return None;
    }
    for (index, entry) in table.iter_mut().enumerate() {
        entry.copy_from_slice(&values[index * 3..index * 3 + 3]);
    }
    Some(table)
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "true" | "1" | "yes" | "on")
}

fn parse_region(value: &str) -> Region {
    match value {
        "ntsc" => Region::Ntsc,
        "pal" => Region::Pal,
        "dendy" => Region::Dendy,
        _ => Region::Auto,
    }
}

fn region_name(region: Region) -> &'static str {
    match region {
        Region::Auto => "auto",
        Region::Ntsc => "ntsc",
        Region::Pal => "pal",
        Region::Dendy => "dendy",
    }
}

fn parse_layout(value: &str) -> Option<[f32; 4]> {
    let mut parts = value.split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    let width = parts.next()?.trim().parse().ok()?;
    let height = parts.next()?.trim().parse().ok()?;
    Some([x, y, width, height])
}

fn layout_text(layout: Option<[f32; 4]>) -> String {
    layout.map_or_else(String::new, |[x, y, width, height]| {
        format!("{x},{y},{width},{height}")
    })
}

fn parse_audio_filter(value: &str) -> AudioFilterModel {
    match value {
        "linear" => AudioFilterModel::Linear,
        "crisp" => AudioFilterModel::Crisp,
        "warm" => AudioFilterModel::Warm,
        "custom" => AudioFilterModel::Custom {
            high_pass_hz: 90.0,
            low_pass_hz: 14_000.0,
        },
        _ => AudioFilterModel::NesRf,
    }
}

fn parse_filter(value: &str) -> VideoFilter {
    match value {
        "composite" => VideoFilter::Composite,
        "crt" => VideoFilter::Crt,
        _ => VideoFilter::Raw,
    }
}

fn filter_name(filter: VideoFilter) -> &'static str {
    match filter {
        VideoFilter::Raw => "raw",
        VideoFilter::Composite => "composite",
        VideoFilter::Crt => "crt",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join("unko-settings-test.conf");
        let settings = Settings {
            region: Region::Pal,
            filter: VideoFilter::Crt,
            rom_dir: PathBuf::from("/games"),
            theme: Theme::Light,
            palette: Palette::Consumer,
            ..Settings::default()
        };
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded, settings);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn mister_accuracy_options_round_trip() {
        let path = std::env::temp_dir().join("unko-settings-mister.conf");
        let settings = Settings {
            audio_source: AudioSource::Expansion,
            overscan: Overscan::Everything,
            mask_edges: MaskEdges::Both,
            ram_clear: RamClear::Random,
            ppu_reset: PpuResetBehavior::Nes,
            ..Settings::default()
        };
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded, settings);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn palette_import_reads_binary_rgb_rgba_and_text() {
        let dir = std::env::temp_dir();
        let rgb_path = dir.join("unko-test-rgb.pal");
        let mut rgb = vec![0u8; 192];
        rgb[0..3].copy_from_slice(&[1, 2, 3]);
        rgb[189..192].copy_from_slice(&[4, 5, 6]);
        fs::write(&rgb_path, &rgb).expect("write");
        let table = load_palette(&rgb_path).expect("rgb palette");
        assert_eq!(table[0], [1, 2, 3]);
        assert_eq!(table[63], [4, 5, 6]);

        let rgba_path = dir.join("unko-test-rgba.pal");
        let mut rgba = vec![0u8; 768];
        rgba[0..4].copy_from_slice(&[7, 8, 9, 255]);
        fs::write(&rgba_path, &rgba).expect("write");
        let table = load_palette(&rgba_path).expect("rgba palette");
        assert_eq!(table[0], [7, 8, 9]);

        let text_path = dir.join("unko-test-text.pal");
        let text = "10 20 30\n".repeat(64);
        fs::write(&text_path, text).expect("write");
        let table = load_palette(&text_path).expect("text palette");
        assert_eq!(table[0], [10, 20, 30]);
        assert_eq!(table[63], [10, 20, 30]);

        let _ = fs::remove_file(rgb_path);
        let _ = fs::remove_file(rgba_path);
        let _ = fs::remove_file(text_path);
    }

    #[test]
    fn audio_mixer_and_filter_settings_round_trip() {
        let path = std::env::temp_dir().join("unko-settings-audio.conf");
        let mut settings = Settings::default();
        settings.audio_latency_ms = 30;
        settings.audio_quality = AudioQuality::High;
        settings.audio_high_pass = 120.0;
        settings.audio_low_pass = 9_000.0;
        settings.audio_filter = settings.custom_audio_filter();
        settings.channel_gains = [0.5, 1.0, 0.25, 1.0, 0.0, 0.75];
        settings.channel_mutes = [false, true, false, false, false, true];
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded, settings);
        assert_eq!(
            loaded.effective_channel_gains(),
            [0.5, 0.0, 0.25, 1.0, 0.0, 0.0]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn display_device_and_library_settings_round_trip() {
        let path = std::env::temp_dir().join("unko-settings-display.conf");
        let settings = Settings {
            port_disabled: [true, false],
            pad_disabled: [false, true, false, true],
            pad_ports: [2, 0, 3, 1],
            grid_columns: 7,
            list_columns: 0b10_1010,
            aspect: Aspect::Pixel87,
            scale: 3,
            supersample: 2,
            shader: ShaderPreset::Crt,
            layout_settings: Some([100.0, 120.0, 780.0, 500.0]),
            layout_devices: Some([420.0, 80.0, 680.0, 480.0]),
            layout_debugger: Some([40.0, 40.0, 900.0, 600.0]),
            library_roots: vec![
                LibraryRoot {
                    system: "nes".into(),
                    path: PathBuf::from("/roms/nes"),
                },
                LibraryRoot {
                    system: "snes".into(),
                    path: PathBuf::from("/roms/snes"),
                },
            ],
            ..Settings::default()
        };
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded, settings);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn custom_hotkeys_and_pad_profile_survive_a_save() {
        let path = std::env::temp_dir().join("unko-settings-hotkeys.conf");
        let mut settings = Settings::default();
        settings.hotkeys.set(HotkeyAction::Pause, egui::Key::P);
        settings.hotkeys.set(HotkeyAction::Save, egui::Key::F7);
        settings.per_game_input = true;
        settings
            .pad_mapping
            .set(NesButton::A, crate::gamepad::Button::East);
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded, settings);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn controller_devices_round_trip() {
        let path = std::env::temp_dir().join("unko-settings-devices.conf");
        let settings = Settings {
            port1_device: PortDevice::Microphone,
            port2_device: PortDevice::Tape,
            ..Settings::default()
        };
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded.port1_device, PortDevice::Microphone);
        assert_eq!(loaded.port2_device, PortDevice::Tape);
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("port1_device=microphone"));
        assert!(text.contains("port2_device=tape"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_zapper_flag_selects_the_zapper_device() {
        let path = std::env::temp_dir().join("unko-settings-legacy-zapper.conf");
        fs::write(&path, "zapper_port2=true\n").expect("write");
        let loaded = Settings::load(&path);
        assert_eq!(loaded.port2_device, PortDevice::Zapper);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_file_uses_defaults() {
        let loaded = Settings::load(Path::new("/definitely/not/here/unko.conf"));
        assert_eq!(loaded, Settings::default());
    }
}
