//! Unko NES accuracy core: a clean-room, reference-parity NES core and its
//! [`unko_core::Core`] adapter.
//!
//! The emulator itself is independent of `nes_core` (the TetaNES-derived
//! fallback) and of every reference emulator. It is built from public
//! hardware documentation; see `PROVENANCE.md` and `docs/accuracy.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod accuracycoin;
pub mod apu;
pub mod bus;
pub mod cart;
pub mod clock;
pub mod cpu;
pub mod nsf;
pub mod ppu;
pub mod resampler;
pub mod system;

pub use accuracycoin::{
    run_accuracycoin, run_accuracycoin_isolated, AccuracyCoinReport, CatalogEntry,
};
pub use apu::Apu;
pub use bus::{CpuBus, RamBus};
pub use cart::{CartError, Cartridge, Header, Mapper, Mirroring};
pub use clock::{Region, Timing};
pub use cpu::{Cpu, Registers, Status};
pub use nsf::{Nsf, NsfError, NsfHeader};
pub use ppu::Ppu;
pub use resampler::Resampler;
pub use system::System;

use crate::clock::Region as ClockRegion;
use unko_core::{
    Accuracy, AudioBuffer, Core, CoreError, CoreInfo, CoreRegistration, Features, FrameBuffer,
    FrameStats, InputState, LoadOptions, PixelFormat, PortDevice, Region as HostRegion, SystemId,
    VideoFilter,
};

/// Stable id of this core implementation.
pub const CORE_ID: &str = "nes_accuracy_core";
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

/// Registration entry for [`unko_core::Registry`].
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
        name: "Unko NES accuracy",
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
    filter: VideoFilter,
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
            filter: VideoFilter::Raw,
        }
    }
}

impl Default for AccuracyCore {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple composite/NTSC display approximation: a 3-tap horizontal luma blend
/// plus a mild scanline attenuation. Display-only; it never feeds back into
/// emulation state.
fn composite_filter(frame: &mut [u8]) {
    let width = WIDTH as usize;
    let height = HEIGHT as usize;
    let mut row = vec![0u8; width * 4];
    for y in 0..height {
        let start = y * STRIDE;
        row.copy_from_slice(&frame[start..start + STRIDE]);
        for x in 0..width {
            let left = x.saturating_sub(1);
            let right = (x + 1).min(width - 1);
            let scanline = if y & 1 == 0 { 1.0 } else { 0.82 };
            for channel in 0..3 {
                let blended = (f32::from(row[left * 4 + channel])
                    + 2.0 * f32::from(row[x * 4 + channel])
                    + f32::from(row[right * 4 + channel]))
                    / 4.0;
                let value = (blended * scanline).clamp(0.0, 255.0) as u8;
                frame[start + x * 4 + channel] = value;
            }
            frame[start + x * 4 + 3] = row[x * 4 + 3];
        }
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
        let mut system = System::from_ines(rom).map_err(map_err)?;
        // `Auto` keeps the timing the header declares; an explicit choice
        // overrides it.
        match options.region {
            HostRegion::Auto => {}
            HostRegion::Ntsc => system.set_region(ClockRegion::Ntsc),
            HostRegion::Pal => system.set_region(ClockRegion::Pal),
            HostRegion::Dendy => system.set_region(ClockRegion::Dendy),
        }
        self.filter = options.video_filter;
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
        if self.filter == VideoFilter::Composite {
            composite_filter(&mut self.frame);
        }
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

    fn set_video_filter(&mut self, filter: VideoFilter) {
        self.filter = filter;
    }

    fn set_port_device(&mut self, port: usize, device: PortDevice) {
        if let Some(system) = self.system.as_mut() {
            let device = match device {
                PortDevice::Gamepad => crate::system::PortDevice::Gamepad,
                PortDevice::Zapper => crate::system::PortDevice::Zapper,
            };
            system.set_port_device(port, device);
        }
    }

    fn set_zapper(&mut self, port: usize, x: u8, y: u8, trigger: bool) {
        if let Some(system) = self.system.as_mut() {
            system.set_zapper(port, x, y, trigger);
        }
    }

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
