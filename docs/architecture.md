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
│ unko-core                                                │
│  Core trait · CoreInfo · Registry · InputState · LoadOptions │
└───────────────▲──────────────────────────────┬───────────────┘
                │ implements                    │ factory()
┌───────────────┴──────────────────────────────▼───────────────┐
│ core crates: nes_accuracy_core (others later)                │
│  SNES core · GB core · ...                                   │
└──────────────────────────────────────────────────────────────┘
```

The host never depends on a core crate. It depends on `unko-core` and builds
a `Registry` from the cores it chooses to link (or, later, load as plugins).
`nes_accuracy_core` is the only NES backend: the crate contains the
reference-parity system itself plus its `Core` ABI adapter. The former
TetaNES adapter has been retired.

`unko-rom` is a sibling boundary for *input*: it discovers loose ROMs and
members of `.zip`/`.7z`/`.rar` archives, parses iNES metadata, and returns
bytes to whichever core is loading. The UI never sees archive libraries.

## The contract

`unko_core::Core`:

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
4. **Buffers are borrowed, not copied, at the boundary.** The PPU owns the
   completed frame and exposes it through `System::video`, so the adapter does
   not copy a full frame per display.
5. **Save states are opaque and versioned by the core.** The host stores them,
   never interprets them.

## Registry

`Registry` maps `SystemId` to zero or more `CoreRegistration`s. A system can have
several cores (accuracy-first, speed-first, experimental); the host chooses by
`info.id` or takes the first. Registration is explicit, so a build only contains
the cores you chose to link.

## NES backend

`nes_accuracy_core` owns the cycle-stepped CPU, PPU, APU, cartridge and system
bus. Its `Core` adapter:

- loads iNES/NES 2.0 and raw FDS images;
- exposes RGBA video and mono f32 audio through borrowed buffers;
- serializes versioned `UNKOST03` save states, including mapper state and a ROM
  fingerprint;
- maps the host input bits to the NES shift-register order.

The core is deterministic and has no window, audio-device or filesystem
dependency. The desktop shell owns SDL3 input, cpal output and all files.

## What is deliberately not here yet

- C ABI / dynamic plugin loading.
- Threading (run-ahead, rollback netplay).
- A GPU frontend, shaders, rewind, debugger UI.
- Multi-instance support beyond "one `Box<dyn Core>` per console".

Those are shell features and should be built on this boundary, not inside the
cores.
