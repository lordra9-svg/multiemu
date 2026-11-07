//! [`multiemu_core::Core`] adapter for `multiemu-nes-accuracy`.
//!
//! This keeps the reference-parity core independent of the host ABI: the core crate
//! knows nothing about `multiemu-core`, and this thin adapter exposes it to the
//! desktop app and CLI beside the TetaNES backend.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use multiemu_core::{
    Accuracy, AudioBuffer, Core, CoreError, CoreInfo, CoreRegistration, Features, FrameBuffer,
    FrameStats, InputState, LoadOptions, PixelFormat, Region, SystemId, VideoFilter,
};
use multiemu_nes_accuracy::system::System;
use multiemu_nes_accuracy::CartError;

/// Stable id of this core implementation.
pub const CORE_ID: &str = "multiemu-nes-accuracy";
/// Output width.
pub const WIDTH: u32 = 256;
/// Output height.
pub const HEIGHT: u32 = 240;
/// Bytes per RGBA row.
pub const STRIDE: usize = WIDTH as usize * 4;

/// Builds the core.
#[must_use]
pub fn factory() -> Box<dyn Core> {
    Box::new(AccuracyCore::new())
}

/// Registration entry for [`multiemu_core::Registry`].
#[must_use]
pub fn registration() -> CoreRegistration {
    CoreRegistration {
        info: info(),
        factory,
    }
}

/// Static metadata.
#[must_use]
pub const fn info() -> CoreInfo {
    CoreInfo {
        id: CORE_ID,
        name: "multiemu accuracy",
        version: env!("CARGO_PKG_VERSION"),
        system: SystemId::Nes,
        accuracy: Accuracy::High,
        features: Features::BATTERY,
    }
}

/// NES console behind the host core interface.
pub struct AccuracyCore {
    system: Option<System>,
    frame: Vec<u8>,
    audio: Vec<f32>,
    sample_rate: f32,
}

impl AccuracyCore {
    /// Creates an unloaded core.
    #[must_use]
    pub fn new() -> Self {
        Self {
            system: None,
            frame: vec![0; WIDTH as usize * HEIGHT as usize * 4],
            audio: Vec::new(),
            sample_rate: 48_000.0,
        }
    }
}

impl Default for AccuracyCore {
    fn default() -> Self {
        Self::new()
    }
}

fn map_err(err: CartError) -> CoreError {
    CoreError::InvalidRom(err.to_string())
}

impl Core for AccuracyCore {
    fn info(&self) -> CoreInfo {
        info()
    }

    fn load(&mut self, rom: &[u8], options: &LoadOptions) -> Result<(), CoreError> {
        if options.region != Region::Auto && options.region != Region::Ntsc {
            return Err(CoreError::Unsupported(
                "only NTSC timing is implemented".into(),
            ));
        }
        let mut system = System::from_ines(rom).map_err(map_err)?;
        let rate = if options.sample_rate > 0.0 {
            options.sample_rate
        } else {
            self.sample_rate
        };
        system.set_sample_rate(rate);
        self.sample_rate = rate;
        self.system = Some(system);
        Ok(())
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        self.system
            .as_mut()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".into()))?
            .reset();
        Ok(())
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        let system = self
            .system
            .as_mut()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".into()))?;
        system.run_frame();
        self.frame.clear();
        self.frame.extend_from_slice(system.video());
        self.audio.clear();
        self.audio.extend_from_slice(system.audio());
        Ok(FrameStats {
            frames: 1,
            samples: self.audio.len(),
        })
    }

    fn video(&self) -> FrameBuffer<'_> {
        FrameBuffer {
            data: &self.frame,
            width: WIDTH,
            height: HEIGHT,
            stride: STRIDE,
            format: PixelFormat::Rgba8,
        }
    }

    fn audio(&self) -> AudioBuffer<'_> {
        AudioBuffer {
            samples: &self.audio,
            sample_rate: self.sample_rate,
            channels: 1,
        }
    }

    fn set_input(&mut self, port: usize, input: InputState) {
        if let Some(system) = self.system.as_mut() {
            system.set_controller(port, input.bits());
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate > 0.0 {
            self.sample_rate = sample_rate;
            if let Some(system) = self.system.as_mut() {
                system.set_sample_rate(sample_rate);
            }
        }
    }

    fn set_video_filter(&mut self, _filter: VideoFilter) {}

    fn save_state(&mut self, _out: &mut Vec<u8>) -> Result<(), CoreError> {
        Err(CoreError::Unsupported(
            "save states are not implemented in the accuracy core yet".into(),
        ))
    }

    fn load_state(&mut self, _state: &[u8]) -> Result<(), CoreError> {
        Err(CoreError::Unsupported(
            "save states are not implemented in the accuracy core yet".into(),
        ))
    }

    fn sram(&mut self) -> &[u8] {
        self.system.as_ref().map_or(&[], System::sram)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_registered() {
        let info = info();
        assert_eq!(info.id, CORE_ID);
        assert_eq!(info.system, SystemId::Nes);
        assert!(info.features.contains(Features::BATTERY));
    }

    #[test]
    fn rejects_missing_rom() {
        let mut core = AccuracyCore::new();
        assert!(core.load(b"not a rom", &LoadOptions::default()).is_err());
    }
}
