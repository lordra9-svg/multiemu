use nes_accuracy_core::System;
fn main() {
    let rom = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let n: u32 = std::env::args().nth(2).unwrap().parse().unwrap();
    let tag = std::env::args().nth(3).unwrap();
    let mut s = System::from_ines(&rom).unwrap();
    for _ in 0..=n {
        s.run_frame();
    }
    std::fs::write(format!("/tmp/opencode/{tag}.raw"), s.video()).unwrap();
    let r = s.cpu.registers;
    println!(
        "{tag}: pc {:04X} mask {:02X} frames {}",
        r.pc, s.ppu.mask, s.ppu.frames
    );
}
