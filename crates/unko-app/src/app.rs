//! Main window: library, player, menus, toolbar, settings.
//!
//! Layout follows Dolphin's desktop shell: menu bar, toolbar, central stack
//! (library or emulation surface), status bar, and one settings window with a
//! category rail. See `docs/frontend.md` for the mapping and the licensing
//! boundary (Dolphin is GPL; this is an original MIT implementation of the UX).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Vec2};
use unko_core::{Region, VideoFilter};

use crate::library::{self, GameEntry};
use crate::player::{self, Player};
use crate::settings::{Settings, Theme};

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
        };
        app.status_until = Some(Instant::now() + Duration::from_secs(6));
        if args.settings {
            app.settings_open = true;
        }
        if let Some(entry) = app.pending_play.take() {
            app.start_entry(entry);
        }
        app
    }

    fn scan(&mut self) {
        self.library = scan(&self.settings.rom_dir);
        self.selected = None;
        self.status = if self.library.is_empty() {
            format!(
                "No ROMs in {} — add .nes files or .zip/.7z/.rar archives",
                self.settings.rom_dir.display()
            )
        } else {
            format!(
                "{} ROMs in {}",
                self.library.len(),
                self.settings.rom_dir.display()
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
        match Player::start(&entry, &self.settings) {
            Ok(player) => {
                self.set_status(format!("Loaded {} — {}", entry.title, player.core_label()));
                self.player = Some(player);
            }
            Err(err) => self.set_status(format!("Load failed: {err}")),
        }
    }

    fn stop_game(&mut self) {
        if let Some(mut player) = self.player.take() {
            player.shutdown();
            self.set_status(format!("Stopped {}", player.title));
        }
    }

    fn apply_settings(&mut self, ctx: &egui::Context, old: &Settings) {
        if let Some(player) = &mut self.player {
            player.set_filter(self.settings.filter);
            player.set_sample_rate(self.settings.sample_rate);
        }
        apply_theme(ctx, self.settings.theme);
        if self.settings.rom_dir != old.rom_dir {
            self.scan();
        }
        let _ = self.settings.save(&Settings::path());
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let (save, load, fullscreen, pause) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F5),
                i.key_pressed(egui::Key::F8),
                i.key_pressed(egui::Key::F11),
                i.key_pressed(egui::Key::Space),
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
                        ui.label(format!("slot {}", player.slot));
                        ui.separator();
                        ui.label(format!("{:.0} Hz", player.sample_rate()));
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
                    if ui.button("Rescan").clicked() {
                        self.scan();
                    }
                });
            });
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
        let width = ui.available_width();
        let title_width = (width * 0.30).clamp(200.0, 380.0);
        let others: f32 = 64.0 + 76.0 + 64.0 + 64.0 + 96.0;
        let file_width = (width - title_width - others - 24.0).max(160.0);
        let columns: [(&str, f32); 6] = [
            ("System", 64.0),
            ("File", file_width),
            ("Size", 76.0),
            ("Region", 64.0),
            ("Mapper", 64.0),
            ("Modified", 96.0),
        ];
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
        for (name, column_width) in columns {
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
                    let cells = [
                        entry.system().to_string(),
                        entry.file_name.clone(),
                        entry.size_label(),
                        entry.region_label().to_string(),
                        entry.mapper_label(),
                        modified_label(entry.modified),
                    ];
                    for (cell, (_, column_width)) in cells.iter().zip(columns) {
                        painter.text(
                            egui::pos2(x, y),
                            Align2::LEFT_CENTER,
                            cell,
                            FontId::proportional(12.0),
                            Color32::from_gray(170),
                        );
                        x += column_width;
                    }
                }
            });
    }

    fn grid(&mut self, ui: &mut egui::Ui, indices: &[usize]) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for &index in indices {
                        let entry = &self.library[index];
                        let selected = self.selected == Some(index);
                        let size = Vec2::new(190.0, 88.0);
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
                            rect.left_top() + Vec2::new(10.0, 18.0),
                            Align2::LEFT_TOP,
                            &entry.title,
                            FontId::proportional(14.0),
                            ui.visuals().strong_text_color(),
                        );
                        painter.text(
                            rect.left_bottom() + Vec2::new(10.0, -14.0),
                            Align2::LEFT_BOTTOM,
                            format!(
                                "NES · mapper {} · {}",
                                entry.mapper_label(),
                                entry.region_label()
                            ),
                            FontId::proportional(11.0),
                            Color32::from_gray(160),
                        );
                    }
                });
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let mut settings = self.settings.clone();
        let mut tab = self.tab;
        let selected_entry = self
            .selected
            .and_then(|index| self.library.get(index))
            .cloned();

        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(760.0)
            .default_height(480.0)
            .resizable(true)
            .show(ctx, |ui| {
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
                            Tab::General => general_tab(ui, &mut settings, ctx),
                            Tab::Interface => interface_tab(ui, &mut settings),
                            Tab::Graphics => graphics_tab(ui, &mut settings),
                            Tab::Audio => audio_tab(ui, &mut settings),
                            Tab::Controllers => controllers_tab(ui, &mut settings),
                            Tab::Paths => paths_tab(ui, &mut settings),
                            Tab::Nes => nes_tab(ui, &mut settings, selected_entry.as_ref()),
                            Tab::Advanced => advanced_tab(ui),
                        }
                    });
                });
            });

        self.settings_open = open;
        self.tab = tab;
        if settings != self.settings {
            let old = self.settings.clone();
            self.settings = settings;
            self.apply_settings(ctx, &old);
        }
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
                ui.label("MIT licensed. NES backend: TetaNES (MIT/Apache-2.0).");
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

        if let Some(player) = &mut self.player {
            player.step(ctx);
        }

        if let Some(player) = &mut self.player {
            player.set_port_device(
                1,
                if self.settings.zapper_port2 {
                    unko_core::PortDevice::Zapper
                } else {
                    unko_core::PortDevice::Gamepad
                },
            );
            viewport(
                ctx,
                player,
                self.settings.integer_scale,
                self.settings.linear_filter,
                self.settings.zapper_port2,
            );
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
        self.about_window(ctx);
        self.open_rom_window(ctx);

        if let Some(entry) = self.pending_play.take() {
            self.start_entry(entry);
        }

        if self.player.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
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

fn viewport(
    ctx: &egui::Context,
    player: &mut Player,
    integer_scale: bool,
    linear_filter: bool,
    zapper: bool,
) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(Color32::BLACK))
        .show(ctx, |ui| {
            let available = ui.max_rect();
            let mut target = fit_aspect(available, 4.0 / 3.0);
            if integer_scale {
                let scale = (target.height() / 240.0).floor().max(1.0);
                let size = Vec2::new(240.0 * scale * 4.0 / 3.0, 240.0 * scale);
                target = Rect::from_center_size(available.center(), size);
            }
            if zapper {
                // Mouse aims and fires the Zapper attached to port 2. The
                // pointer is mapped into the 256x240 frame inside the scaled
                // viewport rect; leaving the rect keeps the trigger released.
                let position = ctx.pointer_latest_pos();
                let trigger = ctx.input(|input| input.pointer.primary_down());
                if let Some(position) = position.filter(|position| target.contains(*position)) {
                    let relative = (position - target.min) / target.size();
                    let x = (relative.x * 256.0).clamp(0.0, 255.0) as u8;
                    let y = (relative.y * 240.0).clamp(0.0, 239.0) as u8;
                    player.set_zapper(1, x, y, trigger);
                } else if trigger {
                    player.set_zapper(1, 0, 0, false);
                }
            }
            player.set_texture_options(if linear_filter {
                egui::TextureOptions::LINEAR
            } else {
                egui::TextureOptions::NEAREST
            });
            if let Some(texture) = player.texture() {
                let id = texture.id();
                ui.painter().image(
                    id,
                    target,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
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
        });
}

fn general_tab(ui: &mut egui::Ui, settings: &mut Settings, _ctx: &egui::Context) {
    ui.heading("General");
    ui.add_space(8.0);
    ui.checkbox(
        &mut settings.pause_on_focus_loss,
        "Pause when the window loses focus",
    );
    ui.checkbox(&mut settings.confirm_stop, "Confirm before stopping a game");
    ui.add_space(12.0);
    ui.label("Theme");
    ui.horizontal(|ui| {
        ui.radio_value(&mut settings.theme, Theme::Dark, "Dark");
        ui.radio_value(&mut settings.theme, Theme::Light, "Light");
    });
}

fn interface_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Interface");
    ui.add_space(8.0);
    ui.checkbox(&mut settings.show_status_bar, "Show status bar");
    ui.checkbox(&mut settings.grid_view, "Game library opens in grid view");
    ui.add_space(8.0);
    ui.label("Toolbar, menu and status layout follow Dolphin's desktop shell.");
}

fn graphics_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Graphics");
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
    ui.add_space(8.0);
    ui.checkbox(&mut settings.integer_scale, "Integer scaling when possible");
    ui.checkbox(
        &mut settings.linear_filter,
        "Smooth (linear) texture filtering",
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new("Filter changes apply to the running game immediately.")
            .weak()
            .small(),
    );
}

fn audio_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Audio");
    ui.add_space(8.0);
    egui::ComboBox::from_label("Sample rate")
        .selected_text(format!("{:.0} Hz", settings.sample_rate))
        .show_ui(ui, |ui| {
            for rate in [44_100.0, 48_000.0, 96_000.0] {
                ui.selectable_value(&mut settings.sample_rate, rate, format!("{rate:.0} Hz"));
            }
        });
    ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).text("Volume"));
    ui.checkbox(&mut settings.muted, "Mute");
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "The core produces f32 samples today; device playback is the next frontend step.",
        )
        .weak()
        .small(),
    );
}

fn controllers_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Controllers");
    ui.add_space(8.0);
    ui.checkbox(
        &mut settings.zapper_port2,
        "Port 2: NES Zapper (mouse aims, left button fires)",
    );
    ui.label(
        RichText::new(
            "Zapper changes apply to the next game started; the crosshair follows the mouse inside the viewport.",
        )
        .weak()
        .small(),
    );
    ui.add_space(8.0);
    ui.label("NES port 1 (keyboard)");
    ui.add_space(4.0);
    egui::Grid::new("controller-map")
        .num_columns(2)
        .spacing([24.0, 6.0])
        .show(ui, |ui| {
            for (button, key) in [
                ("D-pad", "Arrow keys"),
                ("A", "X"),
                ("B", "Z"),
                ("Start", "Enter"),
                ("Select", "Shift"),
            ] {
                ui.label(button);
                ui.label(RichText::new(key).strong());
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    ui.label(
        RichText::new("Gamepad mapping and rebinding are planned for the next milestone.")
            .weak()
            .small(),
    );
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
        if ui.button("Browse…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(&settings.save_dir)
                .pick_folder()
            {
                settings.save_dir = path;
            }
        }
    });
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

fn advanced_tab(ui: &mut egui::Ui) {
    ui.heading("Advanced");
    ui.add_space(8.0);
    ui.label("Core: TetaNES (adapted) — MIT OR Apache-2.0");
    ui.label("State format: core-owned, versioned by the backend");
    ui.label("Settings file: unko.conf");
    ui.add_space(8.0);
    ui.label(
        RichText::new("Logging, run-ahead and rewind controls land with the next milestone.")
            .weak()
            .small(),
    );
}
