//! SDL3 gamepad backend.
//!
//! SDL3 owns hot-plug detection and the platform controller mappings; this
//! layer translates its buttons into the shared [`Button`] enum and polls the
//! four pad slots into NES input states.

use std::time::{Duration, Instant};

use sdl3::event::Event;
use sdl3::gamepad::Button as SdlButton;
use sdl3::gamepad::{Axis, Gamepad};
use sdl3::{EventPump, GamepadSubsystem};
use unko_core::InputState;

use super::{Button, NesButton, PadMapping};

/// Converts an SDL button to the shell's backend-independent button.
fn from_sdl(button: SdlButton) -> Button {
    match button {
        SdlButton::South => Button::South,
        SdlButton::East => Button::East,
        SdlButton::North => Button::North,
        SdlButton::West => Button::West,
        SdlButton::Back => Button::Back,
        SdlButton::Start => Button::Start,
        SdlButton::LeftShoulder => Button::LeftShoulder,
        SdlButton::RightShoulder => Button::RightShoulder,
        SdlButton::LeftStick => Button::LeftStick,
        SdlButton::RightStick => Button::RightStick,
        SdlButton::DPadUp => Button::DPadUp,
        SdlButton::DPadDown => Button::DPadDown,
        SdlButton::DPadLeft => Button::DPadLeft,
        SdlButton::DPadRight => Button::DPadRight,
        SdlButton::Misc1 => Button::Misc1,
        SdlButton::Touchpad => Button::Touchpad,
        _ => Button::South,
    }
}

/// Converts the shell's button to SDL's enum.
fn to_sdl(button: Button) -> SdlButton {
    match button {
        Button::South => SdlButton::South,
        Button::East => SdlButton::East,
        Button::North => SdlButton::North,
        Button::West => SdlButton::West,
        Button::Back => SdlButton::Back,
        Button::Start => SdlButton::Start,
        Button::LeftShoulder => SdlButton::LeftShoulder,
        Button::RightShoulder => SdlButton::RightShoulder,
        Button::LeftStick => SdlButton::LeftStick,
        Button::RightStick => SdlButton::RightStick,
        Button::DPadUp => SdlButton::DPadUp,
        Button::DPadDown => SdlButton::DPadDown,
        Button::DPadLeft => SdlButton::DPadLeft,
        Button::DPadRight => SdlButton::DPadRight,
        Button::Misc1 => SdlButton::Misc1,
        Button::Touchpad => SdlButton::Touchpad,
    }
}

struct Slot {
    id: Option<sdl3::joystick::JoystickId>,
    name: String,
    pad: Option<Gamepad>,
}

/// Connected gamepads and their current NES input state.
pub struct Gamepads {
    _subsystem: GamepadSubsystem,
    events: EventPump,
    slots: [Slot; 4],
    capture: Option<NesButton>,
    captured: Option<(NesButton, Button)>,
    turbo_on: bool,
    last_turbo: Instant,
}

impl Gamepads {
    /// Opens SDL3 and any gamepads that are already connected.
    pub fn new() -> Result<Self, String> {
        sdl3::hint::set("SDL_JOYSTICK_THREAD", "1");
        let sdl = sdl3::init().map_err(|err| format!("SDL3 init: {err}"))?;
        let subsystem = sdl
            .gamepad()
            .map_err(|err| format!("SDL3 gamepad: {err}"))?;
        let events = sdl
            .event_pump()
            .map_err(|err| format!("SDL3 events: {err}"))?;
        let mut gamepads = Self {
            _subsystem: subsystem.clone(),
            events,
            slots: std::array::from_fn(|_| Slot {
                id: None,
                name: String::new(),
                pad: None,
            }),
            capture: None,
            captured: None,
            turbo_on: false,
            last_turbo: Instant::now(),
        };
        gamepads.refresh();
        Ok(gamepads)
    }

    /// Re-enumerates connected gamepads, opening new ones into free slots.
    pub fn refresh(&mut self) {
        let ids = match self._subsystem.gamepads() {
            Ok(ids) => ids,
            Err(_) => return,
        };
        for id in ids {
            if self.slots.iter().any(|slot| slot.id == Some(id)) {
                continue;
            }
            let Some(index) = self.slots.iter().position(|slot| slot.id.is_none()) else {
                break;
            };
            self.open(index, id);
        }
    }

    fn open(&mut self, index: usize, id: sdl3::joystick::JoystickId) {
        let Ok(pad) = self._subsystem.open(id) else {
            return;
        };
        let name = pad
            .name()
            .unwrap_or_else(|| format!("Gamepad {}", index + 1));
        let _ = pad.set_player_index(index as u16);
        self.slots[index] = Slot {
            id: Some(id),
            name,
            pad: Some(pad),
        };
    }

    fn close(&mut self, id: sdl3::joystick::JoystickId) {
        if let Some(index) = self.slots.iter().position(|slot| slot.id == Some(id)) {
            self.slots[index] = Slot {
                id: None,
                name: String::new(),
                pad: None,
            };
        }
    }

    /// Drains SDL events and returns the four NES input states.
    pub fn poll(
        &mut self,
        mapping: PadMapping,
        deadzone: i16,
        turbo: bool,
        turbo_hz: f32,
    ) -> [InputState; 4] {
        let events: Vec<Event> = self.events.poll_iter().collect();
        for event in events {
            match event {
                Event::GamepadAdded { which, .. } => {
                    if let Some(index) = self.slots.iter().position(|slot| slot.id.is_none()) {
                        self.open(index, which);
                    }
                }
                Event::GamepadRemoved { which, .. } => self.close(which),
                Event::GamepadButtonDown { button, .. } => {
                    if let Some(target) = self.capture.take() {
                        self.captured = Some((target, from_sdl(button)));
                    }
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let half = if turbo && turbo_hz > 0.0 {
            Duration::from_secs_f32(0.5 / turbo_hz)
        } else {
            Duration::ZERO
        };
        if half.is_zero() {
            self.turbo_on = true;
        } else if now.duration_since(self.last_turbo) >= half {
            self.turbo_on = !self.turbo_on;
            self.last_turbo = now;
        }

        let mut states = [InputState::empty(); 4];
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(pad) = &slot.pad else {
                continue;
            };
            let mut bits = 0u16;
            for button in NesButton::ALL {
                let bound = mapping.get(button);
                let mut pressed = pad.button(to_sdl(bound));
                if matches!(
                    button,
                    NesButton::Up | NesButton::Down | NesButton::Left | NesButton::Right
                ) {
                    let (axis, negative) = match button {
                        NesButton::Up => (Axis::LeftY, true),
                        NesButton::Down => (Axis::LeftY, false),
                        NesButton::Left => (Axis::LeftX, true),
                        _ => (Axis::LeftX, false),
                    };
                    let value = pad.axis(axis);
                    pressed |= if negative {
                        value < -deadzone
                    } else {
                        value > deadzone
                    };
                }
                if pressed {
                    if turbo && matches!(button, NesButton::A | NesButton::B) && !self.turbo_on {
                        continue;
                    }
                    bits |= button.mask();
                }
            }
            states[index] = InputState::from_bits(bits);
        }
        states
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

    /// Connected gamepad names in slot order.
    #[must_use]
    pub fn names(&self) -> [Option<String>; 4] {
        std::array::from_fn(|index| {
            self.slots[index]
                .pad
                .as_ref()
                .map(|_| self.slots[index].name.clone())
        })
    }

    /// Number of connected gamepads.
    #[must_use]
    pub fn connected(&self) -> usize {
        self.slots.iter().filter(|slot| slot.pad.is_some()).count()
    }
}
