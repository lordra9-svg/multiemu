//! Expansion-port write trace for differential debugging.
//!
//! Usage: `cargo run -p nes_accuracy_core --example expwrites -- <rom> <frame>`
//!
//! Runs `<frame>` frames, traces the next one, and prints `cycle address value`
//! for every write to `$4020-$FFFF`.

use std::env;
use std::fs;

use nes_accuracy_core::System;

fn main() {
    let path = env::args().nth(1).expect("rom path");
    let frame: u32 = env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let rom = fs::read(path).expect("read rom");
    let mut system = System::from_ines(&rom).expect("load");
    for _ in 0..frame {
        system.run_frame();
    }
    system.start_expansion_trace();
    system.run_frame();
    for (cycle, address, value) in system.take_expansion_trace() {
        println!("{cycle} {address:04X} {value:02X}");
    }
}
