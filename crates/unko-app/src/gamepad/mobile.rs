//! Mobile gamepad backend.
//!
//! SDL3 is not built for iOS/Android here: those builds need the platform
//! SDKs and the touch overlay is the primary controller. The backend exposes
//! the desktop API so the shell and settings UI compile unchanged; it reports
//! no pads and no button captures. A native GameController/Gamepad backend can
//! replace it later without touching the rest of the shell.

use unko_core::InputState;

use super::{Button, NesButton, PadMapping};

/// Mobile stand-in for the SDL3 gamepad manager.
pub struct Gamepads {
    capture: Option<NesButton>,
    captured: Option<(NesButton, Button)>,
}

impl Gamepads {
    /// Creates a mobile input manager; never fails.
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            capture: None,
            captured: None,
        })
    }

    /// Returns empty input states; the touch overlay supplies the buttons.
    pub fn poll(
        &mut self,
        _mapping: PadMapping,
        _deadzone: i16,
        _turbo: bool,
        _turbo_hz: f32,
    ) -> [InputState; 4] {
        [InputState::empty(); 4]
    }

    /// Starts capturing the next gamepad button for `button`.
    pub fn begin_capture(&mut self, button: NesButton) {
        self.capture = Some(button);
        self.captured = None;
    }

    /// Returns and clears a captured binding.
    pub fn take_captured(&mut self) -> Option<(NesButton, Button)> {
        self.captured.take()
    }

    /// Whether a capture is pending.
    #[must_use]
    pub fn capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// No connected gamepads.
    #[must_use]
    pub fn names(&self) -> [Option<String>; 4] {
        [None, None, None, None]
    }

    /// Number of connected gamepads.
    #[must_use]
    pub fn connected(&self) -> usize {
        0
    }
}
