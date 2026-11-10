//! NES core adapted from [`tetanes_core`] to the [`unko_core::Core`] contract.
//!
//! TetaNES is dual-licensed MIT OR Apache-2.0; this adapter uses the MIT
//! option. See the repository `NOTICE` and `licenses/tetanes-MIT.txt`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use tetanes_core::common::{NesRegion, ResetKind};
use tetanes_core::control_deck::{Clocked, Config, ControlDeck, Error as DeckError};
use tetanes_core::input::{JoypadBtnState, Player};
use tetanes_core::memory::RamState;
use tetanes_core::video::{Frame, VideoFilter as NesFilter};
use unko_core::{
    Accuracy, AudioBuffer, Core, CoreError, CoreInfo, CoreRegistration, Features, FrameBuffer,
    FrameStats, InputState, LoadOptions, PixelFormat, Region, SystemId, VideoFilter,
};

/// Stable id of this core implementation.
pub const CORE_ID: &str = "nes_core";
/// NES output width in pixels.
pub const WIDTH: u32 = 256;
/// NES output height in pixels.
pub const HEIGHT: u32 = 240;
/// Bytes per RGBA row.
pub const STRIDE: usize = WIDTH as usize * 4;

/// Builds the core, ready to load a ROM.
#[must_use]
pub fn factory() -> Box<dyn Core> {
    Box::new(NesCore::new())
}

/// Registration entry for [`unko_core::Registry`].
#[must_use]
pub fn registration() -> CoreRegistration {
    CoreRegistration {
        info: info(),
        factory,
    }
}

/// Static metadata for this core.
#[must_use]
pub const fn info() -> CoreInfo {
    CoreInfo {
        id: CORE_ID,
        name: "Unko NES core (TetaNES-derived)",
        version: env!("CARGO_PKG_VERSION"),
        system: SystemId::Nes,
        accuracy: Accuracy::High,
        features: Features::SAVE_STATES
            .union(Features::BATTERY)
            .union(Features::EXPANSION_AUDIO),
    }
}

/// A NES console behind the system-agnostic core interface.
pub struct NesCore {
    deck: ControlDeck,
    frame: Frame,
    sample_rate: f32,
    loaded: bool,
}

impl NesCore {
    /// Creates an unloaded NES core with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        let mut config = Config::default();
        // The host owns presentation; ask for one output pixel per PPU pixel.
        config.filter = NesFilter::Pixellate;
        // The host owns save files and passes SRAM in/out explicitly.
        config.sram_dir = None;
        // Deterministic power-on RAM. TetaNES defaults to OS-random bytes,
        // which would break save-state/netplay equivalence between instances.
        config.ram_state = RamState::AllZeros;
        let mut deck = ControlDeck::with_config(config);
        deck.set_sample_rate(48_000.0);
        Self {
            deck,
            frame: Frame::new(),
            sample_rate: 48_000.0,
            loaded: false,
        }
    }

    /// Access to the underlying TetaNES deck for features the common interface
    /// does not cover yet (zapper, debugger, run-ahead, ...).
    #[must_use]
    pub fn deck(&self) -> &ControlDeck {
        &self.deck
    }

    /// Mutable access to the underlying TetaNES deck.
    pub fn deck_mut(&mut self) -> &mut ControlDeck {
        &mut self.deck
    }

    /// Whether a ROM has been loaded.
    #[must_use]
    pub const fn is_loaded(&self) -> bool {
        self.loaded
    }

    fn apply_filter(&mut self, filter: VideoFilter) {
        self.deck.set_filter(match filter {
            VideoFilter::Raw => NesFilter::Pixellate,
            VideoFilter::Composite => NesFilter::Ntsc,
        });
    }
}

impl Default for NesCore {
    fn default() -> Self {
        Self::new()
    }
}

fn map_err(err: DeckError) -> CoreError {
    match err {
        DeckError::UnimplementedMapper(id) => {
            CoreError::Unsupported(format!("mapper {id} is not implemented"))
        }
        DeckError::Cart(err) => CoreError::InvalidRom(err.to_string()),
        DeckError::StateMismatch(err) => CoreError::State(err.to_string()),
        DeckError::SaveState(err) | DeckError::Sram(err) => CoreError::State(err.to_string()),
        DeckError::RomNotLoaded => CoreError::State("no ROM loaded".into()),
        DeckError::Io { .. } => CoreError::Io(err.to_string()),
        other => CoreError::Internal(other.to_string()),
    }
}

impl Core for NesCore {
    fn info(&self) -> CoreInfo {
        info()
    }

    fn load(&mut self, rom: &[u8], options: &LoadOptions) -> Result<(), CoreError> {
        let mut cursor = std::io::Cursor::new(rom);
        self.deck
            .load_rom("rom", &mut cursor)
            .map_err(|err| match map_err(err) {
                CoreError::Internal(msg) => CoreError::InvalidRom(msg),
                other => other,
            })?;

        self.deck.set_region(match options.region {
            Region::Auto => NesRegion::Auto,
            Region::Ntsc => NesRegion::Ntsc,
            Region::Pal => NesRegion::Pal,
            Region::Dendy => NesRegion::Dendy,
        });
        self.set_sample_rate(options.sample_rate);
        self.apply_filter(options.video_filter);
        self.loaded = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        if !self.loaded {
            return Err(CoreError::State("no ROM loaded".into()));
        }
        self.deck.reset(ResetKind::Hard);
        Ok(())
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        if !self.loaded {
            return Err(CoreError::State("no ROM loaded".into()));
        }
        let mut frames = 0;
        loop {
            match self.deck.clock_frame().map_err(map_err)? {
                Clocked::Continue => frames += 1,
                Clocked::Complete | Clocked::Idle | Clocked::Stopped => {
                    frames += 1;
                    break;
                }
            }
        }
        let src = self.deck.frame_buffer();
        self.frame.as_mut_slice().copy_from_slice(src);
        Ok(FrameStats {
            frames,
            samples: self.deck.audio_samples().len(),
        })
    }

    fn video(&self) -> FrameBuffer<'_> {
        FrameBuffer {
            data: self.frame.as_slice(),
            width: WIDTH,
            height: HEIGHT,
            stride: STRIDE,
            format: PixelFormat::Rgba8,
        }
    }

    fn audio(&self) -> AudioBuffer<'_> {
        AudioBuffer {
            samples: self.deck.audio_samples(),
            sample_rate: self.sample_rate,
            channels: 1,
        }
    }

    fn set_input(&mut self, port: usize, input: InputState) {
        let player = match port {
            0 => Player::One,
            1 => Player::Two,
            2 => Player::Three,
            _ => Player::Four,
        };
        self.deck.joypad_mut(player).buttons = JoypadBtnState::from_bits_truncate(input.bits());
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.deck.set_sample_rate(sample_rate);
    }

    fn set_video_filter(&mut self, filter: VideoFilter) {
        self.apply_filter(filter);
    }

    fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
        let len = self
            .deck
            .serialized_state_len()
            .map_err(|err| CoreError::State(err.to_string()))?;
        out.clear();
        out.resize(len, 0);
        let written = self
            .deck
            .serialize_state_into(out)
            .map_err(|err| CoreError::State(err.to_string()))?;
        out.truncate(written);
        Ok(())
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
        self.deck
            .deserialize_state(state)
            .map_err(|err| CoreError::State(err.to_string()))
    }

    fn sram(&mut self) -> &[u8] {
        self.deck.sram()
    }

    fn load_sram(&mut self, sram: &[u8]) -> Result<(), CoreError> {
        self.deck.set_sram(sram);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smallest valid NROM cartridge: 16 KiB PRG, 8 KiB CHR, reset vector
    /// pointing at an infinite `JMP $8000`.
    fn synthetic_nrom() -> Vec<u8> {
        let mut rom = vec![0u8; 16 + 16 * 1024 + 8 * 1024];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1; // 16 KiB PRG
        rom[5] = 1; // 8 KiB CHR
                    // PRG starts at 16; 0x8000 maps to the first byte.
        rom[16] = 0x4c; // JMP
        rom[17] = 0x00; // $8000
        rom[18] = 0x80;
        // Reset vector at PRG offset 0x3ffc -> 0x8000.
        rom[16 + 0x3ffc] = 0x00;
        rom[16 + 0x3ffd] = 0x80;
        rom
    }

    fn loaded() -> NesCore {
        let mut core = NesCore::new();
        core.load(&synthetic_nrom(), &LoadOptions::default())
            .expect("loads synthetic ROM");
        core
    }

    #[test]
    fn boots_and_produces_a_frame() {
        let mut core = loaded();
        let stats = core.run_frame().expect("runs");
        assert!(stats.frames >= 1);
        let frame = core.video();
        assert_eq!(frame.width, WIDTH);
        assert_eq!(frame.height, HEIGHT);
        assert_eq!(frame.data.len(), WIDTH as usize * HEIGHT as usize * 4);
        assert_eq!(frame.stride, STRIDE);
    }

    #[test]
    fn state_round_trips() {
        let mut core = loaded();
        core.run_frame().expect("runs");
        let mut state = Vec::new();
        core.save_state(&mut state).expect("saves");
        assert!(!state.is_empty());
        core.run_frame().expect("runs");
        core.load_state(&state).expect("loads state");
        core.run_frame().expect("runs after load");
    }

    #[test]
    fn rejects_garbage_rom() {
        let mut core = NesCore::new();
        let err = core
            .load(b"not a rom", &LoadOptions::default())
            .expect_err("must reject");
        assert!(matches!(err, CoreError::InvalidRom(_)));
    }

    fn assert_same_output(a: &NesCore, b: &NesCore) {
        assert_eq!(a.video().data, b.video().data, "framebuffer differs");
        let a_audio: Vec<u32> = a.audio().samples.iter().map(|s| s.to_bits()).collect();
        let b_audio: Vec<u32> = b.audio().samples.iter().map(|s| s.to_bits()).collect();
        assert_eq!(a_audio, b_audio, "audio differs");
    }

    #[test]
    fn fresh_instances_are_deterministic() {
        let mut a = loaded();
        let mut b = loaded();
        for _ in 0..8 {
            a.run_frame().expect("a runs");
            b.run_frame().expect("b runs");
            assert_same_output(&a, &b);
        }
    }

    #[test]
    fn state_resume_matches_uninterrupted_run() {
        let mut uninterrupted = loaded();
        let mut resumed = loaded();

        for _ in 0..4 {
            uninterrupted.run_frame().expect("runs");
            resumed.run_frame().expect("runs");
        }
        let mut state = Vec::new();
        uninterrupted.save_state(&mut state).expect("saves");

        // Resume from the midpoint, then compare frame by frame.
        resumed.load_state(&state).expect("loads");
        for _ in 0..4 {
            uninterrupted.run_frame().expect("runs");
            resumed.run_frame().expect("resumes");
            assert_same_output(&uninterrupted, &resumed);
        }
    }
}
