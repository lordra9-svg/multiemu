//! Desktop entry point for unko.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;

use unko_app::{run, Args};

fn main() -> eframe::Result<()> {
    run(parse_args())
}

fn parse_args() -> Args {
    let mut parsed = Args::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--roms" => parsed.roms = args.next().map(PathBuf::from),
            "--rom" => parsed.rom = args.next().map(PathBuf::from),
            "--settings" => parsed.settings = true,
            "--debug" => parsed.debug = true,
            "--portable" => parsed.portable = true,
            "--print-data-dir" => parsed.print_data_dir = true,
            "--data-dir" | "--home" => {
                parsed.data_dir = args.next().map(PathBuf::from);
            }
            "-h" | "--help" => {
                println!(
                    "Unko — desktop frontend\n\n\
                     usage: unko-app [--roms DIR] [--rom PATH] [--settings] [--debug]\n\
                     \x20      [--portable | --data-dir DIR] [--print-data-dir]\n\n\
                     Keyboard: arrows = D-pad, X = A, Z = B, Enter = Start,\n\
                     Shift = Select, Space = pause, F5/F8 = save/load state,\n\
                     F9/F10 = step frame/instruction, F11 = fullscreen."
                );
                std::process::exit(0);
            }
            other => eprintln!("ignoring unknown argument {other}"),
        }
    }
    parsed
}
