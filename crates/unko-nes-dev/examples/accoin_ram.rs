use std::env;
use std::fs;
use unko_nes::System;
use unko_nes_dev::parse_catalog;

fn press(s: &mut System, b: u16) {
    s.set_controller(0, b);
    for _ in 0..4 {
        s.run_frame();
    }
    s.set_controller(0, 0);
    for _ in 0..4 {
        s.run_frame();
    }
}

fn main() {
    let addr = u16::from_str_radix(
        env::args()
            .nth(1)
            .unwrap_or_else(|| "0479".into())
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let frames: u32 = env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(2500);
    let catalog = fs::read_to_string("test-roms/AccuracyCoin.tsv").expect("catalog");
    let entries = parse_catalog(&catalog);
    let entry = entries.iter().find(|e| e.address == addr).expect("entry");
    let rom = fs::read("test-roms/AccuracyCoin.nes").expect("rom");
    let mut s = System::from_ines(&rom).expect("load");
    for _ in 0..120 {
        s.run_frame();
    }
    for _ in 0..80 {
        let p = usize::from(s.read_ram(0x14));
        if p == entry.page {
            break;
        }
        press(&mut s, if p < entry.page { 0x80 } else { 0x40 });
    }
    for _ in 0..24 {
        let raw = s.read_ram(0x16);
        let cur = if raw == 0xFF { -1i32 } else { i32::from(raw) };
        let target = entry.row as i32;
        if cur == target {
            break;
        }
        press(&mut s, if cur < target { 0x20 } else { 0x10 });
    }
    press(&mut s, 0x01);
    for _ in 0..frames {
        s.run_frame();
    }
    println!("result {:04X} = {:02X}", addr, s.read_ram(addr));
    let z: Vec<u8> = (0..32).map(|i| s.read_ram(0x50 + i)).collect();
    println!(
        "zp50: {}",
        z.iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let p: Vec<u8> = (0..32).map(|i| s.read_ram(0x500 + i)).collect();
    println!(
        "p500: {}",
        p.iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
}
