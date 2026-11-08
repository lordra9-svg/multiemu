//! Dumps the `$2007` stress test's raw samples ($0500-$0654).
//!
//! Usage:
//! `cargo run -p multiemu-nes-accuracy --example dump2007 -- <rom> <tsv> <out.bin>`

use std::env;
use std::fs;

use multiemu_nes_accuracy::accuracycoin::{parse_catalog, run_accuracycoin_one};

fn main() {
    let rom_path = env::args().nth(1).expect("rom");
    let catalog_path = env::args().nth(2).expect("tsv");
    let out_path = env::args()
        .nth(3)
        .unwrap_or_else(|| "/tmp/opencode/2007-dump.bin".into());
    let rom = fs::read(&rom_path).expect("read rom");
    let catalog = fs::read_to_string(&catalog_path).expect("read catalog");
    let entry = parse_catalog(&catalog)
        .into_iter()
        .find(|entry| entry.address == 0x048E)
        .expect("$2007 Stress Test catalog row");
    let system = run_accuracycoin_one(&rom, &entry, 120, 1200).expect("run test");
    println!(
        "result ${:04X} = {:02X}",
        entry.address,
        system.read_ram(entry.address)
    );
    let samples: Vec<u8> = (0x0500..=0x0654).map(|a| system.read_ram(a)).collect();
    for (row, chunk) in samples.chunks(16).enumerate() {
        print!("{:04X}:", 0x0500 + row * 16);
        for byte in chunk {
            print!(" {byte:02X}");
        }
        println!();
    }
    fs::write(&out_path, &samples).expect("write dump");
    println!("wrote {out_path} ({} bytes)", samples.len());
}
