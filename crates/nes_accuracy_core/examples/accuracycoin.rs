//! AccuracyCoin battery runner (A5).
//!
//! Usage:
//! `cargo run -p nes_accuracy_core --example accuracycoin -- \
//!      test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv`

use std::env;
use std::fs;

use nes_accuracy_core::accuracycoin::{parse_catalog, run_accuracycoin, run_accuracycoin_isolated};
use nes_accuracy_core::System;

fn main() {
    let rom_path = env::args().nth(1).expect("AccuracyCoin.nes path");
    let catalog_path = env::args()
        .nth(2)
        .unwrap_or_else(|| "test-roms/AccuracyCoin.tsv".into());
    let rom = fs::read(&rom_path).expect("read rom");
    let catalog = fs::read_to_string(&catalog_path).expect("read catalog");
    let entries = parse_catalog(&catalog);
    assert!(!entries.is_empty(), "empty catalog");

    let args: Vec<String> = env::args().collect();
    let isolated = args.iter().any(|arg| arg == "--isolated");
    let limit = args
        .iter()
        .position(|arg| arg == "--limit")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<usize>().ok());
    let only = args
        .iter()
        .position(|arg| arg == "--only")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| u16::from_str_radix(value.trim_start_matches("0x"), 16).ok());
    let test_frames = args
        .iter()
        .position(|arg| arg == "--test-frames")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1200);
    let mut entries: Vec<_> = entries.into_iter().collect();
    if let Some(limit) = limit {
        entries.truncate(limit);
    }
    if let Some(only) = only {
        entries.retain(|entry| entry.address == only);
    }
    let mut system = System::from_ines(&rom).expect("load rom");
    let report = if isolated {
        let report = run_accuracycoin_isolated(&rom, &entries, 120, test_frames);
        system = System::from_ines(&rom).expect("load rom");
        for _ in 0..120 {
            system.run_frame();
        }
        report
    } else {
        run_accuracycoin(&mut system, &entries, 120, 6000)
    };

    println!(
        "AccuracyCoin: {}/{} passed, {} failed, {} skipped, {} unrun ({} frames)",
        report.passed,
        report.total(),
        report.failed,
        report.skipped,
        report.unrun,
        report.frames
    );
    println!(
        "final pc {:04X} halted {}",
        system.cpu.registers.pc, system.cpu.halted
    );
    println!(
        "ROM tallies: total {:02X} pass {:02X} running {:02X} post {:02X}",
        system.read_ram(0x37),
        system.read_ram(0x38),
        system.read_ram(0x35),
        system.read_ram(0x36)
    );
    let values: std::collections::BTreeMap<u16, u8> = report.results.iter().copied().collect();
    let mut suites: std::collections::BTreeMap<&str, (usize, usize, usize, usize)> =
        std::collections::BTreeMap::new();
    for entry in &entries {
        let result = values.get(&entry.address).copied().unwrap_or(0);
        let slot = suites.entry(entry.suite.as_str()).or_insert((0, 0, 0, 0));
        if entry.address < 0x0400 || result == 0xFF {
            slot.2 += 1;
        } else if result == 0 {
            slot.3 += 1;
        } else if result & 1 == 1 {
            slot.0 += 1;
        } else {
            slot.1 += 1;
        }
    }
    for (suite, (pass, fail, skip, unrun)) in suites {
        println!("  {suite:<42} {pass:>3} pass {fail:>3} fail {skip:>2} skip {unrun:>2} unrun");
    }
    for failure in report.failures.iter() {
        println!(
            "FAIL [{}] {} (code {})",
            failure.suite, failure.name, failure.code
        );
    }
    for entry in entries.iter() {
        if entry.address >= 0x0400 && values.get(&entry.address).copied().unwrap_or(0) == 0 {
            println!(
                "UNRUN [{}] {} @ {:04X}",
                entry.suite, entry.name, entry.address
            );
        }
    }

    let mut ppm = b"P6\n256 240\n255\n".to_vec();
    for pixel in system.video().as_chunks::<4>().0 {
        ppm.extend_from_slice(&pixel[..3]);
    }
    fs::write("/tmp/opencode/accuracycoin-result.ppm", ppm).expect("write ppm");
}
