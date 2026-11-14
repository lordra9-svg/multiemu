//! AccuracyCoin boot regression (A5 scaffold).
//!
//! The ROM is not committed; place `AccuracyCoin.nes` in `test-roms/`.
//! Full suite execution needs the correct menu input script and result parsing;
//! this test proves the ROM boots and renders its menu in the accuracy core.

use std::fs;
use std::path::Path;

use unko_nes::System;

#[test]
fn accuracycoin_boots_and_renders_menu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let path = root.join("AccuracyCoin.nes");
    if !path.exists() {
        eprintln!("skipping: test-roms/AccuracyCoin.nes missing");
        return;
    }
    let rom = fs::read(path).expect("rom");
    let mut system = System::from_ines(&rom).expect("load");
    assert_eq!(system.cart.mapper(), 0);
    for _ in 0..180 {
        system.run_frame();
    }
    // Rendering enabled, OAM populated, and the nametable contains menu text.
    assert_ne!(system.ppu.mask & 0x18, 0, "rendering disabled");
    assert!(system.ppu.oam.iter().any(|byte| *byte != 0));
    let text_tiles = system.ppu.vram.iter().filter(|tile| **tile != 0).count();
    assert!(text_tiles > 100, "menu nametable is empty");
    let frame = system.video();
    assert_eq!(frame.len(), 256 * 240 * 4);
    let first = &frame[0..3];
    assert!(
        frame.chunks(4).any(|pixel| pixel[0..3] != *first),
        "frame is uniform"
    );
    println!(
        "AccuracyCoin booted: {text_tiles} nametable tiles, mask {:02X}",
        system.ppu.mask
    );
}

#[test]
fn accuracycoin_runner_parses_results() {
    use unko_nes_dev::{parse_catalog, run_accuracycoin};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let rom_path = root.join("AccuracyCoin.nes");
    let catalog_path = root.join("AccuracyCoin.tsv");
    if !rom_path.exists() || !catalog_path.exists() {
        eprintln!("skipping: AccuracyCoin ROM or catalog missing");
        return;
    }
    let rom = fs::read(rom_path).expect("rom");
    let catalog = fs::read_to_string(catalog_path).expect("catalog");
    let entries = parse_catalog(&catalog);
    assert!(entries.len() > 140, "catalog too small");

    let mut system = System::from_ines(&rom).expect("load");
    let report = run_accuracycoin(&mut system, &entries, 120, 3000);
    println!(
        "AccuracyCoin: {}/{} pass, {} fail, {} skip, {} unrun",
        report.passed,
        report.total(),
        report.failed,
        report.skipped,
        report.unrun
    );
    assert_eq!(report.total(), entries.len());
    assert_eq!(report.skipped, 5, "the five DRAW rows should be skipped");
    assert!(report.passed > 0, "no tests passed");
    // This full-battery mode is the documented workaround path: a failing test
    // can corrupt the saved cursor and stall the $F278 text scanner, which
    // leaves later rows unrun instead of fail. Require that the runner records
    // the non-pass rows either way.
    assert!(
        report.failed > 0 || report.unrun > 0,
        "no failures or unrun rows recorded"
    );
    assert!(report.failures.iter().all(|failure| failure.code > 0));
}

#[test]
fn accuracycoin_isolated_runner_writes_every_row() {
    use unko_nes_dev::{parse_catalog, run_accuracycoin_isolated};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let rom_path = root.join("AccuracyCoin.nes");
    let catalog_path = root.join("AccuracyCoin.tsv");
    if !rom_path.exists() || !catalog_path.exists() {
        eprintln!("skipping: AccuracyCoin ROM or catalog missing");
        return;
    }
    let rom = fs::read(rom_path).expect("rom");
    let catalog = fs::read_to_string(catalog_path).expect("catalog");
    let entries: Vec<_> = parse_catalog(&catalog).into_iter().take(5).collect();
    let report = run_accuracycoin_isolated(&rom, &entries, 60, 300);
    println!(
        "isolated: {}/{} pass, {} fail, {} unrun",
        report.passed,
        report.total(),
        report.failed,
        report.unrun
    );
    assert_eq!(report.total(), entries.len());
    assert_eq!(report.unrun, 0, "navigation failed to run a test");
    assert_eq!(report.passed, 5, "page 1 first rows pass count changed");
    assert_eq!(report.failed, 0, "unexpected failure in page 1 first rows");
}
