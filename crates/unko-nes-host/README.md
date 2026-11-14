# unko-nes-host

The `unko_core::Core` adapter for the standalone [`unko-nes`](../unko-nes)
core. It converts the host-facing configuration types (`PortDevice`,
`AudioQuality`, `Cheat`, `DebugSpace`, `Region`, …) into the native NES types
and exposes the fixed 256×240 RGBA and mono f32 buffers the Unko frontend and
CLI consume.

The stable registry id remains `nes_accuracy_core` so existing save-state
filenames and trace headers keep working.

## Usage

```rust
use unko_core::{Core, LoadOptions};

let rom = std::fs::read("game.nes")?;
let mut core = unko_nes_host::AccuracyCore::new();
core.load(&rom, &LoadOptions::default())?;
core.run_frame()?;
let frame = core.video();
# Ok::<(), Box<dyn std::error::Error>>(())
```

Register it with `unko_core::Registry` through
`unko_nes_host::registration()`. The adapter is only needed for frontends that
speak `unko-core`; embed `unko-nes` directly for everything else.
