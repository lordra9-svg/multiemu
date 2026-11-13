//! One running console, owned by the frontend.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use unko_core::{
    AudioFilterModel, AudioQuality, AudioSource, Core, DebugSpace, InputState, LoadOptions,
    PortDevice, PpuResetBehavior, RamClear, Region, Registry, SystemId, VideoFilter,
};

use crate::audio::AudioOutput;
use crate::debugger::{Action, DebugView, Debugger};
use crate::library::GameEntry;
use crate::recording::{RecordModes, Recorder};
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
    fn advance(&mut self, now: Instant, speed_percent: u32) -> (usize, f32, Duration) {
        let interval = self.frame.mul_f64(100.0 / f64::from(speed_percent.max(1)));
        let dt = now.saturating_duration_since(self.last);
        self.last = now;
        self.accum = self.accum.saturating_add(dt);
        let mut ran = 0;
        while self.accum >= interval && ran < MAX_CATCH_UP {
            self.accum -= interval;
            ran += 1;
        }
        if ran == MAX_CATCH_UP && self.accum >= interval {
            // Still behind after the bounded catch-up: resynchronise.
            self.accum = Duration::ZERO;
        }
        let wait = interval.saturating_sub(self.accum);
        (ran, dt.as_secs_f32(), wait)
    }
}

/// Keyframes plus per-frame inputs for rewind.
///
/// A full state every 30 frames gives tens of seconds of history inside the
/// configured budget; restoring one keyframe and replaying at most 29 frames
/// is cheap enough to step back while paused.
struct RewindBuffer {
    keyframes: std::collections::VecDeque<(u64, Vec<u8>)>,
    inputs: std::collections::VecDeque<(u64, [InputState; 4])>,
    bytes: usize,
    budget: usize,
    interval: u64,
}

impl RewindBuffer {
    fn new(budget_mb: u32) -> Self {
        Self {
            keyframes: std::collections::VecDeque::new(),
            inputs: std::collections::VecDeque::new(),
            bytes: 0,
            budget: usize::try_from(budget_mb).unwrap_or(64) * 1024 * 1024,
            interval: 30,
        }
    }

    fn set_budget(&mut self, budget_mb: u32) {
        self.budget = usize::try_from(budget_mb).unwrap_or(64) * 1024 * 1024;
        self.trim();
    }

    fn clear(&mut self) {
        self.keyframes.clear();
        self.inputs.clear();
        self.bytes = 0;
    }

    fn push_input(&mut self, frame: u64, input: [InputState; 4]) {
        self.inputs.push_back((frame, input));
    }

    fn push_keyframe(&mut self, frame: u64, state: Vec<u8>) {
        self.bytes += state.len();
        self.keyframes.push_back((frame, state));
        self.trim();
    }

    fn trim(&mut self) {
        while self.bytes > self.budget && self.keyframes.len() > 1 {
            if let Some((frame, state)) = self.keyframes.pop_front() {
                self.bytes = self.bytes.saturating_sub(state.len());
                let keep_from = frame + self.interval;
                while self
                    .inputs
                    .front()
                    .is_some_and(|(input_frame, _)| *input_frame < keep_from)
                {
                    self.inputs.pop_front();
                }
            }
        }
        while self
            .inputs
            .front()
            .is_some_and(|(frame, _)| self.keyframes.front().is_some_and(|(kf, _)| frame < kf))
        {
            self.inputs.pop_front();
        }
    }

    fn oldest(&self) -> Option<u64> {
        self.keyframes.front().map(|(frame, _)| *frame)
    }

    /// Restores the newest keyframe at or before `target`, replays the
    /// recorded inputs to `target`, and drops the obsolete future states.
    fn restore(&mut self, core: &mut dyn Core, target: u64) -> Option<u64> {
        let index = self
            .keyframes
            .iter()
            .rposition(|(frame, _)| *frame <= target)?;
        let (start, state) = &self.keyframes[index];
        let start = *start;
        core.load_state(state).ok()?;
        for frame in (start + 1)..=target {
            if let Some((_, input)) = self.inputs.iter().find(|(at, _)| *at == frame) {
                for (port, state) in input.iter().enumerate() {
                    core.set_input(port, *state);
                }
            }
            core.run_frame().ok()?;
        }
        self.keyframes.retain(|(frame, _)| *frame <= target);
        self.inputs.retain(|(frame, _)| *frame <= target);
        self.bytes = self.keyframes.iter().map(|(_, state)| state.len()).sum();
        Some(target)
    }
}

/// A frame copied out of the core before a run-ahead restore.
struct PendingVideo {
    data: Vec<u8>,
    width: u32,
    height: u32,
    stride: usize,
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
    /// Core id, so state files never cross between implementations.
    pub core_id: String,
    /// Where states/SRAM live.
    pub save_dir: PathBuf,
    /// Last operator-facing message.
    pub message: String,
    /// Texture filtering used for the next upload.
    texture_options: egui::TextureOptions,
    pacer: FramePacer,
    next_repaint: Duration,
    speed_percent: u32,
    fast_forward: bool,
    last_input: [InputState; 4],
    rewind: RewindBuffer,
    rewind_enabled: bool,
    movie_record: Option<Vec<[u16; 4]>>,
    movie_state: Option<Vec<u8>>,
    movie_playback: Option<(Vec<[u16; 4]>, usize)>,
    /// Extra frames emulated ahead of the committed frame (0 = off).
    run_ahead: u32,
    sample_rate: f32,
    gamepad: [InputState; 4],
    touch: InputState,
    recorder: Recorder,
    /// Debugger window state and breakpoints.
    pub debug: Debugger,
    /// Which logical controller ports accept input.
    port_enabled: [bool; 4],
    /// Output supersampling factor (1 = off).
    supersample: u32,
}

impl Player {
    /// Loads a ROM and starts a machine.
    pub fn start(entry: &GameEntry, settings: &Settings) -> Result<Self, String> {
        let rom = unko_rom::load(entry).map_err(|err| format!("read ROM: {err}"))?;
        let mut registry = Registry::new();
        // The accuracy core is the default now that it owns save states.
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
                palette: settings.palette,
            },
        )
        .map_err(|err| err.to_string())?;

        // The shell re-applies the selected devices every frame; set the
        // initial pair here so the first frame already has the right devices.
        // The Four Score adapter occupies both ports.
        let (device1, device2) = if settings.port1_device == PortDevice::FourScore
            || settings.port2_device == PortDevice::FourScore
        {
            (PortDevice::FourScore, PortDevice::FourScore)
        } else {
            (settings.port1_device, settings.port2_device)
        };
        core.set_port_device(0, device1);
        core.set_port_device(1, device2);
        core.set_audio_quality(settings.audio_quality);
        core.set_audio_filters(settings.audio_filter_model());
        core.set_channel_gains(settings.effective_channel_gains());
        core.set_audio_source(settings.audio_source);
        core.set_ram_clear(settings.ram_clear);
        core.set_ppu_reset_behavior(settings.ppu_reset);
        core.set_mmc3_alternate(settings.mmc3_alternate);
        if settings.palette == unko_core::Palette::Custom {
            if let Some(table) = crate::settings::load_palette(&settings.custom_palette) {
                core.set_palette_table(table);
            }
        }

        let save_dir = settings.save_dir.clone();
        fs::create_dir_all(&save_dir).map_err(|err| format!("create save dir: {err}"))?;
        // Key saves by a content hash (FNV-1a 64) in addition to the scan CRC so
        // renamed/colliding entries cannot share battery data.
        let stem = format!("{}-{:016x}", entry.save_stem(), rom_fingerprint(&rom));
        let sram_path = save_dir.join(format!("{stem}.sav"));
        if let Ok(sram) = fs::read(&sram_path) {
            core.load_sram(&sram).map_err(|err| err.to_string())?;
        }
        // The Datach X24C02 lives in the adapter, so one file is shared by
        // every Datach game rather than keyed by ROM.
        let eeprom_path = save_dir.join("datach.eeprom");
        if let Ok(eeprom) = fs::read(&eeprom_path) {
            core.load_datach_eeprom(&eeprom);
        }

        let core_id = core.info().id.to_string();
        let mut message = String::new();
        if settings.region == Region::Auto
            && entry.header.as_ref().is_some_and(|header| !header.nes2)
        {
            message = "No NES 2.0 header; defaulting to NTSC".to_string();
        }
        let mut player = Self {
            core,
            texture: None,
            paused: false,
            fps: 0.0,
            frames: 0,
            slot: 0,
            title: entry.title.clone(),
            stem,
            core_id,
            save_dir,
            message,
            texture_options: egui::TextureOptions::NEAREST,
            pacer: FramePacer::new(Instant::now()),
            next_repaint: Duration::ZERO,
            speed_percent: settings.speed_percent,
            fast_forward: false,
            sample_rate: settings.sample_rate,
            gamepad: [InputState::empty(); 4],
            touch: InputState::empty(),
            recorder: Recorder::default(),
            debug: Debugger::default(),
            port_enabled: [true; 4],
            supersample: settings.supersample,
            last_input: [InputState::empty(); 4],
            rewind: RewindBuffer::new(settings.rewind_mb),
            rewind_enabled: true,
            movie_record: None,
            movie_state: None,
            movie_playback: None,
            run_ahead: settings.run_ahead,
        };
        player.push_rewind_keyframe();
        Ok(player)
    }

    /// Advances emulation (bounded catch-up) and refreshes the texture.
    pub fn step(&mut self, ctx: &egui::Context, mut audio: Option<&mut AudioOutput>) {
        self.apply_input(ctx);
        if self.paused {
            self.next_repaint = Duration::from_millis(250);
            return;
        }
        self.update_rewind_enabled();
        let effective_speed = if self.fast_forward {
            300
        } else {
            self.speed_percent
        };
        let (mut ran, dt, wait) = self.pacer.advance(Instant::now(), effective_speed);
        self.next_repaint = wait;
        if ran == 0 && self.frames == 0 {
            ran = 1;
        }
        // Run-ahead predicts input for the frames it shows, so it only makes
        // sense at real-time speed and without an authoritative input source
        // (a movie) or a live debugger stepping instruction by instruction.
        let ahead = if self.speed_percent == 100
            && !self.fast_forward
            && self.movie_record.is_none()
            && self.movie_playback.is_none()
            && self.debug.breakpoints.is_empty()
        {
            self.run_ahead
        } else {
            0
        };

        let mut pending: Option<PendingVideo> = None;
        let mut done = 0;
        while done < ran {
            if !self.consume_movie_input() {
                break;
            }
            if self.rewind_enabled {
                self.rewind.push_input(self.frames + 1, self.last_input);
            }
            if let Some(record) = &mut self.movie_record {
                record.push(self.last_input.map(unko_core::InputState::bits));
            }
            if self.debug.breakpoints.is_empty() || !self.core.debug_supported() {
                if let Err(err) = self.core.run_frame() {
                    self.message = err.to_string();
                    self.paused = true;
                    self.next_repaint = Duration::from_millis(250);
                    break;
                }
            } else {
                match self.run_to_breakpoint() {
                    Ok(Some(pc)) => {
                        self.message = format!("Breakpoint at ${pc:04X}");
                        self.paused = true;
                        self.next_repaint = Duration::from_millis(250);
                        break;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        self.message = err;
                        self.paused = true;
                        self.next_repaint = Duration::from_millis(250);
                        break;
                    }
                }
            }
            self.frames += 1;
            done += 1;
            if self.speed_percent == 100 {
                if let Some(out) = audio.as_deref_mut() {
                    out.push(self.core.audio().samples);
                }
            }
            self.recorder.push_audio(self.core.audio().samples);
            let video = self.core.video();
            self.recorder.push_frame(
                video.data,
                video.width as usize,
                video.height as usize,
                video.stride,
            );

            // Run-ahead: the first frame above is the committed frame. Keep
            // its end state, run `ahead - 1` extra frames for the picture the
            // player will see, then restore the committed state so the next
            // update continues from the right place.
            let mut lead_state = None;
            if ahead > 0 {
                let mut state = Vec::new();
                match self.core.save_state(&mut state) {
                    Ok(()) => {
                        let mut failed = false;
                        for _ in 1..ahead {
                            if let Err(err) = self.core.run_frame() {
                                self.message = err.to_string();
                                self.paused = true;
                                self.next_repaint = Duration::from_millis(250);
                                failed = true;
                                break;
                            }
                        }
                        let video = self.core.video();
                        pending = Some(PendingVideo {
                            data: video.data.to_vec(),
                            width: video.width,
                            height: video.height,
                            stride: video.stride,
                        });
                        if let Err(err) = self.core.load_state(&state) {
                            self.message = err.to_string();
                            self.paused = true;
                            self.next_repaint = Duration::from_millis(250);
                            failed = true;
                        }
                        if failed {
                            break;
                        }
                        lead_state = Some(state);
                    }
                    Err(err) => {
                        self.message = err.to_string();
                        self.paused = true;
                        self.next_repaint = Duration::from_millis(250);
                        break;
                    }
                }
            }
            if self.rewind_enabled && self.frames.is_multiple_of(self.rewind.interval) {
                if let Some(state) = lead_state {
                    self.rewind.push_keyframe(self.frames, state);
                } else {
                    self.push_rewind_keyframe();
                }
            }
        }

        if done > 0 {
            if let Some(video) = pending.take() {
                self.upload_pixels(
                    ctx,
                    &video.data,
                    video.width as usize,
                    video.height as usize,
                    video.stride,
                );
            } else {
                self.upload(ctx);
            }
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

    /// Consumes one movie input (playback) and applies it to the core.
    /// Returns false when a playback ended this frame.
    fn consume_movie_input(&mut self) -> bool {
        let Some((inputs, index)) = &mut self.movie_playback else {
            return true;
        };
        let Some(input) = inputs.get(*index).copied() else {
            self.movie_playback = None;
            self.paused = true;
            self.next_repaint = Duration::from_millis(250);
            self.message = "Movie finished".to_string();
            return false;
        };
        *index += 1;
        for (port, bits) in input.iter().enumerate() {
            self.last_input[port] = InputState::from_bits(*bits);
            self.core.set_input(port, self.last_input[port]);
        }
        true
    }

    /// Starts recording an input movie from the current machine state.
    pub fn start_movie_recording(&mut self) -> Result<(), String> {
        if self.movie_playback.is_some() {
            return Err("stop movie playback first".into());
        }
        let mut state = Vec::new();
        self.core
            .save_state(&mut state)
            .map_err(|err| err.to_string())?;
        self.movie_record = Some(Vec::new());
        self.movie_state = Some(state);
        self.message = "Recording input movie".to_string();
        Ok(())
    }

    /// Stops recording and writes the movie.
    pub fn stop_movie_recording(&mut self, path: &std::path::Path) -> Result<(), String> {
        let Some(inputs) = self.movie_record.take() else {
            return Err("not recording a movie".into());
        };
        let Some(state) = self.movie_state.take() else {
            return Err("missing movie start state".into());
        };
        crate::movie::Movie {
            stem: self.stem.clone(),
            state,
            inputs,
        }
        .save(path)
        .map_err(|err| err.to_string())?;
        self.message = format!("Saved input movie to {}", path.display());
        Ok(())
    }

    /// Loads a movie state and starts playback.
    pub fn start_movie_playback(&mut self, movie: crate::movie::Movie) -> Result<(), String> {
        if self.movie_record.is_some() {
            return Err("stop recording first".into());
        }
        if movie.stem != self.stem {
            return Err("movie belongs to another game".into());
        }
        self.core
            .load_state(&movie.state)
            .map_err(|err| err.to_string())?;
        self.movie_playback = Some((movie.inputs, 0));
        self.paused = false;
        self.pacer.reset();
        self.next_repaint = Duration::ZERO;
        self.rewind.clear();
        self.frames = 0;
        self.message = "Playing input movie".to_string();
        Ok(())
    }

    /// Stops movie playback and leaves the machine paused.
    pub fn stop_movie_playback(&mut self) {
        if self.movie_playback.take().is_some() {
            self.paused = true;
            self.message = "Movie playback stopped".to_string();
        }
    }

    /// Whether an input movie is being recorded.
    #[must_use]
    pub fn movie_recording(&self) -> bool {
        self.movie_record.is_some()
    }

    /// Number of frames captured by the active movie recording.
    #[must_use]
    pub fn movie_recorded_frames(&self) -> usize {
        self.movie_record.as_ref().map_or(0, Vec::len)
    }

    /// Playback progress `(current, total)` when a movie is playing.
    #[must_use]
    pub fn movie_progress(&self) -> Option<(usize, usize)> {
        self.movie_playback
            .as_ref()
            .map(|(inputs, index)| (*index, inputs.len()))
    }

    fn update_rewind_enabled(&mut self) {
        let enabled = self.speed_percent == 100
            && !self.fast_forward
            && !self.recording()
            && self.movie_record.is_none()
            && self.movie_playback.is_none();
        if enabled == self.rewind_enabled {
            return;
        }
        self.rewind_enabled = enabled;
        self.rewind.clear();
        if enabled {
            self.push_rewind_keyframe();
        }
    }

    fn push_rewind_keyframe(&mut self) {
        let mut state = Vec::new();
        if self.core.save_state(&mut state).is_ok() {
            self.rewind.push_keyframe(self.frames, state);
        }
    }

    /// Selects the alternate MMC3A/NEC IRQ revision.
    pub fn set_mmc3_alternate(&mut self, alternate: bool) {
        self.core.set_mmc3_alternate(alternate);
    }

    /// Changes the rewind budget (64, 128 or 256 MiB).
    pub fn set_rewind_mb(&mut self, budget_mb: u32) {
        self.rewind.set_budget(budget_mb);
    }

    /// Number of frames currently available to rewind.
    #[must_use]
    pub fn rewind_available(&self) -> u64 {
        self.frames
            .saturating_sub(self.rewind.oldest().unwrap_or(self.frames))
    }

    /// Steps one frame back while paused. Returns whether the frame changed.
    pub fn rewind_step(&mut self, ctx: &egui::Context) -> bool {
        let Some(oldest) = self.rewind.oldest() else {
            return false;
        };
        let target = self.frames.saturating_sub(1);
        if target < oldest {
            self.message = "Rewind buffer exhausted".to_string();
            return false;
        }
        self.paused = true;
        if self.rewind.restore(self.core.as_mut(), target).is_some() {
            self.frames = target;
            self.pacer.reset();
            self.next_repaint = Duration::from_millis(250);
            self.upload(ctx);
            self.message = format!("Rewound to frame {target}");
            true
        } else {
            false
        }
    }

    /// Steps instructions until the frame completes or a breakpoint is hit.
    ///
    /// Returns the program counter of the breakpoint, or `None` when the frame
    /// completed normally. The first instruction after resuming is always
    /// executed, so resuming from a breakpoint does not trap immediately.
    fn run_to_breakpoint(&mut self) -> Result<Option<u16>, String> {
        let start = self.core.debug_ppu().map(|ppu| ppu.frame);
        for step in 0..2_000_000u32 {
            if step > 0 {
                if let Some(cpu) = self.core.debug_cpu() {
                    if self.debug.breakpoints.contains(&cpu.pc) {
                        return Ok(Some(cpu.pc));
                    }
                }
            }
            self.core.debug_step().map_err(|err| err.to_string())?;
            if self.core.debug_ppu().map(|ppu| ppu.frame) != start {
                return Ok(None);
            }
        }
        Err("breakpoint run exceeded its instruction budget".into())
    }

    /// Assembles the debugger snapshot for the current machine state.
    #[must_use]
    pub fn debug_view(&self) -> Option<DebugView> {
        let cpu = self.core.debug_cpu()?;
        let ppu = self.core.debug_ppu()?;
        let apu = self.core.debug_apu()?;
        let palette = self.core.debug_palette()?;
        let mut listing = Vec::with_capacity(8);
        let mut address = cpu.pc;
        for _ in 0..8 {
            let Some(line) = self.core.debug_disassemble(address) else {
                break;
            };
            let advance = line.bytes.len().max(1) as u16;
            listing.push(line);
            address = address.wrapping_add(advance);
        }
        let base = (self.debug.space.clamp(u32::from(self.debug.address)) as u16) & !0x0F;
        let mut memory = Vec::with_capacity(128);
        for offset in 0..128u16 {
            memory.push(
                self.core
                    .debug_peek(self.debug.space, base.wrapping_add(offset))
                    .unwrap_or(0),
            );
        }
        let mut oam = Vec::with_capacity(256);
        for offset in 0..256u16 {
            oam.push(self.core.debug_peek(DebugSpace::Oam, offset).unwrap_or(0));
        }
        Some(DebugView {
            cpu,
            ppu,
            apu,
            listing,
            memory,
            memory_base: base,
            palette,
            oam,
        })
    }

    /// Executes one CPU instruction and refreshes the displayed frame.
    pub fn debug_step_instruction(&mut self, ctx: &egui::Context) -> Result<u32, String> {
        let cycles = self.core.debug_step().map_err(|err| err.to_string())?;
        self.upload(ctx);
        Ok(cycles)
    }

    /// Executes one display frame while paused and refreshes the texture.
    pub fn debug_step_frame(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.core.run_frame().map_err(|err| err.to_string())?;
        self.frames += 1;
        self.upload(ctx);
        Ok(())
    }

    /// Applies one debugger action. `ctx` is used to refresh the video texture.
    pub fn apply_debug(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Pause => self.paused = true,
            Action::Resume => {
                self.paused = false;
                self.pacer.reset();
                self.next_repaint = Duration::ZERO;
            }
            Action::StepInstruction => {
                self.paused = true;
                match self.debug_step_instruction(ctx) {
                    Ok(cycles) => self.debug.message = format!("stepped {cycles} cycles"),
                    Err(err) => self.debug.message = err,
                }
            }
            Action::StepFrame => {
                self.paused = true;
                match self.debug_step_frame(ctx) {
                    Ok(()) => self.debug.message = "stepped one frame".into(),
                    Err(err) => self.debug.message = err,
                }
            }
            Action::Reset => {
                self.reset();
                self.debug.message = "reset".into();
            }
            Action::Poke(space, address, value) => {
                if self.core.debug_poke(space, address, value) {
                    self.debug.message = format!("wrote ${value:02X} to ${address:04X}");
                } else {
                    self.debug.message = format!("${address:04X} is read-only");
                }
            }
            Action::ToggleBreakpoint(pc) => {
                if let Some(index) = self.debug.breakpoints.iter().position(|value| *value == pc) {
                    self.debug.breakpoints.remove(index);
                    self.debug.message = format!("removed breakpoint ${pc:04X}");
                } else {
                    self.debug.breakpoints.push(pc);
                    self.debug.message = format!("breakpoint set at ${pc:04X}");
                }
            }
            Action::RemoveBreakpoint(pc) => {
                self.debug.breakpoints.retain(|value| *value != pc);
                self.debug.message = format!("removed breakpoint ${pc:04X}");
            }
            Action::ClearBreakpoints => {
                self.debug.breakpoints.clear();
                self.debug.message = "cleared breakpoints".into();
            }
            Action::ShowAddress(space, address) => {
                self.debug.space = space;
                self.debug.address = space.clamp(u32::from(address)) as u16;
            }
        }
        ctx.request_repaint();
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
        let keyboard = ctx.input(|i| {
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
        let mut pads = self.gamepad;
        for (port, state) in pads.iter_mut().enumerate() {
            if !self.port_enabled[port] {
                *state = InputState::empty();
            }
        }
        let keyboard = if self.port_enabled[0] {
            keyboard
        } else {
            InputState::empty()
        };
        let touch = if self.port_enabled[0] {
            self.touch
        } else {
            InputState::empty()
        };
        let first = InputState::from_bits(keyboard.bits() | pads[0].bits() | touch.bits());
        self.last_input[0] = first;
        self.core.set_input(0, first);
        for (port, state) in pads.iter().enumerate().skip(1) {
            self.last_input[port] = *state;
            self.core.set_input(port, *state);
        }
    }

    /// Sets the gamepad state for one controller port.
    pub fn set_gamepad(&mut self, port: usize, state: InputState) {
        if let Some(slot) = self.gamepad.get_mut(port) {
            *slot = state;
        }
    }

    /// Sets the touch overlay state (merged into port 1).
    pub fn set_touch(&mut self, state: InputState) {
        self.touch = state;
    }

    /// Sets the output supersampling factor (`1..=3`).
    pub fn set_supersample(&mut self, factor: u32) {
        self.supersample = factor.clamp(1, 3);
    }

    /// Enables or disables a logical controller port. A disabled port reports
    /// no buttons and ignores keyboard, touch and gamepad input.
    pub fn set_port_enabled(&mut self, port: usize, enabled: bool) {
        if let Some(slot) = self.port_enabled.get_mut(port) {
            *slot = enabled;
        }
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
        self.upload_packed(ctx, pixels, width, height);
    }

    /// Uploads a copied core frame (used by run-ahead after its restore).
    fn upload_pixels(
        &mut self,
        ctx: &egui::Context,
        data: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let pixels = if stride == width * 4 {
            data.to_vec()
        } else {
            let mut packed = Vec::with_capacity(width * height * 4);
            for row in 0..height {
                let start = row * stride;
                packed.extend_from_slice(&data[start..start + width * 4]);
            }
            packed
        };
        self.upload_packed(ctx, pixels, width, height);
    }

    /// Uploads an already packed (stride = width * 4) frame.
    fn upload_packed(
        &mut self,
        ctx: &egui::Context,
        mut pixels: Vec<u8>,
        width: usize,
        height: usize,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let mut upload_width = width;
        let mut upload_height = height;
        // Output supersampling: duplicate each pixel 2x/3x and let the linear
        // texture downscale smooth the result at the final window size. The
        // PPU stays 256x240, so this is a presentation filter, not detail.
        if self.supersample > 1 {
            let factor = self.supersample as usize;
            let mut scaled = Vec::with_capacity(pixels.len() * factor * factor);
            for row in 0..height {
                for _ in 0..factor {
                    for column in 0..width {
                        let start = (row * width + column) * 4;
                        let texel = &pixels[start..start + 4];
                        for _ in 0..factor {
                            scaled.extend_from_slice(texel);
                        }
                    }
                }
            }
            pixels = scaled;
            upload_width *= factor;
            upload_height *= factor;
        }
        let image =
            egui::ColorImage::from_rgba_unmultiplied([upload_width, upload_height], &pixels);
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

    /// Copies the current frame as tightly packed RGBA (for GPU upload).
    #[must_use]
    pub fn frame_rgba(&self) -> Option<(Vec<u8>, u32, u32)> {
        let video = self.core.video();
        let width = video.width as usize;
        let height = video.height as usize;
        if width == 0 || height == 0 {
            return None;
        }
        let pixels = if video.stride == width * 4 {
            video.data.to_vec()
        } else {
            let mut packed = Vec::with_capacity(width * height * 4);
            for row in 0..height {
                let start = row * video.stride;
                packed.extend_from_slice(&video.data[start..start + width * 4]);
            }
            packed
        };
        Some((pixels, video.width, video.height))
    }

    /// Core name/version for the status bar.
    #[must_use]
    pub fn core_label(&self) -> String {
        let info = self.core.info();
        format!("{} {}", info.name, info.version)
    }

    /// Attaches a device to a controller port.
    /// Aims the Zapper and reports its trigger.
    pub fn set_zapper(&mut self, port: usize, x: u8, y: u8, trigger: bool) {
        self.core.set_zapper(port, x, y, trigger);
    }

    /// Sets both controller ports' devices (Four Score replaces both).
    pub fn set_port_devices(&mut self, port1: PortDevice, port2: PortDevice) {
        self.core.set_port_device(0, port1);
        self.core.set_port_device(1, port2);
    }

    /// Sets an Arkanoid paddle's knob and fire button.
    pub fn set_paddle(&mut self, port: usize, position: u8, fire: bool) {
        self.core.set_paddle(port, position, fire);
    }

    /// Sets the pressed Power Pad buttons (bit `i` = button `i + 1`).
    pub fn set_power_pad(&mut self, port: usize, buttons: u16) {
        self.core.set_power_pad(port, buttons);
    }

    /// Replaces the Family BASIC keyboard matrix.
    pub fn set_family_keyboard(&mut self, keys: [u8; 9]) {
        self.core.set_family_keyboard(keys);
    }

    /// Replaces the Subor keyboard matrix.
    pub fn set_subor_keyboard(&mut self, rows: [u8; 13]) {
        self.core.set_subor_keyboard(rows);
    }

    /// Changes the display palette and rebuilds the cached colours.
    pub fn set_palette(&mut self, palette: unko_core::Palette) {
        self.core.set_palette(palette);
    }

    /// Sets the display texture filtering (nearest vs smooth).
    pub fn set_texture_options(&mut self, options: egui::TextureOptions) {
        self.texture_options = options;
    }

    /// Applies a video filter live.
    pub fn set_filter(&mut self, filter: VideoFilter) {
        self.core.set_video_filter(filter);
    }

    /// Selects a playback speed preset (25/50/100/150/200/300%).
    pub fn set_speed(&mut self, percent: u32) {
        self.speed_percent = match percent {
            25 | 50 | 100 | 150 | 200 | 300 => percent,
            _ => 100,
        };
        self.pacer.reset();
    }

    /// Sets extra run-ahead frames (0 = off, clamped to 0-4).
    pub fn set_run_ahead(&mut self, frames: u32) {
        self.run_ahead = frames.min(4);
    }

    /// Holds or releases fast-forward.
    pub fn set_fast_forward(&mut self, fast: bool) {
        if self.fast_forward != fast {
            self.fast_forward = fast;
            self.pacer.reset();
        }
    }

    /// Whether the hold-to-fast-forward key is down.
    #[must_use]
    pub const fn fast_forward(&self) -> bool {
        self.fast_forward
    }

    /// Current speed preset.
    #[must_use]
    pub const fn speed_percent(&self) -> u32 {
        self.speed_percent
    }

    /// Runs exactly one frame while paused and refreshes the texture.
    pub fn advance_frame(&mut self, ctx: &egui::Context) {
        if !self.consume_movie_input() {
            return;
        }
        if self.rewind_enabled {
            self.rewind.push_input(self.frames + 1, self.last_input);
        }
        if let Some(record) = &mut self.movie_record {
            record.push(self.last_input.map(unko_core::InputState::bits));
        }
        if let Err(err) = self.core.run_frame() {
            self.message = err.to_string();
            return;
        }
        self.frames += 1;
        if self.rewind_enabled && self.frames.is_multiple_of(self.rewind.interval) {
            self.push_rewind_keyframe();
        }
        self.upload(ctx);
        self.paused = true;
        self.pacer.reset();
        self.next_repaint = Duration::from_millis(250);
    }

    /// Applies a sample rate live.
    pub fn set_sample_rate(&mut self, rate: f32) {
        self.sample_rate = rate;
        self.core.set_sample_rate(rate);
    }

    /// Reports the Famicom microphone state (`$4016` bit 2).
    pub fn set_microphone(&mut self, pressed: bool) {
        self.core.set_microphone(pressed);
    }

    /// Replaces the active cheat overlay.
    pub fn set_cheats(&mut self, cheats: &[unko_core::Cheat]) {
        self.core.set_cheats(cheats);
    }

    /// Loads a Datach barcode (mapper 157); returns false for other boards.
    pub fn set_datach_barcode(&mut self, code: &str) -> bool {
        self.core.set_datach_barcode(code)
    }

    /// Supplies the 64-entry table used by the custom palette.
    pub fn set_palette_table(&mut self, table: [[u8; 3]; 64]) {
        self.core.set_palette_table(table);
    }

    /// Applies the resampler quality live.
    pub fn set_audio_quality(&mut self, quality: AudioQuality) {
        self.core.set_audio_quality(quality);
    }

    /// Applies the analog output filter model live.
    pub fn set_audio_filters(&mut self, filters: AudioFilterModel) {
        self.core.set_audio_filters(filters);
    }

    /// Applies the per-channel mixer gains live.
    pub fn set_channel_gains(&mut self, gains: [f32; 6]) {
        self.core.set_channel_gains(gains);
    }

    /// Selects which audio sources reach the mixer.
    pub fn set_audio_source(&mut self, source: AudioSource) {
        self.core.set_audio_source(source);
    }

    /// Selects the RAM fill used by the next soft reset.
    pub fn set_ram_clear(&mut self, mode: RamClear) {
        self.core.set_ram_clear(mode);
    }

    /// Selects the PPU register behaviour on a soft reset.
    pub fn set_ppu_reset_behavior(&mut self, behavior: PpuResetBehavior) {
        self.core.set_ppu_reset_behavior(behavior);
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
        self.frames = 0;
        self.rewind.clear();
        self.push_rewind_keyframe();
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
                    self.rewind.clear();
                    self.push_rewind_keyframe();
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
            .join(format!("{}.{}.slot{slot}.thumb", self.stem, self.core_id))
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
        self.recorder.stop();
        let sram = self.core.sram().to_vec();
        if !sram.is_empty() {
            let _ = fs::write(self.save_dir.join(format!("{}.sav", self.stem)), sram);
        }
        let eeprom = self.core.datach_eeprom();
        if !eeprom.is_empty() {
            let _ = fs::write(self.save_dir.join("datach.eeprom"), eeprom);
        }
        let mut state = Vec::new();
        if self.core.save_state(&mut state).is_ok() {
            let _ = fs::write(self.state_path(self.slot), state);
        }
    }

    /// Starts a capture in the save folder's `recordings` directory.
    pub fn start_recording(&mut self, modes: RecordModes) -> Result<String, String> {
        if !modes.any() {
            return Err("no recording mode selected".into());
        }
        let (width, height) = self.resolution();
        self.recorder.start(
            modes,
            &self.save_dir.join("recordings"),
            self.sample_rate,
            width,
            height,
            60.0,
        )
    }

    /// Stops the active capture.
    pub fn stop_recording(&mut self) -> bool {
        let active = self.recorder.is_active();
        self.recorder.stop();
        active
    }

    /// Frames captured so far.
    #[must_use]
    pub fn recording_frames(&self) -> u64 {
        self.recorder.frames()
    }

    /// Whether a capture is running.
    #[must_use]
    pub fn recording(&self) -> bool {
        self.recorder.is_active()
    }

    /// Path for a state slot.
    #[must_use]
    pub fn state_path(&self, slot: u8) -> PathBuf {
        self.save_dir
            .join(format!("{}.{}.slot{slot}.state", self.stem, self.core_id))
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
        let (ran, _, wait) = pacer.advance(start, 100);
        assert_eq!(ran, 0);
        assert_eq!(wait, frame);
        let (ran, _, wait) = pacer.advance(start + frame, 100);
        assert_eq!(ran, 1);
        assert_eq!(wait, frame);
        let (ran, _, _) = pacer.advance(start + frame * 2, 100);
        assert_eq!(ran, 1);
    }

    #[test]
    fn pacer_bounds_catch_up_and_drops_backlog() {
        let start = Instant::now();
        let mut pacer = pacer_at(start);
        let frame = pacer.frame;
        // A 10-frame UI stall must not produce 10 emulated frames.
        let (ran, _, wait) = pacer.advance(start + frame * 10, 100);
        assert_eq!(ran, MAX_CATCH_UP);
        // Backlog is discarded, so the next update waits a full frame
        // instead of immediately running more stale frames.
        assert_eq!(wait, frame);
        let (ran, _, _) = pacer.advance(start + frame * 11, 100);
        assert_eq!(ran, 1);
    }

    fn debug_nrom() -> Vec<u8> {
        let prg = 16 * 1024;
        let mut rom = vec![0u8; 16 + prg + 8 * 1024];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[5] = 1;
        // LDA #$10 / STA $00 / JMP $8005
        rom[16..21].copy_from_slice(&[0xA9, 0x10, 0x85, 0x00, 0x4C]);
        rom[21..23].copy_from_slice(&[0x05, 0x80]);
        let vector = 16 + prg - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        rom
    }

    #[test]
    fn breakpoints_stop_before_the_instruction() {
        let dir = std::env::temp_dir().join("unko-app-debug-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("debug.nes");
        std::fs::write(&path, debug_nrom()).expect("write rom");
        let entry = crate::library::probe_path(&path)
            .expect("probe")
            .into_iter()
            .next()
            .expect("entry");
        let settings = Settings {
            save_dir: dir.join("saves"),
            ..Settings::default()
        };
        let mut player = Player::start(&entry, &settings).expect("start");

        let view = player.debug_view().expect("debug view");
        assert_eq!(view.cpu.pc, 0x8000);
        assert_eq!(
            view.listing.first().map(|line| line.text.as_str()),
            Some("LDA #$10")
        );

        player.debug.breakpoints.push(0x8002);
        assert_eq!(player.run_to_breakpoint().expect("run"), Some(0x8002));
        let view = player.debug_view().expect("debug view");
        assert_eq!(view.cpu.pc, 0x8002);
        assert_eq!(view.cpu.a, 0x10, "the LDA before the breakpoint executed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn minimal_nrom() -> Vec<u8> {
        let mut rom = vec![0u8; 16 + 0x8000 + 0x2000];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 2; // 32 KiB PRG
        rom[5] = 1; // 8 KiB CHR
        for byte in &mut rom[16..16 + 0x8000] {
            *byte = 0xEA;
        }
        // JMP $8000 at the reset vector.
        rom[16..19].copy_from_slice(&[0x4C, 0x00, 0x80]);
        let vector = 16 + 0x8000 - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        rom
    }

    #[test]
    fn rewind_replays_to_the_exact_state() {
        let rom = minimal_nrom();
        let mut core = nes_accuracy_core::AccuracyCore::new();
        core.load(&rom, &unko_core::LoadOptions::default())
            .expect("load");

        let mut rewind = RewindBuffer::new(64);
        let mut initial = Vec::new();
        core.save_state(&mut initial).expect("initial state");
        rewind.push_keyframe(0, initial);

        let input_at =
            |frame: u64| InputState::from_bits(if frame.is_multiple_of(2) { 1 } else { 2 });
        let mut state_at_45 = Vec::new();
        for frame in 1..=60u64 {
            let input = input_at(frame);
            rewind.push_input(frame, [input; 4]);
            for port in 0..4 {
                core.set_input(port, input);
            }
            core.run_frame().expect("frame");
            if frame == 45 {
                core.save_state(&mut state_at_45).expect("state 45");
            }
            if frame.is_multiple_of(30) {
                let mut state = Vec::new();
                core.save_state(&mut state).expect("keyframe");
                rewind.push_keyframe(frame, state);
            }
        }

        assert_eq!(rewind.restore(&mut core, 45), Some(45));
        let mut restored = Vec::new();
        core.save_state(&mut restored).expect("restored state");
        assert_eq!(restored, state_at_45, "rewind replay must be exact");
    }

    #[test]
    #[ignore]
    fn bench_state_and_frame_cost() {
        let rom = minimal_nrom();
        let mut core = nes_accuracy_core::AccuracyCore::new();
        core.load(&rom, &unko_core::LoadOptions::default())
            .expect("load");
        for _ in 0..10 {
            core.run_frame().expect("warm");
        }
        let mut state = Vec::new();
        let start = Instant::now();
        for _ in 0..100 {
            state.clear();
            core.save_state(&mut state).expect("save");
        }
        println!(
            "save_state: {:?} each, {} bytes",
            start.elapsed() / 100,
            state.len()
        );
        let start = Instant::now();
        for _ in 0..100 {
            core.load_state(&state).expect("load");
        }
        println!("load_state: {:?} each", start.elapsed() / 100);
        let start = Instant::now();
        for _ in 0..600 {
            core.run_frame().expect("frame");
        }
        println!("run_frame: {:?} each", start.elapsed() / 600);
    }

    #[test]
    fn run_ahead_restore_matches_plain_emulation() {
        let rom = minimal_nrom();
        let mut plain = nes_accuracy_core::AccuracyCore::new();
        plain
            .load(&rom, &unko_core::LoadOptions::default())
            .expect("load plain");
        let mut ahead = nes_accuracy_core::AccuracyCore::new();
        ahead
            .load(&rom, &unko_core::LoadOptions::default())
            .expect("load ahead");

        for frame in 0..90u64 {
            let input = InputState::from_bits(if frame.is_multiple_of(2) { 0x01 } else { 0x02 });
            for core in [&mut plain, &mut ahead] {
                for port in 0..4 {
                    core.set_input(port, input);
                }
                core.run_frame().expect("frame");
            }
            // The committed frame is done; run three prediction frames and
            // restore, exactly like the frontend does for 4-frame run-ahead.
            let mut state = Vec::new();
            ahead.save_state(&mut state).expect("save lead");
            for _ in 1..4 {
                ahead.run_frame().expect("extra");
            }
            ahead.load_state(&state).expect("restore lead");

            if frame.is_multiple_of(10) {
                let mut a = Vec::new();
                let mut b = Vec::new();
                plain.save_state(&mut a).expect("plain state");
                ahead.save_state(&mut b).expect("ahead state");
                assert_eq!(a, b, "run-ahead drifted at frame {frame}");
            }
        }
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
            let (ran, _, wait) = pacer.advance(now, 100);
            frames += ran;
            assert!(ran <= 1, "steady 60 Hz input ran {ran} frames");
            assert!(wait <= frame, "wait {wait:?} exceeds one frame");
        }
        assert_eq!(frames, 600);
    }
}
