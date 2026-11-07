use multiemu_nes_accuracy::System;
use std::env;
use std::fs;

fn main() {
    let path = env::args().nth(1).expect("rom path");
    let frames: u32 = env::args()
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let rom = fs::read(&path).expect("read");
    let mut system = System::from_ines(&rom).expect("load");
    for frame in 0..frames {
        system.run_frame();
        if frame % 60 == 0 {
            println!("frame {frame} pc {:04X}", system.cpu.registers.pc);
        }
    }
    let frame = system.video();
    let first = &frame[0..3];
    let uniform = frame.chunks(4).all(|p| p[0..3] == *first);
    println!(
        "mapper {} pc {:04X} cpu_cyc {} ppu_frames {} mask {:02X} ctrl {:02X} status {:02X} scanline {} dot {} uniform {uniform} first {:?}",
        system.cart.mapper(),
        system.cpu.registers.pc,
        system.cpu.cycles,
        system.ppu.frames,
        system.ppu.mask,
        system.ppu.ctrl,
        system.ppu.status,
        system.ppu.scanline,
        system.ppu.dot,
        first
    );
    let vram_nonzero = system.ppu.vram.iter().filter(|b| **b != 0).count();
    let chr_nonzero = system.ppu.chr.iter().filter(|b| **b != 0).count();
    let oam_nonzero = system.ppu.oam.iter().filter(|b| **b != 0).count();
    let pc = system.cpu.registers.pc;
    let bytes: Vec<u8> = (0..8)
        .map(|i| system.cart.cpu_read(pc.wrapping_add(i)))
        .collect();
    println!(
        "opcode at {:04X}: {:02X?} A {:02X} X {:02X} Y {:02X} SP {:02X} halted {}",
        pc,
        bytes,
        system.cpu.registers.a,
        system.cpu.registers.x,
        system.cpu.registers.y,
        system.cpu.registers.sp,
        system.cpu.halted
    );
    println!(
        "ppu v {:04X} t {:04X} fine_x {}",
        system.ppu.v, system.ppu.t, system.ppu.fine_x
    );
    println!("vram_nonzero {vram_nonzero} chr_nonzero {chr_nonzero} oam_nonzero {oam_nonzero} palette {:02X?}", &system.ppu.palette[..16]);
}
