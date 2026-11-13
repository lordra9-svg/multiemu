//! Input movie (TAS) format.
//!
//! A movie stores the full machine state at its start plus one 4-port input
//! word per frame, so playback is exact from any starting point. The format is
//! deliberately small and self-describing:
//!
//! ```text
//! "UNKOMOV1" | stem_len:u16 | stem | state_len:u32 | state | frames:u32 | [4 x u16]
//! ```

use std::fs;
use std::path::Path;

/// Magic/version bytes.
pub const MAGIC: &[u8; 8] = b"UNKOMOV1";

/// A recorded input movie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movie {
    /// Save stem of the ROM the movie belongs to.
    pub stem: String,
    /// Machine state at the start of the first recorded frame.
    pub state: Vec<u8>,
    /// Input bits per frame, in `unko_core::InputState` layout.
    pub inputs: Vec<[u16; 4]>,
}

impl Movie {
    /// Writes the movie; the parent directory is created when needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(directory) = path.parent() {
            if !directory.as_os_str().is_empty() {
                let _ = fs::create_dir_all(directory);
            }
        }
        let mut bytes = Vec::with_capacity(16 + self.state.len() + self.inputs.len() * 8);
        bytes.extend_from_slice(MAGIC);
        let stem = self.stem.as_bytes();
        let stem_len = u16::try_from(stem.len()).unwrap_or(u16::MAX);
        bytes.extend_from_slice(&stem_len.to_le_bytes());
        bytes.extend_from_slice(&stem[..usize::from(stem_len)]);
        let state_len = u32::try_from(self.state.len()).unwrap_or(u32::MAX);
        bytes.extend_from_slice(&state_len.to_le_bytes());
        bytes.extend_from_slice(&self.state[..usize::try_from(state_len).unwrap_or(0)]);
        let frames = u32::try_from(self.inputs.len()).unwrap_or(u32::MAX);
        bytes.extend_from_slice(&frames.to_le_bytes());
        for input in self
            .inputs
            .iter()
            .take(usize::try_from(frames).unwrap_or(0))
        {
            for port in input {
                bytes.extend_from_slice(&port.to_le_bytes());
            }
        }
        fs::write(path, bytes)
    }

    /// Reads a movie, rejecting truncated or foreign files.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|err| err.to_string())?;
        let mut at = 0usize;
        let take = |at: &mut usize, count: usize| -> Result<&[u8], String> {
            let end = at
                .checked_add(count)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| "truncated movie".to_string())?;
            let slice = &bytes[*at..end];
            *at = end;
            Ok(slice)
        };
        if take(&mut at, 8)? != MAGIC {
            return Err("not an Unko input movie".into());
        }
        let stem_len = u16::from_le_bytes(
            take(&mut at, 2)?
                .try_into()
                .map_err(|_| "bad stem length".to_string())?,
        );
        let stem = String::from_utf8(take(&mut at, usize::from(stem_len))?.to_vec())
            .map_err(|_| "invalid movie stem".to_string())?;
        let state_len = u32::from_le_bytes(
            take(&mut at, 4)?
                .try_into()
                .map_err(|_| "bad state length".to_string())?,
        );
        let state = take(&mut at, usize::try_from(state_len).unwrap_or(usize::MAX))?.to_vec();
        let frames = u32::from_le_bytes(
            take(&mut at, 4)?
                .try_into()
                .map_err(|_| "bad frame count".to_string())?,
        );
        let mut inputs = Vec::with_capacity(usize::try_from(frames).unwrap_or(0));
        for _ in 0..frames {
            let mut input = [0u16; 4];
            for port in &mut input {
                *port = u16::from_le_bytes(
                    take(&mut at, 2)?
                        .try_into()
                        .map_err(|_| "bad input word".to_string())?,
                );
            }
            inputs.push(input);
        }
        Ok(Self {
            stem,
            state,
            inputs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_round_trips() {
        let path = std::env::temp_dir().join("unko-test.movie");
        let movie = Movie {
            stem: "game-0123456789abcdef".into(),
            state: vec![1, 2, 3, 4, 5],
            inputs: vec![[1, 0, 0, 0], [2, 0, 0, 0], [3, 0, 0, 0]],
        };
        movie.save(&path).expect("save");
        let loaded = Movie::load(&path).expect("load");
        assert_eq!(loaded, movie);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_foreign_or_truncated_files() {
        let path = std::env::temp_dir().join("unko-bad.movie");
        fs::write(&path, b"not a movie").expect("write");
        assert!(Movie::load(&path).is_err());
        let _ = fs::remove_file(path);
    }
}
