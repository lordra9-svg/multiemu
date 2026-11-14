//! Per-CPU-cycle APU channel trace for differential debugging.
//!
//! Usage: `cargo run -p unko_nes --example apudump -- <rom> <frame>`
//!
//! Runs `<frame>` frames, traces the next one, and prints
//! `cycle pulse1 pulse2 triangle noise dmc mixed` per CPU cycle. The
//! reference oracle's `APU_TRACE=<frame>` output has the same fields.

use std::env;
use std::fs;

use unko_nes::System;

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
    system.start_apu_trace();
    system.run_frame();
    for (cycle, values, expansion) in system.take_apu_trace() {
        println!(
            "{cycle} {:.1} {:.1} {:.1} {:.1} {:.1} {:.9} {:.9} {:.9} {:.0} {:.0} {:.0} {:.0} {:.0} {:.0} {:.0} {:.0}",
            values[0],
            values[1],
            values[2],
            values[3],
            values[4],
            values[5],
            values[6],
            values[7],
            expansion[0],
            expansion[1],
            expansion[2],
            expansion[3],
            expansion[4],
            expansion[5],
            expansion[6],
            expansion[7]
        );
    }
}
