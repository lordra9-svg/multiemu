//! One running console, owned by the frontend.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use unko_core::{
    Core, InputState, LoadOptions, PortDevice, Region, Registry, SystemId, VideoFilter,
};

use crate::library::GameEntry;
use crate::settings::Settings;

/// Maximum frames emulated in one UI update. When the desktop stalls the
/// process, the pacer drops the excess backlog instead of fast-forwarding,
/// so the app returns to real time immediately.
const MAX_CATCH_UP: usize = 4;

/// Fixed-timestep frame clock for the desktop shell.
///
/// Uses integer `Duration` arithmetic so a long session cannot drift by
/// fractions of a frame and start emitting double frames.
#[derive(Debug)]
struct FramePacer {
    frame: Duration,
    accum: Duration,
    last: Instant,
}

impl FramePacer {
    fn new(now: Instant) -> Self {
        Self {
            frame: Duration::from_secs_f64(1.0 / 60.0988),
            accum: Duration::ZERO,
            last: now,
        }
    }

    /// Drops any partial frame so the next update starts from *now*.
    fn reset(&mut self) {
        self.accum = Duration::ZERO;
        self.last = Instant::now();
    }

    /// Returns `(frames_to_run, elapsed, wait_until_next_frame)`.
    ///
    /// A stall is allowed to catch up at most [`MAX_CATCH_UP`] frames; the
    /// rest of the backlog is discarded so audio/video stay in real time
    /// instead of playing back the stall.
    fn advance(&mut self, now: Instant) -> (usize, f32, Duration) {
        let dt = now.saturating_duration_since(self.last);
        self.last = now;
        self.accum = self.accum.saturating_add(dt);
        let mut ran = 0;
        while self.accum >= self.frame && ran < MAX_CATCH_UP {
            self.accum -= self.frame;
            ran += 1;
        }
        if ran == MAX_CATCH_UP && self.accum >= self.frame {
            // Still behind after the bounded catch-up: resynchronise.
            self.accum = Duration::ZERO;
        }
        let wait = self.frame.saturating_sub(self.accum);
        (ran, dt.as_secs_f32(), wait)
    }
}

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
    pacer: FramePacer,
    next_repaint: Duration,
    sample_rate: f32,
}

impl Player {
    /// Loads a ROM and starts a machine.
    pub fn start(entry: &GameEntry, settings: &Settings) -> Result<Self, String> {
        let rom = unko_rom::load(entry).map_err(|err| format!("read ROM: {err}"))?;
        let mut registry = Registry::new();
        registry.register(nes_core::registration());
        registry.register(nes_accuracy_core::registration());
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
            pacer: FramePacer::new(Instant::now()),
            next_repaint: Duration::ZERO,
            sample_rate: settings.sample_rate,
        })
    }

    /// Advances emulation (bounded catch-up) and refreshes the texture.
    pub fn step(&mut self, ctx: &egui::Context) {
        self.apply_input(ctx);
        if self.paused {
            self.next_repaint = Duration::from_millis(250);
            return;
        }
        let (mut ran, dt, wait) = self.pacer.advance(Instant::now());
        self.next_repaint = wait;
        if ran == 0 && self.frames == 0 {
            ran = 1;
        }

        let mut done = 0;
        while done < ran {
            if let Err(err) = self.core.run_frame() {
                self.message = err.to_string();
                self.paused = true;
                self.next_repaint = Duration::from_millis(250);
                break;
            }
            self.frames += 1;
            done += 1;
        }

        if done > 0 {
            self.upload(ctx);
            if dt > 0.0001 {
                let instant = done as f32 / dt;
                self.fps = if self.fps == 0.0 {
                    instant
                } else {
                    self.fps * 0.9 + instant * 0.1
                };
            }
        }
    }

    /// How long the UI should wait before the next emulation update.
    #[must_use]
    pub fn next_repaint(&self) -> Duration {
        if self.paused {
            Duration::from_millis(250)
        } else {
            self.next_repaint
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
        self.pacer.reset();
        self.next_repaint = if self.paused {
            Duration::from_millis(250)
        } else {
            Duration::ZERO
        };
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
                    self.pacer.reset();
                    self.next_repaint = Duration::ZERO;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pacer_at(start: Instant) -> FramePacer {
        FramePacer::new(start)
    }

    #[test]
    fn pacer_runs_one_frame_per_interval() {
        let start = Instant::now();
        let mut pacer = pacer_at(start);
        let frame = pacer.frame;
        // The first interval is empty, so `Player::step` presents the
        // power-on frame itself (frames == 0).
        let (ran, _, wait) = pacer.advance(start);
        assert_eq!(ran, 0);
        assert_eq!(wait, frame);
        let (ran, _, wait) = pacer.advance(start + frame);
        assert_eq!(ran, 1);
        assert_eq!(wait, frame);
        let (ran, _, _) = pacer.advance(start + frame * 2);
        assert_eq!(ran, 1);
    }

    #[test]
    fn pacer_bounds_catch_up_and_drops_backlog() {
        let start = Instant::now();
        let mut pacer = pacer_at(start);
        let frame = pacer.frame;
        // A 10-frame UI stall must not produce 10 emulated frames.
        let (ran, _, wait) = pacer.advance(start + frame * 10);
        assert_eq!(ran, MAX_CATCH_UP);
        // Backlog is discarded, so the next update waits a full frame
        // instead of immediately running more stale frames.
        assert_eq!(wait, frame);
        let (ran, _, _) = pacer.advance(start + frame * 11);
        assert_eq!(ran, 1);
    }

    #[test]
    fn pacer_stays_locked_over_a_long_run() {
        let start = Instant::now();
        let mut pacer = pacer_at(start);
        let frame = pacer.frame;
        let mut now = start;
        let mut frames = 0;
        for _ in 0..600 {
            now += frame;
            let (ran, _, wait) = pacer.advance(now);
            frames += ran;
            assert!(ran <= 1, "steady 60 Hz input ran {ran} frames");
            assert!(wait <= frame, "wait {wait:?} exceeds one frame");
        }
        assert_eq!(frames, 600);
    }
}
