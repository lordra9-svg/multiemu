//! Differential test against the `nestest` golden log.
//!
//! The ROM and log are not committed; place them in `test-roms/` at the
//! workspace root (they are distributed with TetaNES under its test data and
//! widely available as homebrew). The test is skipped when the files are absent.

use std::fs;
use std::path::Path;

use nes_accuracy_core::{Cpu, CpuBus, Status};

struct NestestBus {
    ram: [u8; 0x800],
    prg: Vec<u8>,
}

impl NestestBus {
    fn new(prg: Vec<u8>) -> Self {
        Self {
            ram: [0; 0x800],
            prg,
        }
    }
}

impl CpuBus for NestestBus {
    fn read(&mut self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x8000..=0xFFFF => {
                let offset = usize::from(address - 0x8000) % self.prg.len();
                self.prg[offset]
            }
            _ => 0,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        if address <= 0x1FFF {
            self.ram[usize::from(address) & 0x07FF] = value;
        }
    }

    fn tick(&mut self) {}
}

#[derive(Debug)]
struct Golden {
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    p: u8,
    cycles: u64,
}

fn parse_hex_byte(value: &str) -> Option<u8> {
    u8::from_str_radix(value, 16).ok()
}

fn parse_status(value: &str) -> Option<u8> {
    if value.len() == 2 {
        return parse_hex_byte(value);
    }
    let letters = value.as_bytes();
    if letters.len() != 8 {
        return None;
    }
    let flags = [
        (0, Status::NEGATIVE),
        (1, Status::OVERFLOW),
        (2, Status::UNUSED),
        (3, Status::BREAK),
        (4, Status::DECIMAL),
        (5, Status::INTERRUPT),
        (6, Status::ZERO),
        (7, Status::CARRY),
    ];
    let mut bits = 0;
    for (index, flag) in flags {
        if letters[index].is_ascii_uppercase() {
            bits |= flag;
        }
    }
    Some(bits)
}

fn parse_line(line: &str) -> Option<Golden> {
    if !line.starts_with('$') || line.len() < 5 {
        return None;
    }
    let pc = u16::from_str_radix(&line[1..5], 16).ok()?;
    let mut golden = Golden {
        pc,
        a: 0,
        x: 0,
        y: 0,
        sp: 0,
        p: 0,
        cycles: 0,
    };
    for token in line.split_whitespace() {
        if let Some(value) = token.strip_prefix("A:") {
            golden.a = parse_hex_byte(value)?;
        } else if let Some(value) = token.strip_prefix("X:") {
            golden.x = parse_hex_byte(value)?;
        } else if let Some(value) = token.strip_prefix("Y:") {
            golden.y = parse_hex_byte(value)?;
        } else if let Some(value) = token.strip_prefix("SP:") {
            golden.sp = parse_hex_byte(value)?;
        } else if let Some(value) = token.strip_prefix("P:") {
            golden.p = parse_status(value)?;
        } else if let Some(value) = token.strip_prefix("CYC:") {
            golden.cycles = value.parse().ok()?;
        }
    }
    Some(golden)
}

#[test]
fn nestest_matches_golden_log() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let rom_path = root.join("nestest.nes");
    let log_path = root.join("nestest.txt");
    if !rom_path.exists() || !log_path.exists() {
        eprintln!("skipping: test-roms/nestest.nes or nestest.txt is missing");
        return;
    }

    let rom = fs::read(&rom_path).expect("read nestest.nes");
    assert_eq!(&rom[0..4], b"NES\x1a", "not an iNES image");
    let prg_len = usize::from(rom[4]) * 16 * 1024;
    let prg = rom[16..16 + prg_len].to_vec();
    let log = fs::read_to_string(&log_path).expect("read nestest.txt");
    let goldens: Vec<Golden> = log.lines().filter_map(parse_line).collect();
    assert!(goldens.len() > 8000, "unexpected log size");

    let mut bus = NestestBus::new(prg);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    // The published log begins from the deterministic harness state, not the
    // post-reset stack pointer.
    cpu.registers.pc = 0xC000;
    cpu.registers.sp = 0xFD;

    for (index, golden) in goldens.iter().enumerate() {
        let line = index + 1;
        assert_eq!(
            cpu.registers.pc, golden.pc,
            "line {line}: PC {:04X} != {:04X}",
            cpu.registers.pc, golden.pc
        );
        assert_eq!(cpu.registers.a, golden.a, "line {line}: A");
        assert_eq!(cpu.registers.x, golden.x, "line {line}: X");
        assert_eq!(cpu.registers.y, golden.y, "line {line}: Y");
        assert_eq!(cpu.registers.sp, golden.sp, "line {line}: SP");
        assert_eq!(
            cpu.registers.p.bits() & !Status::BREAK,
            golden.p & !Status::BREAK,
            "line {line}: P {:02X} != {:02X}",
            cpu.registers.p.bits(),
            golden.p
        );
        assert_eq!(
            cpu.cycles, golden.cycles,
            "line {line}: CYC {} != {}",
            cpu.cycles, golden.cycles
        );
        let cycles = cpu.step(&mut bus);
        assert!(cycles > 0, "line {line}: instruction took no cycles");
    }
}
