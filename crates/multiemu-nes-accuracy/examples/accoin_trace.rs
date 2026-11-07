use std::collections::BTreeMap;
use std::fs;

use multiemu_nes_accuracy::accuracycoin::parse_catalog;
use multiemu_nes_accuracy::System;

fn main() {
    let rom = fs::read("test-roms/AccuracyCoin.nes").expect("rom");
    let catalog = fs::read_to_string("test-roms/AccuracyCoin.tsv").expect("catalog");
    let entries = parse_catalog(&catalog);
    let names: BTreeMap<u16, String> = entries
        .iter()
        .map(|e| (e.address, format!("[{}] {}", e.suite, e.name)))
        .collect();

    let mut s = System::from_ines(&rom).expect("load");
    for _ in 0..120 {
        s.set_controller(0, 0);
        s.run_frame();
    }
    for _ in 0..4 {
        s.set_controller(0, 0x10);
        s.run_frame();
    }
    for _ in 0..4 {
        s.run_frame();
    }
    s.set_controller(0, 0x08);
    for _ in 0..4 {
        s.run_frame();
    }
    s.set_controller(0, 0);

    let mut prev: BTreeMap<u16, u8> = BTreeMap::new();
    let mut stall = 0u32;
    for frame in 0..6000u32 {
        s.run_frame();
        let pc = s.cpu.registers.pc;
        if (0xF270..=0xF290).contains(&pc) {
            stall += 1;
        } else {
            stall = 0;
        }
        if stall == 120 {
            println!(
                "STALL at frame {frame} pc {pc:04X} tally {:02X}",
                s.read_ram(0x37)
            );
        }
        for entry in &entries {
            let value = s.read_ram(entry.address);
            if value != 0 && prev.get(&entry.address).copied() != Some(value) {
                if prev.len() < 500 {
                    println!(
                        "f{frame:<5} tally {:02X} tab {:02X} cur {:02X} h {:02X} sp {:02X} {:04X} = {:02X} {}",
                        s.read_ram(0x37), s.read_ram(0x14), s.read_ram(0x16), s.read_ram(0x17), s.cpu.registers.sp,
                        entry.address, value,
                        names.get(&entry.address).cloned().unwrap_or_default()
                    );
                }
                prev.insert(entry.address, value);
            }
        }
        if stall > 400 {
            break;
        }
    }
}
