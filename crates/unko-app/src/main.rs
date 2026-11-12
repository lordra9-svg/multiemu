//! Desktop entry point for unko.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod app;
mod audio;
mod gamepad;
mod library;
mod player;
mod recording;
mod settings;

use std::path::PathBuf;

use app::{App, Args};

fn main() -> eframe::Result<()> {
    let args = parse_args();
    let settings = settings::Settings::load(&settings::Settings::path());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("Unko"),
        ..Default::default()
    };

    eframe::run_native(
        "unko",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings, args)))),
    )
}

fn parse_args() -> Args {
    let mut parsed = Args::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--roms" => parsed.roms = args.next().map(PathBuf::from),
            "--rom" => parsed.rom = args.next().map(PathBuf::from),
            "--settings" => parsed.settings = true,
            "-h" | "--help" => {
                println!(
                    "Unko — desktop frontend\n\n\
                     usage: unko-app [--roms DIR] [--rom PATH] [--settings]\n\n\
                     Keyboard: arrows = D-pad, X = A, Z = B, Enter = Start,\n\
                     Shift = Select, Space = pause, F5/F8 = save/load state,\n\
                     F11 = fullscreen."
                );
                std::process::exit(0);
            }
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    parsed
}
