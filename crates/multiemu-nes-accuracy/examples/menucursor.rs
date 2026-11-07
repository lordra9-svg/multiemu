use multiemu_nes_accuracy::System;
use std::fs;

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
    let rom = fs::read("test-roms/AccuracyCoin.nes").expect("rom");
    let mut s = System::from_ines(&rom).expect("load");
    for _ in 0..120 {
        s.run_frame();
    }
    println!(
        "boot page {:02X} row {:02X}",
        s.read_ram(0x14),
        s.read_ram(0x16)
    );
    for _ in 0..16 {
        press(&mut s, 0x80);
    }
    println!(
        "after 16 right: page {:02X} row {:02X}",
        s.read_ram(0x14),
        s.read_ram(0x16)
    );
    for i in 0..10 {
        press(&mut s, 0x20);
        println!(
            "down {i}: page {:02X} row {:02X}",
            s.read_ram(0x14),
            s.read_ram(0x16)
        );
    }
}
