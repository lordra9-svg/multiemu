//! APU test harness: CPU + APU + flat NROM/PPU stub.
//!
//! Test ROMs are not committed; place them in `test-roms/`.

use std::fs;
use std::path::Path;

use nes_accuracy_core::{Apu, Cpu, CpuBus};

struct ApuBus {
    prg: Vec<u8>,
    ram: [u8; 0x800],
    wram: [u8; 0x2000],
    apu: Apu,
    cycles: u64,
    vblank: bool,
}

impl ApuBus {
    fn new(rom: &[u8]) -> Self {
        let prg_len = usize::from(rom[4]) * 16 * 1024;
        Self {
            prg: rom[16..16 + prg_len].to_vec(),
            ram: [0; 0x800],
            wram: [0; 0x2000],
            apu: Apu::new(),
            cycles: 0,
            vblank: false,
        }
    }

    fn step(&mut self) {
        self.cycles += 1;
        let phase = self.cycles % 29_780;
        if phase == 27_480 {
            self.vblank = true;
        }
        // Keep the DMC fed from cartridge memory for the basic tests.
        if self.apu.dmc_needs_sample() {
            let address = self.apu.dmc.address;
            let byte = self.peek(address);
            self.apu.dmc_sample(byte);
        }
        self.apu.tick();
    }

    fn peek(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)],
            0x8000..=0xFFFF => {
                let offset = usize::from(address.wrapping_sub(0x8000)) % self.prg.len();
                self.prg[offset]
            }
            _ => 0,
        }
    }
}

impl CpuBus for ApuBus {
    fn read(&mut self, address: u16) -> u8 {
        self.step();
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x2000..=0x3FFF => match address & 7 {
                2 => {
                    let value = if self.vblank { 0x80 } else { 0 };
                    self.vblank = false;
                    value
                }
                _ => 0,
            },
            0x4015 => self.apu.read_status(),
            0x4016 | 0x4017 => 0,
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)],
            0x8000..=0xFFFF => {
                let offset = usize::from(address.wrapping_sub(0x8000)) % self.prg.len();
                self.prg[offset]
            }
            _ => 0,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        self.step();
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF] = value,
            0x4000..=0x4017 => {
                self.apu.write(address, value);
                if self.apu.dmc_needs_sample() {
                    let sample_address = self.apu.dmc.address;
                    let byte = self.peek(sample_address);
                    self.apu.dmc_sample(byte);
                }
            }
            0x6000..=0x7FFF => self.wram[usize::from(address - 0x6000)] = value,
            _ => {}
        }
    }

    fn tick(&mut self) {
        self.step();
    }

    fn irq_line(&self) -> bool {
        self.apu.irq_line()
    }
}

fn passed(bus: &ApuBus) -> bool {
    let text = result_text(bus).to_lowercase();
    bus.ram[0xF0] == 1
        || (bus.wram[1] == 0xDE && bus.wram[2] == 0xB0 && bus.wram[0] == 0)
        || (text.contains("passed") && !text.contains("failed"))
}

fn result_text(bus: &ApuBus) -> String {
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

fn run(name: &str, max_steps: u64) -> Option<(ApuBus, Cpu)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let rom = fs::read(root.join(name)).ok()?;
    let mut bus = ApuBus::new(&rom);
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
        if same > 2_000_000 {
            break;
        }
    }
    Some((bus, cpu))
}

#[test]
fn apu_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let mut checked = 0;
    for name in [
        "01.len_ctr.nes",
        "02.len_table.nes",
        "03.irq_flag.nes",
        "04.clock_jitter.nes",
        "05.len_timing_mode0.nes",
        "06.len_timing_mode1.nes",
        "07.irq_flag_timing.nes",
        "08.irq_timing.nes",
        "09.reset_timing.nes",
        "dmc_rates.nes",
        "dmc_basics.nes",
        "10.len_halt_timing.nes",
        "11.len_reload_timing.nes",
    ] {
        if !root.join(name).exists() {
            eprintln!("skipping {name}: not in test-roms/");
            continue;
        }
        let (bus, cpu) = run(name, 40_000_000).expect("rom present");
        println!(
            "--- {name} --- pc {:04X} cyc {} result {:02X}",
            cpu.registers.pc, cpu.cycles, bus.ram[0xF0]
        );
        assert!(
            passed(&bus),
            "{name} failed with result {:02X}",
            bus.ram[0xF0]
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("skipping APU suite: no test ROMs in test-roms/");
    }
}

#[test]
#[ignore = "APU baseline probe"]
fn apu_probe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    for name in [
        "len_ctr.nes",
        "len_table.nes",
        "irq_flag.nes",
        "irq_timing.nes",
        "square.nes",
        "noise.nes",
        "dmc_basics.nes",
        "dmc_latency.nes",
        "dmc_rates.nes",
        "dmc_status.nes",
        "dmc_status_irq.nes",
        "dmc_pitch.nes",
        "sweep_cutoff.nes",
        "sweep_sub.nes",
        "lin_ctr.nes",
        "noise_pitch.nes",
        "square_pitch.nes",
        "01.len_ctr.nes",
        "02.len_table.nes",
        "03.irq_flag.nes",
        "04.clock_jitter.nes",
    ] {
        if !root.join(name).exists() {
            continue;
        }
        let (bus, cpu) = run(name, 40_000_000).expect("rom present");
        println!(
            "{name}: pc {:04X} cyc {} F8 {:02X} zp {:02X?} wram {:02X?} text {:?}",
            cpu.registers.pc,
            cpu.cycles,
            bus.ram[0xF8],
            &bus.ram[0xEF..0xFA],
            &bus.wram[..8],
            result_text(&bus)
        );
    }
}
