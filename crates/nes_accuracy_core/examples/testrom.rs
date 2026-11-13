//! Runs one test ROM and prints its console (`$6000`) result.
//!
//! Usage: `cargo run -p nes_accuracy_core --example testrom -- <rom> [frames]`
//!
//! Blargg-style test ROMs write a status byte and ASCII result text to the
//! cartridge's work RAM at `$6000`. The probe polls that window each frame and
//! stops as soon as "Passed" or "Failed" appears, or at the frame budget. It
//! also prints the `$F0`/`$F8` completion bytes and the number of distinct
//! colors in the final frame, which covers tests that report on screen.

use std::env;
use std::fs;

use nes_accuracy_core::System;

fn result_text(system: &System) -> String {
    let mut text = String::new();
    for address in 0x6000..0x6200 {
        let byte = system.cart.cpu_read(address);
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

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: testrom <rom> [frames]");
    let budget: u32 = args
        .next()
        .map(|value| value.parse().expect("frames must be a number"))
        .unwrap_or(600);
    let rom = fs::read(&path).expect("read rom");
    let mut system = System::from_ines(&rom).expect("load rom");
    if std::env::var_os("UNKO_MMC3_ALT").is_some() {
        system.set_mmc3_alternate(true);
    }
    let mut text = String::new();
    let mut ran = 0;
    for _ in 0..budget {
        system.run_frame();
        ran += 1;
        text = result_text(&system);
        let lower = text.to_lowercase();
        if lower.contains("passed") || lower.contains("failed") {
            break;
        }
    }
    let colors = {
        let frame = system.video();
        frame
            .chunks(4)
            .map(|pixel| (pixel[0], pixel[1], pixel[2]))
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    println!(
        "frames {ran}/{budget} pc {:04X} cycles {} halted {} f0 {:02X} f8 {:02X} colors {colors}",
        system.cpu.registers.pc,
        system.cpu.cycles,
        system.cpu.halted,
        system.read_ram(0x00F0),
        system.read_ram(0x00F8),
    );
    println!("6000: {}", text.trim());
}
