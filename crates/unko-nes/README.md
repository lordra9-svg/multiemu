# unko-nes

A standalone, reference-parity NES/Famicom core written in Rust. The crate has
no dependency on the Unko shell or on `unko-core`; it can be embedded directly
in other Rust projects.

`unko-nes` implements:

- cycle-stepped 6502/2A03 CPU, PPU and APU;
- iNES and NES 2.0 loading, plus raw FDS images with a user-supplied BIOS;
- the mapper set used by the Unko accuracy suite and the MMC3/MMC5/MMC6
  variants, expansion audio (VRC6, VRC7, MMC5, Namco 163, Sunsoft 5B);
- NSF/NSFE playback;
- NTSC, PAL and Dendy timing;
- optional save states, cheats and debugger helpers (all default-on).

## Usage

```toml
[dependencies]
unko-nes = { git = "https://github.com/lordra9-svg/unko", branch = "main" }
```

```rust
let rom = std::fs::read("game.nes")?;
let mut nes = unko_nes::System::from_ines(&rom)?;
nes.set_sample_rate(48_000.0);
nes.set_controller(0, 0);

for _ in 0..60 {
    nes.run_frame();
}
let rgba = nes.video();   // 256 × 240 × 4 bytes, RGBA8
let audio = nes.audio();  // mono f32 samples for the frame
# Ok::<(), Box<dyn std::error::Error>>(())
```

`System` also exposes `from_fds`, `save_state`/`load_state`, battery SRAM,
controller/peripheral setters, region and palette selection, and the APU/PPU
debug helpers. See the crate documentation for the full surface.

## Features

| Feature | Default | Meaning |
|---|---:|---|
| `save-states` | yes | Whole-machine snapshot serialization. |
| `cheats` | yes | Game Genie and RAM cheat application. |
| `debug` | yes | Debugger peek/poke and state inspection helpers. |
| `apu-trace` | no | Per-CPU-cycle APU trace used by the differential examples. |

Build a lean core with `default-features = false`.

## Host adapter

`unko-nes-host` is a separate crate that implements the system-agnostic
`unko_core::Core` interface on top of `unko-nes`. It is what the Unko desktop
app and CLI link. Projects that only want the NES core do not need it.

## Licence

MIT. Parts of the core are adapted from ares under its ISC licence; see the
repository `NOTICE`, `licenses/ares-LICENSE.txt` and `PROVENANCE.md`.
