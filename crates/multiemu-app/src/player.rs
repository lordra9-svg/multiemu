//! One running console, owned by the frontend.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use multiemu_core::{
    Core, InputState, LoadOptions, PortDevice, Region, Registry, SystemId, VideoFilter,
};

use crate::library::GameEntry;
use crate::settings::Settings;

const NTSC_FRAME: f32 = 1.0 / 60.0988;

/// A loaded machine plus frontend state (pause, slot, counters).
pub struct Player {
    core: Box<dyn Core>,
    texture: Option<egui::TextureHandle>,
    /// Pause without dropping the machine.
    pub paused: bool,
    /// Smoothed frames per second.
    pub fps: f32,
    /// Frames since launch.
    pub frames: u64,
    /// Selected save-state slot.
    pub slot: u8,
    /// Display title.
    pub title: String,
    /// Stable save name (title + CRC).
    pub stem: String,
    /// Where states/SRAM live.
    pub save_dir: PathBuf,
    /// Last operator-facing message.
    pub message: String,
    /// Texture filtering used for the next upload.
    texture_options: egui::TextureOptions,
    accum: f32,
    last_tick: Instant,
    sample_rate: f32,
}

impl Player {
    /// Loads a ROM and starts a machine.
    pub fn start(entry: &GameEntry, settings: &Settings) -> Result<Self, String> {
        let rom = multiemu_rom::load(entry).map_err(|err| format!("read ROM: {err}"))?;
        let mut registry = Registry::new();
        registry.register(multiemu_nes::registration());
        registry.register(multiemu_nes_accuracy_core::registration());
        let mut core = registry
            .create_default(SystemId::Nes)
            .ok_or("no NES core registered")?;
        core.load(
            &rom,
            &LoadOptions {
                region: settings.region,
                sample_rate: settings.sample_rate,
                video_filter: settings.filter,
            },
        )
        .map_err(|err| err.to_string())?;

        core.set_port_device(
            1,
            if settings.zapper_port2 {
                PortDevice::Zapper
            } else {
                PortDevice::Gamepad
            },
        );

        let save_dir = settings.save_dir.clone();
        fs::create_dir_all(&save_dir).map_err(|err| format!("create save dir: {err}"))?;
        // Key saves by a content hash (FNV-1a 64) in addition to the scan CRC so
        // renamed/colliding entries cannot share battery data.
        let stem = format!("{}-{:016x}", entry.save_stem(), rom_fingerprint(&rom));
        let sram_path = save_dir.join(format!("{stem}.sav"));
        if let Ok(sram) = fs::read(&sram_path) {
            core.load_sram(&sram).map_err(|err| err.to_string())?;
        }

        Ok(Self {
            core,
            texture: None,
            paused: false,
            fps: 0.0,
            frames: 0,
            slot: 0,
            title: entry.title.clone(),
            stem,
            save_dir,
            message: String::new(),
            texture_options: egui::TextureOptions::NEAREST,
            accum: NTSC_FRAME,
            last_tick: Instant::now(),
            sample_rate: settings.sample_rate,
        })
    }

    /// Advances emulation (bounded catch-up) and refreshes the texture.
    pub fn step(&mut self, ctx: &egui::Context) {
        self.apply_input(ctx);
        if self.paused {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.25);
        self.last_tick = now;
        self.accum += dt;

        let mut ran = 0;
        while (self.accum >= NTSC_FRAME && ran < 4) || (ran == 0 && self.frames == 0) {
            if let Err(err) = self.core.run_frame() {
                self.message = err.to_string();
                self.paused = true;
                break;
            }
            self.frames += 1;
            ran += 1;
            self.accum = (self.accum - NTSC_FRAME).max(0.0);
        }

        if ran > 0 {
            self.upload(ctx);
            if dt > 0.0001 {
                let instant = ran as f32 / dt;
                self.fps = if self.fps == 0.0 {
                    instant
                } else {
                    self.fps * 0.9 + instant * 0.1
                };
            }
        }
    }

    fn apply_input(&mut self, ctx: &egui::Context) {
        let input = ctx.input(|i| {
            let mut state = InputState::empty();
            state = state.with(InputState::UP, i.key_down(egui::Key::ArrowUp));
            state = state.with(InputState::DOWN, i.key_down(egui::Key::ArrowDown));
            state = state.with(InputState::LEFT, i.key_down(egui::Key::ArrowLeft));
            state = state.with(InputState::RIGHT, i.key_down(egui::Key::ArrowRight));
            state = state.with(InputState::A, i.key_down(egui::Key::X));
            state = state.with(InputState::B, i.key_down(egui::Key::Z));
            state = state.with(InputState::START, i.key_down(egui::Key::Enter));
            state = state.with(InputState::SELECT, i.modifiers.shift);
            state
        });
        self.core.set_input(0, input);
    }

    fn upload(&mut self, ctx: &egui::Context) {
        let video = self.core.video();
        let width = video.width as usize;
        let height = video.height as usize;
        if width == 0 || height == 0 {
            return;
        }
        let pixels: Vec<u8> = if video.stride == width * 4 {
            video.data.to_vec()
        } else {
            let mut packed = Vec::with_capacity(width * height * 4);
            for row in 0..height {
                let start = row * video.stride;
                packed.extend_from_slice(&video.data[start..start + width * 4]);
            }
            packed
        };
        let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels);
        match &mut self.texture {
            Some(texture) => texture.set(image, self.texture_options),
            None => {
                self.texture = Some(ctx.load_texture("nes-frame", image, self.texture_options));
            }
        }
    }

    /// Texture for the viewport, if a frame has been produced.
    #[must_use]
    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    /// Current output resolution.
    #[must_use]
    pub fn resolution(&self) -> (u32, u32) {
        let video = self.core.video();
        (video.width, video.height)
    }

    /// Core name/version for the status bar.
    #[must_use]
    pub fn core_label(&self) -> String {
        let info = self.core.info();
        format!("{} {}", info.name, info.version)
    }

    /// Attaches a device to a controller port.
    pub fn set_port_device(&mut self, port: usize, device: PortDevice) {
        self.core.set_port_device(port, device);
    }

    /// Aims the Zapper and reports its trigger.
    pub fn set_zapper(&mut self, port: usize, x: u8, y: u8, trigger: bool) {
        self.core.set_zapper(port, x, y, trigger);
    }

    /// Sets the display texture filtering (nearest vs smooth).
    pub fn set_texture_options(&mut self, options: egui::TextureOptions) {
        self.texture_options = options;
    }

    /// Applies a video filter live.
    pub fn set_filter(&mut self, filter: VideoFilter) {
        self.core.set_video_filter(filter);
    }

    /// Applies a sample rate live.
    pub fn set_sample_rate(&mut self, rate: f32) {
        self.sample_rate = rate;
        self.core.set_sample_rate(rate);
    }

    /// Pauses/resumes.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.last_tick = Instant::now();
        self.accum = 0.0;
    }

    /// Hard reset.
    pub fn reset(&mut self) {
        if let Err(err) = self.core.reset() {
            self.message = err.to_string();
        }
    }

    /// Saves a state into `slot`.
    pub fn save_state(&mut self, slot: u8) {
        let mut state = Vec::new();
        match self.core.save_state(&mut state) {
            Ok(()) => {
                let path = self.state_path(slot);
                match fs::write(&path, state) {
                    Ok(()) => {
                        let thumb = self.thumbnail_bytes();
                        let _ = fs::write(self.thumb_path(slot), thumb);
                        self.slot = slot;
                        self.message = format!("Saved state slot {slot}");
                    }
                    Err(err) => self.message = format!("state write: {err}"),
                }
            }
            Err(err) => self.message = err.to_string(),
        }
    }

    /// Loads a state from `slot`.
    pub fn load_state(&mut self, slot: u8) {
        let path = self.state_path(slot);
        match fs::read(&path) {
            Ok(state) => match self.core.load_state(&state) {
                Ok(()) => {
                    self.slot = slot;
                    self.last_tick = Instant::now();
                    self.accum = 0.0;
                    self.message = format!("Loaded state slot {slot}");
                }
                Err(err) => self.message = err.to_string(),
            },
            Err(err) => self.message = format!("state read: {err}"),
        }
    }

    /// Path of the saved thumbnail for `slot`.
    #[must_use]
    pub fn thumb_path(&self, slot: u8) -> PathBuf {
        self.save_dir
            .join(format!("{}.slot{slot}.thumb", self.stem))
    }

    /// Saved thumbnail (`64x60` RGBA) for `slot`, if one exists.
    #[must_use]
    pub fn thumbnail(&self, slot: u8) -> Option<Vec<u8>> {
        fs::read(self.thumb_path(slot)).ok()
    }

    fn thumbnail_bytes(&self) -> Vec<u8> {
        const THUMB_W: usize = 64;
        const THUMB_H: usize = 60;
        let video = self.core.video();
        let mut out = Vec::with_capacity(THUMB_W * THUMB_H * 4);
        if video.width == 0 || video.height == 0 {
            return out;
        }
        for ty in 0..THUMB_H {
            for tx in 0..THUMB_W {
                let sx = tx * video.width as usize / THUMB_W;
                let sy = ty * video.height as usize / THUMB_H;
                let index = sy * video.stride + sx * 4;
                if index + 4 <= video.data.len() {
                    out.extend_from_slice(&video.data[index..index + 4]);
                } else {
                    out.extend_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        out
    }

    /// Writes the current frame as a PPM next to the save data.
    pub fn screenshot(&mut self) {
        let video = self.core.video();
        let path = self
            .save_dir
            .join(format!("{}-{:06}.ppm", self.stem, self.frames % 1_000_000));
        let mut out = Vec::with_capacity(video.width as usize * video.height as usize * 3);
        for row in 0..video.height as usize {
            let start = row * video.stride;
            let line = &video.data[start..start + video.width as usize * 4];
            for pixel in line.as_chunks::<4>().0 {
                out.extend_from_slice(&pixel[..3]);
            }
        }
        let mut file = format!("P6\n{} {}\n255\n", video.width, video.height).into_bytes();
        file.extend_from_slice(&out);
        match fs::write(&path, file) {
            Ok(()) => self.message = format!("Screenshot: {}", path.display()),
            Err(err) => self.message = format!("screenshot: {err}"),
        }
    }

    /// Persists SRAM and a state, then drops the machine.
    pub fn shutdown(&mut self) {
        let sram = self.core.sram().to_vec();
        if !sram.is_empty() {
            let _ = fs::write(self.save_dir.join(format!("{}.sav", self.stem)), sram);
        }
        let mut state = Vec::new();
        if self.core.save_state(&mut state).is_ok() {
            let _ = fs::write(self.state_path(self.slot), state);
        }
    }

    /// Path for a state slot.
    #[must_use]
    pub fn state_path(&self, slot: u8) -> PathBuf {
        self.save_dir
            .join(format!("{}.slot{slot}.state", self.stem))
    }

    /// Requested sample rate, for the status bar.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

/// Parses a region string from the UI.
#[must_use]
pub fn region_label(region: Region) -> &'static str {
    match region {
        Region::Auto => "Auto",
        Region::Ntsc => "NTSC",
        Region::Pal => "PAL",
        Region::Dendy => "Dendy",
    }
}

/// Stable 64-bit content fingerprint (FNV-1a).
fn rom_fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
