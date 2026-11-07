use multiemu_nes_accuracy::accuracycoin::parse_catalog;
use multiemu_nes_accuracy::System;
use std::env;
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
    let addr = u16::from_str_radix(
        env::args()
            .nth(1)
            .unwrap_or_else(|| "0456".into())
            .trim_start_matches("0x"),
        16,
    )
    .expect("hex address");
    let frames: u32 = env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000);
    let catalog = fs::read_to_string("test-roms/AccuracyCoin.tsv").expect("catalog");
    let entries = parse_catalog(&catalog);
    let entry = entries.iter().find(|e| e.address == addr).expect("entry");
    println!(
        "target page {} row {} [{}] {}",
        entry.page, entry.row, entry.suite, entry.name
    );

    let rom = fs::read("test-roms/AccuracyCoin.nes").expect("rom");
    let mut s = System::from_ines(&rom).expect("load");
    for _ in 0..120 {
        s.set_controller(0, 0);
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
    println!(
        "cursor page {:02X} row {:02X}",
        s.read_ram(0x14),
        s.read_ram(0x16)
    );
    press(&mut s, 0x01);
    let mut last = s.cpu.registers.pc;
    let mut same = 0u32;
    let mut printed = 0;
    for frame in 0..frames {
        s.run_frame();
        let pc = s.cpu.registers.pc;
        if pc == last {
            same += 1;
        } else {
            same = 0;
            last = pc;
        }
        if same == 240 && printed < 3 {
            printed += 1;
            let bytes: Vec<u8> = (0..10)
                .map(|i| s.cart.cpu_read(pc.wrapping_add(i)))
                .collect();
            println!("stuck f{frame} pc {pc:04X} ppu {}:{} mask {:02X} ctrl {:02X} status {:02X} a {:02X} x {:02X} y {:02X} sp {:02X} bytes {bytes:02X?}",
                s.ppu.scanline, s.ppu.dot, s.ppu.mask, s.ppu.ctrl, s.ppu.status,
                s.cpu.registers.a, s.cpu.registers.x, s.cpu.registers.y, s.cpu.registers.sp);
        }
    }
    println!(
        "result {:04X} = {:02X} final pc {:04X}",
        addr,
        s.read_ram(addr),
        s.cpu.registers.pc
    );
}
