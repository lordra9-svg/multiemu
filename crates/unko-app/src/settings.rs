//! Persisted frontend settings.
//!
//! Small, explicit key/value file rather than a serialization dependency; the
//! set is stable enough that a human can edit it when diagnosing a problem.

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use unko_core::{AudioFilterModel, AudioQuality, Palette, PortDevice, Region, VideoFilter};

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
}

impl HotkeyAction {
    /// Every action, in display order.
    pub const ALL: [Self; 6] = [
        Self::Pause,
        Self::Save,
        Self::Load,
        Self::Fullscreen,
        Self::Screenshot,
        Self::Reset,
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
        }
    }

    /// Parses a persisted key name.
    #[must_use]
    pub fn parse(self, action: HotkeyAction, value: &str) -> egui::Key {
        egui::Key::from_name(value).unwrap_or_else(|| self.get(action))
    }
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rom_dir: default_data_root().join("roms"),
            save_dir: default_data_root().join("saves"),
            region: Region::Auto,
            filter: VideoFilter::Raw,
            palette: Palette::Canonical,
            sample_rate: 48_000.0,
            audio_latency_ms: 60,
            audio_quality: AudioQuality::Standard,
            audio_filter: AudioFilterModel::NesRf,
            audio_high_pass: 90.0,
            audio_low_pass: 14_000.0,
            channel_gains: [1.0; 6],
            channel_mutes: [false; 6],
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
integer_scale={}
linear_filter={}
palette={}
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
",
            self.rom_dir.display(),
            self.save_dir.display(),
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
            self.integer_scale,
            self.linear_filter,
            self.palette.name(),
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
        );
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
            port1_device: PortDevice::FamilyKeyboard,
            port2_device: PortDevice::FourScore,
            ..Settings::default()
        };
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded.port1_device, PortDevice::FamilyKeyboard);
        assert_eq!(loaded.port2_device, PortDevice::FourScore);
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("port1_device=keyboard"));
        assert!(text.contains("port2_device=fourscore"));
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
