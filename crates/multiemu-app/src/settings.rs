//! Persisted frontend settings.
//!
//! Small, explicit key/value file rather than a serialization dependency; the
//! set is stable enough that a human can edit it when diagnosing a problem.

use std::fs;
use std::path::{Path, PathBuf};

use multiemu_core::{Region, VideoFilter};

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
    /// Requested audio sample rate.
    pub sample_rate: f32,
    /// Keep integer pixel scaling when possible.
    pub integer_scale: bool,
    /// Use linear texture filtering (smooth scaling) instead of nearest.
    pub linear_filter: bool,
    /// Attach a Zapper light gun to port 2 (mouse aims and fires).
    pub zapper_port2: bool,
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            rom_dir: PathBuf::from("roms"),
            save_dir: PathBuf::from("saves"),
            region: Region::Auto,
            filter: VideoFilter::Raw,
            sample_rate: 48_000.0,
            integer_scale: true,
            linear_filter: false,
            zapper_port2: false,
            theme: Theme::Dark,
            pause_on_focus_loss: true,
            confirm_stop: false,
            show_status_bar: true,
            grid_view: false,
            volume: 0.8,
            muted: false,
        }
    }
}

impl Settings {
    /// Default config path (portable mode, like Dolphin's `portable.txt`).
    pub fn path() -> PathBuf {
        PathBuf::from("multiemu.conf")
    }

    /// Loads settings, falling back to defaults for missing/unknown keys.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(text) = fs::read_to_string(path) else {
            return settings;
        };
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
                "sample_rate" => settings.sample_rate = value.parse().unwrap_or(48_000.0),
                "integer_scale" => settings.integer_scale = parse_bool(value),
                "linear_filter" => settings.linear_filter = parse_bool(value),
                "zapper_port2" => settings.zapper_port2 = parse_bool(value),
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
                _ => {}
            }
        }
        settings
    }

    /// Writes the settings file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = format!(
            "\
# multiemu settings
rom_dir={}
save_dir={}
region={}
filter={}
sample_rate={}
integer_scale={}
linear_filter={}
zapper_port2={}
theme={}
pause_on_focus_loss={}
confirm_stop={}
show_status_bar={}
grid_view={}
volume={}
muted={}
",
            self.rom_dir.display(),
            self.save_dir.display(),
            region_name(self.region),
            filter_name(self.filter),
            self.sample_rate,
            self.integer_scale,
            self.linear_filter,
            self.zapper_port2,
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
        );
        fs::write(path, text)
    }
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

fn parse_filter(value: &str) -> VideoFilter {
    match value {
        "composite" => VideoFilter::Composite,
        _ => VideoFilter::Raw,
    }
}

fn filter_name(filter: VideoFilter) -> &'static str {
    match filter {
        VideoFilter::Raw => "raw",
        VideoFilter::Composite => "composite",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join("multiemu-settings-test.conf");
        let settings = Settings {
            region: Region::Pal,
            filter: VideoFilter::Composite,
            rom_dir: PathBuf::from("/games"),
            theme: Theme::Light,
            ..Settings::default()
        };
        settings.save(&path).expect("writes");
        let loaded = Settings::load(&path);
        assert_eq!(loaded, settings);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_file_uses_defaults() {
        let loaded = Settings::load(Path::new("/definitely/not/here/multiemu.conf"));
        assert_eq!(loaded, Settings::default());
    }
}
