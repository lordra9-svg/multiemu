//! Gamepad binding model and per-platform input backends.
//!
//! The binding table, button names and defaults are platform independent.
//! The backend that actually reads pads differs: SDL3 on desktop (it owns hot
//! plug and the platform mappings) and a no-op backend on mobile, where the
//! touch overlay is the primary controller and SDL is not available without
//! the platform SDKs.

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use unko_core::InputState;

#[cfg(any(target_os = "android", target_os = "ios"))]
mod mobile;
#[cfg(any(target_os = "android", target_os = "ios"))]
pub use mobile::Gamepads;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use desktop::Gamepads;

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
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
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

/// A physical gamepad button in SDL3's naming, independent of the backend.
///
/// Keeping our own enum lets the binding table, settings files and UI compile
/// on mobile, where the SDL3 backend is not built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// South face button (A on Xbox, cross on PlayStation).
    South,
    /// East face button (B on Xbox, circle on PlayStation).
    East,
    /// North face button (Y on Xbox, triangle on PlayStation).
    North,
    /// West face button (X on Xbox, square on PlayStation).
    West,
    /// Back / select / share.
    Back,
    /// Start / options.
    Start,
    /// Left shoulder.
    LeftShoulder,
    /// Right shoulder.
    RightShoulder,
    /// Left stick click.
    LeftStick,
    /// Right stick click.
    RightStick,
    /// D-pad up.
    DPadUp,
    /// D-pad down.
    DPadDown,
    /// D-pad left.
    DPadLeft,
    /// D-pad right.
    DPadRight,
    /// Misc button 1.
    Misc1,
    /// Touchpad click.
    Touchpad,
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
