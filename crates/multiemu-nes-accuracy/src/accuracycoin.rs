//! AccuracyCoin battery runner (A5).
//!
//! Boots the interactive ROM, presses Start while the cursor is on the page
//! index (which makes AccuracyCoin run every test), waits for the run to
//! finish, then reads the per-test result bytes out of CPU RAM.
//!
//! Result encoding (from the ROM's own source): `value & 1 == 1` passes,
//! `value & 1 == 0` fails with code `value >> 1`, and `$FF` means skipped.

use crate::system::System;

/// One AccuracyCoin catalog row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Suite (page) name.
    pub suite: String,
    /// Test name.
    pub name: String,
    /// CPU RAM address holding the result byte.
    pub address: u16,
    /// Zero-based page (suite) index in the ROM menu.
    pub page: usize,
    /// Zero-based row within the page.
    pub row: usize,
}

/// Parses the upstream `SOURCE_CATALOG.tsv` format.
///
/// Lines are `suite<TAB>name<TAB>0xADDR`. Blank lines and `#` comments are
/// ignored.
#[must_use]
pub fn parse_catalog(tsv: &str) -> Vec<CatalogEntry> {
    let mut entries: Vec<CatalogEntry> = Vec::new();
    let mut page = 0usize;
    let mut row = 0usize;
    let mut last_suite: Option<String> = None;
    for line in tsv.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(suite), Some(name), Some(address)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let address = address
            .trim()
            .strip_prefix("0x")
            .and_then(|hex| u16::from_str_radix(hex, 16).ok());
        if let Some(address) = address {
            if last_suite.as_deref() != Some(suite) {
                if last_suite.is_some() {
                    page += 1;
                }
                row = 0;
                last_suite = Some(suite.to_string());
            } else {
                row += 1;
            }
            entries.push(CatalogEntry {
                suite: suite.to_string(),
                name: name.to_string(),
                address,
                page,
                row,
            });
        }
    }
    entries
}

/// One failed test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedTest {
    /// Suite name.
    pub suite: String,
    /// Test name.
    pub name: String,
    /// Error code reported by the ROM.
    pub code: u8,
}

/// Outcome of a full battery run.
#[derive(Debug, Clone, Default)]
pub struct AccuracyCoinReport {
    /// Tests that passed.
    pub passed: usize,
    /// Tests that failed.
    pub failed: usize,
    /// Tests the ROM skipped.
    pub skipped: usize,
    /// Results that were never written (test did not run).
    pub unrun: usize,
    /// Failing tests with their error codes.
    pub failures: Vec<FailedTest>,
    /// Frames spent running the battery.
    pub frames: u32,
    /// Raw `(result address, value)` pairs, for harnesses that want per-test
    /// detail without another emulator instance.
    pub results: Vec<(u16, u8)>,
}

impl AccuracyCoinReport {
    /// Total catalog entries considered.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.passed + self.failed + self.skipped + self.unrun
    }
}

const START: u16 = 0x08;
const UP: u16 = 0x10;
const DOWN: u16 = 0x20;
const LEFT: u16 = 0x40;
const RIGHT: u16 = 0x80;
const A: u16 = 0x01;

/// Runs the whole AccuracyCoin battery.
///
/// `system` must already be loaded with the AccuracyCoin ROM and power-on
/// reset. The function assumes the ROM reaches its main menu within
/// `startup_frames`, nudges the cursor to the page index, presses Start, then
/// runs `max_frames` frames so the whole battery can finish.
pub fn run_accuracycoin(
    system: &mut System,
    entries: &[CatalogEntry],
    startup_frames: u32,
    max_frames: u32,
) -> AccuracyCoinReport {
    for _ in 0..startup_frames {
        system.set_controller(0, 0);
        system.run_frame();
    }
    // Move the cursor to the page index at the top of the page.
    for _ in 0..4 {
        system.set_controller(0, UP);
        system.run_frame();
    }
    system.set_controller(0, START);
    for _ in 0..4 {
        system.run_frame();
    }
    system.set_controller(0, 0);

    // Run a fixed budget. The ROM clears `RunningAllTests` mid-battery on some
    // emulators (a failing test corrupts the engine variable), so early-stopping
    // on it loses the later pages. 6000 frames is ~1.3x the full battery.
    let mut frames = 0u32;
    while frames < max_frames {
        system.run_frame();
        frames += 1;
    }

    let mut report = AccuracyCoinReport {
        frames,
        ..AccuracyCoinReport::default()
    };
    for entry in entries {
        // Addresses below $0400 are the ROM's print-only/DRAW rows; the engine
        // skips them rather than storing a result.
        if entry.address < 0x0400 {
            report.skipped += 1;
            continue;
        }
        let value = system.read_ram(entry.address);
        report.results.push((entry.address, value));
        match value {
            0xFF => report.skipped += 1,
            0 => report.unrun += 1,
            value if value & 1 == 1 => report.passed += 1,
            value => {
                report.failed += 1;
                report.failures.push(FailedTest {
                    suite: entry.suite.clone(),
                    name: entry.name.clone(),
                    code: value >> 1,
                });
            }
        }
    }
    report
}

/// Runs every catalog row in isolation.
///
/// The full-battery loop can lose its place when a failing test corrupts the
/// engine's saved test index (AccuracyCoin then re-runs part of a page until
/// its text pointer lands in code and spins at `$F278`). This mode boots a
/// fresh console for every row, navigates the menu to it, presses A, and reads
/// that row's result address. It is slower but writes every result.
#[must_use]
pub fn run_accuracycoin_isolated(
    rom: &[u8],
    entries: &[CatalogEntry],
    boot_frames: u32,
    test_frames: u32,
) -> AccuracyCoinReport {
    let mut report = AccuracyCoinReport::default();
    for entry in entries {
        let Ok(mut system) = System::from_ines(rom) else {
            continue;
        };
        for _ in 0..boot_frames {
            system.set_controller(0, 0);
            system.run_frame();
        }
        // Closed-loop navigation using the ROM's own cursor variables.
        // $14 = page index, $16 = row (0 = page header, 1.. = tests).
        for _ in 0..8 {
            if system.read_ram(0x16) == 0 {
                break;
            }
            press(&mut system, UP, 1);
        }
        for _ in 0..80 {
            let current = usize::from(system.read_ram(0x14));
            if current == entry.page {
                break;
            }
            press(
                &mut system,
                if current < entry.page { RIGHT } else { LEFT },
                1,
            );
        }
        for _ in 0..24 {
            // $16 = $FF is the page header; the first test is row 0.
            let raw = system.read_ram(0x16);
            let cursor = if raw == 0xFF { -1i32 } else { i32::from(raw) };
            let target = entry.row as i32;
            if cursor == target {
                break;
            }
            press(&mut system, if cursor < target { DOWN } else { UP }, 1);
        }
        press(&mut system, A, 1);
        for _ in 0..test_frames {
            system.set_controller(0, 0);
            system.run_frame();
        }
        report.frames += boot_frames + test_frames;

        let value = if entry.address < 0x0400 {
            0xFF
        } else {
            system.read_ram(entry.address)
        };
        report.results.push((entry.address, value));
        match value {
            0xFF => report.skipped += 1,
            0 => report.unrun += 1,
            value if value & 1 == 1 => report.passed += 1,
            value => {
                report.failed += 1;
                report.failures.push(FailedTest {
                    suite: entry.suite.clone(),
                    name: entry.name.clone(),
                    code: value >> 1,
                });
            }
        }
    }
    report
}

fn press(system: &mut System, button: u16, count: u32) {
    for _ in 0..count {
        system.set_controller(0, button);
        for _ in 0..4 {
            system.run_frame();
        }
        system.set_controller(0, 0);
        for _ in 0..4 {
            system.run_frame();
        }
    }
}
