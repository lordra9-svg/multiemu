//! Main window: library, player, menus, toolbar, settings.
//!
//! Layout follows Dolphin's desktop shell: menu bar, toolbar, central stack
//! (library or emulation surface), status bar, and one settings window with a
//! category rail. See `docs/frontend.md` for the mapping and the licensing
//! boundary (Dolphin is GPL; this is an original MIT implementation of the UX).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Vec2};
use unko_core::{
    AudioFilterModel, AudioQuality, AudioSource, MaskEdges, Overscan, PortDevice, PpuResetBehavior,
    RamClear, Region, VideoFilter,
};

use crate::audio::AudioOutput;
use crate::cheats::{self, CheatEntry};
use crate::debugger::{self, Action};
use crate::gamepad::{
    button_name, mapping_from_text, mapping_to_text, Gamepads, NesButton, PadMapping,
};
use crate::library::{self, GameEntry};
use crate::player::{self, Player};
use crate::recording::RecordModes;
use crate::settings::{Aspect, HotkeyAction, Settings, ShaderPreset, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Interface,
    Graphics,
    Audio,
    Controllers,
    Paths,
    Nes,
    Advanced,
}

impl Tab {
    const ALL: [Tab; 8] = [
        Tab::General,
        Tab::Interface,
        Tab::Graphics,
        Tab::Audio,
        Tab::Controllers,
        Tab::Paths,
        Tab::Nes,
        Tab::Advanced,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Interface => "Interface",
            Tab::Graphics => "Graphics",
            Tab::Audio => "Audio",
            Tab::Controllers => "Controllers",
            Tab::Paths => "Paths",
            Tab::Nes => "NES",
            Tab::Advanced => "Advanced",
        }
    }
}

/// Application state.
pub struct App {
    settings: Settings,
    library: Vec<GameEntry>,
    search: String,
    selected: Option<usize>,
    player: Option<Player>,
    settings_open: bool,
    tab: Tab,
    about_open: bool,
    open_rom_open: bool,
    open_rom_text: String,
    fullscreen: bool,
    status: String,
    status_until: Option<Instant>,
    pending_play: Option<GameEntry>,
    window_title: String,
    gamepads: Option<Gamepads>,
    input_states: [unko_core::InputState; 4],
    gamepad_error: Option<String>,
    audio: Option<AudioOutput>,
    audio_error: Option<String>,
    hotkey_capture: Option<HotkeyAction>,
    active_mapping: PadMapping,
    paused_by_focus: bool,
    debug_on_start: bool,
    devices_open: bool,
    cheats_open: bool,
    cheats: Vec<CheatEntry>,
    cheat_code: String,
    cheat_addr: String,
    cheat_value: String,
    cheat_error: Option<String>,
    microphone: bool,
    datach_code: String,
    confirm_reset: bool,
    window_generation: u32,
    #[cfg(not(target_os = "ios"))]
    gpu_shader: Option<crate::gpu_shader::Handle>,
}

/// Command-line overrides.
#[derive(Debug, Default)]
pub struct Args {
    /// ROM folder to scan.
    pub roms: Option<PathBuf>,
    /// ROM to launch immediately.
    pub rom: Option<PathBuf>,
    /// Open the settings window on launch.
    pub settings: bool,
    /// Open the debugger window when a game starts.
    pub debug: bool,
}

impl App {
    /// Builds the app, loading the library from the configured folder.
    pub fn new(cc: &eframe::CreationContext<'_>, mut settings: Settings, args: Args) -> Self {
        if let Some(dir) = args.roms {
            settings.rom_dir = dir;
        }
        apply_theme(&cc.egui_ctx, settings.theme);
        let library = scan(&settings.rom_dir);
        let status = if library.is_empty() {
            format!(
                "No ROMs in {} — add .nes files or .zip/.7z/.rar archives",
                settings.rom_dir.display()
            )
        } else {
            format!("{} ROMs in {}", library.len(), settings.rom_dir.display())
        };
        let pending_play = args.rom.as_deref().and_then(resolve_first);
        let (gamepads, gamepad_error) = match Gamepads::new() {
            Ok(gamepads) => (Some(gamepads), None),
            Err(err) => (None, Some(err)),
        };
        let (audio, audio_error) =
            match AudioOutput::new(settings.sample_rate, settings.audio_latency_ms) {
                Ok(audio) => (Some(audio), None),
                Err(err) => (None, Some(err)),
            };
        if let Some(audio) = &audio {
            audio.set_volume(settings.volume);
            audio.set_muted(settings.muted);
        }
        let active_mapping = settings.pad_mapping;
        #[cfg(not(target_os = "ios"))]
        let gpu_shader = cc.gl.as_ref().and_then(|gl| {
            crate::gpu_shader::GpuShader::new(gl)
                .map(|shader| std::sync::Arc::new(std::sync::Mutex::new(shader)))
        });
        let mut app = Self {
            settings,
            library,
            search: String::new(),
            selected: None,
            player: None,
            settings_open: false,
            tab: Tab::General,
            about_open: false,
            open_rom_open: false,
            open_rom_text: String::new(),
            fullscreen: false,
            status,
            status_until: None,
            pending_play,
            window_title: "Unko".into(),
            gamepads,
            input_states: [unko_core::InputState::empty(); 4],
            gamepad_error,
            audio,
            audio_error,
            hotkey_capture: None,
            active_mapping,
            paused_by_focus: false,
            debug_on_start: args.debug,
            devices_open: false,
            cheats_open: false,
            cheats: Vec::new(),
            cheat_code: String::new(),
            cheat_addr: String::new(),
            cheat_value: String::new(),
            cheat_error: None,
            microphone: false,
            datach_code: String::new(),
            confirm_reset: false,
            window_generation: 0,
            #[cfg(not(target_os = "ios"))]
            gpu_shader,
        };
        app.status_until = Some(Instant::now() + Duration::from_secs(6));
        if args.settings {
            app.settings_open = true;
        }
        if let Some(entry) = app.pending_play.take() {
            app.start_entry(entry);
        }
        if let Some(player) = &mut app.player {
            if args.debug {
                player.debug.open = true;
            }
        }
        app
    }

    fn scan(&mut self) {
        let mut roots = vec![self.settings.rom_dir.clone()];
        for root in &self.settings.library_roots {
            if root.system == "nes" {
                roots.push(root.path.clone());
            }
        }
        let mut library = Vec::new();
        for root in roots {
            library.extend(scan(&root));
        }
        let mut seen = std::collections::HashSet::new();
        library.retain(|entry| seen.insert(entry.source.display_path()));
        self.library = library;
        self.selected = None;
        self.status = if self.library.is_empty() {
            format!(
                "No ROMs in {} — add .nes files or .zip/.7z/.rar archives",
                self.settings.rom_dir.display()
            )
        } else if self.settings.library_roots.is_empty() {
            format!(
                "{} ROMs in {}",
                self.library.len(),
                self.settings.rom_dir.display()
            )
        } else {
            format!(
                "{} ROMs in {} + {} extra root(s)",
                self.library.len(),
                self.settings.rom_dir.display(),
                self.settings.library_roots.len()
            )
        };
        self.status_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_until = Some(Instant::now() + Duration::from_secs(6));
    }

    fn start_game(&mut self, path: &Path) {
        match library::probe_path(path) {
            Ok(entries) => match entries.into_iter().next() {
                Some(entry) => self.start_entry(entry),
                None => self.set_status(format!("No playable NES image in {}", path.display())),
            },
            Err(err) => self.set_status(format!("{err}")),
        }
    }

    fn start_entry(&mut self, entry: GameEntry) {
        let mut settings = self.settings.clone();
        if let Some(audio) = &self.audio {
            settings.sample_rate = audio.sample_rate();
        }
        match Player::start(&entry, &settings) {
            Ok(player) => {
                self.active_mapping = if self.settings.per_game_input {
                    let path = self
                        .settings
                        .save_dir
                        .join(format!("{}.pad", entry.save_stem()));
                    fs::read_to_string(path)
                        .map(|text| mapping_from_text(&text))
                        .unwrap_or(self.settings.pad_mapping)
                } else {
                    self.settings.pad_mapping
                };
                self.set_status(format!("Loaded {} — {}", entry.title, player.core_label()));
                self.player = Some(player);
                if let Some(player) = &self.player {
                    let path = self
                        .settings
                        .save_dir
                        .join(format!("{}.cheats", player.stem));
                    self.cheats = cheats::load(&path);
                }
                self.apply_cheats();
                if self.debug_on_start {
                    if let Some(player) = &mut self.player {
                        player.debug.open = true;
                    }
                }
            }
            Err(err) => self.set_status(format!("Load failed: {err}")),
        }
    }

    fn start_recording(&mut self, modes: RecordModes) {
        let Some(player) = &mut self.player else {
            return;
        };
        match player.start_recording(modes) {
            Ok(paths) => self.set_status(format!("Recording to {paths}")),
            Err(err) => self.set_status(format!("Recording failed: {err}")),
        }
    }

    fn stop_game(&mut self) {
        self.paused_by_focus = false;
        if self.settings.per_game_input {
            if let Some(player) = &self.player {
                let path = self.settings.save_dir.join(format!("{}.pad", player.stem));
                let _ = fs::write(path, mapping_to_text(self.active_mapping));
            }
        }
        if let Some(mut player) = self.player.take() {
            player.shutdown();
            self.set_status(format!("Stopped {}", player.title));
        }
        self.cheats.clear();
        self.cheats_open = false;
        self.cheat_error = None;
    }

    fn apply_cheats(&mut self) {
        let active: Vec<unko_core::Cheat> = self
            .cheats
            .iter()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.cheat.clone())
            .collect();
        if let Some(player) = &mut self.player {
            player.set_cheats(&active);
        }
    }

    fn save_cheats(&mut self) {
        let Some(player) = &self.player else {
            return;
        };
        let path = self
            .settings
            .save_dir
            .join(format!("{}.cheats", player.stem));
        if let Err(err) = cheats::save(&path, &self.cheats) {
            self.set_status(format!("Cheat save failed: {err}"));
        }
    }

    fn apply_settings(&mut self, ctx: &egui::Context, old: &Settings) {
        self.settings.zapper_port2 = self.settings.port2_device == PortDevice::Zapper;
        if let Some(player) = &mut self.player {
            player.set_filter(self.settings.filter);
            player.set_palette(self.settings.palette);
            if self.settings.palette == unko_core::Palette::Custom {
                if let Some(table) = crate::settings::load_palette(&self.settings.custom_palette) {
                    player.set_palette_table(table);
                }
            }
            let rate = self
                .audio
                .as_ref()
                .map_or(self.settings.sample_rate, |audio| audio.sample_rate());
            player.set_sample_rate(rate);
        }
        if self.settings.pad_mapping != old.pad_mapping {
            self.active_mapping = self.settings.pad_mapping;
        }
        if let Some(player) = &mut self.player {
            if self.settings.audio_quality != old.audio_quality {
                player.set_audio_quality(self.settings.audio_quality);
            }
            if self.settings.audio_filter_model() != old.audio_filter_model() {
                player.set_audio_filters(self.settings.audio_filter_model());
            }
            if self.settings.effective_channel_gains() != old.effective_channel_gains() {
                player.set_channel_gains(self.settings.effective_channel_gains());
            }
            if self.settings.supersample != old.supersample {
                player.set_supersample(self.settings.supersample);
            }
            if self.settings.speed_percent != old.speed_percent {
                player.set_speed(self.settings.speed_percent);
            }
            if self.settings.rewind_mb != old.rewind_mb {
                player.set_rewind_mb(self.settings.rewind_mb);
            }
            if self.settings.run_ahead != old.run_ahead {
                player.set_run_ahead(self.settings.run_ahead);
            }
            if self.settings.audio_source != old.audio_source {
                player.set_audio_source(self.settings.audio_source);
            }
            if self.settings.ram_clear != old.ram_clear {
                player.set_ram_clear(self.settings.ram_clear);
            }
            if self.settings.ppu_reset != old.ppu_reset {
                player.set_ppu_reset_behavior(self.settings.ppu_reset);
            }
        }
        if let Some(audio) = &self.audio {
            audio.set_volume(self.settings.volume);
            audio.set_muted(self.settings.muted);
            if self.settings.audio_latency_ms != old.audio_latency_ms {
                audio.set_latency_ms(self.settings.audio_latency_ms);
            }
        }
        apply_theme(ctx, self.settings.theme);
        if self.settings.rom_dir != old.rom_dir {
            self.scan();
        }
        let _ = self.settings.save(&Settings::path());
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.hotkey_capture {
            let key = ctx.input(|i| {
                i.events.iter().find_map(|event| {
                    if let egui::Event::Key {
                        key, pressed: true, ..
                    } = event
                    {
                        Some(*key)
                    } else {
                        None
                    }
                })
            });
            if let Some(key) = key {
                self.settings.hotkeys.set(action, key);
                self.hotkey_capture = None;
                let _ = self.settings.save(&Settings::path());
                self.set_status(format!("{} → {}", action.label(), key.name()));
            }
            return;
        }
        let (step_instruction, step_frame) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::F10),
                input.key_pressed(egui::Key::F9),
            )
        });
        if let Some(player) = &mut self.player {
            if step_instruction {
                player.apply_debug(Action::StepInstruction, ctx);
            }
            if step_frame {
                player.apply_debug(Action::StepFrame, ctx);
            }
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        let keys = self.settings.hotkeys;
        let (fast_forward, frame_advance) = ctx.input(|i| {
            (
                i.key_down(keys.fast_forward),
                i.key_pressed(keys.frame_advance),
            )
        });
        if let Some(player) = &mut self.player {
            player.set_fast_forward(fast_forward);
            if frame_advance {
                player.paused = true;
                player.advance_frame(ctx);
            }
        }
        let (save, load, fullscreen, pause, screenshot, reset) = ctx.input(|i| {
            (
                i.key_pressed(keys.save),
                i.key_pressed(keys.load),
                i.key_pressed(keys.fullscreen),
                i.key_pressed(keys.pause),
                i.key_pressed(keys.screenshot),
                i.key_pressed(keys.reset),
            )
        });
        if fullscreen {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if let Some(player) = &mut self.player {
            if save {
                player.save_state(player.slot);
            }
            if load {
                player.load_state(player.slot);
            }
            if pause {
                player.toggle_pause();
            }
            if screenshot {
                player.screenshot();
            }
            if reset {
                player.reset();
            }
        }
    }

    fn menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu-bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM...").clicked() {
                        self.open_rom_open = true;
                        ui.close_menu();
                    }
                    if ui.button("Set ROM folder...").clicked() {
                        self.tab = Tab::Paths;
                        self.settings_open = true;
                        ui.close_menu();
                    }
                    if ui.button("Rescan library").clicked() {
                        self.scan();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Emulation", |ui| {
                    let has_game = self.player.is_some();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Play / Pause"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.toggle_pause();
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Stop"))
                        .clicked()
                    {
                        self.stop_game();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Reset"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.reset();
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Frame advance (.)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.paused = true;
                            player.advance_frame(ctx);
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Rewind (Backspace)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.paused = true;
                            player.rewind_step(ctx);
                        }
                        ui.close_menu();
                    }
                    ui.menu_button("Rewind buffer", |ui| {
                        for budget in [64, 128, 256] {
                            if ui
                                .radio_value(
                                    &mut self.settings.rewind_mb,
                                    budget,
                                    format!("{budget} MiB"),
                                )
                                .changed()
                            {
                                if let Some(player) = &mut self.player {
                                    player.set_rewind_mb(budget);
                                }
                                let _ = self.settings.save(&Settings::path());
                            }
                        }
                    });
                    ui.menu_button("Run-ahead", |ui| {
                        for frames in 0..=4 {
                            let label = if frames == 0 {
                                "Off".to_string()
                            } else {
                                format!("{frames} frame{}", if frames == 1 { "" } else { "s" })
                            };
                            if ui
                                .radio_value(&mut self.settings.run_ahead, frames, label)
                                .changed()
                            {
                                if let Some(player) = &mut self.player {
                                    player.set_run_ahead(frames);
                                }
                                let _ = self.settings.save(&Settings::path());
                            }
                        }
                        ui.separator();
                        ui.label(
                            RichText::new(
                                "Runs extra frames to hide input latency; suspended at \
                                 non-100% speeds, during movies and while debugging.",
                            )
                            .weak()
                            .small(),
                        );
                    });
                    ui.menu_button("Movie", |ui| {
                        let has_game = self.player.is_some();
                        let recording = self
                            .player
                            .as_ref()
                            .is_some_and(|player| player.movie_recording());
                        let playing = self
                            .player
                            .as_ref()
                            .is_some_and(|player| player.movie_progress().is_some());
                        if ui
                            .add_enabled(
                                has_game && !recording && !playing,
                                egui::Button::new("Record movie"),
                            )
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                match player.start_movie_recording() {
                                    Ok(()) => self.set_status("Recording input movie"),
                                    Err(err) => self.set_status(format!("Movie: {err}")),
                                }
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(recording, egui::Button::new("Stop recording"))
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                let path = player
                                    .save_dir
                                    .join("recordings")
                                    .join(format!("{}.movie", player.stem));
                                match player.stop_movie_recording(&path) {
                                    Ok(()) => {
                                        self.set_status(format!(
                                            "Saved movie to {}",
                                            path.display()
                                        ));
                                    }
                                    Err(err) => self.set_status(format!("Movie: {err}")),
                                }
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                has_game && !recording && !playing,
                                egui::Button::new("Play movie..."),
                            )
                            .clicked()
                        {
                            let start = self
                                .player
                                .as_ref()
                                .map(|player| player.save_dir.join("recordings"));
                            let mut dialog = rfd::FileDialog::new()
                                .add_filter("Unko input movie", &["movie"])
                                .set_title("Play input movie");
                            if let Some(directory) = start {
                                dialog = dialog.set_directory(directory);
                            }
                            if let Some(path) = dialog.pick_file() {
                                match crate::movie::Movie::load(&path) {
                                    Ok(movie) => {
                                        if let Some(player) = &mut self.player {
                                            match player.start_movie_playback(movie) {
                                                Ok(()) => self.set_status(format!(
                                                    "Playing {}",
                                                    path.display()
                                                )),
                                                Err(err) => {
                                                    self.set_status(format!("Movie: {err}"));
                                                }
                                            }
                                        }
                                    }
                                    Err(err) => self.set_status(format!("Movie: {err}")),
                                }
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(playing, egui::Button::new("Stop playback"))
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                player.stop_movie_playback();
                            }
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Speed", |ui| {
                        for percent in [25, 50, 100, 150, 200, 300] {
                            if ui
                                .radio_value(
                                    &mut self.settings.speed_percent,
                                    percent,
                                    format!("{percent}%"),
                                )
                                .changed()
                            {
                                if let Some(player) = &mut self.player {
                                    player.set_speed(percent);
                                }
                                let _ = self.settings.save(&Settings::path());
                            }
                        }
                        ui.separator();
                        ui.label(RichText::new("Hold Tab to fast-forward").weak().small());
                    });
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Save state (F5)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            let slot = player.slot;
                            player.save_state(slot);
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Load state (F8)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            let slot = player.slot;
                            player.load_state(slot);
                        }
                        ui.close_menu();
                    }
                    let thumbs: Vec<Option<Vec<u8>>> = (0..10u8)
                        .map(|slot| self.player.as_ref().and_then(|p| p.thumbnail(slot)))
                        .collect();
                    let thumb_stem = self
                        .player
                        .as_ref()
                        .map(|p| p.stem.clone())
                        .unwrap_or_default();
                    ui.menu_button("State slot", |ui| {
                        for slot in 0..10u8 {
                            ui.horizontal(|ui| {
                                if let Some(bytes) = &thumbs[slot as usize] {
                                    if bytes.len() == 64 * 60 * 4 {
                                        let image = egui::ColorImage::from_rgba_unmultiplied(
                                            [64, 60],
                                            bytes,
                                        );
                                        let texture = ui.ctx().load_texture(
                                            format!("{thumb_stem}-thumb-{slot}"),
                                            image,
                                            egui::TextureOptions::NEAREST,
                                        );
                                        ui.image((texture.id(), egui::vec2(48.0, 45.0)));
                                    }
                                }
                                let selected = self.player.as_ref().is_some_and(|p| p.slot == slot);
                                if ui
                                    .add_enabled(
                                        has_game,
                                        egui::Button::new(format!("Slot {slot}"))
                                            .selected(selected),
                                    )
                                    .clicked()
                                {
                                    if let Some(player) = &mut self.player {
                                        player.slot = slot;
                                    }
                                    ui.close_menu();
                                }
                            });
                        }
                    });
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Debugger..."))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.debug.open = true;
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Step instruction (F10)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.apply_debug(Action::StepInstruction, ctx);
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Step frame (F9)"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.apply_debug(Action::StepFrame, ctx);
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Screenshot"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.screenshot();
                        }
                        ui.close_menu();
                    }
                    if ui.button("Fullscreen (F11)").clicked() {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .selectable_label(!self.settings.grid_view, "List")
                        .clicked()
                    {
                        self.settings.grid_view = false;
                        ui.close_menu();
                    }
                    if ui
                        .selectable_label(self.settings.grid_view, "Grid")
                        .clicked()
                    {
                        self.settings.grid_view = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .checkbox(&mut self.settings.show_status_bar, "Status bar")
                        .changed()
                    {
                        let _ = self.settings.save(&Settings::path());
                    }
                    if ui.button("Fullscreen (F11)").clicked() {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                        ui.close_menu();
                    }
                });
                ui.menu_button("Tools", |ui| {
                    for tab in [
                        Tab::Graphics,
                        Tab::Audio,
                        Tab::Controllers,
                        Tab::Nes,
                        Tab::Paths,
                    ] {
                        if ui.button(tab.label()).clicked() {
                            self.tab = tab;
                            self.settings_open = true;
                            ui.close_menu();
                        }
                    }
                    if ui.button("Devices").clicked() {
                        self.devices_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.player.is_some(), egui::Button::new("Cheats"))
                        .clicked()
                    {
                        self.cheats_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Recording", |ui| {
                        let has_game = self.player.is_some();
                        let recording = self.player.as_ref().is_some_and(|p| p.recording());
                        if ui
                            .add_enabled(has_game && !recording, egui::Button::new("Record WAV"))
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: true,
                                png: false,
                                gif: false,
                            });
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                has_game && !recording,
                                egui::Button::new("Record PNG frames"),
                            )
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: false,
                                png: true,
                                gif: false,
                            });
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(has_game && !recording, egui::Button::new("Record GIF"))
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: false,
                                png: false,
                                gif: true,
                            });
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(
                                has_game && !recording,
                                egui::Button::new("Record WAV + PNG + GIF"),
                            )
                            .clicked()
                        {
                            self.start_recording(RecordModes {
                                wav: true,
                                png: true,
                                gif: true,
                            });
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(recording, egui::Button::new("Stop recording"))
                            .clicked()
                        {
                            if let Some(player) = &mut self.player {
                                let frames = player.recording_frames();
                                player.stop_recording();
                                self.set_status(format!("Recording stopped after {frames} frames"));
                            }
                            ui.close_menu();
                        }
                    });
                    ui.separator();
                    if ui.button("Settings...").clicked() {
                        self.settings_open = true;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About Unko").clicked() {
                        self.about_open = true;
                        ui.close_menu();
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("Unko").weak());
                });
            });
        });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(38.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui.button("Open").clicked() {
                        self.open_rom_open = true;
                    }
                    if ui.button("Refresh").clicked() {
                        self.scan();
                    }
                    ui.separator();
                    let has_game = self.player.is_some();
                    let paused = self.player.as_ref().is_some_and(|p| p.paused);
                    if ui
                        .add_enabled(
                            has_game,
                            egui::Button::new(if paused { "Resume" } else { "Pause" }),
                        )
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.toggle_pause();
                        }
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Stop"))
                        .clicked()
                    {
                        self.stop_game();
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Reset"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.reset();
                        }
                    }
                    ui.separator();
                    if ui
                        .add_enabled(has_game, egui::Button::new("Fullscreen"))
                        .clicked()
                    {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                    }
                    if ui
                        .add_enabled(has_game, egui::Button::new("Screenshot"))
                        .clicked()
                    {
                        if let Some(player) = &mut self.player {
                            player.screenshot();
                        }
                    }
                    ui.separator();
                    if ui.button("Config").clicked() {
                        self.settings_open = true;
                    }
                    if ui.button("Graphics").clicked() {
                        self.tab = Tab::Graphics;
                        self.settings_open = true;
                    }
                    if ui.button("Controllers").clicked() {
                        self.tab = Tab::Controllers;
                        self.settings_open = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(player) = &self.player {
                            ui.label(RichText::new(&player.title).strong());
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status-bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                match &self.player {
                    Some(player) => {
                        let state = if player.paused { "Paused" } else { "Running" };
                        ui.label(RichText::new(state).strong());
                        ui.separator();
                        ui.label(&player.title);
                        ui.separator();
                        let (width, height) = player.resolution();
                        ui.label(format!("{width}×{height}"));
                        ui.separator();
                        ui.label(format!("{:.1} fps", player.fps));
                        ui.separator();
                        let speed = if player.fast_forward() {
                            "FF".to_string()
                        } else {
                            format!("{}%", player.speed_percent())
                        };
                        ui.label(speed);
                        ui.separator();
                        if player.rewind_available() > 0 {
                            ui.label(format!("rewind {}", player.rewind_available()));
                            ui.separator();
                        }
                        ui.label(format!("slot {}", player.slot));
                        ui.separator();
                        ui.label(format!("{:.0} Hz", player.sample_rate()));
                        if player.recording() {
                            ui.separator();
                            ui.label(
                                RichText::new(format!("REC {}", player.recording_frames()))
                                    .color(Color32::from_rgb(0xE0, 0x60, 0x60)),
                            );
                        }
                        if player.movie_recording() {
                            ui.separator();
                            let frames = player.movie_recorded_frames();
                            ui.label(
                                RichText::new(format!("MOVIE REC {frames}"))
                                    .color(Color32::from_rgb(0xE0, 0x60, 0x60)),
                            );
                        }
                        if let Some((index, total)) = player.movie_progress() {
                            ui.separator();
                            ui.label(
                                RichText::new(format!("MOVIE {index}/{total}"))
                                    .color(Color32::from_rgb(0x60, 0xC0, 0xE0)),
                            );
                        }
                    }
                    None => {
                        ui.label("No game loaded");
                        ui.separator();
                        ui.label(format!("{} ROMs", self.library.len()));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.status.is_empty()
                        && self.status_until.is_none_or(|until| Instant::now() < until)
                    {
                        ui.label(RichText::new(&self.status).weak());
                    }
                });
            });
        });
    }

    fn library(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            let before = (self.settings.grid_columns, self.settings.list_columns);
            ui.horizontal(|ui| {
                ui.heading("Library");
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search games...")
                        .desired_width(240.0),
                );
                if !self.search.is_empty() && ui.button("Clear").clicked() {
                    self.search.clear();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.settings.grid_view {
                        "List view"
                    } else {
                        "Grid view"
                    };
                    if ui.button(label).clicked() {
                        self.settings.grid_view = !self.settings.grid_view;
                        let _ = self.settings.save(&Settings::path());
                    }
                    ui.menu_button("Columns", |ui| {
                        ui.label("Grid columns (1-8)");
                        ui.add(egui::Slider::new(&mut self.settings.grid_columns, 1..=8));
                        ui.separator();
                        ui.label("List columns");
                        let mut mask = self.settings.list_columns;
                        for (name, bit) in LIST_COLUMN_BITS {
                            let mut shown = mask & bit != 0;
                            if ui.checkbox(&mut shown, name).changed() {
                                if shown {
                                    mask |= bit;
                                } else {
                                    mask &= !bit;
                                }
                            }
                        }
                        self.settings.list_columns = mask;
                    });
                    if ui.button("Rescan").clicked() {
                        self.scan();
                    }
                });
            });
            if before != (self.settings.grid_columns, self.settings.list_columns) {
                let _ = self.settings.save(&Settings::path());
            }
            ui.separator();

            let indices = self.filtered();
            if indices.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(
                        RichText::new("No games found")
                            .size(18.0)
                            .color(Color32::GRAY),
                    );
                    ui.add_space(8.0);
                    ui.label(format!(
                        "Put .nes files in {} or change Paths in Settings.",
                        self.settings.rom_dir.display()
                    ));
                });
                return;
            }

            if self.settings.grid_view {
                self.grid(ui, &indices);
            } else {
                self.table(ui, &indices);
            }
        });
    }

    fn filtered(&self) -> Vec<usize> {
        let query = self.search.trim().to_lowercase();
        self.library
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty()
                    || entry.title.to_lowercase().contains(&query)
                    || entry.file_name.to_lowercase().contains(&query)
                    || entry.mapper_label().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn table(&mut self, ui: &mut egui::Ui, indices: &[usize]) {
        const COLUMNS: [(&str, u8, f32); 6] = [
            ("System", 0x01, 0.9),
            ("File", 0x02, 2.0),
            ("Size", 0x04, 0.9),
            ("Region", 0x08, 0.9),
            ("Mapper", 0x10, 1.0),
            ("Modified", 0x20, 1.4),
        ];
        let width = ui.available_width();
        let title_width = (width * 0.28).clamp(180.0, 360.0);
        let visible: Vec<(&str, u8, f32)> = COLUMNS
            .iter()
            .copied()
            .filter(|(_, bit, _)| self.settings.list_columns & bit != 0)
            .collect();
        let total_weight: f32 = visible.iter().map(|(_, _, weight)| weight).sum();
        let usable = (width - title_width - 28.0).max(120.0);
        let widths: Vec<f32> = visible
            .iter()
            .map(|(_, _, weight)| usable * weight / total_weight.max(0.001))
            .collect();

        let header = ui
            .allocate_exact_size(Vec2::new(width, 22.0), Sense::hover())
            .0;
        let painter = ui.painter();
        let mut x = header.left() + 8.0;
        painter.text(
            egui::pos2(x, header.center().y),
            Align2::LEFT_CENTER,
            "Title",
            FontId::proportional(12.0),
            Color32::GRAY,
        );
        x += title_width;
        for ((name, _, _), column_width) in visible.iter().zip(&widths) {
            painter.text(
                egui::pos2(x, header.center().y),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(12.0),
                Color32::GRAY,
            );
            x += column_width;
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for &index in indices {
                    let entry = &self.library[index];
                    let selected = self.selected == Some(index);
                    let row_height = 24.0;
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), row_height),
                        Sense::click(),
                    );
                    if response.clicked() {
                        self.selected = Some(index);
                    }
                    if response.double_clicked() {
                        self.pending_play = Some(entry.clone());
                    }
                    response.context_menu(|ui| {
                        if ui.button("Play").clicked() {
                            self.pending_play = Some(entry.clone());
                            ui.close_menu();
                        }
                        if ui.button("Copy path").clicked() {
                            ui.ctx().copy_text(entry.source.display_path());
                            ui.close_menu();
                        }
                    });

                    let bg = if selected {
                        ui.visuals().selection.bg_fill.gamma_multiply(0.55)
                    } else if response.hovered() {
                        ui.visuals().widgets.hovered.bg_fill
                    } else {
                        Color32::TRANSPARENT
                    };
                    if bg != Color32::TRANSPARENT {
                        ui.painter().rect_filled(rect, 2.0, bg);
                    }

                    let painter = ui.painter().with_clip_rect(rect);
                    let y = rect.center().y;
                    let text_color = if selected {
                        ui.visuals().strong_text_color()
                    } else {
                        ui.visuals().text_color()
                    };
                    let mut x = rect.left() + 8.0;
                    painter.text(
                        egui::pos2(x, y),
                        Align2::LEFT_CENTER,
                        &entry.title,
                        FontId::proportional(13.0),
                        text_color,
                    );
                    x += title_width;
                    for ((_, bit, _), column_width) in visible.iter().zip(&widths) {
                        painter.text(
                            egui::pos2(x, y),
                            Align2::LEFT_CENTER,
                            column_cell(entry, *bit),
                            FontId::proportional(12.0),
                            Color32::from_gray(170),
                        );
                        x += column_width;
                    }
                }
            });
    }

    fn grid(&mut self, ui: &mut egui::Ui, indices: &[usize]) {
        let columns = self.settings.grid_columns.clamp(1, 8) as usize;
        let gap = 8.0;
        let available = ui.available_width();
        let cell_width =
            ((available - gap * (columns.saturating_sub(1)) as f32) / columns as f32).max(80.0);
        let cell_height = (cell_width * 0.52).clamp(64.0, 140.0);
        ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for &index in indices {
                        let entry = &self.library[index];
                        let selected = self.selected == Some(index);
                        let size = Vec2::new(cell_width, cell_height);
                        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                        if response.clicked() {
                            self.selected = Some(index);
                        }
                        if response.double_clicked() {
                            self.pending_play = Some(entry.clone());
                        }
                        response.context_menu(|ui| {
                            if ui.button("Play").clicked() {
                                self.pending_play = Some(entry.clone());
                                ui.close_menu();
                            }
                        });
                        let bg = if selected {
                            ui.visuals().selection.bg_fill.gamma_multiply(0.55)
                        } else if response.hovered() {
                            ui.visuals().widgets.hovered.bg_fill
                        } else {
                            ui.visuals().extreme_bg_color
                        };
                        ui.painter().rect_filled(rect, 4.0, bg);
                        ui.painter().rect_stroke(
                            rect,
                            4.0,
                            Stroke::new(1.0_f32, Color32::from_gray(70)),
                            egui::StrokeKind::Inside,
                        );
                        let painter = ui.painter().with_clip_rect(rect.shrink(8.0));
                        painter.text(
                            rect.left_top() + Vec2::new(10.0, 12.0),
                            Align2::LEFT_TOP,
                            &entry.title,
                            FontId::proportional(13.0),
                            ui.visuals().strong_text_color(),
                        );
                        painter.text(
                            rect.left_bottom() + Vec2::new(10.0, -10.0),
                            Align2::LEFT_BOTTOM,
                            format!(
                                "{} · {} · {}",
                                entry.system(),
                                entry.mapper_label(),
                                entry.region_label()
                            ),
                            FontId::proportional(10.0),
                            Color32::from_gray(160),
                        );
                    }
                });
            });
    }

    fn detached_windows(&self) -> bool {
        self.settings.detachable_windows && !cfg!(any(target_os = "android", target_os = "ios"))
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        if self.detached_windows() {
            let id = egui::ViewportId::from_hash_of(("unko-settings", self.window_generation));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("Settings — Unko")
                .with_min_inner_size([520.0, 360.0])
                .with_resizable(true);
            builder = if let Some([x, y, width, height]) = self.settings.layout_settings {
                builder
                    .with_position([x, y])
                    .with_inner_size([width, height])
            } else {
                builder.with_inner_size([780.0, 540.0])
            };
            let mut closed = false;
            let mut layout = None;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                self.settings_frame(ctx, &mut open, false);
                let info = ctx.input(|input| input.viewport().clone());
                if let Some(outer) = info.outer_rect {
                    let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                    layout = Some([outer.min.x, outer.min.y, size.x, size.y]);
                }
                if ctx.input(|input| input.viewport().close_requested()) {
                    closed = true;
                }
            });
            if let Some(layout) = layout {
                self.settings.layout_settings = Some(layout);
            }
            if closed {
                open = false;
                let _ = self.settings.save(&Settings::path());
            }
        } else {
            self.settings_frame(ctx, &mut open, true);
        }
        self.settings_open = open;
    }

    fn settings_frame(&mut self, ctx: &egui::Context, open: &mut bool, embedded: bool) {
        let mut settings = self.settings.clone();
        let mut tab = self.tab;
        let selected_entry = self
            .selected
            .and_then(|index| self.library.get(index))
            .cloned();
        let mut gamepads = self.gamepads.take();
        let gamepad_error = self.gamepad_error.clone();
        let mut hotkey_capture = self.hotkey_capture.take();
        let mut reset_requested = false;
        let mut layout_reset = false;

        let mut body = |ui: &mut egui::Ui| {
            ui.horizontal_top(|ui| {
                ui.set_min_width(140.0);
                ui.vertical(|ui| {
                    for entry in Tab::ALL {
                        if ui.selectable_label(tab == entry, entry.label()).clicked() {
                            tab = entry;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_width(560.0);
                    match tab {
                        Tab::General => {
                            reset_requested =
                                general_tab(ui, &mut settings, ctx, &mut layout_reset);
                        }
                        Tab::Interface => interface_tab(ui, &mut settings),
                        Tab::Graphics => graphics_tab(ui, &mut settings, ctx),
                        Tab::Audio => audio_tab(
                            ui,
                            &mut settings,
                            self.audio.as_ref(),
                            self.audio_error.as_deref(),
                        ),
                        Tab::Controllers => controllers_tab(
                            ui,
                            &mut settings,
                            gamepads.as_mut(),
                            gamepad_error.as_deref(),
                            &mut hotkey_capture,
                        ),
                        Tab::Paths => paths_tab(ui, &mut settings),
                        Tab::Nes => nes_tab(ui, &mut settings, selected_entry.as_ref()),
                        Tab::Advanced => advanced_tab(ui, &mut settings),
                    }
                });
            });
        };

        if embedded {
            egui::Window::new("Settings")
                .open(open)
                .default_width(760.0)
                .default_height(480.0)
                .resizable(true)
                .show(ctx, |ui| body(ui));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(6.0);
                body(ui);
            });
        }

        self.gamepads = gamepads;
        self.hotkey_capture = hotkey_capture;
        self.tab = tab;
        if settings != self.settings {
            let old = self.settings.clone();
            self.settings = settings;
            self.apply_settings(ctx, &old);
        }
        if layout_reset {
            self.window_generation = self.window_generation.wrapping_add(1);
        }
        if reset_requested {
            self.confirm_reset = true;
        }
    }

    fn cheats_window(&mut self, ctx: &egui::Context) {
        if !self.cheats_open {
            return;
        }
        let mut open = self.cheats_open;
        if self.detached_windows() {
            let id = egui::ViewportId::from_hash_of(("unko-cheats", self.window_generation));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("Cheats — Unko")
                .with_min_inner_size([560.0, 360.0])
                .with_resizable(true);
            builder = if let Some([x, y, width, height]) = self.settings.layout_cheats {
                builder
                    .with_position([x, y])
                    .with_inner_size([width, height])
            } else {
                builder.with_inner_size([700.0, 480.0])
            };
            let mut closed = false;
            let mut layout = None;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                self.cheats_frame(ctx, false);
                let info = ctx.input(|input| input.viewport().clone());
                if let Some(outer) = info.outer_rect {
                    let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                    layout = Some([outer.min.x, outer.min.y, size.x, size.y]);
                }
                if ctx.input(|input| input.viewport().close_requested()) {
                    closed = true;
                }
            });
            if let Some(layout) = layout {
                self.settings.layout_cheats = Some(layout);
            }
            if closed {
                open = false;
                let _ = self.settings.save(&Settings::path());
            }
        } else {
            self.cheats_frame(ctx, true);
        }
        self.cheats_open = open;
    }

    fn cheats_frame(&mut self, ctx: &egui::Context, embedded: bool) {
        if self.player.is_none() {
            if embedded {
                egui::Window::new("Cheats").show(ctx, |ui| {
                    ui.label("Load a game to edit its cheats.");
                });
            }
            return;
        }
        let mut entries = self.cheats.clone();
        let mut code = self.cheat_code.clone();
        let mut address = self.cheat_addr.clone();
        let mut value = self.cheat_value.clone();
        let mut error = self.cheat_error.clone();
        let mut add_genie = false;
        let mut add_ram = false;
        let mut changed = false;

        let mut body = |ui: &mut egui::Ui| {
            ui.heading("Cheats");
            ui.label(
                RichText::new(
                    "Game Genie codes patch PRG reads; raw RAM writes apply before each frame. \
                     Up to 32 entries, stored per game beside the save states.",
                )
                .weak()
                .small(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Game Genie");
                ui.add(
                    egui::TextEdit::singleline(&mut code)
                        .hint_text("SXIOPO")
                        .desired_width(140.0),
                );
                if ui
                    .add_enabled(entries.len() < 32, egui::Button::new("Add"))
                    .clicked()
                {
                    add_genie = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Raw RAM");
                ui.add(
                    egui::TextEdit::singleline(&mut address)
                        .hint_text("0010")
                        .desired_width(70.0),
                );
                ui.label("=");
                ui.add(
                    egui::TextEdit::singleline(&mut value)
                        .hint_text("99")
                        .desired_width(50.0),
                );
                if ui
                    .add_enabled(entries.len() < 32, egui::Button::new("Add"))
                    .clicked()
                {
                    add_ram = true;
                }
            });
            if let Some(error) = &error {
                ui.colored_label(Color32::from_rgb(0xE0, 0x80, 0x60), error);
            }
            ui.separator();
            if entries.is_empty() {
                ui.label(RichText::new("No cheats for this game.").weak());
            }
            let mut remove = None;
            for (index, entry) in entries.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut entry.enabled, "").changed() {
                        changed = true;
                    }
                    ui.label(entry.label());
                    if ui.button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                entries.remove(index);
                changed = true;
            }
        };

        if embedded {
            egui::Window::new("Cheats")
                .default_width(680.0)
                .default_height(420.0)
                .resizable(true)
                .show(ctx, |ui| body(ui));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(6.0);
                body(ui);
            });
        }

        if add_genie {
            if entries.len() >= 32 {
                error = Some("32 cheats maximum".into());
            } else {
                match nes_accuracy_core::GameGenie::parse(&code) {
                    Ok(decoded) => {
                        entries.push(CheatEntry {
                            cheat: unko_core::Cheat::GameGenie {
                                code: decoded.code().to_string(),
                                address: decoded.address(),
                                value: decoded.value(),
                                compare: decoded.compare(),
                            },
                            enabled: true,
                        });
                        code.clear();
                        error = None;
                        changed = true;
                    }
                    Err(err) => error = Some(err.to_string()),
                }
            }
        }
        if add_ram {
            if entries.len() >= 32 {
                error = Some("32 cheats maximum".into());
            } else {
                let parse_address =
                    u16::from_str_radix(address.trim().trim_start_matches("0x"), 16);
                let parse_value = u8::from_str_radix(value.trim().trim_start_matches("0x"), 16);
                match (parse_address, parse_value) {
                    (Ok(addr), Ok(val)) if addr < 0x2000 => {
                        entries.push(CheatEntry {
                            cheat: unko_core::Cheat::Ram {
                                address: addr,
                                value: val,
                            },
                            enabled: true,
                        });
                        address.clear();
                        value.clear();
                        error = None;
                        changed = true;
                    }
                    (Ok(_), Ok(_)) => {
                        error = Some("Raw RAM cheats use $0000-$07FF".into());
                    }
                    _ => error = Some("Address and value must be hexadecimal".into()),
                }
            }
        }

        self.cheats = entries;
        self.cheat_code = code;
        self.cheat_addr = address;
        self.cheat_value = value;
        self.cheat_error = error;
        if changed {
            self.apply_cheats();
            self.save_cheats();
        }
    }

    fn devices_window(&mut self, ctx: &egui::Context) {
        if !self.devices_open {
            return;
        }
        let mut open = self.devices_open;
        if self.detached_windows() {
            let id = egui::ViewportId::from_hash_of(("unko-devices", self.window_generation));
            let mut builder = egui::ViewportBuilder::default()
                .with_title("Devices — Unko")
                .with_min_inner_size([420.0, 320.0])
                .with_resizable(true);
            builder = if let Some([x, y, width, height]) = self.settings.layout_devices {
                builder
                    .with_position([x, y])
                    .with_inner_size([width, height])
            } else {
                builder.with_inner_size([680.0, 520.0])
            };
            let mut closed = false;
            let mut layout = None;
            ctx.show_viewport_immediate(id, builder, |ctx, _class| {
                self.devices_frame(ctx, false);
                let info = ctx.input(|input| input.viewport().clone());
                if let Some(outer) = info.outer_rect {
                    let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                    layout = Some([outer.min.x, outer.min.y, size.x, size.y]);
                }
                if ctx.input(|input| input.viewport().close_requested()) {
                    closed = true;
                }
            });
            if let Some(layout) = layout {
                self.settings.layout_devices = Some(layout);
            }
            if closed {
                open = false;
                let _ = self.settings.save(&Settings::path());
            }
        } else {
            self.devices_frame(ctx, true);
        }
        self.devices_open = open;
    }

    fn devices_frame(&mut self, ctx: &egui::Context, embedded: bool) {
        let names = self.gamepads.as_ref().map(Gamepads::names);
        let connected = self.gamepads.as_ref().map_or(0, Gamepads::connected);
        let gamepad_error = self.gamepad_error.clone();
        let mut settings = self.settings.clone();
        let mut microphone = self.microphone;
        let mut datach_code = self.datach_code.clone();
        let mut datach_scan = false;
        let has_player = self.player.is_some();
        let mut body = |ui: &mut egui::Ui| {
            ui.heading("NES ports");
            ui.label(
                RichText::new(
                    "The two physical ports carry players 1/2, or players 1-4 \
                     with a Four Score adapter. A disabled port ignores \
                     keyboard, touch and gamepad input.",
                )
                .weak()
                .small(),
            );
            for port in 0..2 {
                let device = if port == 0 {
                    settings.port1_device
                } else {
                    settings.port2_device
                };
                ui.horizontal(|ui| {
                    ui.checkbox(&mut settings.port_disabled[port], "Disabled");
                    ui.label(format!("Port {}: {}", port + 1, device_label(device)));
                });
            }
            if settings.port1_device == PortDevice::Microphone
                || settings.port2_device == PortDevice::Microphone
            {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut microphone,
                        "Microphone input (hold while making noise)",
                    );
                });
            }
            ui.add_space(10.0);
            ui.separator();
            ui.heading("Gamepads");
            match &names {
                Some(names) => {
                    egui::Grid::new("devices-pads")
                        .num_columns(3)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Pad");
                            ui.label("NES player");
                            ui.label("Disabled");
                            ui.end_row();
                            for (slot, name) in names.iter().enumerate() {
                                ui.label(match name {
                                    Some(name) => format!("{}: {name}", slot + 1),
                                    None => format!("{}: (not connected)", slot + 1),
                                });
                                egui::ComboBox::from_id_salt(("pad-port", slot))
                                    .selected_text(format!(
                                        "Player {}",
                                        settings.pad_ports[slot] + 1
                                    ))
                                    .show_ui(ui, |ui| {
                                        for port in 0..4u8 {
                                            ui.selectable_value(
                                                &mut settings.pad_ports[slot],
                                                port,
                                                format!("Player {}", port + 1),
                                            );
                                        }
                                    });
                                ui.checkbox(&mut settings.pad_disabled[slot], "");
                                ui.end_row();
                            }
                        });
                    ui.label(
                        RichText::new(format!(
                            "{connected} connected. Two pads can drive the same \
                             player; their buttons are merged."
                        ))
                        .weak()
                        .small(),
                    );
                }
                None => {
                    ui.colored_label(
                        Color32::from_rgb(0xE0, 0x80, 0x60),
                        gamepad_error
                            .as_deref()
                            .unwrap_or("SDL3 gamepad backend unavailable"),
                    );
                    ui.label("Keyboard and the touch overlay still work.");
                }
            }
            ui.add_space(10.0);
            ui.separator();
            ui.heading("Sources");
            ui.label("Keyboard and touch: player 1 (unless port 1 is disabled).");
            ui.label("Gamepads: routed through the matrix above.");
            ui.label("Zapper, paddle, Power Pad and Family BASIC use the port's device.");
            ui.add_space(10.0);
            ui.separator();
            ui.heading("Datach barcode");
            ui.label(
                RichText::new(
                    "Mapper 157 only. Enter a 12-digit UPC-A or 13-digit EAN-13 code; the \
                     scanner returns it as the documented bar pattern.",
                )
                .weak()
                .small(),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut datach_code)
                        .hint_text("012345678905")
                        .desired_width(190.0),
                );
                if ui
                    .add_enabled(has_player, egui::Button::new("Scan"))
                    .clicked()
                {
                    datach_scan = true;
                }
            });
            if settings.port1_device == PortDevice::MiraclePiano
                || settings.port2_device == PortDevice::MiraclePiano
            {
                ui.label(
                    RichText::new(
                        "Miracle Piano: the MIDI transport is modelled; received bytes are \
                         injected by tests because the piano firmware is not available.",
                    )
                    .weak()
                    .small(),
                );
            }
        };

        if embedded {
            egui::Window::new("Devices")
                .default_width(660.0)
                .resizable(true)
                .show(ctx, |ui| body(ui));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(6.0);
                body(ui);
            });
        }
        self.microphone = microphone;
        self.datach_code = datach_code;
        if datach_scan {
            let result = self
                .player
                .as_mut()
                .map(|player| player.set_datach_barcode(&self.datach_code));
            match result {
                Some(true) => self.set_status("Datach barcode scanned"),
                Some(false) => self.set_status("Barcode invalid or not a Datach cartridge"),
                None => {}
            }
        }
        if settings != self.settings {
            let old = self.settings.clone();
            self.settings = settings;
            self.apply_settings(ctx, &old);
        }
    }

    fn reset_confirmation(&mut self, ctx: &egui::Context) {
        if !self.confirm_reset {
            return;
        }
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Reset all settings?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    "This restores configuration, window layout, hotkeys, gamepad \
                     bindings, per-port devices and per-game input profiles, and \
                     removes saved state thumbnails.",
                );
                ui.label(
                    RichText::new("Save states, SRAM and recordings are never deleted.").strong(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                });
            });
        if confirmed {
            self.confirm_reset = false;
            self.reset_all_settings(ctx);
        } else if !open {
            self.confirm_reset = false;
        }
    }

    fn reset_all_settings(&mut self, ctx: &egui::Context) {
        let old = self.settings.clone();
        // Per-game profiles and state thumbnails are regenerable; state files,
        // SRAM and recordings are not touched.
        if let Ok(entries) = std::fs::read_dir(&old.save_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let extension = path.extension().and_then(|value| value.to_str());
                if matches!(extension, Some("pad" | "thumb")) {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        self.settings = Settings::default();
        self.active_mapping = self.settings.pad_mapping;
        self.tab = Tab::General;
        self.window_generation = self.window_generation.wrapping_add(1);
        self.apply_settings(ctx, &old);
        self.scan();
        self.set_status("Reset all settings to defaults");
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = self.about_open;
        egui::Window::new("About Unko")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Unko");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.label("A multi-system, multi-core emulator shell.");
                ui.label("MIT licensed. NES backend: Unko's accuracy core.");
                ui.add_space(6.0);
                ui.label("Frontend structure inspired by Dolphin's desktop UX;");
                ui.label("no Dolphin code, icons or assets are used.");
                ui.add_space(6.0);
                ui.label("No ROMs are included.");
            });
        self.about_open = open;
    }

    fn open_rom_window(&mut self, ctx: &egui::Context) {
        if !self.open_rom_open {
            return;
        }
        let mut open = self.open_rom_open;
        egui::Window::new("Open ROM")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Path to a ROM or archive (.nes, .zip, .7z, .rar):");
                ui.add(
                    egui::TextEdit::singleline(&mut self.open_rom_text)
                        .desired_width(420.0)
                        .hint_text("/path/to/game.nes or /path/to/game.zip"),
                );
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        let path = PathBuf::from(self.open_rom_text.trim());
                        if path.is_file() {
                            self.start_game(&path);
                            self.open_rom_open = false;
                        } else {
                            self.set_status(format!("Not a file: {}", path.display()));
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.open_rom_open = false;
                    }
                });
                ui.label(
                    RichText::new("Native file dialogs land when the desktop portal is available.")
                        .weak()
                        .small(),
                );
            });
        self.open_rom_open = open;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shortcuts(ctx);
        self.menu(ctx);
        self.toolbar(ctx);
        if self.settings.show_status_bar {
            self.status_bar(ctx);
        }

        // Save power when the window is inactive, without overwriting the
        // user's own pause state.
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
        if self.settings.pause_on_focus_loss && !focused {
            if let Some(player) = &mut self.player {
                if !player.paused {
                    player.toggle_pause();
                    self.paused_by_focus = true;
                }
            }
        } else if self.paused_by_focus {
            if let Some(player) = &mut self.player {
                if player.paused {
                    player.toggle_pause();
                }
            }
            self.paused_by_focus = false;
        }

        if self.settings.gamepad_enabled {
            if let Some(gamepads) = &mut self.gamepads {
                let polled = gamepads.poll(
                    self.active_mapping,
                    self.settings.pad_deadzone,
                    self.settings.turbo_enabled,
                    self.settings.turbo_hz,
                );
                let mut routed = [unko_core::InputState::empty(); 4];
                for (slot, state) in polled.iter().enumerate() {
                    if self.settings.pad_disabled[slot] {
                        continue;
                    }
                    let port = usize::from(self.settings.pad_ports[slot].min(3));
                    routed[port] =
                        unko_core::InputState::from_bits(routed[port].bits() | state.bits());
                }
                self.input_states = routed;
            }
        } else {
            self.input_states = [unko_core::InputState::empty(); 4];
        }
        if let Some(audio) = &self.audio {
            let running = self.player.as_ref().is_some_and(|player| {
                !player.paused && player.speed_percent() == 100 && !player.fast_forward()
            });
            audio.set_active(running && !self.settings.muted);
        }
        let detached_windows = self.detached_windows();
        let window_generation = self.window_generation;
        let port_disabled = self.settings.port_disabled;
        let audio = self.audio.as_mut();
        if let Some(player) = &mut self.player {
            for (port, state) in self.input_states.iter().enumerate() {
                player.set_gamepad(port, *state);
            }
            // The two physical ports gate players 1/2 and, for a Four Score,
            // the extra players 3/4 that ride the same ports.
            player.set_port_enabled(0, !port_disabled[0]);
            player.set_port_enabled(1, !port_disabled[1]);
            player.set_port_enabled(2, !port_disabled[0]);
            player.set_port_enabled(3, !port_disabled[1]);
            player.set_microphone(self.microphone);
            player.step(ctx, audio);
        }

        if let Some(player) = &mut self.player {
            let port1 = self.settings.port1_device;
            let port2 = self.settings.port2_device;
            // The Four Score adapter occupies both ports.
            let (device1, device2) =
                if port1 == PortDevice::FourScore || port2 == PortDevice::FourScore {
                    (PortDevice::FourScore, PortDevice::FourScore)
                } else {
                    (port1, port2)
                };
            player.set_port_devices(device1, device2);
            let zapper_port = if port1 == PortDevice::Zapper {
                Some(0)
            } else if port2 == PortDevice::Zapper {
                Some(1)
            } else {
                None
            };
            let paddle_port = if port1 == PortDevice::Paddle {
                Some(0)
            } else if port2 == PortDevice::Paddle {
                Some(1)
            } else {
                None
            };
            let power_pad_port =
                if port1 == PortDevice::PowerPad || port1 == PortDevice::FamilyTrainer {
                    Some(0)
                } else if port2 == PortDevice::PowerPad || port2 == PortDevice::FamilyTrainer {
                    Some(1)
                } else {
                    None
                };
            if port1 == PortDevice::FamilyKeyboard || port2 == PortDevice::FamilyKeyboard {
                player.set_family_keyboard(family_keyboard_matrix(ctx));
            }
            if port1 == PortDevice::SuborKeyboard || port2 == PortDevice::SuborKeyboard {
                player.set_subor_keyboard(subor_keyboard_matrix(ctx));
            }
            let view = viewport(
                ctx,
                player,
                &ViewportOptions {
                    integer_scale: self.settings.integer_scale,
                    linear_filter: self.settings.linear_filter,
                    aspect: self.settings.aspect,
                    scale: self.settings.scale,
                    supersample: self.settings.supersample,
                    shader: self.settings.shader,
                    brightness: self.settings.brightness,
                    sharpness: self.settings.sharpness,
                    overscan: self.settings.overscan,
                    mask_edges: self.settings.mask_edges,
                    zapper_port,
                    paddle_port,
                    power_pad_port,
                    input: self.input_states,
                    show_input: self.settings.show_input_display,
                    show_touch: self.settings.show_touch_controls,
                },
                #[cfg(not(target_os = "ios"))]
                self.gpu_shader.as_ref(),
            );
            player.set_touch(view.touch);
            if let (Some(port), Some((position, fire))) = (paddle_port, view.paddle) {
                player.set_paddle(port, position, fire);
            }
            if let (Some(port), Some(mask)) = (power_pad_port, view.power_pad) {
                player.set_power_pad(port, mask);
            }
            if player.debug.open {
                if let Some(view) = player.debug_view() {
                    let mut layout = self.settings.layout_debugger;
                    let actions = debugger::window(
                        &mut player.debug,
                        ctx,
                        &view,
                        detached_windows,
                        window_generation,
                        &mut layout,
                    );
                    self.settings.layout_debugger = layout;
                    for action in actions {
                        player.apply_debug(action, ctx);
                    }
                }
            }
        } else {
            self.library(ctx);
        }

        let desired_title = match &self.player {
            Some(player) => format!("Unko — {}", player.title),
            None => "Unko".to_string(),
        };
        if desired_title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(desired_title.clone()));
            self.window_title = desired_title;
        }

        self.settings_window(ctx);
        self.devices_window(ctx);
        self.cheats_window(ctx);
        self.about_window(ctx);
        self.open_rom_window(ctx);
        self.reset_confirmation(ctx);

        if let Some(entry) = self.pending_play.take() {
            self.start_entry(entry);
        }

        if let Some(player) = &self.player {
            ctx.request_repaint_after(player.next_repaint());
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    #[cfg(not(target_os = "ios"))]
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.shutdown();
        if let (Some(gl), Some(shader)) = (gl, &self.gpu_shader) {
            if let Ok(mut shader) = shader.lock() {
                shader.destroy(gl);
            }
        }
    }

    #[cfg(target_os = "ios")]
    fn on_exit(&mut self) {
        self.shutdown();
    }
}

impl App {
    /// Stops the audio stream and persists settings when the shell exits.
    fn shutdown(&mut self) {
        if let Some(player) = &mut self.player {
            player.shutdown();
        }
        let _ = self.settings.save(&Settings::path());
    }
}

fn scan(dir: &std::path::Path) -> Vec<GameEntry> {
    if dir.is_dir() {
        library::scan(dir)
    } else {
        Vec::new()
    }
}

fn resolve_first(path: &Path) -> Option<GameEntry> {
    library::probe_path(path).ok()?.into_iter().next()
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    ctx.set_visuals(match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    });
}

fn modified_label(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return "?".into();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| value.as_secs());
    let ago = now.saturating_sub(seconds);
    if ago < 3600 {
        format!("{}m ago", ago / 60)
    } else if ago < 86_400 {
        format!("{}h ago", ago / 3600)
    } else {
        format!("{}d ago", ago / 86_400)
    }
}

fn fit_aspect(available: Rect, aspect: f32) -> Rect {
    let size = available.size();
    let (width, height) = if size.x / size.y > aspect {
        (size.y * aspect, size.y)
    } else {
        (size.x, size.x / aspect)
    };
    Rect::from_center_size(available.center(), Vec2::new(width, height))
}

/// Masks the 8-pixel software-mask columns that analog TVs commonly cropped.
fn paint_mask(ui: &egui::Ui, target: Rect, mask: MaskEdges) {
    let width = target.width() * 8.0 / 256.0;
    let black = Color32::BLACK;
    if matches!(mask, MaskEdges::Left | MaskEdges::Both | MaskEdges::Auto) {
        ui.painter().rect_filled(
            Rect::from_min_max(
                target.left_top(),
                egui::pos2(target.left() + width, target.bottom()),
            ),
            0.0,
            black,
        );
    }
    if mask == MaskEdges::Both {
        ui.painter().rect_filled(
            Rect::from_min_max(
                egui::pos2(target.right() - width, target.top()),
                target.right_bottom(),
            ),
            0.0,
            black,
        );
    }
}

/// Draws the selected output post-process preset over the game image.
fn paint_shader(ui: &egui::Ui, target: Rect, shader: ShaderPreset) {
    let step = (target.height() / 240.0).max(1.0);
    match shader {
        ShaderPreset::Off => {}
        ShaderPreset::Scanlines | ShaderPreset::Crt => {
            let painter = ui.painter();
            let mut y = target.top();
            let mut row = 0;
            while y < target.bottom() {
                if row & 1 == 1 {
                    let line = Rect::from_min_size(
                        egui::pos2(target.left(), y),
                        Vec2::new(target.width(), (step * 0.45).max(1.0)),
                    );
                    painter.rect_filled(
                        line,
                        0.0,
                        Color32::from_black_alpha(if shader == ShaderPreset::Crt {
                            90
                        } else {
                            64
                        }),
                    );
                }
                y += step;
                row += 1;
            }
            if shader == ShaderPreset::Crt {
                // Cheap vignette: two nested borders instead of a shader.
                let border = (target.width().min(target.height()) * 0.06).max(6.0);
                painter.rect_stroke(
                    target.shrink(border * 0.5),
                    6.0,
                    Stroke::new(border, Color32::from_black_alpha(60)),
                    egui::StrokeKind::Inside,
                );
                painter.rect_stroke(
                    target,
                    6.0,
                    Stroke::new(border * 2.0, Color32::from_black_alpha(35)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        ShaderPreset::NtscBudget | ShaderPreset::NtscPremium => {}
        ShaderPreset::Lcd => {
            let painter = ui.painter();
            let color = Color32::from_black_alpha(48);
            let mut x = target.left();
            while x <= target.right() {
                painter.line_segment(
                    [egui::pos2(x, target.top()), egui::pos2(x, target.bottom())],
                    Stroke::new(1.0_f32, color),
                );
                x += step;
            }
            let mut y = target.top();
            while y <= target.bottom() {
                painter.line_segment(
                    [egui::pos2(target.left(), y), egui::pos2(target.right(), y)],
                    Stroke::new(1.0_f32, color),
                );
                y += step;
            }
        }
    }
}

struct ViewportOptions {
    integer_scale: bool,
    linear_filter: bool,
    aspect: Aspect,
    scale: u32,
    supersample: u32,
    shader: ShaderPreset,
    brightness: f32,
    sharpness: f32,
    overscan: Overscan,
    mask_edges: MaskEdges,
    zapper_port: Option<usize>,
    paddle_port: Option<usize>,
    power_pad_port: Option<usize>,
    input: [unko_core::InputState; 4],
    show_input: bool,
    show_touch: bool,
}

/// Input the viewport produced this frame (touch, paddle or mat).
struct ViewportInput {
    touch: unko_core::InputState,
    paddle: Option<(u8, bool)>,
    power_pad: Option<u16>,
}

fn viewport(
    ctx: &egui::Context,
    player: &mut Player,
    options: &ViewportOptions,
    #[cfg(not(target_os = "ios"))] gpu: Option<&crate::gpu_shader::Handle>,
) -> ViewportInput {
    let ViewportOptions {
        integer_scale,
        linear_filter,
        aspect,
        scale,
        supersample,
        shader,
        brightness,
        sharpness,
        overscan,
        mask_edges,
        zapper_port,
        paddle_port,
        power_pad_port,
        input,
        show_input,
        show_touch,
    } = *options;
    let mut touch = unko_core::InputState::empty();
    let mut paddle = None;
    let mut power_pad = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(Color32::BLACK))
        .show(ctx, |ui| {
            let available = ui.max_rect();
            let mut target = match aspect.ratio() {
                Some(ratio) => fit_aspect(available, ratio),
                None => available,
            };
            if scale > 0 {
                let factor = scale as f32;
                let size = Vec2::new(256.0 * factor, 240.0 * factor);
                if size.x <= available.width() && size.y <= available.height() {
                    target = Rect::from_center_size(available.center(), size);
                }
            } else if integer_scale {
                let factor = (target.height() / 240.0).floor().max(1.0);
                let size = Vec2::new(240.0 * factor * 4.0 / 3.0, 240.0 * factor);
                target = Rect::from_center_size(available.center(), size);
            }
            if let Some(port) = zapper_port {
                // Mouse aims and fires the Zapper. The pointer is mapped into
                // the 256x240 frame inside the scaled viewport rect; leaving
                // the rect keeps the trigger released.
                let position = ctx.pointer_latest_pos();
                let trigger = ctx.input(|input| input.pointer.primary_down());
                if let Some(position) = position.filter(|position| target.contains(*position)) {
                    let relative = (position - target.min) / target.size();
                    let x = (relative.x * 256.0).clamp(0.0, 255.0) as u8;
                    let y = (relative.y * 240.0).clamp(0.0, 239.0) as u8;
                    player.set_zapper(port, x, y, trigger);
                } else if trigger {
                    player.set_zapper(port, 0, 0, false);
                }
            }
            if paddle_port.is_some() {
                // The paddle knob follows the horizontal mouse position (or
                // the touch point); the primary button fires.
                let position = ctx.pointer_latest_pos();
                let down = ctx.input(|input| input.pointer.primary_down());
                paddle = Some(
                    match position.filter(|position| target.contains(*position)) {
                        Some(position) => {
                            let relative = (position.x - target.min.x) / target.width();
                            ((relative * 255.0).clamp(0.0, 255.0) as u8, down)
                        }
                        None => (128, false),
                    },
                );
            }
            player.set_texture_options(if linear_filter || supersample > 1 {
                egui::TextureOptions::LINEAR
            } else {
                egui::TextureOptions::NEAREST
            });
            if let Some(texture) = player.texture() {
                let id = texture.id();
                let (vx, vy, vw, vh) = overscan.visible();
                let uv = Rect::from_min_max(
                    egui::pos2(f32::from(vx) / 256.0, f32::from(vy) / 240.0),
                    egui::pos2(f32::from(vx + vw) / 256.0, f32::from(vy + vh) / 240.0),
                );
                #[cfg(not(target_os = "ios"))]
                let gpu_drawn = if shader.is_gpu() {
                    match (gpu, player.frame_rgba()) {
                        (Some(gpu), Some((pixels, width, height))) => {
                            crate::gpu_shader::GpuShader::queue(
                                gpu,
                                ui,
                                target,
                                uv,
                                crate::gpu_shader::ShaderParams {
                                    preset: shader.shader_index(),
                                    brightness,
                                    sharpness,
                                    linear: linear_filter || supersample > 1,
                                },
                                pixels,
                                width,
                                height,
                            );
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                };
                #[cfg(target_os = "ios")]
                let gpu_drawn = false;
                if !gpu_drawn {
                    ui.painter().image(id, target, uv, Color32::WHITE);
                    paint_shader(ui, target, shader);
                }
                paint_mask(ui, target, mask_edges);
            } else {
                ui.painter().text(
                    available.center(),
                    Align2::CENTER_CENTER,
                    "Starting...",
                    FontId::proportional(18.0),
                    Color32::GRAY,
                );
            }

            if player.paused {
                let text = "Paused — Space or Pause to resume";
                let galley = ui.painter().layout_no_wrap(
                    text.into(),
                    FontId::proportional(16.0),
                    Color32::WHITE,
                );
                let bg = Rect::from_center_size(
                    available.center(),
                    galley.size() + Vec2::new(24.0, 12.0),
                );
                ui.painter().rect_filled(
                    bg,
                    egui::CornerRadius::same(4),
                    Color32::from_black_alpha(180),
                );
                ui.painter()
                    .galley(bg.center() - galley.size() / 2.0, galley, Color32::WHITE);
            }

            if !player.message.is_empty() {
                ui.painter().text(
                    available.left_bottom() + Vec2::new(12.0, -12.0),
                    Align2::LEFT_BOTTOM,
                    &player.message,
                    FontId::proportional(13.0),
                    Color32::from_gray(200),
                );
            }

            if show_input {
                let text = format!(
                    "P1 {}   P2 {}",
                    input_label(input[0]),
                    input_label(input[1])
                );
                let galley =
                    ui.painter()
                        .layout_no_wrap(text, FontId::monospace(13.0), Color32::WHITE);
                let pos = target.left_top() + Vec2::new(10.0, 10.0);
                let bg = Rect::from_min_size(pos, galley.size() + Vec2::new(12.0, 8.0));
                ui.painter().rect_filled(
                    bg,
                    egui::CornerRadius::same(4),
                    Color32::from_black_alpha(160),
                );
                ui.painter()
                    .galley(pos + Vec2::new(6.0, 4.0), galley, Color32::WHITE);
            }

            if show_touch {
                let pointer = ctx.pointer_latest_pos();
                let down = ctx.input(|i| i.pointer.primary_down());
                for (button, rect, label) in touch_buttons(target) {
                    let pressed = down && pointer.is_some_and(|pos| rect.contains(pos));
                    if pressed {
                        touch = touch.with(button, true);
                    }
                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::same(8),
                        if pressed {
                            Color32::from_white_alpha(150)
                        } else {
                            Color32::from_white_alpha(55)
                        },
                    );
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                }
            }

            if power_pad_port.is_some() {
                // Twelve-button mat: 3 rows x 4 columns, numbered row-major.
                let pointer = ctx.pointer_latest_pos();
                let down = ctx.input(|i| i.pointer.primary_down());
                let cell = (target.width().min(target.height()) * 0.075).clamp(30.0, 58.0);
                let grid = Vec2::new(cell * 4.0, cell * 3.0);
                let origin =
                    target.center_bottom() + Vec2::new(-grid.x / 2.0, -grid.y - cell * 0.6);
                let mut mask = 0u16;
                for row in 0..3 {
                    for column in 0..4 {
                        let rect = Rect::from_min_size(
                            origin + Vec2::new(column as f32 * cell, row as f32 * cell),
                            Vec2::splat(cell - 5.0),
                        );
                        let number = row * 4 + column + 1;
                        let pressed = down && pointer.is_some_and(|pos| rect.contains(pos));
                        if pressed {
                            mask |= 1 << (number - 1);
                        }
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(6),
                            if pressed {
                                Color32::from_white_alpha(170)
                            } else {
                                Color32::from_white_alpha(60)
                            },
                        );
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            number.to_string(),
                            FontId::proportional(15.0),
                            Color32::WHITE,
                        );
                    }
                }
                power_pad = Some(mask);
            }
        });
    ViewportInput {
        touch,
        paddle,
        power_pad,
    }
}

/// Builds the Family BASIC keyboard matrix from the host keyboard.
///
/// The 9x8 matrix follows the NESdev wiki layout: each row has two column
/// halves of four keys, selected by `$4016` bit 0. Keys that do not exist on a
/// US layout (STOP, kana, GRPH, RSHIFT) are not mapped; Backslash types the
/// yen key, which is its documented historical equivalent.
/// Host-key map for the Subor keyboard matrix. A position is 0-3 for the
/// lower nibble and 4-7 for the upper nibble of each row.
const SUBOR_MAP: &[(egui::Key, usize, u8)] = &[
    (egui::Key::Num4, 0, 0),
    (egui::Key::G, 0, 1),
    (egui::Key::F, 0, 2),
    (egui::Key::C, 0, 3),
    (egui::Key::F2, 0, 4),
    (egui::Key::E, 0, 5),
    (egui::Key::Num5, 0, 6),
    (egui::Key::V, 0, 7),
    (egui::Key::Num2, 1, 0),
    (egui::Key::D, 1, 1),
    (egui::Key::S, 1, 2),
    (egui::Key::End, 1, 3),
    (egui::Key::F1, 1, 4),
    (egui::Key::W, 1, 5),
    (egui::Key::Num3, 1, 6),
    (egui::Key::X, 1, 7),
    (egui::Key::Insert, 2, 0),
    (egui::Key::Backspace, 2, 1),
    (egui::Key::PageDown, 2, 2),
    (egui::Key::ArrowRight, 2, 3),
    (egui::Key::F8, 2, 4),
    (egui::Key::PageUp, 2, 5),
    (egui::Key::Delete, 2, 6),
    (egui::Key::Home, 2, 7),
    (egui::Key::Num9, 3, 0),
    (egui::Key::I, 3, 1),
    (egui::Key::L, 3, 2),
    (egui::Key::Comma, 3, 3),
    (egui::Key::F5, 3, 4),
    (egui::Key::O, 3, 5),
    (egui::Key::Num0, 3, 6),
    (egui::Key::Period, 3, 7),
    (egui::Key::CloseBracket, 4, 0),
    (egui::Key::Enter, 4, 1),
    (egui::Key::ArrowUp, 4, 2),
    (egui::Key::ArrowLeft, 4, 3),
    (egui::Key::F7, 4, 4),
    (egui::Key::OpenBracket, 4, 5),
    (egui::Key::Backslash, 4, 6),
    (egui::Key::ArrowDown, 4, 7),
    (egui::Key::Q, 5, 0),
    (egui::Key::Z, 5, 2),
    (egui::Key::Tab, 5, 3),
    (egui::Key::Escape, 5, 4),
    (egui::Key::A, 5, 5),
    (egui::Key::Num1, 5, 6),
    (egui::Key::Num7, 6, 0),
    (egui::Key::Y, 6, 1),
    (egui::Key::K, 6, 2),
    (egui::Key::M, 6, 3),
    (egui::Key::F4, 6, 4),
    (egui::Key::U, 6, 5),
    (egui::Key::Num8, 6, 6),
    (egui::Key::J, 6, 7),
    (egui::Key::Minus, 7, 0),
    (egui::Key::Semicolon, 7, 1),
    (egui::Key::Quote, 7, 2),
    (egui::Key::Slash, 7, 3),
    (egui::Key::F6, 7, 4),
    (egui::Key::P, 7, 5),
    (egui::Key::Equals, 7, 6),
    (egui::Key::T, 8, 0),
    (egui::Key::H, 8, 1),
    (egui::Key::N, 8, 2),
    (egui::Key::Space, 8, 3),
    (egui::Key::F3, 8, 4),
    (egui::Key::R, 8, 5),
    (egui::Key::Num6, 8, 6),
    (egui::Key::B, 8, 7),
    (egui::Key::F11, 10, 3),
    (egui::Key::F12, 10, 4),
    (egui::Key::F10, 11, 4),
    (egui::Key::Backtick, 12, 0),
    (egui::Key::F9, 12, 4),
];

fn subor_keyboard_matrix(ctx: &egui::Context) -> [u8; 13] {
    let mut rows = [0u8; 13];
    ctx.input(|input| {
        for &(key, row, bit) in SUBOR_MAP {
            if input.key_down(key) {
                rows[row] |= 1 << bit;
            }
        }
        // The matrix's Shift and Ctrl cells are modifiers, not key codes.
        if input.modifiers.shift {
            rows[7] |= 1 << 7;
        }
        if input.modifiers.ctrl {
            rows[5] |= 1 << 7;
        }
    });
    rows
}

fn family_keyboard_matrix(ctx: &egui::Context) -> [u8; 9] {
    // (host key, row, column half, position within the half)
    const MAP: &[(egui::Key, u8, u8, u8)] = &[
        (egui::Key::CloseBracket, 0, 0, 0),
        (egui::Key::OpenBracket, 0, 0, 1),
        (egui::Key::Enter, 0, 0, 2),
        (egui::Key::F8, 0, 0, 3),
        (egui::Key::Backslash, 0, 1, 1),
        (egui::Key::F7, 1, 0, 3),
        (egui::Key::Slash, 1, 1, 2),
        (egui::Key::K, 2, 0, 0),
        (egui::Key::L, 2, 0, 1),
        (egui::Key::O, 2, 0, 2),
        (egui::Key::F6, 2, 0, 3),
        (egui::Key::Num0, 2, 1, 0),
        (egui::Key::P, 2, 1, 1),
        (egui::Key::Comma, 2, 1, 2),
        (egui::Key::Period, 2, 1, 3),
        (egui::Key::J, 3, 0, 0),
        (egui::Key::U, 3, 0, 1),
        (egui::Key::I, 3, 0, 2),
        (egui::Key::F5, 3, 0, 3),
        (egui::Key::Num8, 3, 1, 0),
        (egui::Key::Num9, 3, 1, 1),
        (egui::Key::N, 3, 1, 2),
        (egui::Key::M, 3, 1, 3),
        (egui::Key::H, 4, 0, 0),
        (egui::Key::G, 4, 0, 1),
        (egui::Key::Y, 4, 0, 2),
        (egui::Key::F4, 4, 0, 3),
        (egui::Key::Num7, 4, 1, 1),
        (egui::Key::V, 4, 1, 2),
        (egui::Key::B, 4, 1, 3),
        (egui::Key::D, 5, 0, 0),
        (egui::Key::R, 5, 0, 1),
        (egui::Key::T, 5, 0, 2),
        (egui::Key::F3, 5, 0, 3),
        (egui::Key::Num4, 5, 1, 0),
        (egui::Key::Num5, 5, 1, 1),
        (egui::Key::C, 5, 1, 2),
        (egui::Key::F, 5, 1, 3),
        (egui::Key::A, 6, 0, 0),
        (egui::Key::S, 6, 0, 1),
        (egui::Key::W, 6, 0, 2),
        (egui::Key::F2, 6, 0, 3),
        (egui::Key::Num3, 6, 1, 0),
        (egui::Key::E, 6, 1, 1),
        (egui::Key::Z, 6, 1, 2),
        (egui::Key::X, 6, 1, 3),
        (egui::Key::Q, 7, 0, 1),
        (egui::Key::Escape, 7, 0, 2),
        (egui::Key::F1, 7, 0, 3),
        (egui::Key::Num1, 7, 1, 1),
        (egui::Key::ArrowLeft, 8, 0, 0),
        (egui::Key::ArrowRight, 8, 0, 1),
        (egui::Key::ArrowUp, 8, 0, 2),
        (egui::Key::Home, 8, 0, 3),
        (egui::Key::Insert, 8, 1, 0),
        (egui::Key::Backspace, 8, 1, 1),
        (egui::Key::Space, 8, 1, 2),
        (egui::Key::ArrowDown, 8, 1, 3),
    ];
    let mut keys = [0u8; 9];
    ctx.input(|input| {
        let bit = |half: u8, position: u8| -> u8 {
            if half == 0 {
                3 - position
            } else {
                4 + (3 - position)
            }
        };
        for (key, row, half, position) in MAP {
            if input.key_down(*key) {
                keys[usize::from(*row)] |= 1 << bit(*half, *position);
            }
        }
        if input.modifiers.shift {
            keys[7] |= 1 << bit(1, 3); // LSHIFT
        }
        if input.modifiers.ctrl {
            keys[7] |= 1 << bit(0, 0); // CTR
        }
        // Shifted punctuation that lives on another matrix key.
        // Keys whose shifted symbol is a separate matrix key: press the
        // symbol that matches the shift state instead of both switches.
        let shift = input.modifiers.shift;
        if input.key_down(egui::Key::Semicolon) {
            keys[1] |= 1 << if shift { bit(0, 1) } else { bit(0, 0) };
        }
        if input.key_down(egui::Key::Minus) {
            keys[1] |= 1 << if shift { bit(1, 3) } else { bit(1, 1) };
        }
        if input.key_down(egui::Key::Num2) {
            if shift {
                keys[1] |= 1 << bit(0, 2); // @
            } else {
                keys[7] |= 1 << bit(1, 0); // 2
            }
        }
        if input.key_down(egui::Key::Num6) {
            if shift {
                keys[1] |= 1 << bit(1, 0); // ^
            } else {
                keys[4] |= 1 << bit(1, 0); // 6
            }
        }
    });
    keys
}

fn input_label(state: unko_core::InputState) -> String {
    let mut parts = Vec::new();
    for (bit, label) in [
        (unko_core::InputState::UP, "↑"),
        (unko_core::InputState::DOWN, "↓"),
        (unko_core::InputState::LEFT, "←"),
        (unko_core::InputState::RIGHT, "→"),
        (unko_core::InputState::A, "A"),
        (unko_core::InputState::B, "B"),
        (unko_core::InputState::START, "Start"),
        (unko_core::InputState::SELECT, "Select"),
    ] {
        if state.bits() & bit != 0 {
            parts.push(label);
        }
    }
    if parts.is_empty() {
        "—".into()
    } else {
        parts.join(" ")
    }
}

fn touch_buttons(target: Rect) -> [(u16, Rect, &'static str); 8] {
    let cell = (target.width().min(target.height()) * 0.075).clamp(28.0, 54.0);
    let pad = target.left_bottom() + Vec2::new(cell * 1.4, -cell * 2.8);
    let dpad = |dx: f32, dy: f32| {
        Rect::from_center_size(pad + Vec2::new(dx * cell, dy * cell), Vec2::splat(cell))
    };
    let right = target.right_bottom();
    let a = Rect::from_center_size(
        right + Vec2::new(-cell * 2.2, -cell * 2.4),
        Vec2::splat(cell),
    );
    let b = Rect::from_center_size(
        right + Vec2::new(-cell * 3.5, -cell * 1.5),
        Vec2::splat(cell),
    );
    let start = Rect::from_center_size(
        right + Vec2::new(-cell * 1.0, -cell * 0.5),
        Vec2::new(cell * 1.6, cell * 0.7),
    );
    let select = Rect::from_center_size(
        right + Vec2::new(-cell * 3.0, -cell * 0.5),
        Vec2::new(cell * 1.6, cell * 0.7),
    );
    [
        (unko_core::InputState::UP, dpad(0.0, -1.0), "↑"),
        (unko_core::InputState::DOWN, dpad(0.0, 1.0), "↓"),
        (unko_core::InputState::LEFT, dpad(-1.0, 0.0), "←"),
        (unko_core::InputState::RIGHT, dpad(1.0, 0.0), "→"),
        (unko_core::InputState::A, a, "A"),
        (unko_core::InputState::B, b, "B"),
        (unko_core::InputState::START, start, "Start"),
        (unko_core::InputState::SELECT, select, "Select"),
    ]
}

fn general_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    _ctx: &egui::Context,
    reset_layout: &mut bool,
) -> bool {
    ui.heading("General");
    ui.add_space(8.0);
    ui.checkbox(
        &mut settings.pause_on_focus_loss,
        "Pause when the window loses focus",
    );
    ui.checkbox(&mut settings.confirm_stop, "Confirm before stopping a game");
    ui.checkbox(
        &mut settings.detachable_windows,
        "Open Settings, Devices and the Debugger as separate OS windows",
    );
    if settings.detachable_windows && ui.button("Reset window layout").clicked() {
        *reset_layout = true;
    }
    ui.add_space(12.0);
    ui.label("Theme");
    ui.horizontal(|ui| {
        ui.radio_value(&mut settings.theme, Theme::Dark, "Dark");
        ui.radio_value(&mut settings.theme, Theme::Light, "Light");
    });
    ui.add_space(18.0);
    ui.separator();
    let mut reset = false;
    if ui.button("Reset all settings…").clicked() {
        reset = true;
    }
    ui.label(
        RichText::new(
            "Restores configuration, window layout, hotkeys, gamepad bindings, \
             per-port devices and per-game profiles. Save states, SRAM and \
             recordings are kept.",
        )
        .weak()
        .small(),
    );
    reset
}

fn interface_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Interface");
    ui.add_space(8.0);
    ui.checkbox(&mut settings.show_status_bar, "Show status bar");
    ui.checkbox(&mut settings.grid_view, "Game library opens in grid view");
    ui.horizontal(|ui| {
        ui.label("Playback speed");
        egui::ComboBox::from_id_salt("playback-speed")
            .selected_text(format!("{}%", settings.speed_percent))
            .show_ui(ui, |ui| {
                for percent in [25, 50, 100, 150, 200, 300] {
                    ui.selectable_value(
                        &mut settings.speed_percent,
                        percent,
                        format!("{percent}%"),
                    );
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Rewind buffer");
        egui::ComboBox::from_id_salt("rewind-budget")
            .selected_text(format!("{} MiB", settings.rewind_mb))
            .show_ui(ui, |ui| {
                for budget in [64, 128, 256] {
                    ui.selectable_value(&mut settings.rewind_mb, budget, format!("{budget} MiB"));
                }
            });
    });
    ui.label(
        RichText::new(
            "Hold the fast-forward hotkey (Tab) to temporarily run at 300%; hold Backspace \
             while paused to rewind one frame at a time.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.label("Toolbar, menu and status layout follow Dolphin's desktop shell.");
}

fn graphics_tab(ui: &mut egui::Ui, settings: &mut Settings, ctx: &egui::Context) {
    ui.heading("Graphics");
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Window size");
        for (label, size) in [
            ("1280x960", [1280.0, 960.0]),
            ("1600x1200", [1600.0, 1200.0]),
            ("1920x1080", [1920.0, 1080.0]),
        ] {
            if ui.button(label).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::from(size)));
            }
        }
    });
    ui.add_space(8.0);
    ui.label("Video filter");
    ui.radio_value(
        &mut settings.filter,
        VideoFilter::Raw,
        "Raw pixels (nearest)",
    );
    ui.radio_value(
        &mut settings.filter,
        VideoFilter::Composite,
        "Composite / NTSC simulation",
    );
    ui.radio_value(
        &mut settings.filter,
        VideoFilter::Crt,
        "CRT (composite + scanlines + bloom)",
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Palette");
        egui::ComboBox::from_id_salt("palette")
            .selected_text(palette_label(settings.palette))
            .show_ui(ui, |ui| {
                for palette in [
                    unko_core::Palette::Canonical,
                    unko_core::Palette::Consumer,
                    unko_core::Palette::Rgb,
                    unko_core::Palette::Monochrome,
                    unko_core::Palette::Vivid,
                    unko_core::Palette::Soft,
                    unko_core::Palette::Pal,
                    unko_core::Palette::Studio,
                    unko_core::Palette::Custom,
                ] {
                    ui.selectable_value(&mut settings.palette, palette, palette_label(palette));
                }
            });
        ui.label("Palette file");
        let mut path = settings.custom_palette.display().to_string();
        if ui
            .add(egui::TextEdit::singleline(&mut path).desired_width(320.0))
            .changed()
        {
            settings.custom_palette = PathBuf::from(path);
            settings.palette = unko_core::Palette::Custom;
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if ui.button("Import .pal…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("NES palette", &["pal", "palette"])
                .pick_file()
            {
                settings.custom_palette = path;
                settings.palette = unko_core::Palette::Custom;
            }
        }
    });
    ui.label(
        RichText::new(
            "Canonical matches the reference oracle; the other palettes are display presets.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.label("Aspect");
    ui.horizontal(|ui| {
        for aspect in [Aspect::Fit, Aspect::Tv43, Aspect::Pixel87] {
            ui.radio_value(&mut settings.aspect, aspect, aspect.label());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Overscan");
        egui::ComboBox::from_id_salt("overscan")
            .selected_text(settings.overscan.label())
            .show_ui(ui, |ui| {
                for overscan in [
                    Overscan::Normal,
                    Overscan::Vertical,
                    Overscan::Borders,
                    Overscan::Everything,
                ] {
                    ui.selectable_value(&mut settings.overscan, overscan, overscan.label());
                }
            });
        ui.label("Mask edges");
        egui::ComboBox::from_id_salt("mask-edges")
            .selected_text(settings.mask_edges.label())
            .show_ui(ui, |ui| {
                for mask in [
                    MaskEdges::Off,
                    MaskEdges::Left,
                    MaskEdges::Both,
                    MaskEdges::Auto,
                ] {
                    ui.selectable_value(&mut settings.mask_edges, mask, mask.label());
                }
            });
    });
    ui.checkbox(&mut settings.integer_scale, "Integer scaling when possible");
    ui.add(egui::Slider::new(&mut settings.scale, 0..=6).text("Fixed integer scale (0 = auto)"));
    ui.horizontal(|ui| {
        ui.label("Supersampling");
        egui::ComboBox::from_id_salt("supersample")
            .selected_text(format!("{}x", settings.supersample))
            .show_ui(ui, |ui| {
                for factor in [1, 2, 3] {
                    ui.selectable_value(&mut settings.supersample, factor, format!("{factor}x"));
                }
            });
        ui.label("(output upscale, then linear downscale)");
    });
    ui.checkbox(
        &mut settings.linear_filter,
        "Smooth (linear) texture filtering",
    );
    ui.horizontal(|ui| {
        ui.label("Output shader");
        egui::ComboBox::from_id_salt("output-shader")
            .selected_text(settings.shader.label())
            .show_ui(ui, |ui| {
                for shader in [
                    ShaderPreset::Off,
                    ShaderPreset::Scanlines,
                    ShaderPreset::Lcd,
                    ShaderPreset::Crt,
                    ShaderPreset::NtscBudget,
                    ShaderPreset::NtscPremium,
                ] {
                    ui.selectable_value(&mut settings.shader, shader, shader.label());
                }
            });
    });
    ui.add(
        egui::Slider::new(&mut settings.brightness, 0.5..=1.5)
            .text("Brightness")
            .fixed_decimals(2),
    );
    ui.add(
        egui::Slider::new(&mut settings.sharpness, 0.0..=1.0)
            .text("Sharpness")
            .fixed_decimals(2),
    );
    ui.label(
        RichText::new(
            "Scanline, LCD, CRT and NTSC presets run in a GPU fragment shader. \
             NTSC Budget uses a 3-tap chroma filter, Premium a 7-tap filter; \
             brightness and sharpness apply to every preset.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Filter, aspect, scale and shader changes apply to the running game immediately.",
        )
        .weak()
        .small(),
    );
}

fn system_label(system: &str) -> &'static str {
    match system {
        "snes" => "SNES (stub)",
        "n64" => "N64 (stub)",
        _ => "NES",
    }
}

fn palette_label(palette: unko_core::Palette) -> &'static str {
    match palette {
        unko_core::Palette::Canonical => "Canonical 2C02",
        unko_core::Palette::Consumer => "Consumer TV (warm)",
        unko_core::Palette::Rgb => "RGB monitor (saturated)",
        unko_core::Palette::Monochrome => "Monochrome",
        unko_core::Palette::Custom => "Custom (.pal)",
        unko_core::Palette::Vivid => "Vivid (generated)",
        unko_core::Palette::Soft => "Soft (generated)",
        unko_core::Palette::Pal => "PAL-like (generated)",
        unko_core::Palette::Studio => "Studio (generated)",
    }
}

fn audio_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    audio: Option<&AudioOutput>,
    audio_error: Option<&str>,
) {
    ui.heading("Audio");
    ui.add_space(8.0);
    egui::Grid::new("audio-output-grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Sample rate");
            egui::ComboBox::from_id_salt("sample-rate")
                .selected_text(format!("{:.0} Hz", settings.sample_rate))
                .show_ui(ui, |ui| {
                    for rate in [44_100.0, 48_000.0, 96_000.0] {
                        ui.selectable_value(
                            &mut settings.sample_rate,
                            rate,
                            format!("{rate:.0} Hz"),
                        );
                    }
                });
            ui.end_row();

            ui.label("Buffer latency");
            egui::ComboBox::from_id_salt("audio-latency")
                .selected_text(format!("{} ms", settings.audio_latency_ms))
                .show_ui(ui, |ui| {
                    for ms in [15, 30, 60, 100, 150, 250] {
                        ui.selectable_value(&mut settings.audio_latency_ms, ms, format!("{ms} ms"));
                    }
                });
            ui.end_row();

            ui.label("Resampler");
            egui::ComboBox::from_id_salt("audio-quality")
                .selected_text(settings.audio_quality.label())
                .show_ui(ui, |ui| {
                    for quality in [
                        AudioQuality::Fast,
                        AudioQuality::Standard,
                        AudioQuality::High,
                    ] {
                        ui.selectable_value(&mut settings.audio_quality, quality, quality.label());
                    }
                });
            ui.end_row();

            ui.label("Output filter");
            let custom_model = settings.custom_audio_filter();
            egui::ComboBox::from_id_salt("audio-filter")
                .selected_text(settings.audio_filter.label())
                .show_ui(ui, |ui| {
                    for model in [
                        AudioFilterModel::NesRf,
                        AudioFilterModel::Crisp,
                        AudioFilterModel::Warm,
                        AudioFilterModel::Linear,
                    ] {
                        ui.selectable_value(&mut settings.audio_filter, model, model.label());
                    }
                    ui.selectable_value(&mut settings.audio_filter, custom_model, "Custom");
                });
            ui.end_row();
        });
    if matches!(settings.audio_filter, AudioFilterModel::Custom { .. }) {
        let mut custom = settings.custom_audio_filter();
        let AudioFilterModel::Custom {
            high_pass_hz,
            low_pass_hz,
        } = &mut custom
        else {
            unreachable!()
        };
        ui.add(egui::Slider::new(high_pass_hz, 0.0..=1_000.0).text("High-pass (Hz, 0 = off)"));
        ui.add(
            egui::Slider::new(low_pass_hz, 2_000.0..=20_000.0)
                .text("Low-pass (Hz)")
                .logarithmic(true),
        );
        if let AudioFilterModel::Custom {
            high_pass_hz,
            low_pass_hz,
        } = custom
        {
            settings.audio_high_pass = high_pass_hz;
            settings.audio_low_pass = low_pass_hz;
            settings.audio_filter = custom;
        }
    }
    ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).text("Volume"));
    ui.checkbox(&mut settings.muted, "Mute");
    ui.add_space(10.0);
    ui.separator();
    ui.heading("Mixer");
    ui.horizontal(|ui| {
        ui.label("Audio source");
        egui::ComboBox::from_id_salt("audio-source")
            .selected_text(settings.audio_source.label())
            .show_ui(ui, |ui| {
                for source in [
                    AudioSource::Both,
                    AudioSource::Internal,
                    AudioSource::Expansion,
                    AudioSource::None,
                ] {
                    ui.selectable_value(&mut settings.audio_source, source, source.label());
                }
            });
    });
    ui.label(
        RichText::new(
            "Per-channel gains. Muting keeps the slider value so it can be restored;              the expansion row covers VRC6/7, MMC5, Namco 163, 5B and FDS audio.",
        )
        .weak()
        .small(),
    );
    const CHANNELS: [&str; 6] = [
        "Pulse 1",
        "Pulse 2",
        "Triangle",
        "Noise",
        "DMC",
        "Expansion",
    ];
    for (index, name) in CHANNELS.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut settings.channel_mutes[index], "M");
            ui.label(format!("{name:9}"));
            ui.add(
                egui::Slider::new(&mut settings.channel_gains[index], 0.0..=1.0).show_value(true),
            );
        });
    }
    if ui.button("Reset mixer").clicked() {
        settings.channel_gains = [1.0; 6];
        settings.channel_mutes = [false; 6];
    }
    ui.add_space(10.0);
    match audio {
        Some(audio) => {
            ui.label(
                RichText::new(format!(
                    "Output: {:.0} Hz, {} channel(s), {}, {} underruns, {} dropped samples",
                    audio.sample_rate(),
                    audio.channels(),
                    if audio.is_active() {
                        "stream running"
                    } else {
                        "stream paused"
                    },
                    audio.underruns(),
                    audio.dropped()
                ))
                .weak()
                .small(),
            );
            ui.label(
                RichText::new(format!(
                    "Buffered: {:.1} ms (target {} ms, {:.0} samples queued)",
                    audio.queued_ms(),
                    audio.latency_ms(),
                    audio.queued_samples() as f32
                ))
                .weak()
                .small(),
            );
            ui.label(
                RichText::new(
                    "If the device does not support the selected rate, the device rate is used and the core resamples.",
                )
                .weak()
                .small(),
            );
        }
        None => {
            ui.colored_label(
                Color32::from_rgb(0xE0, 0x80, 0x60),
                format!(
                    "No audio output: {}",
                    audio_error.unwrap_or("device unavailable")
                ),
            );
        }
    }
}

fn controllers_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    gamepads: Option<&mut Gamepads>,
    gamepad_error: Option<&str>,
    hotkey_capture: &mut Option<HotkeyAction>,
) {
    ui.heading("Controllers");
    ui.add_space(8.0);
    if let Some(err) = gamepad_error {
        ui.colored_label(
            Color32::from_rgb(0xE0, 0x80, 0x60),
            format!("SDL3 gamepad unavailable: {err}"),
        );
        ui.label(RichText::new("Keyboard input is still available.").weak());
        ui.add_space(8.0);
    }
    ui.horizontal(|ui| {
        ui.label("Port 1");
        device_combo(ui, "port1-device", &mut settings.port1_device);
        ui.label("Port 2");
        device_combo(ui, "port2-device", &mut settings.port2_device);
    });
    match (settings.port1_device, settings.port2_device) {
        (PortDevice::FourScore, _) | (_, PortDevice::FourScore) => ui.label(
            RichText::new(
                "Four Score: players 1-4 are read from gamepads 1-4 (it occupies both ports).",
            )
            .weak()
            .small(),
        ),
        (PortDevice::Paddle, _) | (_, PortDevice::Paddle) => ui.label(
            RichText::new("Paddle: the mouse steers horizontally, the left button fires.")
                .weak()
                .small(),
        ),
        (PortDevice::PowerPad | PortDevice::FamilyTrainer, _)
        | (_, PortDevice::PowerPad | PortDevice::FamilyTrainer) => ui.label(
            RichText::new("Power Pad: click the 12-pad overlay in the viewport.")
                .weak()
                .small(),
        ),
        (PortDevice::FamilyKeyboard, _) | (_, PortDevice::FamilyKeyboard) => ui.label(
            RichText::new("Family BASIC keyboard: host keys map to the documented matrix.")
                .weak()
                .small(),
        ),
        _ => ui.label(""),
    };
    ui.add_space(8.0);
    ui.checkbox(&mut settings.gamepad_enabled, "Enable SDL3 gamepads");
    if let Some(gamepads) = gamepads {
        if let Some((button, pad)) = gamepads.take_captured() {
            settings.pad_mapping.set(button, pad);
        }
        let names = gamepads.names();
        let connected = gamepads.connected();
        ui.add_space(6.0);
        if connected == 0 {
            ui.label(
                RichText::new("No gamepads detected. Connect one and it will appear here.").weak(),
            );
        } else {
            egui::Grid::new("gamepad-list")
                .num_columns(2)
                .spacing([18.0, 4.0])
                .show(ui, |ui| {
                    for (index, name) in names.iter().enumerate() {
                        if let Some(name) = name {
                            ui.label(format!("Player {}", index + 1));
                            ui.label(RichText::new(name).strong());
                            ui.end_row();
                        }
                    }
                });
        }
        ui.add_space(8.0);
        ui.label("NES port 1 (keyboard)");
        ui.add_space(4.0);
        egui::Grid::new("controller-map")
            .num_columns(3)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                ui.label("NES");
                ui.label("Keyboard");
                ui.label("Gamepad");
                ui.end_row();
                for (button, key) in [
                    ("D-pad", "Arrow keys"),
                    ("A", "X"),
                    ("B", "Z"),
                    ("Start", "Enter"),
                    ("Select", "Shift"),
                ] {
                    ui.label(button);
                    ui.label(RichText::new(key).strong());
                    ui.label("");
                    ui.end_row();
                }
            });
        ui.add_space(8.0);
        ui.label("Gamepad bindings");
        egui::Grid::new("gamepad-bindings")
            .num_columns(3)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                for button in NesButton::ALL {
                    ui.label(button.label());
                    ui.label(RichText::new(button_name(settings.pad_mapping.get(button))).strong());
                    let label = if gamepads.capturing() {
                        "Press a button..."
                    } else {
                        "Bind"
                    };
                    if ui
                        .add_enabled(!gamepads.capturing(), egui::Button::new(label))
                        .clicked()
                    {
                        gamepads.begin_capture(button);
                    }
                    ui.end_row();
                }
            });
        ui.horizontal(|ui| {
            if ui.button("Reset gamepad bindings").clicked() {
                settings.pad_mapping.reset();
            }
            ui.label(
                RichText::new("Bindings apply immediately; analog sticks also act as the D-pad.")
                    .weak()
                    .small(),
            );
        });
        ui.add(egui::Slider::new(&mut settings.pad_deadzone, 0..=32_000).text("Stick dead zone"));
        ui.add_space(8.0);
        ui.checkbox(&mut settings.turbo_enabled, "Auto-fire A/B on gamepad");
        if settings.turbo_enabled {
            ui.add(egui::Slider::new(&mut settings.turbo_hz, 2.0..=30.0).text("Auto-fire Hz"));
        }
    }
    ui.add_space(8.0);
    ui.label("Keyboard shortcuts");
    egui::Grid::new("hotkeys")
        .num_columns(3)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            for action in HotkeyAction::ALL {
                ui.label(action.label());
                ui.label(RichText::new(settings.hotkeys.get(action).name()).strong());
                let label = if *hotkey_capture == Some(action) {
                    "Press a key..."
                } else {
                    "Set"
                };
                if ui.button(label).clicked() {
                    *hotkey_capture = Some(action);
                }
                ui.end_row();
            }
        });
    if ui.button("Reset shortcuts").clicked() {
        settings.hotkeys = crate::settings::Hotkeys::default();
    }
    ui.add_space(8.0);
    ui.checkbox(
        &mut settings.per_game_input,
        "Keep gamepad bindings per game",
    );
    ui.checkbox(
        &mut settings.show_input_display,
        "Show input display overlay",
    );
    ui.checkbox(
        &mut settings.show_touch_controls,
        "Show on-screen touch controller",
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Device changes apply immediately. Zapper and paddle follow the mouse inside the viewport.",
        )
        .weak()
        .small(),
    );
}

/// Human label for a controller-port device.
fn device_label(device: PortDevice) -> &'static str {
    match device {
        PortDevice::Gamepad => "Gamepad",
        PortDevice::Zapper => "Zapper",
        PortDevice::Paddle => "Paddle",
        PortDevice::PowerPad => "Power Pad",
        PortDevice::FamilyTrainer => "Family Trainer",
        PortDevice::FamilyKeyboard => "Family BASIC keyboard",
        PortDevice::SuborKeyboard => "Subor keyboard",
        PortDevice::FourScore => "Four Score (4 players)",
        PortDevice::Microphone => "Famicom microphone",
        PortDevice::Tape => "Family BASIC data recorder",
        PortDevice::MiraclePiano => "Miracle Piano (MIDI)",
    }
}

/// Device picker for one controller port.
fn device_combo(ui: &mut egui::Ui, id: &str, device: &mut PortDevice) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(device_label(*device))
        .show_ui(ui, |ui| {
            for option in [
                PortDevice::Gamepad,
                PortDevice::Zapper,
                PortDevice::Paddle,
                PortDevice::PowerPad,
                PortDevice::FamilyTrainer,
                PortDevice::FamilyKeyboard,
                PortDevice::SuborKeyboard,
                PortDevice::FourScore,
                PortDevice::Microphone,
                PortDevice::Tape,
                PortDevice::MiraclePiano,
            ] {
                if ui
                    .selectable_label(*device == option, device_label(option))
                    .clicked()
                {
                    *device = option;
                }
            }
        });
}

fn paths_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Paths");
    ui.add_space(8.0);
    ui.label("ROM folder");
    ui.horizontal(|ui| {
        let mut rom_dir = settings.rom_dir.display().to_string();
        if ui.text_edit_singleline(&mut rom_dir).changed() {
            settings.rom_dir = PathBuf::from(rom_dir);
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if ui.button("Browse…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(&settings.rom_dir)
                .pick_folder()
            {
                settings.rom_dir = path;
            }
        }
    });
    ui.label("Save/state folder");
    ui.horizontal(|ui| {
        let mut save_dir = settings.save_dir.display().to_string();
        if ui.text_edit_singleline(&mut save_dir).changed() {
            settings.save_dir = PathBuf::from(save_dir);
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if ui.button("Browse…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(&settings.save_dir)
                .pick_folder()
            {
                settings.save_dir = path;
            }
        }
    });
    ui.add_space(12.0);
    ui.separator();
    ui.heading("Library roots");
    ui.label(
        RichText::new(
            "Additional folders tagged by system. Only NES roots are scanned in this milestone; \
             the SNES/N64 tags are placeholders for the multi-system library.",
        )
        .weak()
        .small(),
    );
    let mut remove = None;
    for (index, root) in settings.library_roots.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let mut system = root.system.clone();
            egui::ComboBox::from_id_salt(("root-system", index))
                .selected_text(system_label(&system))
                .show_ui(ui, |ui| {
                    for candidate in ["nes", "snes", "n64"] {
                        ui.selectable_value(
                            &mut system,
                            candidate.to_string(),
                            system_label(candidate),
                        );
                    }
                });
            root.system = system;
            let mut path = root.path.display().to_string();
            if ui.text_edit_singleline(&mut path).changed() {
                root.path = PathBuf::from(path);
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&root.path)
                    .pick_folder()
                {
                    root.path = path;
                }
            }
            if ui.button("Remove").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        settings.library_roots.remove(index);
    }
    if ui.button("Add library root").clicked() {
        settings.library_roots.push(crate::settings::LibraryRoot {
            system: "nes".into(),
            path: settings.rom_dir.clone(),
        });
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new("Changing the ROM folder rescans the library when you close Settings.")
            .weak()
            .small(),
    );
}

fn nes_tab(ui: &mut egui::Ui, settings: &mut Settings, entry: Option<&GameEntry>) {
    ui.heading("NES");
    ui.add_space(8.0);
    egui::ComboBox::from_label("Region")
        .selected_text(player::region_label(settings.region))
        .show_ui(ui, |ui| {
            for region in [Region::Auto, Region::Ntsc, Region::Pal, Region::Dendy] {
                ui.selectable_value(&mut settings.region, region, player::region_label(region));
            }
        });
    ui.add_space(8.0);
    match entry.and_then(|entry| entry.header.map(|header| (entry, header))) {
        Some((entry, header)) => {
            ui.label(RichText::new(&entry.title).strong());
            egui::Grid::new("nes-header")
                .num_columns(2)
                .spacing([24.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Mapper");
                    ui.label(format!("{}:{}", header.mapper, header.submapper));
                    ui.end_row();
                    ui.label("PRG");
                    ui.label(format!("{} KiB", header.prg_kib));
                    ui.end_row();
                    ui.label("CHR");
                    ui.label(format!("{} KiB", header.chr_kib));
                    ui.end_row();
                    ui.label("Battery");
                    ui.label(if header.battery { "yes" } else { "no" });
                    ui.end_row();
                    ui.label("Format");
                    ui.label(if header.nes2 { "NES 2.0" } else { "iNES" });
                    ui.end_row();
                });
        }
        None => {
            ui.label("Select a game in the library to see its header details.");
        }
    }
    ui.add_space(12.0);
    ui.label(
        RichText::new(
            "Region changes apply to the next launch. Power-on RAM is deterministic (zeroed) for state/netplay equivalence.",
        )
        .weak()
        .small(),
    );
}

fn advanced_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Advanced");
    ui.add_space(8.0);
    ui.label("Core: Unko NES accuracy core");
    ui.label("State format: UNKOST06, ROM-fingerprinted and core-tagged");
    ui.label("Settings file: unko.conf");
    ui.add_space(10.0);
    ui.separator();
    ui.heading("Soft-reset accuracy");
    ui.horizontal(|ui| {
        ui.label("PPU reset behavior");
        egui::ComboBox::from_id_salt("ppu-reset")
            .selected_text(settings.ppu_reset.label())
            .show_ui(ui, |ui| {
                for behavior in [PpuResetBehavior::Famicom, PpuResetBehavior::Nes] {
                    ui.selectable_value(&mut settings.ppu_reset, behavior, behavior.label());
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("RAM clear");
        egui::ComboBox::from_id_salt("ram-clear")
            .selected_text(settings.ram_clear.label())
            .show_ui(ui, |ui| {
                for mode in [
                    RamClear::Keep,
                    RamClear::Zero,
                    RamClear::Fill,
                    RamClear::Random,
                ] {
                    ui.selectable_value(&mut settings.ram_clear, mode, mode.label());
                }
            });
    });
    ui.label(
        RichText::new(
            "These apply on the next Emulation -> Reset (or the reset hotkey), matching the \
             MiSTer options. A power-on load starts from zeroed work RAM.",
        )
        .weak()
        .small(),
    );
    ui.add_space(10.0);
    ui.separator();
    ui.heading("ROM headers");
    ui.label(
        RichText::new(
            "Auto region needs a NES 2.0 header; iNES images default to NTSC, as MiSTer warns. \
             Bulk header repair is an external tool, not part of the core.",
        )
        .weak()
        .small(),
    );
    ui.label(
        RichText::new("Logging, run-ahead and rewind controls land with the next milestone.")
            .weak()
            .small(),
    );
}

const LIST_COLUMN_BITS: [(&str, u8); 6] = [
    ("System", 0x01),
    ("File", 0x02),
    ("Size", 0x04),
    ("Region", 0x08),
    ("Mapper", 0x10),
    ("Modified", 0x20),
];

fn column_cell(entry: &crate::library::GameEntry, bit: u8) -> String {
    match bit {
        0x01 => entry.system().to_string(),
        0x02 => entry.file_name.clone(),
        0x04 => entry.size_label(),
        0x08 => entry.region_label().to_string(),
        0x10 => entry.mapper_label(),
        _ => modified_label(entry.modified),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix_with(events: Vec<egui::Event>) -> [u8; 9] {
        let ctx = egui::Context::default();
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let input = egui::RawInput {
            modifiers,
            events,
            ..egui::RawInput::default()
        };
        let mut keys = [0u8; 9];
        let _ = ctx.run(input, |ctx| {
            keys = family_keyboard_matrix(ctx);
        });
        keys
    }

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn family_keyboard_maps_letters_to_the_documented_matrix() {
        // A = row 6, column half 0, first key -> bit 3.
        let keys = matrix_with(vec![key_event(egui::Key::A, egui::Modifiers::NONE)]);
        assert_eq!(keys[6], 1 << 3);
        // 1 = row 7, half 1, second key -> bit 4 + (3 - 1) = 6.
        let keys = matrix_with(vec![key_event(egui::Key::Num1, egui::Modifiers::NONE)]);
        assert_eq!(keys[7], 1 << 6);
        // Enter = row 0, half 0, third key -> bit 1.
        let keys = matrix_with(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(keys[0], 1 << 1);
    }

    #[test]
    fn family_keyboard_does_not_press_both_shift_variants() {
        let keys = matrix_with(vec![key_event(egui::Key::Num2, egui::Modifiers::SHIFT)]);
        assert_eq!(keys[1] & (1 << 1), 1 << 1, "@ is pressed");
        assert_eq!(keys[7] & (1 << 7), 0, "2 is not pressed while shifted");
        let keys = matrix_with(vec![key_event(egui::Key::Minus, egui::Modifiers::SHIFT)]);
        assert_eq!(keys[1] & (1 << 4), 1 << 4, "_ is pressed");
        assert_eq!(keys[1] & (1 << 6), 0, "- is not pressed while shifted");
    }

    #[test]
    fn family_keyboard_maps_shifted_punctuation() {
        let keys = matrix_with(vec![key_event(egui::Key::Num2, egui::Modifiers::SHIFT)]);
        // "@" is row 1, half 0, third key -> bit 1, plus the 2 key and shift.
        assert_eq!(keys[1] & (1 << 1), 1 << 1);
        assert_eq!(keys[7] & (1 << 4), 1 << 4, "LSHIFT is held");
    }
}
