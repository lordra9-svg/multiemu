//! Core discovery: which implementations exist for which system.
//!
//! The registry is how "multi-emulator" stays honest: a system can have more
//! than one core, and the host chooses by id (or takes the first/default).

use crate::core::Core;
use crate::types::{CoreInfo, SystemId};

/// Constructor for a fresh, unloaded core.
pub type Factory = fn() -> Box<dyn Core>;

/// One registerable core implementation.
pub struct CoreRegistration {
    /// Static metadata, including the system it serves.
    pub info: CoreInfo,
    /// Creates an instance.
    pub factory: Factory,
}

/// All cores linked into this build.
#[derive(Default)]
pub struct Registry {
    cores: Vec<CoreRegistration>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self { cores: Vec::new() }
    }

    /// Adds a core. Later registrations for the same id are ignored.
    pub fn register(&mut self, registration: CoreRegistration) {
        if self
            .cores
            .iter()
            .any(|existing| existing.info.id == registration.info.id)
        {
            return;
        }
        self.cores.push(registration);
    }

    /// Every registered core.
    pub fn iter(&self) -> impl Iterator<Item = &CoreRegistration> {
        self.cores.iter()
    }

    /// Systems with at least one core, in registration order.
    #[must_use]
    pub fn systems(&self) -> Vec<SystemId> {
        let mut systems = Vec::new();
        for core in &self.cores {
            if !systems.contains(&core.info.system) {
                systems.push(core.info.system);
            }
        }
        systems
    }

    /// Cores serving `system`.
    pub fn cores_for(&self, system: SystemId) -> impl Iterator<Item = &CoreRegistration> {
        self.cores
            .iter()
            .filter(move |core| core.info.system == system)
    }

    /// Looks up a core by system and id.
    #[must_use]
    pub fn find(&self, system: SystemId, id: &str) -> Option<&CoreRegistration> {
        self.cores
            .iter()
            .find(|core| core.info.system == system && core.info.id == id)
    }

    /// Instantiates a core by system and id.
    #[must_use]
    pub fn create(&self, system: SystemId, id: &str) -> Option<Box<dyn Core>> {
        self.find(system, id).map(|core| (core.factory)())
    }

    /// Instantiates the first registered core for `system`.
    #[must_use]
    pub fn create_default(&self, system: SystemId) -> Option<Box<dyn Core>> {
        self.cores_for(system).next().map(|core| (core.factory)())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CoreError;
    use crate::types::{
        Accuracy, AudioBuffer, Features, FrameBuffer, FrameStats, LoadOptions, PixelFormat,
    };

    struct Dummy;

    impl Core for Dummy {
        fn info(&self) -> CoreInfo {
            CoreInfo {
                id: "dummy",
                name: "Dummy",
                version: "0",
                system: SystemId::Nes,
                accuracy: Accuracy::Compatible,
                features: Features::empty(),
            }
        }

        fn load(&mut self, _rom: &[u8], _options: &LoadOptions) -> Result<(), CoreError> {
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            Ok(())
        }

        fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
            Ok(FrameStats::default())
        }

        fn video(&self) -> FrameBuffer<'_> {
            FrameBuffer {
                data: &[],
                width: 0,
                height: 0,
                stride: 0,
                format: PixelFormat::Rgba8,
            }
        }

        fn audio(&self) -> AudioBuffer<'_> {
            AudioBuffer {
                samples: &[],
                sample_rate: 48_000.0,
                channels: 1,
            }
        }

        fn set_input(&mut self, _port: usize, _input: crate::input::InputState) {}

        fn set_sample_rate(&mut self, _sample_rate: f32) {}

        fn set_video_filter(&mut self, _filter: crate::types::VideoFilter) {}

        fn save_state(&mut self, _out: &mut Vec<u8>) -> Result<(), CoreError> {
            Ok(())
        }

        fn load_state(&mut self, _state: &[u8]) -> Result<(), CoreError> {
            Ok(())
        }

        fn sram(&mut self) -> &[u8] {
            &[]
        }
    }

    fn registration() -> CoreRegistration {
        CoreRegistration {
            info: Dummy.info(),
            factory: || Box::new(Dummy),
        }
    }

    #[test]
    fn registers_and_creates() {
        let mut registry = Registry::new();
        registry.register(registration());
        assert_eq!(registry.systems(), vec![SystemId::Nes]);
        assert!(registry.create(SystemId::Nes, "dummy").is_some());
        assert!(registry.create(SystemId::Snes, "dummy").is_none());
        assert!(registry.create_default(SystemId::Nes).is_some());
    }

    #[test]
    fn duplicate_ids_are_ignored() {
        let mut registry = Registry::new();
        registry.register(registration());
        registry.register(registration());
        assert_eq!(registry.iter().count(), 1);
    }
}
