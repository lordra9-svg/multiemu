//! ROM Manager window: DAT audit, 1G1R selection and safe rebuilding.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

use egui::{Color32, RichText};
use unko_rom_manager::{
    parse_dat_path, rebuild, scan_path, summary_line, write_csv, write_fixdat, Catalog,
    MatchReport, MatchStatus, OneGameOneRom, RebuildOptions, ScanOptions, ScanProgress,
};

type Job = std::result::Result<Vec<MatchReport>, String>;

/// ROM Manager state.
pub struct RomManager {
    /// Window visibility.
    pub open: bool,
    dat_paths: Vec<PathBuf>,
    dat_input: String,
    source: PathBuf,
    output: PathBuf,
    region_order: String,
    language_order: String,
    one_g1r: bool,
    include_bios: bool,
    prefer_parent: bool,
    prefer_verified: bool,
    prefer_retail: bool,
    prefer_newer: bool,
    filter: String,
    busy: bool,
    status: String,
    progress: Arc<Mutex<ScanProgress>>,
    rx: Option<Receiver<Job>>,
    reports: Vec<MatchReport>,
    selected: usize,
}

impl RomManager {
    /// Creates an idle manager.
    #[must_use]
    pub fn new(save_dir: PathBuf, source: PathBuf) -> Self {
        Self {
            open: false,
            dat_paths: Vec::new(),
            dat_input: String::new(),
            source,
            output: save_dir.join("rom-manager").join("rebuilt"),
            region_order: "USA,Europe,World,Japan".into(),
            language_order: "En".into(),
            one_g1r: true,
            include_bios: false,
            prefer_parent: true,
            prefer_verified: true,
            prefer_retail: true,
            prefer_newer: true,
            filter: String::new(),
            busy: false,
            status: "Choose one or more DAT files and a source folder, then scan.".into(),
            progress: Arc::new(Mutex::new(ScanProgress::default())),
            rx: None,
            reports: Vec::new(),
            selected: 0,
        }
    }

    /// Adds DAT files and opens the window.
    pub fn preload(&mut self, dats: Vec<PathBuf>, open: bool) {
        if !dats.is_empty() {
            self.dat_paths = dats;
        }
        self.open |= open;
    }

    /// Polls the background scan.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(reports)) => {
                self.reports = reports;
                self.selected = 0;
                self.busy = false;
                self.rx = None;
                if let Some(report) = self.reports.first() {
                    self.status = summary_line(report);
                } else {
                    self.status = "No DAT produced a report.".into();
                }
            }
            Ok(Err(error)) => {
                self.busy = false;
                self.rx = None;
                self.status = error;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.busy = false;
                self.rx = None;
                self.status = "ROM Manager worker stopped".into();
            }
        }
    }

    /// Draws the window.
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut open = self.open;
        egui::Window::new("ROM Manager")
            .open(&mut open)
            .resizable(true)
            .default_width(920.0)
            .default_height(620.0)
            .show(ctx, |ui| self.body(ui));
        self.open = open;
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        ui.heading("ROM Manager");
        ui.label(
            RichText::new(
                "DAT audit, 1G1R selection and safe copying. The manager is a standalone core; it does not change the NES emulator.",
            )
            .weak()
            .small(),
        );
        ui.add_space(6.0);

        ui.label("DAT files");
        let mut remove = None;
        for (index, path) in self.dat_paths.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.monospace(path.display().to_string());
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.dat_paths.remove(index);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.dat_input)
                    .hint_text("path to .dat or .xml")
                    .desired_width(500.0),
            );
            if ui.button("Add path").clicked() && !self.dat_input.trim().is_empty() {
                self.dat_paths.push(PathBuf::from(self.dat_input.trim()));
                self.dat_input.clear();
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse DATs…").clicked() {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("DAT", &["dat", "xml"])
                    .pick_files()
                {
                    self.dat_paths.extend(paths);
                }
            }
        });

        ui.add_space(4.0);
        ui.label("Source folder");
        ui.horizontal(|ui| {
            let mut source = self.source.display().to_string();
            if ui.text_edit_singleline(&mut source).changed() {
                self.source = PathBuf::from(source);
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.source)
                    .pick_folder()
                {
                    self.source = path;
                }
            }
        });
        ui.label("Output folder");
        ui.horizontal(|ui| {
            let mut output = self.output.display().to_string();
            if ui.text_edit_singleline(&mut output).changed() {
                self.output = PathBuf::from(output);
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.output)
                    .pick_folder()
                {
                    self.output = path;
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.one_g1r, "1G1R selection");
            ui.checkbox(&mut self.include_bios, "Include [BIOS] entries");
        });
        ui.horizontal(|ui| {
            ui.add_enabled_ui(self.one_g1r, |ui| {
                ui.checkbox(&mut self.prefer_parent, "Prefer parent");
                ui.checkbox(&mut self.prefer_verified, "Prefer verified");
                ui.checkbox(&mut self.prefer_retail, "Prefer retail");
                ui.checkbox(&mut self.prefer_newer, "Prefer newer revisions");
            });
        });
        ui.horizontal(|ui| {
            ui.label("Regions");
            ui.add(
                egui::TextEdit::singleline(&mut self.region_order)
                    .desired_width(260.0)
                    .hint_text("USA,Europe,World,Japan"),
            );
            ui.label("Languages");
            ui.add(
                egui::TextEdit::singleline(&mut self.language_order)
                    .desired_width(120.0)
                    .hint_text("En"),
            );
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can_scan =
                !self.busy && !self.dat_paths.is_empty() && !self.source.as_os_str().is_empty();
            if ui
                .add_enabled(can_scan, egui::Button::new("Scan / audit"))
                .clicked()
            {
                self.start_scan();
            }
            if self.busy {
                let progress = self
                    .progress
                    .lock()
                    .map(|progress| progress.clone())
                    .unwrap_or_default();
                ui.spinner();
                ui.label(format!(
                    "{} files, {} members, {:.1} MiB",
                    progress.files,
                    progress.members,
                    progress.bytes as f64 / (1024.0 * 1024.0)
                ));
            }
        });
        ui.label(&self.status);
        if self.busy {
            return;
        }
        let report_index = self.selected;
        let action = {
            let Some(report) = self.reports.get(report_index) else {
                return;
            };
            ui.add_space(4.0);
            if self.reports.len() > 1 {
                egui::ComboBox::from_id_salt("rom-manager-dat")
                    .selected_text(&report.dat.name)
                    .show_ui(ui, |ui| {
                        for (index, candidate) in self.reports.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, index, &candidate.dat.name);
                        }
                    });
            }
            ui.label(
                RichText::new(format!("{} ({})", report.dat.name, report.dat.version)).strong(),
            );
            ui.label(summary_line(report));
            let selected_games = self.selected_games(report_index);
            ui.label(format!(
                "1G1R: {} games selected ({} complete in the source)",
                selected_games.len(),
                selected_games
                    .iter()
                    .filter(|index| report
                        .games
                        .get(**index)
                        .is_some_and(|game| game.status == MatchStatus::Complete))
                    .count()
            ));

            let mut action = None;
            ui.horizontal(|ui| {
                if ui.button("Export fixdat…").clicked() {
                    action = Some(Action::Fixdat);
                }
                if ui.button("Export CSV…").clicked() {
                    action = Some(Action::Csv);
                }
                if ui.button("Rebuild selected…").clicked() {
                    action = Some(Action::Rebuild);
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("game name")
                        .desired_width(300.0),
                );
            });
            ui.separator();
            let filter = self.filter.to_lowercase();
            let mut order: Vec<usize> = (0..report.dat.games.len()).collect();
            order.sort_by_key(|index| match report.games[*index].status {
                MatchStatus::Complete => 0,
                MatchStatus::Partial => 1,
                MatchStatus::Missing => 2,
            });
            let mut shown = 0usize;
            egui::ScrollArea::vertical()
                .max_height(330.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("rom-manager-games")
                        .striped(true)
                        .num_columns(4)
                        .show(ui, |ui| {
                            ui.strong("Game");
                            ui.strong("Status");
                            ui.strong("ROMs");
                            ui.strong("Source");
                            ui.end_row();
                            for index in order.iter().copied() {
                                let game = &report.dat.games[index];
                                if !filter.is_empty() && !game.name.to_lowercase().contains(&filter)
                                {
                                    continue;
                                }
                                if shown >= 500 {
                                    break;
                                }
                                shown += 1;
                                let status = report.games[index];
                                let color = match status.status {
                                    MatchStatus::Complete => Color32::LIGHT_GREEN,
                                    MatchStatus::Partial => Color32::YELLOW,
                                    MatchStatus::Missing => Color32::GRAY,
                                };
                                ui.label(&game.name);
                                ui.colored_label(color, status.status.label());
                                ui.label(format!("{}/{}", status.matched, status.total));
                                let source = game
                                    .roms
                                    .iter()
                                    .enumerate()
                                    .find_map(|(rom, _)| {
                                        report
                                            .flat_index(index, rom)
                                            .and_then(|flat| report.source(flat))
                                            .map(|rom| rom.origin.display())
                                    })
                                    .unwrap_or_default();
                                ui.label(RichText::new(source).small().weak());
                                ui.end_row();
                            }
                        });
                    if shown >= 500 {
                        ui.label(RichText::new("Showing the first 500 matching games.").weak());
                    }
                });
            action
        };
        match action {
            Some(Action::Fixdat) => self.export_fixdat(report_index),
            Some(Action::Csv) => self.export_csv(report_index),
            Some(Action::Rebuild) => {
                let selected = self.selected_games(report_index);
                self.rebuild(report_index, &selected);
            }
            None => {}
        }
    }

    /// Starts a scan when DAT files and a source folder are present.
    pub fn start_scan(&mut self) {
        if self.busy {
            return;
        }
        let dats = self.dat_paths.clone();
        let source = self.source.clone();
        let options = ScanOptions::default();
        let progress = Arc::clone(&self.progress);
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.busy = true;
        self.status = "Scanning…".into();
        thread::spawn(move || {
            let result = (|| -> Job {
                let mut report_progress = |scan: &ScanProgress| {
                    if let Ok(mut current) = progress.lock() {
                        *current = scan.clone();
                    }
                };
                let (scanned, _) = scan_path(&source, &options, &mut report_progress)
                    .map_err(|err| err.to_string())?;
                let mut reports = Vec::new();
                for path in dats {
                    let dat = parse_dat_path(&path)
                        .map_err(|err| format!("{}: {err}", path.display()))?;
                    reports.push(Catalog::new(dat).match_all(scanned.clone()));
                }
                Ok(reports)
            })();
            let _ = tx.send(result);
        });
    }

    fn selected_games(&self, report_index: usize) -> Vec<usize> {
        let Some(report) = self.reports.get(report_index) else {
            return Vec::new();
        };
        if !self.one_g1r {
            return (0..report.dat.games.len()).collect();
        }
        OneGameOneRom {
            regions: split_list(&self.region_order),
            languages: split_list(&self.language_order),
            prefer_parent: self.prefer_parent,
            prefer_verified: self.prefer_verified,
            prefer_retail: self.prefer_retail,
            prefer_newer: self.prefer_newer,
            exclude_bios: !self.include_bios,
        }
        .select(&report.dat)
    }

    fn export_fixdat(&mut self, report_index: usize) {
        let Some(report) = self.reports.get(report_index) else {
            return;
        };
        let path = self.suggested_path(report_index, "fixdat.xml");
        let result = write_fixdat(report, &path);
        self.status = match result {
            Ok(()) => format!("Fixdat written to {}", path.display()),
            Err(err) => err.to_string(),
        };
    }

    fn export_csv(&mut self, report_index: usize) {
        let Some(report) = self.reports.get(report_index) else {
            return;
        };
        let path = self.suggested_path(report_index, "report.csv");
        let result = write_csv(report, &path);
        self.status = match result {
            Ok(()) => format!("CSV written to {}", path.display()),
            Err(err) => err.to_string(),
        };
    }

    fn rebuild(&mut self, report_index: usize, selected: &[usize]) {
        let Some(report) = self.reports.get(report_index) else {
            return;
        };
        let result = rebuild(
            report,
            selected,
            &RebuildOptions {
                output: self.output.clone(),
                ..RebuildOptions::default()
            },
        );
        self.status = format!(
            "Rebuilt {} files ({} skipped, {} errors) into {}",
            result.written,
            result.skipped,
            result.errors.len(),
            self.output.display()
        );
        if let Some(error) = result.errors.first() {
            self.status.push_str(&format!(" — {error}"));
        }
    }

    fn suggested_path(&self, report_index: usize, suffix: &str) -> PathBuf {
        let stem = self
            .reports
            .get(report_index)
            .map_or("dat".into(), |report| sanitize(&report.dat.name));
        self.output
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            .join(format!("{stem}.{suffix}"))
    }
}

enum Action {
    Fixdat,
    Csv,
    Rebuild,
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn sanitize(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' ' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    while out.ends_with([' ', '.']) {
        out.pop();
    }
    if out.is_empty() {
        "dat".into()
    } else {
        out
    }
}
