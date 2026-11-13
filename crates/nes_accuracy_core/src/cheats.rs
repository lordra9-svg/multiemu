//! Game Genie code decoding.
//!
//! The NES Game Genie is a pass-through cartridge that intercepts CPU reads
//! in `$8000-$FFFF`. A six-character code substitutes one byte; an
//! eight-character code also compares the original byte, so it only fires in
//! the intended bank. Codes are a runtime overlay and are not part of a save
//! state.
//!
//! The nibble alphabet and bit shuffling follow the published Game Genie
//! format (the canonical `SXIOPO` and `YYKPOYZZ` examples are pinned by the
//! tests). No emulator code was copied.

use std::fmt;

/// Errors returned when a code cannot be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenieError {
    /// The code was not six or eight characters long.
    InvalidLength(usize),
    /// The code contained a character outside the Game Genie alphabet.
    InvalidCharacter(char),
}

impl fmt::Display for GenieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength(length) => {
                write!(f, "Game Genie codes are 6 or 8 characters, got {length}")
            }
            Self::InvalidCharacter(character) => {
                write!(f, "invalid Game Genie character '{character}'")
            }
        }
    }
}

impl std::error::Error for GenieError {}

/// A decoded Game Genie code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameGenie {
    code: String,
    address: u16,
    value: u8,
    compare: Option<u8>,
}

impl GameGenie {
    /// Decodes a six- or eight-character code (case-insensitive).
    pub fn parse(code: &str) -> Result<Self, GenieError> {
        let length = code.chars().count();
        if length != 6 && length != 8 {
            return Err(GenieError::InvalidLength(length));
        }
        let letters: Vec<u8> = code
            .chars()
            .map(letter_value)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                let bad = code
                    .chars()
                    .find(|character| letter_value(*character).is_none())
                    .unwrap_or('?');
                GenieError::InvalidCharacter(bad)
            })?;
        let n = &letters;
        let address = 0x8000
            | ((u16::from(n[3]) & 7) << 12)
            | ((u16::from(n[5]) & 7) << 8)
            | ((u16::from(n[4]) & 8) << 8)
            | ((u16::from(n[2]) & 7) << 4)
            | ((u16::from(n[1]) & 8) << 4)
            | (u16::from(n[4]) & 7)
            | (u16::from(n[3]) & 8);
        let high = if letters.len() == 6 { n[5] } else { n[7] };
        let value = ((n[1] & 7) << 4) | ((n[0] & 8) << 4) | (n[0] & 7) | (high & 8);
        let compare = (letters.len() == 8)
            .then(|| ((n[7] & 7) << 4) | ((n[6] & 8) << 4) | (n[6] & 7) | (n[5] & 8));
        Ok(Self {
            code: code.to_ascii_uppercase(),
            address,
            value,
            compare,
        })
    }

    /// Canonical upper-case code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// CPU address in `$8000-$FFFF`.
    #[must_use]
    pub const fn address(&self) -> u16 {
        self.address
    }

    /// Substitute byte.
    #[must_use]
    pub const fn value(&self) -> u8 {
        self.value
    }

    /// Compare byte for eight-character codes.
    #[must_use]
    pub const fn compare(&self) -> Option<u8> {
        self.compare
    }

    /// Applies the substitution to an original ROM byte, honoring the
    /// optional compare.
    #[must_use]
    pub const fn apply(&self, original: u8) -> u8 {
        match self.compare {
            Some(compare) if original != compare => original,
            _ => self.value,
        }
    }
}

const fn letter_value(character: char) -> Option<u8> {
    Some(match character.to_ascii_uppercase() {
        'A' => 0x0,
        'P' => 0x1,
        'Z' => 0x2,
        'L' => 0x3,
        'G' => 0x4,
        'I' => 0x5,
        'T' => 0x6,
        'Y' => 0x7,
        'E' => 0x8,
        'O' => 0x9,
        'X' => 0xA,
        'U' => 0xB,
        'K' => 0xC,
        'S' => 0xD,
        'V' => 0xE,
        'N' => 0xF,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_six_character_codes() {
        let code = GameGenie::parse("SXIOPO").expect("code");
        assert_eq!(code.address(), 0x91D9);
        assert_eq!(code.value(), 0xAD);
        assert_eq!(code.compare(), None);
        assert_eq!(code.apply(0x00), 0xAD);
        assert_eq!(code.code(), "SXIOPO");
    }

    #[test]
    fn decodes_eight_character_codes_with_compare() {
        let code = GameGenie::parse("yykpoyzz").expect("code");
        assert_eq!(code.address(), 0x9F41);
        assert_eq!(code.value(), 0x77);
        assert_eq!(code.compare(), Some(0x22));
        assert_eq!(code.apply(0x22), 0x77);
        assert_eq!(code.apply(0x23), 0x23);
        assert_eq!(code.code(), "YYKPOYZZ");
    }

    #[test]
    fn rejects_bad_lengths_and_letters() {
        assert_eq!(GameGenie::parse("ABC"), Err(GenieError::InvalidLength(3)));
        assert_eq!(
            GameGenie::parse("!BCDEF"),
            Err(GenieError::InvalidCharacter('!'))
        );
    }
}
