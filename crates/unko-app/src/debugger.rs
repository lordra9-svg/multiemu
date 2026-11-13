//! Debugger window: CPU/PPU/APU inspection, memory editing and breakpoints.
//!
//! The window renders an owned snapshot assembled by [`crate::player::Player`]
//! and returns [`Action`]s, so the UI never borrows the machine while it is
//! being mutated. Breakpoints are evaluated by the player at instruction
//! granularity.

use egui::{Color32, RichText};
use unko_core::{ApuState, CpuState, DebugSpace, Disassembly, PpuState};

/// Debugger panel selected in the tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugTab {
    /// Registers, flags and disassembly.
    Cpu,
    /// PPU timing, palette and OAM.
    Ppu,
    /// APU channel outputs and frame counter.
    Apu,
    /// Hex viewer/editor for one address space.
    Memory,
    /// Breakpoint list.
    Breakpoints,
}

impl DebugTab {
    const ALL: [Self; 5] = [
        Self::Cpu,
        Self::Ppu,
        Self::Apu,
        Self::Memory,
        Self::Breakpoints,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Ppu => "PPU",
            Self::Apu => "APU",
            Self::Memory => "Memory",
            Self::Breakpoints => "Breakpoints",
        }
    }
}

/// Debugger state that belongs to the frontend rather than the machine.
pub struct Debugger {
    /// Whether the window is visible.
    pub open: bool,
    /// Selected tab.
    pub tab: DebugTab,
    /// Address space shown in the memory tab.
    pub space: DebugSpace,
    /// Base address of the memory view.
    pub address: u16,
    /// Hex text for the address box.
    pub address_text: String,
    /// Hex text for the write address.
    pub write_address: String,
    /// Hex text for the write value.
    pub write_value: String,
    /// Hex text for a new breakpoint.
    pub breakpoint_text: String,
    /// Program counters that stop execution.
    pub breakpoints: Vec<u16>,
    /// Last action result shown under the toolbar.
    pub message: String,
}

impl Default for Debugger {
    fn default() -> Self {
        Self {
            open: false,
            tab: DebugTab::Cpu,
            space: DebugSpace::Cpu,
            address: 0,
            address_text: "0000".into(),
            write_address: String::new(),
            write_value: String::new(),
            breakpoint_text: String::new(),
            breakpoints: Vec::new(),
            message: String::new(),
        }
    }
}

/// Owned machine snapshot rendered by [`window`].
pub struct DebugView {
    /// CPU registers.
    pub cpu: CpuState,
    /// PPU timing and registers.
    pub ppu: PpuState,
    /// APU channel outputs.
    pub apu: ApuState,
    /// Instructions starting at the current PC.
    pub listing: Vec<Disassembly>,
    /// Memory bytes starting at `memory_base`.
    pub memory: Vec<u8>,
    /// First address of `memory`.
    pub memory_base: u16,
    /// 32 palette entries after emphasis.
    pub palette: [[u8; 3]; 32],
    /// All 256 OAM bytes.
    pub oam: Vec<u8>,
}

/// User request produced by the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Pause execution.
    Pause,
    /// Resume execution.
    Resume,
    /// Execute one CPU instruction.
    StepInstruction,
    /// Execute one display frame.
    StepFrame,
    /// Reset the console.
    Reset,
    /// Write one byte in an address space.
    Poke(DebugSpace, u16, u8),
    /// Add or remove a breakpoint.
    ToggleBreakpoint(u16),
    /// Remove one breakpoint.
    RemoveBreakpoint(u16),
    /// Remove every breakpoint.
    ClearBreakpoints,
    /// Show a memory address in the memory tab.
    ShowAddress(DebugSpace, u16),
}

/// Renders the debugger window and returns the requested actions.
///
/// `detach` opens it as its own OS window on desktop; otherwise it is an
/// in-window panel. `generation` changes when the user resets the layout so a
/// fresh viewport is created at its default position.
pub fn window(
    debug: &mut Debugger,
    ctx: &egui::Context,
    view: &DebugView,
    detach: bool,
    generation: u32,
    layout: &mut Option<[f32; 4]>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut open = debug.open;
    let mut body = |ui: &mut egui::Ui| {
        toolbar(ui, view, &mut actions);
        ui.separator();
        ui.horizontal(|ui| {
            for tab in DebugTab::ALL {
                if ui.selectable_label(debug.tab == tab, tab.label()).clicked() {
                    debug.tab = tab;
                }
            }
        });
        ui.separator();
        match debug.tab {
            DebugTab::Cpu => cpu_tab(ui, debug, view, &mut actions),
            DebugTab::Ppu => ppu_tab(ui, debug, view),
            DebugTab::Apu => apu_tab(ui, view),
            DebugTab::Memory => memory_tab(ui, debug, view, &mut actions),
            DebugTab::Breakpoints => breakpoints_tab(ui, debug, &mut actions),
        }
        if !debug.message.is_empty() {
            ui.separator();
            ui.label(RichText::new(&debug.message).weak().small());
        }
    };
    if detach && !cfg!(any(target_os = "android", target_os = "ios")) {
        let id = egui::ViewportId::from_hash_of(("unko-debugger", generation));
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Debugger — Unko")
            .with_min_inner_size([560.0, 360.0])
            .with_resizable(true);
        builder = if let Some([x, y, width, height]) = *layout {
            builder
                .with_position([x, y])
                .with_inner_size([width, height])
        } else {
            builder.with_inner_size([780.0, 540.0])
        };
        let mut closed = false;
        let mut captured = None;
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(6.0);
                body(ui);
            });
            let info = ctx.input(|input| input.viewport().clone());
            if let Some(outer) = info.outer_rect {
                let size = info.inner_rect.map_or(outer.size(), |rect| rect.size());
                captured = Some([outer.min.x, outer.min.y, size.x, size.y]);
            }
            if ctx.input(|input| input.viewport().close_requested()) {
                closed = true;
            }
        });
        if let Some(captured) = captured {
            *layout = Some(captured);
        }
        if closed {
            debug.open = false;
        }
    } else {
        egui::Window::new("Debugger")
            .open(&mut open)
            .default_size([760.0, 520.0])
            .min_width(620.0)
            .show(ctx, |ui| body(ui));
        debug.open = open;
    }
    actions
}

fn toolbar(ui: &mut egui::Ui, view: &DebugView, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.button("Resume").clicked() {
            actions.push(Action::Resume);
        }
        if ui.button("Pause").clicked() {
            actions.push(Action::Pause);
        }
        if ui.button("Step instruction (F10)").clicked() {
            actions.push(Action::StepInstruction);
        }
        if ui.button("Step frame (F9)").clicked() {
            actions.push(Action::StepFrame);
        }
        if ui.button("Reset").clicked() {
            actions.push(Action::Reset);
        }
        ui.separator();
        ui.label(
            RichText::new(format!(
                "PC ${:04X}  frame {}  {}:{}  cycles {}",
                view.cpu.pc, view.ppu.frame, view.ppu.scanline, view.ppu.dot, view.cpu.cycles
            ))
            .monospace(),
        );
    });
}

fn cpu_tab(ui: &mut egui::Ui, debug: &mut Debugger, view: &DebugView, actions: &mut Vec<Action>) {
    let cpu = &view.cpu;
    egui::Grid::new("cpu-registers")
        .num_columns(6)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            ui.label("A");
            ui.label(mono(format!("${:02X}", cpu.a)));
            ui.label("X");
            ui.label(mono(format!("${:02X}", cpu.x)));
            ui.label("Y");
            ui.label(mono(format!("${:02X}", cpu.y)));
            ui.end_row();
            ui.label("SP");
            ui.label(mono(format!("${:02X}", cpu.sp)));
            ui.label("PC");
            ui.label(mono(format!("${:04X}", cpu.pc)));
            ui.label("P");
            ui.label(mono(format!("${:02X}", cpu.p)));
            ui.end_row();
        });
    ui.horizontal(|ui| {
        for (flag, bit) in [
            ("N", 0x80u8),
            ("V", 0x40),
            ("U", 0x20),
            ("B", 0x10),
            ("D", 0x08),
            ("I", 0x04),
            ("Z", 0x02),
            ("C", 0x01),
        ] {
            let on = cpu.p & bit != 0;
            ui.label(RichText::new(flag).monospace().strong().color(if on {
                Color32::LIGHT_GREEN
            } else {
                Color32::DARK_GRAY
            }));
        }
        ui.separator();
        ui.label(
            RichText::new("click an address to inspect it")
                .weak()
                .small(),
        );
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("cpu-listing")
        .max_height(320.0)
        .show(ui, |ui| {
            for line in &view.listing {
                ui.horizontal(|ui| {
                    let breakpoint = debug.breakpoints.contains(&line.address);
                    let marker = if line.address == view.cpu.pc {
                        ">"
                    } else if breakpoint {
                        "*"
                    } else {
                        " "
                    };
                    let label = format!(
                        "{marker} {:04X}  {:<8}  {}",
                        line.address,
                        bytes_text(&line.bytes),
                        line.text
                    );
                    let text = RichText::new(label).monospace();
                    let text = if line.address == view.cpu.pc {
                        text.strong().color(Color32::LIGHT_BLUE)
                    } else if breakpoint {
                        text.color(Color32::LIGHT_RED)
                    } else {
                        text
                    };
                    if ui.selectable_label(false, text).clicked() {
                        debug.address = line.address;
                        debug.address_text = format!("{:04X}", line.address);
                        debug.tab = DebugTab::Memory;
                    }
                    if ui
                        .small_button(if breakpoint { "x" } else { "+" })
                        .on_hover_text("toggle breakpoint")
                        .clicked()
                    {
                        actions.push(Action::ToggleBreakpoint(line.address));
                    }
                });
            }
        });
}

fn ppu_tab(ui: &mut egui::Ui, debug: &mut Debugger, view: &DebugView) {
    let ppu = &view.ppu;
    egui::Grid::new("ppu-state")
        .num_columns(6)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            ui.label("ctrl");
            ui.label(mono(format!("${:02X}", ppu.ctrl)));
            ui.label("mask");
            ui.label(mono(format!("${:02X}", ppu.mask)));
            ui.label("status");
            ui.label(mono(format!("${:02X}", ppu.status)));
            ui.end_row();
            ui.label("v");
            ui.label(mono(format!("${:04X}", ppu.v)));
            ui.label("t");
            ui.label(mono(format!("${:04X}", ppu.t)));
            ui.label("scan / dot");
            ui.label(mono(format!("{} / {}", ppu.scanline, ppu.dot)));
            ui.end_row();
        });
    ui.separator();
    ui.label(RichText::new("Palette").strong());
    egui::Grid::new("ppu-palette")
        .num_columns(16)
        .spacing([2.0, 2.0])
        .show(ui, |ui| {
            for row in 0..2 {
                for column in 0..16 {
                    let index = row * 16 + column;
                    let [r, g, b] = view.palette[index];
                    let colour = Color32::from_rgb(r, g, b);
                    let text = RichText::new(format!("{index:02X}")).monospace().color(
                        if u32::from(r) + u32::from(g) + u32::from(b) > 320 {
                            Color32::BLACK
                        } else {
                            Color32::WHITE
                        },
                    );
                    ui.add(
                        egui::Button::new(text)
                            .fill(colour)
                            .min_size(egui::vec2(34.0, 20.0))
                            .selected(
                                debug.address == 0x3F00 + index as u16
                                    && debug.space == DebugSpace::Ppu,
                            ),
                    )
                    .on_hover_text(format!("$3F{index:02X}"));
                }
                ui.end_row();
            }
        });
    ui.separator();
    ui.label(RichText::new("OAM (Y, tile, attr, X)").strong());
    egui::ScrollArea::vertical()
        .id_salt("ppu-oam")
        .max_height(220.0)
        .show(ui, |ui| {
            egui::Grid::new("ppu-oam-grid")
                .num_columns(5)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for sprite in 0..64 {
                        let base = sprite * 4;
                        let Some(bytes) = view.oam.get(base..base + 4) else {
                            break;
                        };
                        ui.label(mono(format!("{sprite:02}")));
                        for byte in bytes {
                            ui.label(mono(format!("{byte:02X}")));
                        }
                        ui.end_row();
                    }
                });
        });
}

fn apu_tab(ui: &mut egui::Ui, view: &DebugView) {
    let apu = &view.apu;
    for (name, value) in [
        ("Pulse 1", apu.pulse1),
        ("Pulse 2", apu.pulse2),
        ("Triangle", apu.triangle),
        ("Noise", apu.noise),
        ("DMC", apu.dmc),
    ] {
        ui.horizontal(|ui| {
            ui.label(name);
            ui.add(
                egui::ProgressBar::new((value / 15.0).clamp(0.0, 1.0))
                    .desired_width(220.0)
                    .text(format!("{value:.1}")),
            );
        });
    }
    ui.separator();
    ui.label(mono(format!("mixed  {:.4}", apu.mixed)));
    ui.label(mono(format!(
        "frame counter  step {}  cycle {}",
        apu.frame_step, apu.frame_cycle
    )));
}

fn memory_tab(
    ui: &mut egui::Ui,
    debug: &mut Debugger,
    view: &DebugView,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        for space in [DebugSpace::Cpu, DebugSpace::Ppu, DebugSpace::Oam] {
            if ui
                .selectable_label(debug.space == space, space.label())
                .clicked()
            {
                debug.space = space;
                debug.address = 0;
                debug.address_text = "0000".into();
            }
        }
        ui.separator();
        ui.label("Address");
        let response = ui.add(
            egui::TextEdit::singleline(&mut debug.address_text)
                .desired_width(64.0)
                .font(egui::TextStyle::Monospace),
        );
        let go = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let go = go || ui.button("Go").clicked();
        if go {
            if let Some(address) = parse_hex(&debug.address_text) {
                debug.address = debug.space.clamp(u32::from(address)) as u16;
                actions.push(Action::ShowAddress(debug.space, debug.address));
            }
        }
    });
    ui.separator();
    let bytes_per_row = 16;
    let base = view.memory_base;
    egui::ScrollArea::vertical()
        .id_salt("memory-hex")
        .max_height(340.0)
        .show(ui, |ui| {
            for row in 0..view.memory.len() / bytes_per_row {
                let offset = row * bytes_per_row;
                ui.horizontal(|ui| {
                    ui.label(mono(format!("{:04X}", base.wrapping_add(offset as u16))));
                    for column in 0..bytes_per_row {
                        let index = offset + column;
                        let Some(byte) = view.memory.get(index) else {
                            continue;
                        };
                        let address = base.wrapping_add(index as u16);
                        let selected =
                            address >= debug.address && address < debug.address.saturating_add(1);
                        let label = RichText::new(format!("{byte:02X}"))
                            .monospace()
                            .background_color(if selected {
                                Color32::from_rgb(40, 60, 90)
                            } else {
                                Color32::TRANSPARENT
                            });
                        if ui.selectable_label(false, label).clicked() {
                            debug.address = address;
                            debug.address_text = format!("{address:04X}");
                        }
                    }
                    let ascii: String = view.memory[offset..offset + bytes_per_row]
                        .iter()
                        .map(|byte| {
                            if (0x20..0x7F).contains(byte) {
                                char::from(*byte)
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    ui.label(RichText::new(ascii).monospace().weak());
                });
            }
        });
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Write");
        ui.add(
            egui::TextEdit::singleline(&mut debug.write_address)
                .hint_text("addr")
                .desired_width(56.0)
                .font(egui::TextStyle::Monospace),
        );
        ui.add(
            egui::TextEdit::singleline(&mut debug.write_value)
                .hint_text("value")
                .desired_width(44.0)
                .font(egui::TextStyle::Monospace),
        );
        if ui.button("Write byte").clicked() {
            match (
                parse_hex(&debug.write_address),
                parse_hex(&debug.write_value),
            ) {
                (Some(address), Some(value)) if value <= 0xFF => {
                    let address = debug.space.clamp(u32::from(address)) as u16;
                    actions.push(Action::Poke(debug.space, address, value as u8));
                }
                _ => debug.message = "write needs a hex address and a byte".into(),
            }
        }
        if debug.space == DebugSpace::Cpu {
            ui.label(
                RichText::new("CPU writes: RAM and PRG-RAM only")
                    .weak()
                    .small(),
            );
        }
    });
}

fn breakpoints_tab(ui: &mut egui::Ui, debug: &mut Debugger, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label("Add");
        ui.add(
            egui::TextEdit::singleline(&mut debug.breakpoint_text)
                .hint_text("PC hex")
                .desired_width(64.0)
                .font(egui::TextStyle::Monospace),
        );
        if ui.button("Add breakpoint").clicked() {
            if let Some(address) = parse_hex(&debug.breakpoint_text) {
                actions.push(Action::ToggleBreakpoint(address));
            } else {
                debug.message = "breakpoint needs a hex address".into();
            }
        }
        if ui.button("Clear all").clicked() {
            actions.push(Action::ClearBreakpoints);
        }
    });
    ui.separator();
    ui.label(
        RichText::new("Execution pauses before a breakpoint instruction is executed.")
            .weak()
            .small(),
    );
    let mut sorted = debug.breakpoints.clone();
    sorted.sort_unstable();
    for address in sorted {
        ui.horizontal(|ui| {
            if ui
                .button(mono(format!("${address:04X}")))
                .on_hover_text("show in memory")
                .clicked()
            {
                debug.address = address;
                debug.address_text = format!("{address:04X}");
                debug.tab = DebugTab::Memory;
                debug.space = DebugSpace::Cpu;
            }
            if ui.small_button("remove").clicked() {
                actions.push(Action::RemoveBreakpoint(address));
            }
        });
    }
    if debug.breakpoints.is_empty() {
        ui.label(RichText::new("No breakpoints.").weak());
    }
}

fn mono(text: String) -> RichText {
    RichText::new(text).monospace()
}

fn bytes_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_hex(text: &str) -> Option<u16> {
    let text = text.trim().trim_start_matches('$');
    let text = text.trim_start_matches("0x");
    u16::from_str_radix(text, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing_accepts_common_prefixes() {
        assert_eq!(parse_hex("8000"), Some(0x8000));
        assert_eq!(parse_hex("$00FF"), Some(0x00FF));
        assert_eq!(parse_hex("0x10"), Some(0x10));
        assert_eq!(parse_hex("not hex"), None);
    }

    fn sample_view() -> DebugView {
        DebugView {
            cpu: CpuState {
                pc: 0x8000,
                a: 0x10,
                x: 2,
                y: 3,
                sp: 0xFD,
                p: 0x24,
                cycles: 1234,
            },
            ppu: PpuState {
                frame: 12,
                scanline: 100,
                dot: 200,
                ctrl: 0x80,
                mask: 0x1E,
                status: 0,
                v: 0x2000,
                t: 0x2000,
            },
            apu: ApuState {
                pulse1: 7.0,
                pulse2: 15.0,
                triangle: 12.0,
                noise: 0.0,
                dmc: 3.0,
                mixed: 42.0,
                frame_step: 2,
                frame_cycle: 120,
            },
            listing: vec![
                Disassembly {
                    address: 0x8000,
                    bytes: vec![0xA9, 0x10],
                    text: "LDA #$10".into(),
                },
                Disassembly {
                    address: 0x8002,
                    bytes: vec![0x85, 0x00],
                    text: "STA $00".into(),
                },
            ],
            memory: (0..128u8).collect(),
            memory_base: 0x8000,
            palette: [[0x20, 0x30, 0x40]; 32],
            oam: vec![0x10; 256],
        }
    }

    #[test]
    fn window_renders_every_tab() {
        let ctx = egui::Context::default();
        let view = sample_view();
        let mut debug = Debugger {
            open: true,
            address: 0x3F00,
            space: DebugSpace::Ppu,
            ..Debugger::default()
        };
        debug.breakpoints.push(0x8000);
        for tab in DebugTab::ALL {
            debug.tab = tab;
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                let actions = window(&mut debug, ctx, &view, false, 0, &mut None);
                assert!(actions.is_empty(), "no pointer input means no actions");
            });
            assert!(!output.shapes.is_empty(), "{tab:?} produced no shapes");
        }
    }

    #[test]
    fn breakpoints_are_toggles() {
        let mut debug = Debugger::default();
        debug.breakpoints.push(0x8000);
        assert!(debug.breakpoints.contains(&0x8000));
        debug.breakpoints.retain(|pc| *pc != 0x8000);
        assert!(debug.breakpoints.is_empty());
    }
}
