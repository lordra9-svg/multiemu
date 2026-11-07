//! Controller state shared by the host and every core.

/// One controller's buttons, packed in NES shift-register order.
///
/// The bit assignments match the order a real NES reports buttons on `$4016`
/// (`A`, `B`, `Select`, `Start`, `Up`, `Down`, `Left`, `Right`), which also
/// makes the value easy to trace on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputState(u16);

impl InputState {
    /// A button.
    pub const A: u16 = 1 << 0;
    /// B button.
    pub const B: u16 = 1 << 1;
    /// Select button.
    pub const SELECT: u16 = 1 << 2;
    /// Start button.
    pub const START: u16 = 1 << 3;
    /// D-pad up.
    pub const UP: u16 = 1 << 4;
    /// D-pad down.
    pub const DOWN: u16 = 1 << 5;
    /// D-pad left.
    pub const LEFT: u16 = 1 << 6;
    /// D-pad right.
    pub const RIGHT: u16 = 1 << 7;

    /// No buttons held.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Builds state from a raw bit pattern.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Raw bit pattern.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Returns a copy with `button` held or released.
    #[must_use]
    pub const fn with(self, button: u16, pressed: bool) -> Self {
        if pressed {
            Self(self.0 | button)
        } else {
            Self(self.0 & !button)
        }
    }

    /// Whether a button bit is held.
    #[must_use]
    pub const fn is_pressed(self, button: u16) -> bool {
        self.0 & button != 0
    }
}

/// A named button, for mapping UI actions to [`InputState`] bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// A button.
    A,
    /// B button.
    B,
    /// Select.
    Select,
    /// Start.
    Start,
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// D-pad left.
    Left,
    /// D-pad right.
    Right,
}

impl Button {
    /// The [`InputState`] bit this button corresponds to.
    #[must_use]
    pub const fn bit(self) -> u16 {
        match self {
            Self::A => InputState::A,
            Self::B => InputState::B,
            Self::Select => InputState::SELECT,
            Self::Start => InputState::START,
            Self::Up => InputState::UP,
            Self::Down => InputState::DOWN,
            Self::Left => InputState::LEFT,
            Self::Right => InputState::RIGHT,
        }
    }
}
