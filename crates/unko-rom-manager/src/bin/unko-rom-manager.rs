//! Thin wrapper around the library CLI.

fn main() {
    if let Err(err) = unko_rom_manager::cli::run(std::env::args().skip(1)) {
        eprintln!("unko-rom-manager: {err}");
        std::process::exit(1);
    }
}
