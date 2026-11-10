//! NSF / NSFE (NES Sound Format) parsing and player wiring.
//!
//! An NSF file carries a header and a raw 6502 payload. The payload is mapped
//! into a synthetic NROM image, together with a small player stub that calls
//! the payload's INIT routine once and its PLAY routine from the NMI handler
//! (once per frame, matching the common 60.09 Hz NTSC speed). Games that use a
//! different speed or bankswitching are out of scope here; the header fields
//! are still exposed for callers that want to handle them.

use crate::system::System;

/// NSF header magic.
const NSF_MAGIC: &[u8; 5] = b"NESM\x1a";
/// NSFE header magic.
const NSFE_MAGIC: &[u8; 4] = b"NSFE";
/// Address the player stub is placed at in the synthetic NROM image.
const STUB_ADDR: u16 = 0xFF00;
/// NMI handler address inside the stub.
const NMI_ADDR: u16 = 0xFF20;
/// Size of a 32 KiB NROM PRG bank set.
const PRG_SIZE: usize = 0x8000;

/// Why an NSF file could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NsfError {
    /// The file is shorter than the format requires.
    Truncated,
    /// The magic number is neither `NESM\x1A` nor `NSFE`.
    BadMagic,
    /// The payload would overwrite the player stub or the vectors.
    PayloadTooLarge,
    /// The synthetic cartridge image is invalid.
    Cart(String),
    /// The payload exceeds the 32 KiB NROM window.
    UnsupportedBankswitching,
}

impl std::fmt::Display for NsfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "NSF file is truncated"),
            Self::BadMagic => write!(f, "not an NSF/NSFE file"),
            Self::PayloadTooLarge => write!(f, "NSF payload is too large"),
            Self::Cart(err) => write!(f, "invalid cartridge: {err}"),
            Self::UnsupportedBankswitching => {
                write!(f, "bank-switched NSF payloads are not supported")
            }
        }
    }
}

impl std::error::Error for NsfError {}

/// Parsed NSF/NSFE metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NsfHeader {
    /// Format version (1 for NSF, 0 for NSFE).
    pub version: u8,
    /// Number of songs in the file.
    pub songs: u8,
    /// First song to play (1-based).
    pub start_song: u8,
    /// Address the payload is loaded at.
    pub load_addr: u16,
    /// Address of the INIT routine.
    pub init_addr: u16,
    /// Address of the PLAY routine.
    pub play_addr: u16,
    /// Song name (NUL-padded).
    pub title: String,
    /// Artist name.
    pub artist: String,
    /// Copyright line.
    pub copyright: String,
    /// NTSC playback speed in microseconds per play call.
    pub ntsc_speed: u16,
    /// PAL playback speed in microseconds per play call.
    pub pal_speed: u16,
    /// Bankswitch flags from the NSF header (0 for NSFE unless set).
    pub bankswitch: u8,
}

/// An NSF file: header plus the player-ready cartridge image.
#[derive(Debug, Clone)]
pub struct Nsf {
    header: NsfHeader,
    payload: Vec<u8>,
    rom: Vec<u8>,
}

fn read_u16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn read_string(data: &[u8], at: usize, len: usize) -> String {
    let slice = &data[at..at + len];
    let end = slice.iter().position(|&byte| byte == 0).unwrap_or(len);
    String::from_utf8_lossy(&slice[..end]).trim().to_string()
}

impl Nsf {
    /// Parses an NSF or NSFE file.
    pub fn parse(data: &[u8]) -> Result<Self, NsfError> {
        let (header, payload) = if data.starts_with(NSF_MAGIC) {
            parse_nsf(data)?
        } else if data.starts_with(NSFE_MAGIC) {
            parse_nsfe(data)?
        } else {
            return Err(NsfError::BadMagic);
        };
        let rom = build_rom(&header, &payload)?;
        Ok(Self {
            header,
            payload,
            rom,
        })
    }

    /// Header fields.
    #[must_use]
    pub const fn header(&self) -> &NsfHeader {
        &self.header
    }

    /// Raw payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Synthetic iNES image (32 KiB NROM, payload at the load address, player
    /// stub at `$FF00`, vectors set).
    #[must_use]
    pub fn rom_image(&self) -> &[u8] {
        &self.rom
    }

    /// Builds a running [`System`] for this NSF.
    pub fn system(&self) -> Result<System, NsfError> {
        let mut system =
            System::from_ines(&self.rom).map_err(|err| NsfError::Cart(err.to_string()))?;
        // NSF playback starts the player immediately; the console power-on
        // warm-up (which ignores PPU register writes for one frame) does not
        // apply to a music driver.
        system.ppu.end_warmup();
        Ok(system)
    }
}

/// Parses the fixed-layout `NESM\x1A` header (128 bytes).
fn parse_nsf(data: &[u8]) -> Result<(NsfHeader, Vec<u8>), NsfError> {
    if data.len() < 0x80 {
        return Err(NsfError::Truncated);
    }
    let load = read_u16(data, 0x08);
    let header = NsfHeader {
        version: data[0x05],
        songs: data[0x06],
        start_song: data[0x07],
        load_addr: if load == 0 { 0x8000 } else { load },
        init_addr: read_u16(data, 0x0A),
        play_addr: read_u16(data, 0x0C),
        title: read_string(data, 0x0E, 32),
        artist: read_string(data, 0x2E, 32),
        copyright: read_string(data, 0x4E, 32),
        ntsc_speed: read_u16(data, 0x6E),
        bankswitch: data[0x70],
        pal_speed: read_u16(data, 0x78),
    };
    Ok((header, data[0x80..].to_vec()))
}

/// Parses an `NSFE` chunk file (INFO chunk + DATA chunk).
fn parse_nsfe(data: &[u8]) -> Result<(NsfHeader, Vec<u8>), NsfError> {
    if data.len() < 8 {
        return Err(NsfError::Truncated);
    }
    let mut header = NsfHeader {
        version: 0,
        songs: 1,
        start_song: 1,
        load_addr: 0x8000,
        ntsc_speed: 16_639,
        pal_speed: 19_956,
        ..NsfHeader::default()
    };
    let mut payload = Vec::new();
    let mut at = 4usize;
    while at + 8 <= data.len() {
        let id = &data[at..at + 4];
        let len = u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]);
        let len = len as usize;
        at += 8;
        if at + len > data.len() {
            return Err(NsfError::Truncated);
        }
        match id {
            b"INFO" => {
                if len < 6 {
                    return Err(NsfError::Truncated);
                }
                header.load_addr = read_u16(data, at);
                header.init_addr = read_u16(data, at + 2);
                header.play_addr = read_u16(data, at + 4);
            }
            b"DATA" => payload = data[at..at + len].to_vec(),
            b"NEND" => break,
            _ => {}
        }
        at += len;
    }
    if payload.is_empty() {
        return Err(NsfError::Truncated);
    }
    Ok((header, payload))
}

/// Builds a 32 KiB NROM image with the payload and the player stub.
fn build_rom(header: &NsfHeader, payload: &[u8]) -> Result<Vec<u8>, NsfError> {
    let stub_offset = usize::from(STUB_ADDR - 0x8000);
    let load_offset = usize::from(header.load_addr.wrapping_sub(0x8000));
    if load_offset >= stub_offset || load_offset + payload.len() > stub_offset {
        return Err(NsfError::PayloadTooLarge);
    }
    let mut prg = vec![0u8; PRG_SIZE];
    prg[load_offset..load_offset + payload.len()].copy_from_slice(payload);

    // Player stub at $FF00:
    //   SEI; CLD; LDX #start-1; LDA #0; JSR init;
    //   LDA #$80; STA $2000; JMP self
    // NMI at $FF10: JSR play; RTI
    let init = header.init_addr;
    let play = header.play_addr;
    let song = header.start_song.saturating_sub(1);
    let stub = vec![
        0x78, // SEI
        0xD8, // CLD
        0xA2,
        song, // LDX #song
        0xA9,
        0x00, // LDA #0
        0x20,
        init as u8,
        (init >> 8) as u8, // JSR init
        0xA9,
        0x80, // LDA #$80
        0x8D,
        0x00,
        0x20, // STA $2000 (enable NMI)
        0x4C,
        0x0E,
        0xFF, // JMP $FF0E (self)
    ];
    prg[stub_offset..stub_offset + stub.len()].copy_from_slice(&stub);
    let nmi = [0x20, play as u8, (play >> 8) as u8, 0x40]; // JSR play; RTI
    let nmi_offset = usize::from(NMI_ADDR - 0x8000);
    prg[nmi_offset..nmi_offset + nmi.len()].copy_from_slice(&nmi);

    let vector = |addr: u16| [addr as u8, (addr >> 8) as u8];
    prg[0x7FFA..0x7FFC].copy_from_slice(&vector(NMI_ADDR));
    prg[0x7FFC..0x7FFE].copy_from_slice(&vector(STUB_ADDR));
    prg[0x7FFE..0x8000].copy_from_slice(&vector(STUB_ADDR));

    let mut rom = Vec::with_capacity(16 + PRG_SIZE + 0x2000);
    rom.extend_from_slice(b"NES\x1A");
    rom.push(2); // 2 × 16 KiB PRG
    rom.push(1); // 1 × 8 KiB CHR
    rom.extend_from_slice(&[0u8; 10]);
    rom.extend_from_slice(&prg);
    rom.extend_from_slice(&vec![0u8; 0x2000]);
    Ok(rom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_nsf() -> Vec<u8> {
        let mut data = vec![0u8; 0x80];
        data[0..5].copy_from_slice(NSF_MAGIC);
        data[0x05] = 1;
        data[0x06] = 2;
        data[0x07] = 1;
        data[0x08..0x0A].copy_from_slice(&0x8000u16.to_le_bytes());
        data[0x0A..0x0C].copy_from_slice(&0x8000u16.to_le_bytes()); // INIT
        data[0x0C..0x0E].copy_from_slice(&0x8006u16.to_le_bytes()); // PLAY
        data[0x0E..0x12].copy_from_slice(b"Test");
        data[0x6E..0x70].copy_from_slice(&16_639u16.to_le_bytes());
        // INIT: LDA #$5A; STA $0300; RTS
        data.extend_from_slice(&[0xA9, 0x5A, 0x8D, 0x00, 0x03, 0x60]);
        // PLAY: INC $0301; RTS
        data.extend_from_slice(&[0xEE, 0x01, 0x03, 0x60]);
        data
    }

    #[test]
    fn parses_nsf_header_and_payload() {
        let nsf = Nsf::parse(&synthetic_nsf()).expect("parse");
        let header = nsf.header();
        assert_eq!(header.version, 1);
        assert_eq!(header.songs, 2);
        assert_eq!(header.load_addr, 0x8000);
        assert_eq!(header.init_addr, 0x8000);
        assert_eq!(header.play_addr, 0x8006);
        assert_eq!(header.title, "Test");
        assert_eq!(header.ntsc_speed, 16_639);
        assert_eq!(nsf.payload().len(), 10);
    }

    #[test]
    fn rom_image_maps_payload_and_vectors() {
        let nsf = Nsf::parse(&synthetic_nsf()).expect("parse");
        let rom = nsf.rom_image();
        assert_eq!(&rom[0..4], b"NES\x1A");
        let prg = &rom[16..16 + 0x8000];
        // Payload at $8000.
        assert_eq!(&prg[0..6], &[0xA9, 0x5A, 0x8D, 0x00, 0x03, 0x60]);
        // Stub at $FF00 (offset 0x7F00) starts with SEI/CLD.
        assert_eq!(&prg[0x7F00..0x7F02], &[0x78, 0xD8]);
        // NMI vector points at the stub's NMI handler.
        assert_eq!(&prg[0x7FFA..0x7FFC], &[0x20, 0xFF]);
        assert_eq!(&prg[0x7FFC..0x7FFE], &[0x00, 0xFF]);
    }

    #[test]
    fn player_calls_init_and_play() {
        let nsf = Nsf::parse(&synthetic_nsf()).expect("parse");
        let mut system = nsf.system().expect("system");
        // The first call reports the reset's power-on frame without running.
        for _ in 0..5 {
            system.run_frame();
        }
        assert_eq!(system.read_ram(0x0300), 0x5A, "INIT did not run");
        assert!(
            system.read_ram(0x0301) > 0,
            "PLAY did not run from the NMI handler"
        );
        assert!(!system.audio().is_empty(), "no audio produced");
    }

    #[test]
    fn parses_nsfe_info_chunk() {
        let mut data = Vec::new();
        data.extend_from_slice(NSFE_MAGIC);
        let mut info = Vec::new();
        info.extend_from_slice(&0x8000u16.to_le_bytes());
        info.extend_from_slice(&0x8123u16.to_le_bytes());
        info.extend_from_slice(&0x8456u16.to_le_bytes());
        data.extend_from_slice(b"INFO");
        data.extend_from_slice(&(info.len() as u32).to_le_bytes());
        data.extend_from_slice(&info);
        data.extend_from_slice(b"DATA");
        data.extend_from_slice(&4u32.to_le_bytes());
        data.extend_from_slice(&[0xEA, 0xEA, 0xEA, 0x60]);
        data.extend_from_slice(b"NEND");
        data.extend_from_slice(&0u32.to_le_bytes());
        let nsf = Nsf::parse(&data).expect("parse nsfe");
        assert_eq!(nsf.header().load_addr, 0x8000);
        assert_eq!(nsf.header().init_addr, 0x8123);
        assert_eq!(nsf.header().play_addr, 0x8456);
        assert_eq!(nsf.payload(), &[0xEA, 0xEA, 0xEA, 0x60]);
    }

    #[test]
    fn rejects_non_nsf_data() {
        assert_eq!(Nsf::parse(b"plain rom").unwrap_err(), NsfError::BadMagic);
        assert_eq!(Nsf::parse(b"NESM\x1a").unwrap_err(), NsfError::Truncated);
    }
}
