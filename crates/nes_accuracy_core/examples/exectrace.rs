//! Per-instruction `(CPU cycle, PC)` trace for differential debugging.
//!
//! Usage: `cargo run -p nes_accuracy_core --example exectrace -- <rom> <frame>`
//!
//! Runs `<frame>` frames, traces the next one, and prints `cycle pc`.

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
    system.start_exec_trace();
    system.run_frame();
    for (cycle, pc) in system.take_exec_trace() {
        println!("{cycle} {pc:04X}");
    }
}
