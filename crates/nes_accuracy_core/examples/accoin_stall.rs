use std::collections::HashMap;
use std::env;
use std::fs;

use nes_accuracy_core::System;

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "test-roms/AccuracyCoin.nes".into());
    let rom = fs::read(&path).expect("rom");
    let mut s = System::from_ines(&rom).expect("load");
    // Mirror the runner's Start trigger.
    for _ in 0..120 {
        s.set_controller(0, 0);
        s.run_frame();
    }
    for _ in 0..4 {
        s.set_controller(0, 0x10);
        s.run_frame();
    }
    for _ in 0..4 {
        s.run_frame();
    }
    s.set_controller(0, 0x08);
    for _ in 0..4 {
        s.run_frame();
    }
    s.set_controller(0, 0);
    for _ in 0..6000 {
        s.run_frame();
    }

    println!(
        "state pc {:04X} a {:02X} x {:02X} y {:02X} sp {:02X} halted {} cyc {}",
        s.cpu.registers.pc,
        s.cpu.registers.a,
        s.cpu.registers.x,
        s.cpu.registers.y,
        s.cpu.registers.sp,
        s.cpu.halted,
        s.cpu.cycles
    );
    println!(
        "ppu {}:{} mask {:02X} ctrl {:02X} status {:02X} nmi_pending {} frame {}",
        s.ppu.scanline,
        s.ppu.dot,
        s.ppu.mask,
        s.ppu.ctrl,
        s.ppu.status,
        s.ppu.nmi_pending,
        s.ppu.frames
    );
    println!(
        "apu irq {} frame_irq {} dmc_irq {} running {:02X}",
        s.apu.irq_line(),
        s.apu.frame_irq,
        s.apu.dmc.irq,
        s.read_ram(0x35)
    );

    let ptr = u16::from(s.read_ram(0x00)) | (u16::from(s.read_ram(0x01)) << 8);
    let y = usize::from(s.cpu.registers.y);
    println!(
        "zp00 {:02X} zp01 {:02X} ptr {:04X} y {:02X} byte {:02X}",
        s.read_ram(0x00),
        s.read_ram(0x01),
        ptr,
        y,
        s.read_ram(ptr.wrapping_add(y as u16))
    );
    let page = ptr & 0xFF00;
    let has_ff = (0..256u16).any(|i| s.read_ram(page + i) == 0xFF);
    println!(
        "scan page {page:04X} has_ff {has_ff} first {:02X?}",
        (0..16u16).map(|i| s.read_ram(page + i)).collect::<Vec<_>>()
    );
    let mut hist: HashMap<u16, u32> = HashMap::new();
    for _ in 0..300_000u32 {
        let pc = s.cpu.registers.pc;
        *hist.entry(pc).or_insert(0) += 1;
        let mut cpu = s.cpu;
        cpu.step(&mut s);
        s.cpu = cpu;
    }
    let mut hot: Vec<(u16, u32)> = hist.into_iter().collect();
    hot.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    println!("hot pcs:");
    for (pc, count) in hot.iter().take(12) {
        let bytes: Vec<u8> = (0..8)
            .map(|i| s.cart.cpu_read(pc.wrapping_add(i)))
            .collect();
        println!("  {pc:04X} x{count:<7} bytes {bytes:02X?}");
    }
    println!(
        "end pc {:04X} ppu {}:{} status {:02X} running {:02X}",
        s.cpu.registers.pc,
        s.ppu.scanline,
        s.ppu.dot,
        s.ppu.status,
        s.read_ram(0x35)
    );
}
