//! PPU system harness for the blargg `vbl_nmi` suite.
//!
//! Test ROMs are not committed; place them in `test-roms/`. The suite records
//! which ROMs pass today and keeps the remaining timing races explicit.

use std::fs;
use std::path::Path;

use nes_accuracy_core::{Cpu, CpuBus, Ppu, Timing};

struct SystemBus {
    prg: Vec<u8>,
    ram: [u8; 0x800],
    wram: [u8; 0x2000],
    ppu: Ppu,
    before_dots: u32,
    stall: u32,
}

impl SystemBus {
    fn new(rom: &[u8]) -> Self {
        let prg_len = usize::from(rom[4]) * 16 * 1024;
        let chr_len = usize::from(rom[5]) * 8 * 1024;
        let prg = rom[16..16 + prg_len].to_vec();
        let chr_start = 16 + prg_len;
        let mut chr = vec![0u8; 0x2000];
        let chr_ram = chr_len == 0;
        if chr_len > 0 {
            let len = chr_len.min(0x2000);
            chr[..len].copy_from_slice(&rom[chr_start..chr_start + len]);
        }
        Self {
            prg,
            ram: [0; 0x800],
            wram: [0; 0x2000],
            ppu: Ppu::new(Timing::NTSC, chr, true, chr_ram),
            before_dots: 2,
            stall: 0,
        }
    }

    fn prg_byte(&self, address: u16) -> u8 {
        let offset = usize::from(address.wrapping_sub(0x8000));
        self.prg[offset % self.prg.len()]
    }

    fn peek(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)],
            0x8000..=0xFFFF => self.prg_byte(address),
            _ => 0,
        }
    }
}

impl CpuBus for SystemBus {
    fn read(&mut self, address: u16) -> u8 {
        self.ppu.step_dots(self.before_dots);
        let value = match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x2000..=0x3FFF => self.ppu.read_register(address),
            0x4015 => 0,
            0x4016 | 0x4017 => 0,
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)],
            0x8000..=0xFFFF => self.prg_byte(address),
            _ => 0,
        };
        self.ppu.step_dots(3 - self.before_dots);
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        self.ppu.step_dots(self.before_dots);
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF] = value,
            0x2000..=0x3FFF => self.ppu.write_register(address, value),
            0x4014 => {
                let page = u16::from(value) << 8;
                let start = self.ppu.oam_addr;
                for index in 0..256u16 {
                    let target = start.wrapping_add(index as u8);
                    self.ppu.oam[usize::from(target)] = self.peek(page + index);
                }
                self.stall = 513;
                self.ppu.step_dots(513 * 3);
            }
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)] = value,
            0x8000..=0xFFFF => {}
            _ => {}
        }
        self.ppu.step_dots(3 - self.before_dots);
    }

    fn tick(&mut self) {
        self.ppu.step_dots(3);
    }

    fn nmi_line(&self) -> bool {
        self.ppu.nmi_line
    }

    fn take_nmi(&mut self) -> bool {
        if self.ppu.nmi_pending {
            self.ppu.nmi_pending = false;
            true
        } else {
            false
        }
    }

    fn nmi_suppressed(&self) -> bool {
        self.ppu.nmi_suppressed
    }

    fn take_stall(&mut self) -> u32 {
        let stall = self.stall;
        self.stall = 0;
        stall
    }
}

fn result_text(bus: &SystemBus) -> String {
    let mut text = String::new();
    for &byte in &bus.wram[..0x0200] {
        if byte == 0 || !byte.is_ascii() {
            if text.ends_with('\n') {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            continue;
        }
        text.push(char::from(byte));
    }
    text
}

/// Runs until the ROM falls into its completion trap or the step budget ends.
fn run(name: &str, max_steps: u64) -> Option<(SystemBus, Cpu)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let rom = fs::read(root.join(name)).ok()?;
    let mut bus = SystemBus::new(&rom);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let mut last_pc = cpu.registers.pc;
    let mut same = 0u32;
    for _ in 0..max_steps {
        cpu.step(&mut bus);
        if cpu.registers.pc == last_pc {
            same += 1;
        } else {
            same = 0;
            last_pc = cpu.registers.pc;
        }
        if same > 1_000_000 {
            break;
        }
    }
    Some((bus, cpu))
}

fn passed(bus: &SystemBus) -> bool {
    let text = result_text(bus).to_lowercase();
    (text.contains("passed") && !text.contains("failed"))
        || (text.is_empty() && bus.ram[0xF8] == 1)
        || (text.is_empty() && bus.ram[0xF0] == 1)
}

const ALL_TESTS: [&str; 12] = [
    "vbl_nmi_basics.nes",
    "vbl_nmi_clear_timing.nes",
    "vbl_nmi_control.nes",
    "vbl_nmi_disable.nes",
    "vbl_nmi_even_odd_frames.nes",
    "vbl_nmi_even_odd_timing.nes",
    "vbl_nmi_frame_basics.nes",
    "vbl_nmi_off_timing.nes",
    "vbl_nmi_on_timing.nes",
    "vbl_nmi_set_time.nes",
    "vbl_nmi_suppression.nes",
    "vbl_nmi_timing.nes",
];

#[test]
fn vbl_nmi_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let mut checked = 0;
    for name in ALL_TESTS {
        if !root.join(name).exists() {
            eprintln!("skipping {name}: not in test-roms/");
            continue;
        }
        let (bus, cpu) = run(name, 15_000_000).expect("rom present");
        let text = result_text(&bus);
        println!(
            "--- {name} --- pc {:04X} cyc {} F8 {:02X}\n{text}",
            cpu.registers.pc, cpu.cycles, bus.ram[0xF8]
        );
        assert!(passed(&bus), "{name} regressed:\n{text}");
        checked += 1;
    }
    assert!(checked > 0, "no vbl_nmi test ROMs found in test-roms/");
}

const SPRITE_PASSING: [&str; 19] = [
    "spr_hit_alignment.nes",
    "spr_hit_basics.nes",
    "spr_hit_corners.nes",
    "spr_hit_double_height.nes",
    "spr_hit_edge_timing.nes",
    "spr_hit_flip.nes",
    "spr_hit_left_clip.nes",
    "spr_hit_right_edge.nes",
    "spr_hit_screen_bottom.nes",
    "spr_hit_timing_basics.nes",
    "spr_hit_timing_order.nes",
    "spr_overflow_basics.nes",
    "spr_overflow_details.nes",
    "spr_overflow_obscure.nes",
    "oam_read.nes",
    "oam_stress.nes",
    "spr_overflow_emulator.nes",
    "spr_overflow_timing.nes",
    "sprite_ram.nes",
];

#[test]
fn sprite_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let mut checked = 0;
    for name in SPRITE_PASSING {
        if !root.join(name).exists() {
            eprintln!("skipping {name}");
            continue;
        }
        let (bus, cpu) = run(name, 30_000_000).expect("rom present");
        let text = result_text(&bus);
        println!(
            "--- {name} --- pc {:04X} cyc {} F8 {:02X}\n{text}",
            cpu.registers.pc, cpu.cycles, bus.ram[0xF8]
        );
        assert!(passed(&bus), "{name} regressed:\n{text}");
        checked += 1;
    }
    assert!(checked > 0, "no sprite ROMs found in test-roms/");
}
