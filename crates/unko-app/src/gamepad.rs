//! SDL3 gamepad input for the desktop shell.
//!
//! The same module is used by the desktop, iOS and Android targets; SDL3 owns
//! hot-plug and platform mapping, and this layer only translates gamepad
//! buttons into the core's [`InputState`].

use std::time::{Duration, Instant};

use sdl3::event::Event;
pub use sdl3::gamepad::Button;
use sdl3::gamepad::{Axis, Gamepad};
use sdl3::{EventPump, GamepadSubsystem};
use unko_core::InputState;

/// A NES controller button that can be bound to a gamepad button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NesButton {
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// D-pad left.
    Left,
    /// D-pad right.
    Right,
    /// A button.
    A,
    /// B button.
    B,
    /// Start.
    Start,
    /// Select.
    Select,
}

impl NesButton {
    /// Every bindable button, in UI order.
    pub const ALL: [Self; 8] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::A,
        Self::B,
        Self::Start,
        Self::Select,
    ];

    /// Stable setting key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::A => "a",
            Self::B => "b",
            Self::Start => "start",
            Self::Select => "select",
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Up => "D-pad up",
            Self::Down => "D-pad down",
            Self::Left => "D-pad left",
            Self::Right => "D-pad right",
            Self::A => "A",
            Self::B => "B",
            Self::Start => "Start",
            Self::Select => "Select",
        }
    }

    /// Core input bit.
    #[must_use]
    pub const fn mask(self) -> u16 {
        match self {
            Self::Up => InputState::UP,
            Self::Down => InputState::DOWN,
            Self::Left => InputState::LEFT,
            Self::Right => InputState::RIGHT,
            Self::A => InputState::A,
            Self::B => InputState::B,
            Self::Start => InputState::START,
            Self::Select => InputState::SELECT,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Up => 0,
            Self::Down => 1,
            Self::Left => 2,
            Self::Right => 3,
            Self::A => 4,
            Self::B => 5,
            Self::Start => 6,
            Self::Select => 7,
        }
    }
}

/// Gamepad-to-NES binding table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadMapping {
    bindings: [Button; 8],
}

impl Default for PadMapping {
    fn default() -> Self {
        Self {
            bindings: [
                Button::DPadUp,
                Button::DPadDown,
                Button::DPadLeft,
                Button::DPadRight,
                Button::South,
                Button::West,
                Button::Start,
                Button::Back,
            ],
        }
    }
}

impl PadMapping {
    /// Returns the gamepad button bound to `button`.
    #[must_use]
    pub const fn get(self, button: NesButton) -> Button {
        self.bindings[button.index()]
    }

    /// Binds `button` to a gamepad button.
    pub fn set(&mut self, button: NesButton, gamepad: Button) {
        self.bindings[button.index()] = gamepad;
    }

    /// Restores the standard NES-style layout.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Parses a persisted button name, or returns the default binding.
    #[must_use]
    pub fn parse(self, button: NesButton, value: &str) -> Button {
        button_from_name(value).unwrap_or_else(|| Self::default().get(button))
    }
}

/// Serializes a mapping for a per-game profile file.
#[must_use]
pub fn mapping_to_text(mapping: PadMapping) -> String {
    let mut text = String::new();
    for button in NesButton::ALL {
        text.push_str(&format!(
            "{}={}\n",
            button.key(),
            button_name(mapping.get(button))
        ));
    }
    text
}

/// Parses a per-game profile, falling back to defaults for missing keys.
#[must_use]
pub fn mapping_from_text(text: &str) -> PadMapping {
    let mut mapping = PadMapping::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if let Some(button) = NesButton::ALL
            .iter()
            .find(|button| button.key() == key.trim())
        {
            let bound = mapping.parse(*button, value.trim());
            mapping.set(*button, bound);
        }
    }
    mapping
}

/// Stable name for a gamepad button (used in `unko.conf`).
#[must_use]
pub fn button_name(button: Button) -> &'static str {
    match button {
        Button::South => "south",
        Button::East => "east",
        Button::North => "north",
        Button::West => "west",
        Button::Back => "back",
        Button::Start => "start",
        Button::LeftShoulder => "leftshoulder",
        Button::RightShoulder => "rightshoulder",
        Button::LeftStick => "leftstick",
        Button::RightStick => "rightstick",
        Button::DPadUp => "dpup",
        Button::DPadDown => "dpdown",
        Button::DPadLeft => "dpleft",
        Button::DPadRight => "dpright",
        Button::Misc1 => "misc1",
        Button::Touchpad => "touchpad",
        _ => "south",
    }
}

/// Parses a persisted gamepad button name.
#[must_use]
pub fn button_from_name(value: &str) -> Option<Button> {
    Some(match value {
        "south" => Button::South,
        "east" => Button::East,
        "north" => Button::North,
        "west" => Button::West,
        "back" => Button::Back,
        "start" => Button::Start,
        "leftshoulder" => Button::LeftShoulder,
        "rightshoulder" => Button::RightShoulder,
        "leftstick" => Button::LeftStick,
        "rightstick" => Button::RightStick,
        "dpup" => Button::DPadUp,
        "dpdown" => Button::DPadDown,
        "dpleft" => Button::DPadLeft,
        "dpright" => Button::DPadRight,
        "misc1" => Button::Misc1,
        "touchpad" => Button::Touchpad,
        _ => return None,
    })
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
                        self.captured = Some((target, button));
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
                let mut pressed = pad.button(bound);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_names_round_trip() {
        for button in [
            Button::South,
            Button::East,
            Button::North,
            Button::West,
            Button::Back,
            Button::Start,
            Button::LeftShoulder,
            Button::RightShoulder,
            Button::DPadUp,
            Button::DPadDown,
            Button::DPadLeft,
            Button::DPadRight,
        ] {
            assert_eq!(button_from_name(button_name(button)), Some(button));
        }
    }

    #[test]
    fn mapping_defaults_and_overrides() {
        let mut mapping = PadMapping::default();
        assert_eq!(mapping.get(NesButton::A), Button::South);
        mapping.set(NesButton::A, Button::East);
        assert_eq!(mapping.get(NesButton::A), Button::East);
        mapping.reset();
        assert_eq!(mapping.get(NesButton::A), Button::South);
        assert_eq!(
            mapping.parse(NesButton::Start, "not-a-button"),
            Button::Start
        );
    }

    #[test]
    fn per_game_profile_round_trips() {
        let mut mapping = PadMapping::default();
        mapping.set(NesButton::A, Button::North);
        mapping.set(NesButton::Start, Button::RightShoulder);
        let text = mapping_to_text(mapping);
        assert_eq!(mapping_from_text(&text), mapping);
        assert_eq!(mapping_from_text("a=east"), {
            let mut expected = PadMapping::default();
            expected.set(NesButton::A, Button::East);
            expected
        });
    }
}
