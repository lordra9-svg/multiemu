# Architecture

## Goal

One shell, many systems, and more than one core per system when that is useful.
"Multi-emulator" here means: the application does not know which emulator is
running behind the interface, and it can switch cores without changing the UI,
input, audio, screenshot or save-state code.

## The boundary

```
┌──────────────────────────────────────────────────────────────┐
│ host (CLI today, GUI later)                                  │
│  - pacing, window, audio device, input binding, files        │
└───────────────▲──────────────────────────────┬───────────────┘
                │ FrameBuffer / AudioBuffer     │ run_frame()
                │                               │
┌───────────────┴──────────────────────────────▼───────────────┐
│ multiemu-core                                                │
│  Core trait · CoreInfo · Registry · InputState · LoadOptions │
└───────────────▲──────────────────────────────┬───────────────┘
                │ implements                    │ factory()
┌───────────────┴──────────────────────────────▼───────────────┐
│ core crates: multiemu-nes (tetanes-core today, others later) │
│  SNES core · GB core · ...                                   │
└──────────────────────────────────────────────────────────────┘
```

The host never depends on a core crate. It depends on `multiemu-core` and builds
a `Registry` from the cores it chooses to link (or, later, load as plugins).
The NES adapter is the only crate that mentions TetaNES.

`multiemu-nes-accuracy-core` is a second NES backend: it wraps the
reference-parity `multiemu-nes-accuracy` system in the `Core` ABI, so the CLI and
desktop app can select it with `--core multiemu-nes-accuracy`.

`multiemu-rom` is a sibling boundary for *input*: it discovers loose ROMs and
members of `.zip`/`.7z`/`.rar` archives, parses iNES metadata, and returns
bytes to whichever core is loading. The UI never sees archive libraries.

## The contract

`multiemu_core::Core`:

| Method | Responsibility |
|---|---|
| `info()` | static metadata: id, system, accuracy class, features |
| `load(rom, options)` | parse, reset, apply region/sample-rate/filter |
| `reset()` | hard reset, keep the cartridge |
| `run_frame()` | advance exactly one display frame |
| `video()` | borrow the finished frame (RGBA8, 256×240 for NES) |
| `audio()` | borrow this frame's f32 samples |
| `set_input(port, state)` | replace one controller's buttons |
| `set_sample_rate` / `set_video_filter` | host preferences |
| `save_state` / `load_state` | full machine state, opaque bytes |
| `sram()` / `load_sram` | battery RAM in/out, host owns the files |

Rules that keep the boundary honest:

1. **The core never owns the window, the audio device, or a file path.** If it
   needs a file, the host passes bytes or a callback.
2. **The core is deterministic.** Same ROM, inputs and options must produce the
   same frames and states. No wall-clock time, no OS RNG, no threads in core
   code.
3. **One display frame per `run_frame`.** A core that runs multiple console
   frames (fast-forward) reports the count in `FrameStats`.
4. **Buffers are borrowed, not copied, at the boundary.** The NES adapter copies
   TetaNES' frame into its own buffer once per frame because TetaNES' accessor
   needs `&mut self`; that is an implementation detail, not an API change.
5. **Save states are opaque and versioned by the core.** The host stores them,
   never interprets them.

## Registry

`Registry` maps `SystemId` to zero or more `CoreRegistration`s. A system can have
several cores (accuracy-first, speed-first, experimental); the host chooses by
`info.id` or takes the first. Registration is explicit, so a build only contains
the cores you chose to link.

## NES backend

`multiemu-nes` adapts `tetanes-core`:

- `ControlDeck` owns CPU/PPU/APU/bus/cart.
- The adapter forces `VideoFilter::Pixellate` so the host gets raw pixels, and
  sets `sram_dir: None` so TetaNES performs no filesystem I/O.
- `run_frame` loops `clock_frame()` until a display frame completes and copies
  the RGBA frame into a stable buffer.
- Input bits map 1:1 to TetaNES' `JoypadBtnState` because both use the NES
  shift-register order.

If TetaNES stops meeting the accuracy or mapper-coverage bar, the adapter is the
only thing that changes; the host and the ABI stay put. A second NES core can be
registered alongside it.

## What is deliberately not here yet

- C ABI / dynamic plugin loading.
- Threading (run-ahead, rollback netplay).
- A GPU frontend, shaders, rewind, debugger UI.
- Multi-instance support beyond "one `Box<dyn Core>` per console".

Those are shell features and should be built on this boundary, not inside the
cores.
