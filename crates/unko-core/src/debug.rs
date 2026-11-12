//! Optional debugger surface shared by frontends and cores.
//!
//! A core advertises support by returning `Some` from [`Core::debug_cpu`] and
//! friends. Frontends must treat every accessor as optional so a future core
//! without debug hooks still works; all methods default to "unsupported".

/// Address space used by debugger reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebugSpace {
    /// CPU bus address space (`$0000-$FFFF`).
    Cpu,
    /// PPU address space (`$0000-$3FFF`: CHR, nametables, palette).
    Ppu,
    /// Object attribute memory (256 bytes).
    Oam,
}

impl DebugSpace {
    /// Human label for the space.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Ppu => "PPU",
            Self::Oam => "OAM",
        }
    }

    /// Number of addressable bytes in the space.
    #[must_use]
    pub const fn size(self) -> u32 {
        match self {
            Self::Cpu => 0x1_0000,
            Self::Ppu => 0x4000,
            Self::Oam => 0x100,
        }
    }

    /// Clamps an address into the space.
    #[must_use]
    pub const fn clamp(self, address: u32) -> u32 {
        match self {
            Self::Cpu => address & 0xFFFF,
            Self::Ppu => address & 0x3FFF,
            Self::Oam => address & 0xFF,
        }
    }
}

/// Snapshot of the CPU registers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuState {
    /// Accumulator.
    pub a: u8,
    /// X index.
    pub x: u8,
    /// Y index.
    pub y: u8,
    /// Stack pointer.
    pub sp: u8,
    /// Processor status bits.
    pub p: u8,
    /// Program counter.
    pub pc: u16,
    /// Cycles since reset.
    pub cycles: u64,
}

/// Snapshot of the PPU state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PpuState {
    /// Completed frames.
    pub frame: u64,
    /// Current scanline (`0..=261` on NTSC).
    pub scanline: u16,
    /// Current dot within the scanline.
    pub dot: u16,
    /// `$2000` value.
    pub ctrl: u8,
    /// `$2001` value.
    pub mask: u8,
    /// `$2002` value.
    pub status: u8,
    /// Current VRAM address (`v`).
    pub v: u16,
    /// Temporary VRAM address (`t`).
    pub t: u16,
}

/// Snapshot of the APU channel outputs and frame counter.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ApuState {
    /// Pulse 1 DAC output.
    pub pulse1: f32,
    /// Pulse 2 DAC output.
    pub pulse2: f32,
    /// Triangle DAC output.
    pub triangle: f32,
    /// Noise DAC output.
    pub noise: f32,
    /// DMC DAC output.
    pub dmc: f32,
    /// Mixed analog output before resampling.
    pub mixed: f32,
    /// Frame-counter step index.
    pub frame_step: u8,
    /// Cycles into the current frame-counter step.
    pub frame_cycle: u32,
}

/// One decoded instruction for a disassembly listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
    /// Address of the opcode.
    pub address: u16,
    /// Instruction bytes (1-3).
    pub bytes: Vec<u8>,
    /// Formatted instruction, for example `LDA $2002`.
    pub text: String,
}
