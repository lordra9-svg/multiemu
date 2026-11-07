use multiemu_nes_accuracy::System;
use std::env;
use std::fs;

fn press(s: &mut System, b: u16, n: u32) {
    for _ in 0..n {
        s.set_controller(0, b);
        s.run_frame();
        s.run_frame();
        s.set_controller(0, 0);
        s.run_frame();
        s.run_frame();
    }
}

fn main() {
    let arg: Vec<String> = env::args().collect();
    let button: u16 = match arg.get(1).map(String::as_str) {
        Some("right") => 0x80,
        Some("left") => 0x40,
        Some("down") => 0x20,
        Some("up") => 0x10,
        Some("a") => 0x01,
        _ => 0x20,
    };
    let n: u32 = arg.get(2).and_then(|v| v.parse().ok()).unwrap_or(1);
    let rom = fs::read("test-roms/AccuracyCoin.nes").expect("rom");
    let mut s = System::from_ines(&rom).expect("load");
    for _ in 0..120 {
        s.set_controller(0, 0);
        s.run_frame();
    }
    press(&mut s, button, n);
    for _ in 0..20 {
        s.run_frame();
    }
    let mut ppm = b"P6\n256 240\n255\n".to_vec();
    for p in s.video().as_chunks::<4>().0 {
        ppm.extend_from_slice(&p[..3]);
    }
    let name = format!(
        "/tmp/opencode/nav-{}-{}.ppm",
        arg.get(1).map(String::as_str).unwrap_or("down"),
        n
    );
    fs::write(&name, ppm).expect("write");
    println!("wrote {name}");
}
