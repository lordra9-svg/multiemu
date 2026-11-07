//! Blargg-style NROM/MMC1 test runner.
//!
//! These ROMs are not committed; place them in `test-roms/`. The runner uses a
//! minimal PPU/APU stub and reads the ASCII result the test writes to `$6000`.

use std::fs;
use std::path::Path;

use multiemu_nes_accuracy::{Cpu, CpuBus};

struct TestBus {
    prg: Vec<u8>,
    ram: [u8; 0x800],
    wram: [u8; 0x2000],
    chr: [u8; 0x2000],
    mmc1: Mmc1,
    is_mmc1: bool,
    apu: Apu,
}

#[derive(Default)]
struct Apu {
    cycle: u64,
    step: usize,
    mode5: bool,
    inhibit: bool,
    frame_irq: bool,
    enabled: u8,
    lengths: [u8; 4],
    halt: [bool; 4],
}

impl Apu {
    fn tick(&mut self) {
        self.cycle += 1;
        let steps: &[u64] = if self.mode5 {
            &[7457, 14913, 22371, 37281, 37282]
        } else {
            &[7457, 14913, 22371, 29829, 29830]
        };
        if self.step < steps.len() && self.cycle == steps[self.step] {
            self.clock_step(self.step);
            self.step += 1;
            if self.step == steps.len() {
                self.step = 0;
                self.cycle = 0;
            }
        }
    }

    fn clock_step(&mut self, step: usize) {
        let clock_length = if self.mode5 {
            matches!(step, 0 | 2 | 3)
        } else {
            matches!(step, 1 | 3)
        };
        if clock_length {
            for channel in 0..4 {
                if !self.halt[channel] && self.lengths[channel] > 0 {
                    self.lengths[channel] -= 1;
                }
            }
        }
        if !self.mode5 && step == 3 && !self.inhibit {
            self.frame_irq = true;
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x4000 | 0x4004 | 0x4008 | 0x400C => {
                let channel = match address {
                    0x4000 => 0,
                    0x4004 => 1,
                    0x4008 => 2,
                    _ => 3,
                };
                self.halt[channel] = value & 0x20 != 0;
            }
            0x4003 | 0x4007 | 0x400B | 0x400F => {
                let channel = match address {
                    0x4003 => 0,
                    0x4007 => 1,
                    0x400B => 2,
                    _ => 3,
                };
                self.lengths[channel] = LENGTH_TABLE[usize::from(value >> 3)];
            }
            0x4015 => {
                self.enabled = value;
                for channel in 0..4 {
                    if value & (1 << channel) == 0 {
                        self.lengths[channel] = 0;
                    }
                }
            }
            0x4017 => {
                self.mode5 = value & 0x80 != 0;
                self.inhibit = value & 0x40 != 0;
                self.cycle = 0;
                self.step = 0;
                self.frame_irq = false;
            }
            _ => {}
        }
    }

    fn read_status(&mut self) -> u8 {
        let mut value = 0;
        for (channel, length) in self.lengths.iter().enumerate() {
            if *length > 0 && self.enabled & (1 << channel) != 0 {
                value |= 1 << channel;
            }
        }
        self.frame_irq = false;
        value
    }
}

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

#[derive(Default)]
struct Mmc1 {
    shift: u8,
    count: u8,
    control: u8,
    chr0: u8,
    chr1: u8,
    prg: u8,
}

impl TestBus {
    fn new(rom: &[u8]) -> Self {
        let prg_len = usize::from(rom[4]) * 16 * 1024;
        let chr_len = usize::from(rom[5]) * 8 * 1024;
        let mapper = (rom[7] & 0xF0) | (rom[6] >> 4);
        let mut bus = Self {
            prg: rom[16..16 + prg_len].to_vec(),
            ram: [0; 0x800],
            wram: [0; 0x2000],
            chr: [0; 0x2000],
            mmc1: Mmc1 {
                control: 0x0C,
                ..Mmc1::default()
            },
            is_mmc1: mapper == 1,
            apu: Apu::default(),
        };
        if chr_len > 0 {
            let start = 16 + prg_len;
            bus.chr[..chr_len.min(0x2000)]
                .copy_from_slice(&rom[start..start + chr_len.min(0x2000)]);
        }
        bus
    }

    fn prg_byte(&self, address: u16) -> u8 {
        let offset = usize::from(address.wrapping_sub(0x8000));
        if self.is_mmc1 {
            let bank_count = (self.prg.len() / 0x4000).max(1);
            let control = self.mmc1.control;
            let mode = (control >> 2) & 3;
            let bank = usize::from(self.mmc1.prg & 0x0F);
            let (first, second) = match mode {
                0 | 1 => {
                    let b = bank & !1;
                    (b, b + 1)
                }
                2 => (0, bank),
                _ => (bank, bank_count - 1),
            };
            if offset < 0x4000 {
                self.prg[(first % bank_count) * 0x4000 + offset]
            } else {
                self.prg[(second % bank_count) * 0x4000 + (offset - 0x4000)]
            }
        } else {
            self.prg[offset % self.prg.len()]
        }
    }

    fn write_mapper(&mut self, address: u16, value: u8) {
        if !self.is_mmc1 || address < 0x8000 {
            return;
        }
        if value & 0x80 != 0 {
            self.mmc1.shift = 0;
            self.mmc1.count = 0;
            self.mmc1.control |= 0x0C;
            return;
        }
        self.mmc1.shift |= (value & 1) << self.mmc1.count;
        self.mmc1.count += 1;
        if self.mmc1.count == 5 {
            let register = (address >> 13) & 3;
            match register {
                0 => self.mmc1.control = self.mmc1.shift & 0x1F,
                1 => self.mmc1.chr0 = self.mmc1.shift,
                2 => self.mmc1.chr1 = self.mmc1.shift,
                _ => self.mmc1.prg = self.mmc1.shift,
            }
            self.mmc1.shift = 0;
            self.mmc1.count = 0;
        }
    }
}

impl CpuBus for TestBus {
    fn read(&mut self, address: u16) -> u8 {
        self.apu.tick();
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x2000..=0x3FFF => match address & 7 {
                2 => 0x80, // vblank set
                7 => 0,
                _ => 0,
            },
            0x4015 => self.apu.read_status(),
            0x4016 | 0x4017 => 0,
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)],
            0x8000..=0xFFFF => self.prg_byte(address),
            _ => 0,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        self.apu.tick();
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF] = value,
            0x4000..=0x4017 => self.apu.write(address, value),
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)] = value,
            0x8000..=0xFFFF => self.write_mapper(address, value),
            _ => {}
        }
    }

    fn tick(&mut self) {
        self.apu.tick();
    }

    fn irq_line(&self) -> bool {
        self.apu.frame_irq
    }
}

fn result_text(bus: &TestBus) -> String {
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

fn run(name: &str, max_cycles: u64) -> Option<(bool, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let path = root.join(name);
    let rom = fs::read(path).ok()?;
    let mut bus = TestBus::new(&rom);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    while cpu.cycles < max_cycles {
        cpu.step(&mut bus);
        if cpu.cycles % 250_000 < 12 {
            let text = result_text(&bus);
            let lower = text.to_lowercase();
            if lower.contains("passed") || lower.contains("failed") {
                let passed = lower.contains("passed") && !lower.contains("failed");
                return Some((passed, text));
            }
        }
    }
    Some((false, format!("timeout: {}", result_text(&bus))))
}

fn check(name: &str, max_cycles: u64) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    if !root.join(name).exists() {
        eprintln!("skipping {name}: not in test-roms/");
        return;
    }
    let (passed, text) = run(name, max_cycles).expect("rom present");
    println!("--- {name} ---\n{text}");
    assert!(passed, "{name} failed:\n{text}");
}

#[test]
fn blargg_instr_timing() {
    check("instr_timing.nes", 80_000_000);
}

#[test]
#[ignore = "needs PPU/APU IRQ timing from A2/A3"]
fn blargg_timing_test() {
    check("timing_test.nes", 80_000_000);
}

#[test]
fn blargg_instr_basics() {
    check("instr_basics.nes", 20_000_000);
}

#[test]
#[ignore = "needs PPU NMI/DMA timing from A2"]
fn blargg_interrupts() {
    for name in [
        "int_branch_delays_irq.nes",
        "int_cli_latency.nes",
        "int_irq_and_dma.nes",
        "int_nmi_and_brk.nes",
        "int_nmi_and_irq.nes",
    ] {
        check(name, 20_000_000);
    }
}
