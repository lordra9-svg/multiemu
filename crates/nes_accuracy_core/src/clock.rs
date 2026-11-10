//! Region timing as data.
//!
//! The NES family shares a master clock of `6 × colourburst`, but divides it
//! differently per region. Keeping the numbers in one table avoids `#[cfg]`
//! forks and makes PAL/Dendy first-class from the start.
//!
//! Values:
//!
//! | Region | Master (Hz) | PPU divide | CPU divide | Lines | Dots/line |
//! |---|---|---|---|---|---|
//! | NTSC | 21,477,272 | 4 | 12 | 262 | 341 |
//! | PAL  | 26,601,712 | 5 | 16 | 312 | 341 |
//! | Dendy| 26,601,712 | 5 | 15 | 312 | 341 |
//!
//! NTSC has an odd-frame dot skip; PAL/Dendy do not. That is a property of the
//! PPU model, not of this table.

/// NES timing region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    /// NTSC (Japan/US).
    Ntsc,
    /// PAL (Europe).
    Pal,
    /// Dendy (Russian PAL famiclone, faster CPU).
    Dendy,
}

/// Derived timing for one region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Region.
    pub region: Region,
    /// Master clock in Hz (`6 × colourburst`).
    pub master_hz: u64,
    /// Master clocks per PPU dot.
    pub ppu_divider: u32,
    /// Master clocks per CPU cycle.
    pub cpu_divider: u32,
    /// Scanlines per frame (including vblank).
    pub scanlines: u16,
    /// Dots per scanline.
    pub dots_per_line: u16,
}

impl Timing {
    /// NTSC timing.
    pub const NTSC: Self = Self {
        region: Region::Ntsc,
        master_hz: 21_477_272,
        ppu_divider: 4,
        cpu_divider: 12,
        scanlines: 262,
        dots_per_line: 341,
    };

    /// PAL timing.
    pub const PAL: Self = Self {
        region: Region::Pal,
        master_hz: 26_601_712,
        ppu_divider: 5,
        cpu_divider: 16,
        scanlines: 312,
        dots_per_line: 341,
    };

    /// Dendy timing: PAL PPU, NTSC-like CPU ratio.
    pub const DENDY: Self = Self {
        region: Region::Dendy,
        master_hz: 26_601_712,
        ppu_divider: 5,
        cpu_divider: 15,
        scanlines: 312,
        dots_per_line: 341,
    };

    /// Timing for a region.
    #[must_use]
    pub const fn for_region(region: Region) -> Self {
        match region {
            Region::Ntsc => Self::NTSC,
            Region::Pal => Self::PAL,
            Region::Dendy => Self::DENDY,
        }
    }

    /// PPU dot clock in Hz.
    #[must_use]
    pub const fn ppu_hz(self) -> u64 {
        self.master_hz / self.ppu_divider as u64
    }

    /// CPU clock in Hz.
    #[must_use]
    pub const fn cpu_hz(self) -> u64 {
        self.master_hz / self.cpu_divider as u64
    }

    /// Master clocks in one scanline.
    #[must_use]
    pub const fn master_per_scanline(self) -> u64 {
        self.dots_per_line as u64 * self.ppu_divider as u64
    }

    /// Master clocks in one frame.
    #[must_use]
    pub const fn master_per_frame(self) -> u64 {
        self.master_per_scanline() * self.scanlines as u64
    }

    /// CPU cycles in one frame (rounded down; the scheduler uses the master
    /// clock, so this is informational).
    #[must_use]
    pub const fn cpu_cycles_per_frame(self) -> u64 {
        self.master_per_frame() / self.cpu_divider as u64
    }

    /// Frame rate in Hz.
    #[must_use]
    pub fn frame_rate(self) -> f64 {
        self.master_hz as f64 / self.master_per_frame() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_matches_known_clocks() {
        let timing = Timing::NTSC;
        assert_eq!(timing.ppu_hz(), 5_369_318);
        assert_eq!(timing.cpu_hz(), 1_789_772);
        assert!((timing.frame_rate() - 60.0988).abs() < 0.001);
    }

    #[test]
    fn pal_and_dendy_share_master_but_not_cpu() {
        assert_eq!(Timing::PAL.master_hz, Timing::DENDY.master_hz);
        assert_eq!(Timing::PAL.ppu_hz(), Timing::DENDY.ppu_hz());
        assert_ne!(Timing::PAL.cpu_hz(), Timing::DENDY.cpu_hz());
        assert!(Timing::DENDY.cpu_hz() > Timing::PAL.cpu_hz());
    }

    #[test]
    fn frame_geometry_is_consistent() {
        for timing in [Timing::NTSC, Timing::PAL, Timing::DENDY] {
            assert_eq!(
                timing.master_per_frame(),
                timing.master_per_scanline() * u64::from(timing.scanlines)
            );
            assert!(timing.cpu_cycles_per_frame() > 20_000);
        }
    }
}
