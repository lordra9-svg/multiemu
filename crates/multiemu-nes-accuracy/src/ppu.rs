//! 2C02 PPU: registers, timing, vblank/NMI and memory.
//!
//! A2 starts with the timing and register contract used by the `vbl_nmi`
//! suites; rendering, sprite evaluation and the odd-frame skip are added next.

use crate::clock::Timing;

/// PPU status bits.
pub const STATUS_VBLANK: u8 = 0x80;
/// Sprite-zero hit.
pub const STATUS_SPRITE0: u8 = 0x40;
/// Sprite overflow.
pub const STATUS_OVERFLOW: u8 = 0x20;

/// 2C02 state.
#[derive(Debug, Clone)]
pub struct Ppu {
    /// PPUCTRL ($2000).
    pub ctrl: u8,
    /// PPUMASK ($2001).
    pub mask: u8,
    /// PPUSTATUS ($2002).
    pub status: u8,
    /// OAMADDR ($2003).
    pub oam_addr: u8,
    /// Current VRAM address (loopy `v`).
    pub v: u16,
    /// Temporary VRAM address (loopy `t`).
    pub t: u16,
    /// Fine X scroll.
    pub fine_x: u8,
    /// First/second write latch.
    pub write_latch: bool,
    /// Read buffer for $2007.
    pub read_buffer: u8,
    /// PPU I/O bus latch: the last value written to a PPU register.
    pub io_bus: u8,
    /// Dots since the I/O bus was last driven; bits decay to zero.
    pub io_bus_age: u32,
    /// Scanline, 0..=261.
    pub scanline: u16,
    /// Dot, 0..=340.
    pub dot: u16,
    /// Completed frames.
    pub frames: u64,
    /// Nametable VRAM (2 KiB).
    pub vram: [u8; 0x800],
    /// Palette RAM (32 bytes).
    pub palette: [u8; 32],
    /// Object attribute memory.
    pub oam: [u8; 256],
    /// CHR memory (8 KiB window; mappers can replace this later).
    pub chr: Vec<u8>,
    /// Whether CHR is writable RAM.
    pub chr_ram: bool,
    /// Nametable mirroring: true = vertical.
    pub vertical_mirroring: bool,
    /// NMI output line.
    pub nmi_line: bool,
    /// Latched NMI edge waiting for the CPU.
    pub nmi_pending: bool,
    /// NMI suppressed by a $2002 read at the vblank start dot.
    pub nmi_suppressed: bool,
    /// VBL flag suppressed by a $2002 read one dot before it would be set.
    pub vblank_suppressed: bool,
    /// Dot within scanline 241 where an NMI edge is suppressed.
    pub nmi_suppress_dot: u16,
    /// Dot within scanline 241 where the VBL flag is suppressed.
    pub vblank_suppress_dot: u16,
    /// Dot within scanline 241 where VBL is set.
    pub vblank_set_dot: u16,
    /// Dot within scanline 261 where VBL is cleared.
    pub vblank_clear_dot: u16,
    /// Dot window within scanline 241 where disabling NMI cancels a pending edge.
    pub disable_cancel_dot: u16,
    /// Rendering state sampled at dot 339 of the pre-render line.
    pub rendering_at_339: bool,
    /// Whether rendering was enabled at the previous dot.
    prev_rendering: bool,
    /// Secondary-OAM address captured when rendering was disabled.
    corruption_seed: u8,
    /// OAM corruption waits for the next rendered dot.
    corruption_pending: bool,
    /// OAM indices selected for the current scanline.
    pub sprites: [u8; 8],
    /// Number of valid entries in [`Ppu::sprites`].
    pub sprite_count: usize,
    /// Sprites selected for the next scanline (evaluated during this line).
    next_sprites: [u8; 8],
    next_count: usize,
    /// Secondary-OAM slot that will be treated as sprite zero (the first
    /// in-range sprite evaluated at dot 66).
    next_zero: Option<usize>,
    /// Secondary OAM bytes for the selected sprites.
    secondary_oam: [u8; 32],
    /// Current secondary OAM write index.
    secondary_oamaddr: u8,
    /// OAM2 address overflowed; sprite fetch then re-reads index 0.
    oam2_overflow: bool,
    /// OAM2 byte address reached by the sprite-fetch stage.
    fetch_addr: u8,
    /// OAM byte read on the previous dot.
    oam_fetch: u8,
    /// $2004 latch: the byte the OAM data bus currently exposes.
    oam_data: u8,
    /// Parallel sprite-evaluation state used for the $2004 latch.
    eval2_main: u8,
    eval2_temp: u8,
    eval2_temp_overflow: bool,
    eval2_main_overflow: bool,
    eval2_soam: [u8; 32],
    eval2_mode: u8,
    eval2_copy: u8,
    eval2_found: bool,
    /// Sprite evaluation byte/sprite cursor.
    eval_hi: u8,
    eval_lo: u8,
    /// A sprite has been found in range.
    spr_in_range: bool,
    oam_id: [u8; 8],
    oam_main_overflow: bool,
    oam_temp: u8,
    oam_temp_overflow: bool,
    legacy_data: u8,
    /// Sprite 0 is in range for the next line.
    spr_zero_in_range: bool,
    /// Evaluation finished early.
    oam_eval_done: bool,
    /// Overflow countdown used by the hardware bug.
    overflow_count: u8,
    /// Timing data.
    pub timing: Timing,
    /// Accumulated RGBA frame (256x240).
    frame: Vec<u8>,
    /// Background shift registers (MSB is the next pixel).
    bg_lo: u16,
    bg_hi: u16,
    bg_attr_lo: u16,
    bg_attr_hi: u16,
    /// Latched fetch bytes for the current tile.
    nt_byte: u8,
    at_byte: u8,
    pat_lo: u8,
    pat_hi: u8,
    /// Latched sprite state for the next scanline.
    spr_id: [u8; 8],
    spr_x: [u8; 8],
    spr_counting: [bool; 8],
    spr_lo: [u8; 8],
    spr_hi: [u8; 8],
    spr_attr: [u8; 8],
    /// Low byte of the last background fetch address (the PPU octal latch).
    last_fetch_low: u8,
    /// Low byte latched from `v` when `$2006` changes it mid-fetch.
    hybrid_low: u8,
    /// Countdown to the background low-plane fetch corrupted by a $2007 read
    /// during rendering (ALE/read octal-latch feedback).
    ale_countdown: Option<u8>,
    /// Set when a `$2006` write changes `v` in the middle of a fetch; the
    /// next nametable read combines the new high byte with the latched low
    /// byte.
    hybrid_fetch: bool,
    /// A `$2001` write waiting out the PPU's 2-dot register latency.
    pending_mask: Option<u8>,
    /// Dots remaining before the pending `$2001` value is applied.
    mask_delay: u8,
}

impl Ppu {
    /// Creates a PPU for a region.
    #[must_use]
    pub fn new(timing: Timing, chr: Vec<u8>, vertical_mirroring: bool, chr_ram: bool) -> Self {
        Self {
            ctrl: 0,
            mask: 0,
            status: 0,
            oam_addr: 0,
            v: 0,
            t: 0,
            fine_x: 0,
            write_latch: false,
            read_buffer: 0,
            io_bus: 0,
            io_bus_age: 0,
            scanline: 0,
            dot: 40,
            frames: 0,
            vram: [0; 0x800],
            palette: [0; 32],
            oam: [0; 256],
            chr,
            chr_ram,
            vertical_mirroring,
            nmi_line: false,
            nmi_pending: false,
            nmi_suppressed: false,
            vblank_suppressed: false,
            rendering_at_339: false,
            prev_rendering: false,
            corruption_seed: 0,
            corruption_pending: false,
            sprites: [0; 8],
            sprite_count: 0,
            next_sprites: [0; 8],
            next_count: 0,
            next_zero: None,
            secondary_oam: [0xFF; 32],
            secondary_oamaddr: 0,
            oam2_overflow: false,
            fetch_addr: 0,
            oam_fetch: 0,
            oam_data: 0xFF,
            eval2_main: 0,
            eval2_temp: 0,
            eval2_temp_overflow: false,
            eval2_main_overflow: false,
            eval2_soam: [0xFF; 32],
            eval2_mode: 0,
            eval2_copy: 0,
            eval2_found: false,
            eval_hi: 0,
            eval_lo: 0,
            spr_in_range: false,
            oam_id: [64; 8],
            oam_main_overflow: false,
            oam_temp: 0,
            oam_temp_overflow: false,
            legacy_data: 0xFF,
            spr_zero_in_range: false,
            oam_eval_done: false,
            overflow_count: 0,
            nmi_suppress_dot: 1,
            vblank_suppress_dot: 1,
            vblank_set_dot: 1,
            vblank_clear_dot: 1,
            disable_cancel_dot: 3,
            timing,
            frame: vec![0; 256 * 240 * 4],
            bg_lo: 0,
            bg_hi: 0,
            bg_attr_lo: 0,
            bg_attr_hi: 0,
            nt_byte: 0,
            at_byte: 0,
            pat_lo: 0,
            pat_hi: 0,
            spr_id: [64; 8],
            spr_x: [0; 8],
            spr_counting: [false; 8],
            spr_lo: [0; 8],
            spr_hi: [0; 8],
            spr_attr: [0; 8],
            last_fetch_low: 0,
            hybrid_low: 0,
            ale_countdown: None,
            hybrid_fetch: false,
            pending_mask: None,
            mask_delay: 0,
        }
    }

    /// Advances one PPU dot.
    pub fn step_dot(&mut self) {
        if let Some(value) = self.pending_mask {
            if self.mask_delay == 0 {
                self.mask = value;
                self.pending_mask = None;
            } else {
                self.mask_delay -= 1;
            }
        }
        self.io_bus_age = self.io_bus_age.saturating_add(1);
        if self.io_bus_age > 1_500_000 {
            self.io_bus = 0;
        }
        // OAM corruption: disabling rendering during a rendered line seeds a
        // row; the next rendered dot copies OAM row 0 into that row.
        let rendering = self.mask & 0x18 != 0;
        let rendered_line = self.scanline < 240 || self.scanline == 261;
        if self.prev_rendering && !rendering && rendered_line {
            self.corruption_seed = self.secondary_oamaddr & 0x1F;
            self.corruption_pending = true;
        }
        if self.corruption_pending && rendering && rendered_line {
            let seed = usize::from(self.corruption_seed & 0x1F);
            let row0: [u8; 8] = self.oam[0..8].try_into().unwrap_or([0; 8]);
            for (i, byte) in row0.iter().enumerate() {
                self.oam[seed * 8 + i] = *byte;
            }
            self.secondary_oam[seed] = self.secondary_oam[0];
            self.corruption_pending = false;
        }
        self.prev_rendering = rendering;

        // Secondary OAM is filled from the previous line's evaluation at the
        // start of the line.
        if self.dot == 0 {
            self.sprites = self.next_sprites;
            self.sprite_count = self.next_count;
            if self.scanline < 240 || self.scanline == 261 {
                self.oam_id = [64; 8];
            }
        }
        if self.mask & 0x18 != 0 && (257..=320).contains(&self.dot) {
            if self.dot == 257 {
                self.fetch_addr = 0;
            } else if self.dot.is_multiple_of(2) && !self.oam2_overflow {
                self.fetch_addr = self.fetch_addr.wrapping_add(1);
            }
            let slot = ((self.dot - 257) / 8) as usize;
            if (self.dot - 257) % 8 == 4 {
                self.latch_sprite(slot);
            }
        }
        if matches!(self.dot, 63 | 255 | 339) && self.mask & 0x18 != 0 {
            self.oam2_overflow = false;
        }
        if self.dot == 320 && self.mask & 0x18 != 0 {
            self.oam2_overflow = true;
        }
        if self.dot == 339 && self.mask & 0x18 != 0 {
            // Only slots that actually received a sprite from this line's
            // fetch are armed. Empty slots keep their previous shifter state,
            // which is what lets a stale sprite appear on line 0 when the
            // pre-render line's dot 339 is blanked.
            for slot in 0..8 {
                if self.spr_id[slot] != 64 {
                    self.spr_counting[slot] = true;
                }
            }
        }
        if (1..=64).contains(&self.dot) {
            self.oam_fetch = 0xFF;
            // Secondary OAM is cleared here; its address advances every two
            // dots and seeds OAM corruption.
            if self.mask & 0x18 != 0 {
                self.secondary_oamaddr = ((self.dot / 2) & 0x1F) as u8;
            }
        }
        if self.dot == 1 && self.mask & 0x18 != 0 {
            // The OAM2 clear phase only runs while rendering is enabled;
            // otherwise the previous contents persist (frozen-OAM2 test).
            self.next_count = 0;
            self.next_zero = None;
            self.secondary_oam = [0xFF; 32];
            self.secondary_oamaddr = 0;
            self.spr_in_range = false;
            self.spr_zero_in_range = false;
            self.oam_eval_done = false;
            self.overflow_count = 0;
        }
        // OAMADDR is reset to 0 during the sprite fetch interval.
        if self.dot == 257 && self.mask & 0x18 != 0 && self.scanline <= 261 {
            self.oam_addr = 0;
        }
        // Sprite evaluation for the NEXT line runs during dots 65-256, one OAM
        // byte per two dots.
        if self.dot == 65 && self.mask & 0x18 != 0 {
            self.eval_hi = (self.oam_addr >> 2) & 0x3F;
            self.eval_lo = self.oam_addr & 0x03;
            self.spr_in_range = false;
            self.spr_zero_in_range = false;
            self.oam_eval_done = false;
            self.overflow_count = 0;
            self.secondary_oamaddr = 0;
            self.next_count = 0;
            self.next_zero = None;
        }
        if self.scanline < 240 && self.mask & 0x18 != 0 {
            self.eval_cycle_ares();
            self.update_oam_latch();
        }
        // Background fetch/shift pipeline and pixel composition.
        let rendered_line = self.scanline < 240 || self.scanline == 261;
        let rendering = self.mask & 0x18 != 0;
        if rendered_line && (1..=256).contains(&self.dot) {
            if self.scanline < 240 {
                self.render_pixel_dot(self.dot - 1);
            }
            if rendering {
                self.shift_background();
                self.fetch_background(self.dot);
                self.load_background(self.dot);
            }
        } else if rendered_line && rendering && (321..=336).contains(&self.dot) {
            self.shift_background();
            self.fetch_background(self.dot);
            self.load_background(self.dot);
        }

        // Live VRAM address updates for background fetches. These only happen
        // on visible lines and the pre-render line, never in vblank.
        if (self.scanline < 240 || self.scanline == 261) && self.mask & 0x18 != 0 {
            match self.dot {
                8 | 16 | 24 | 32 | 40 | 48 | 56 | 64 | 72 | 80 | 88 | 96 | 104 | 112 | 120
                | 128 | 136 | 144 | 152 | 160 | 168 | 176 | 184 | 192 | 200 | 208 | 216 | 224
                | 232 | 240 | 248 | 328 | 336 => self.increment_coarse_x(),
                256 => self.increment_y(),
                257 => self.copy_horizontal(),
                _ => {}
            }
            if self.scanline == 261 && (280..=304).contains(&self.dot) {
                self.copy_vertical();
            }
        }

        // Vblank starts at (241, 1) and ends at (261, 1).
        if self.scanline == 241 && self.dot == self.vblank_set_dot {
            if !self.vblank_suppressed {
                self.status |= STATUS_VBLANK;
            }
            if self.ctrl & 0x80 != 0 && !self.nmi_suppressed {
                self.nmi_line = true;
                self.nmi_pending = true;
            }
        }
        if self.scanline == 261 && self.dot == self.vblank_clear_dot {
            self.status &= !(STATUS_VBLANK | STATUS_SPRITE0 | STATUS_OVERFLOW);
            self.nmi_line = false;
            self.nmi_suppressed = false;
            self.vblank_suppressed = false;
        }

        self.dot += 1;
        // The pre-render line is one dot shorter on odd frames when rendering
        // is enabled. The decision is sampled at dot 339, the dot before the
        // skipped one.
        if self.scanline == 261 && self.dot == 339 {
            self.rendering_at_339 = self.mask & 0x18 != 0;
        }
        if self.scanline == 261 && self.dot == 340 && self.frames & 1 == 1 && self.rendering_at_339
        {
            self.dot = 0;
            self.scanline = 0;
            self.frames += 1;
            return;
        }
        if self.dot == self.timing.dots_per_line {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline == self.timing.scanlines {
                self.scanline = 0;
                self.frames += 1;
            }
        }
    }

    /// Advances a number of dots.
    pub fn step_dots(&mut self, dots: u32) {
        for _ in 0..dots {
            self.step_dot();
        }
    }

    /// Reads a PPU register in `$2000..=$2007`.
    ///
    /// Write-only registers return the PPU I/O bus; `$2002` drives bits 7-5
    /// and leaves the low bits from the I/O bus.
    pub fn read_register(&mut self, index: u16) -> u8 {
        match index & 7 {
            2 => {
                if self.scanline == 241 && self.dot == self.vblank_suppress_dot {
                    self.vblank_suppressed = true;
                }
                if self.scanline == 241 && self.dot == self.nmi_suppress_dot {
                    self.nmi_suppressed = true;
                    self.nmi_pending = false;
                }
                // The vblank bit is latched at the start of the CPU read, but
                // the sprite-zero and overflow bits are sampled at the end of
                // the read cycle. If this read ends on the pre-render clear
                // dot they must already read as zero.
                let mut sampled = self.status;
                if self.scanline == 261 && self.dot == self.vblank_clear_dot {
                    sampled &= !(STATUS_SPRITE0 | STATUS_OVERFLOW);
                }
                let value = (sampled & 0xE0) | (self.io_bus & 0x1F);
                self.io_bus = value;
                self.io_bus_age = 0;
                self.status &= !STATUS_VBLANK;
                self.nmi_line = false;
                self.write_latch = false;
                value
            }
            4 => {
                if self.scanline < 240 && self.mask & 0x18 != 0 && (321..=340).contains(&self.dot) {
                    let address = if self.fetch_addr < 0x20 {
                        self.fetch_addr + 1
                    } else {
                        self.fetch_addr
                    };
                    self.secondary_oam[usize::from(address & 0x1F)]
                } else if (self.scanline < 240 || self.scanline == 261) && self.mask & 0x18 != 0 {
                    // While rendering, $2004 exposes the internal OAM data
                    // latch rather than OAM[OAMADDR].
                    self.legacy_data
                } else {
                    self.oam[usize::from(self.oam_addr)]
                }
            }
            7 => {
                if self.scanline < 240 && self.mask & 0x18 != 0 {
                    // A $2007 read takes over the PPU memory bus for two
                    // cycles; two fetch groups later the low-plane fetch sees
                    // the octal latch value (address/data pins shared).
                    self.ale_countdown = Some(0);
                }
                let address = self.v & 0x3FFF;
                let value = if address >= 0x3F00 {
                    let mut palette = self.read_palette(address) & 0x3F;
                    if self.mask & 0x01 != 0 {
                        // Greyscale mode zeros the low four palette bits on reads.
                        palette &= 0x30;
                    }
                    self.read_buffer = self.read_vram(address & 0x2FFF);
                    palette | (self.io_bus & 0xC0)
                } else {
                    let buffered = self.read_buffer;
                    self.read_buffer = self.read_vram(address);
                    buffered
                };
                self.io_bus = value;
                self.io_bus_age = 0;
                self.increment_vram_address();
                value
            }
            _ => self.io_bus,
        }
    }

    /// Writes a PPU register in `$2000..=$2007`.
    pub fn write_register(&mut self, index: u16, value: u8) {
        self.io_bus = value;
        self.io_bus_age = 0;
        match index & 7 {
            0 => {
                let was_enabled = self.ctrl & 0x80 != 0;
                self.ctrl = value;
                self.t = (self.t & !0x0C00) | (u16::from(value & 0x03) << 10);
                if value & 0x80 != 0 && !was_enabled && self.status & STATUS_VBLANK != 0 {
                    let on_clear_dot = self.scanline == 261 && self.dot == self.vblank_clear_dot;
                    if !self.nmi_suppressed && !on_clear_dot {
                        self.nmi_line = true;
                        self.nmi_pending = true;
                    }
                } else if value & 0x80 == 0 {
                    self.nmi_line = false;
                    if self.scanline == 241 && self.dot <= self.disable_cancel_dot {
                        self.nmi_pending = false;
                    }
                }
            }
            1 => {
                // PPU register writes reach their destination a couple of dots
                // after the bus write; model the smallest delay. Pre-render
                // writes stay immediate because the odd-frame skip decision is
                // sampled at dot 339 with the calibrated phase.
                if self.scanline == 261 {
                    self.mask = value;
                } else {
                    self.pending_mask = Some(value);
                    self.mask_delay = std::env::var("MD")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(2);
                }
            }
            3 => self.oam_addr = value,
            4 => {
                if (self.scanline < 240 || self.scanline == 261) && self.mask & 0x18 != 0 {
                    self.oam_addr = self.oam_addr.wrapping_add(4) & 0xFC;
                } else {
                    let mut value = value;
                    if self.oam_addr & 3 == 2 {
                        value &= 0xE3;
                    }
                    self.oam[usize::from(self.oam_addr)] = value;
                    self.oam_addr = self.oam_addr.wrapping_add(1);
                }
            }
            5 => {
                if !self.write_latch {
                    self.t = (self.t & !0x001F) | (u16::from(value) >> 3);
                    self.fine_x = value & 0x07;
                } else {
                    self.t = (self.t & !0x73E0)
                        | ((u16::from(value) & 0xF8) << 2)
                        | ((u16::from(value) & 0x07) << 12);
                }
                self.write_latch = !self.write_latch;
            }
            6 => {
                if !self.write_latch {
                    self.t = (self.t & 0x00FF) | ((u16::from(value) & 0x3F) << 8);
                } else {
                    let old_low = self.v as u8;
                    self.t = (self.t & 0xFF00) | u16::from(value);
                    self.v = self.t;
                    if (self.scanline < 240 || self.scanline == 261) && self.mask & 0x18 != 0 {
                        self.hybrid_low = old_low.wrapping_add(1);
                        self.hybrid_fetch = true;
                    }
                }
                self.write_latch = !self.write_latch;
            }
            7 => {
                if self.scanline < 240 && self.mask & 0x18 != 0 {
                    // A $2007 read takes over the PPU memory bus for two
                    // cycles; two fetch groups later the low-plane fetch sees
                    // the octal latch value (address/data pins shared).
                    self.ale_countdown = Some(0);
                }
                let address = self.v & 0x3FFF;
                self.write_vram(address, value);
                self.increment_vram_address();
            }
            _ => {}
        }
    }

    fn nametable_index(&self, address: u16) -> usize {
        let index = usize::from(address & 0x0FFF);
        let table = index / 0x0400;
        let offset = index % 0x0400;
        let mapped = if self.vertical_mirroring {
            table % 2
        } else {
            match table {
                0 => 0,
                1 => 0,
                2 => 1,
                _ => 1,
            }
        };
        mapped * 0x0400 + offset
    }

    /// Increments `v` after a `$2007` access. During rendering this performs a
    /// coarse-X and Y increment simultaneously instead of the normal +1/+32.
    fn increment_vram_address(&mut self) {
        if (self.scanline < 240 || self.scanline == 261) && self.mask & 0x18 != 0 {
            let mut v = self.v;
            if v & 0x001F == 31 {
                v &= !0x001F;
                v ^= 0x0400;
            } else {
                v += 1;
            }
            let fine_y = (v >> 12) & 0x07;
            if fine_y < 7 {
                v += 0x1000;
            } else {
                v &= !0x7000;
                let mut coarse_y = (v >> 5) & 0x1F;
                if coarse_y == 29 {
                    coarse_y = 0;
                    v ^= 0x0800;
                } else if coarse_y == 31 {
                    coarse_y = 0;
                } else {
                    coarse_y += 1;
                }
                v = (v & !0x03E0) | (coarse_y << 5);
            }
            self.v = v & 0x7FFF;
        } else {
            self.v = self
                .v
                .wrapping_add(if self.ctrl & 0x04 != 0 { 32 } else { 1 })
                & 0x7FFF;
        }
    }

    fn read_vram(&self, address: u16) -> u8 {
        if address < 0x2000 {
            let index = usize::from(address) % self.chr.len().max(1);
            self.chr.get(index).copied().unwrap_or(0)
        } else if address < 0x3F00 {
            self.vram[self.nametable_index(address)]
        } else {
            self.read_palette(address)
        }
    }

    fn write_vram(&mut self, address: u16, value: u8) {
        if address < 0x2000 {
            if self.chr_ram {
                let index = usize::from(address) & 0x1FFF;
                if let Some(byte) = self.chr.get_mut(index) {
                    *byte = value;
                }
            }
        } else if address < 0x3F00 {
            let index = self.nametable_index(address);
            self.vram[index] = value;
        } else {
            self.write_palette(address, value);
        }
    }

    fn palette_index(address: u16) -> usize {
        let mut index = usize::from(address & 0x1F);
        if matches!(index, 0x10 | 0x14 | 0x18 | 0x1C) {
            index &= !0x10;
        }
        index
    }

    fn read_palette(&self, address: u16) -> u8 {
        self.palette[Self::palette_index(address)]
    }

    fn write_palette(&mut self, address: u16, value: u8) {
        let index = Self::palette_index(address);
        self.palette[index] = value & 0x3F;
    }

    fn sprite_height(&self) -> u16 {
        if self.ctrl & 0x20 != 0 {
            16
        } else {
            8
        }
    }

    fn advance_eval2_temp(&mut self) {
        self.eval2_temp = self.eval2_temp.wrapping_add(1) & 0x1F;
        if self.eval2_temp == 0 {
            self.eval2_temp_overflow = true;
            self.eval2_mode = 0;
            self.eval2_copy = 0;
            self.eval2_found = false;
        }
    }

    /// Advances the internal OAM data latch, following the hardware sprite
    /// evaluation pipeline. Only called while rendering is enabled on a
    /// visible line.
    fn update_oam_latch(&mut self) {
        let lx = self.dot;
        if lx == 0 {
            return;
        }
        if lx <= 64 {
            if lx & 1 == 1 {
                self.legacy_data = 0xFF;
            } else {
                self.eval2_soam[usize::from(self.eval2_temp & 0x1F)] = self.legacy_data;
                self.eval2_temp = (self.eval2_temp + 1) & 0x1F;
                if lx == 64 {
                    self.eval2_temp = 0;
                }
            }
            return;
        }
        if lx <= 256 {
            if self.eval2_temp_overflow && !self.eval2_main_overflow {
                if lx & 1 == 1 {
                    match self.eval2_mode {
                        1 => {
                            if self.eval2_copy > 0 {
                                self.legacy_data = self.oam[usize::from(self.eval2_main)];
                                self.eval2_main = self.eval2_main.wrapping_add(1);
                                self.eval2_copy -= 1;
                            } else {
                                self.legacy_data = self.eval2_soam[0];
                                self.eval2_main = (self.eval2_main & 0xFC).wrapping_add(4);
                                self.eval2_mode = 2;
                            }
                        }
                        2 => {
                            self.legacy_data = self.eval2_soam[0];
                            self.eval2_mode = 0;
                        }
                        _ => {
                            self.legacy_data = self.oam[usize::from(self.eval2_main)];
                        }
                    }
                    return;
                }
                if self.eval2_mode == 1 {
                    self.legacy_data = self.eval2_soam[0];
                    return;
                }
                if self.eval2_mode == 2 {
                    self.legacy_data = self.eval2_soam[0];
                    self.eval2_mode = 0;
                    return;
                }
                let height = u32::from(self.sprite_height());
                let ly = if self.scanline == 261 {
                    -1i32
                } else {
                    i32::from(self.scanline)
                };
                let y = (ly - i32::from(self.legacy_data)) as u32;
                if self.eval2_main & 3 == 0 && y < height {
                    self.eval2_found = true;
                    self.eval2_mode = 1;
                    self.eval2_copy = 2;
                    self.eval2_main = self.eval2_main.wrapping_add(1);
                    self.legacy_data = self.eval2_soam[0];
                    return;
                }
                self.eval2_main =
                    (self.eval2_main & 3) | (((self.eval2_main >> 2).wrapping_add(1) & 0x3F) << 2);
                if self.eval2_found {
                    self.eval2_main &= 0xFC;
                } else {
                    self.eval2_main =
                        (self.eval2_main & 0xFC) | (self.eval2_main.wrapping_add(1) & 3);
                }
                if self.eval2_main >> 2 == 0 {
                    self.eval2_main_overflow = true;
                }
                self.legacy_data = self.eval2_soam[0];
                return;
            }
            if lx & 1 == 1 {
                self.legacy_data = self.oam[usize::from(self.eval2_main)];
                return;
            }
            if self.eval2_main_overflow {
                self.eval2_main =
                    (self.eval2_main & 3) | (((self.eval2_main >> 2).wrapping_add(1) & 0x3F) << 2);
                let index = if self.eval2_temp_overflow {
                    0
                } else {
                    self.eval2_temp & 0x1F
                };
                self.legacy_data = self.eval2_soam[usize::from(index)];
                return;
            }
            let height = u32::from(self.sprite_height());
            let ly = if self.scanline == 261 {
                -1i32
            } else {
                i32::from(self.scanline)
            };
            let y = (ly - i32::from(self.legacy_data)) as u32;
            let soam_index = if self.eval2_temp_overflow {
                0
            } else {
                self.eval2_temp & 0x1F
            };
            if self.eval2_temp_overflow {
                self.legacy_data = self.eval2_soam[usize::from(soam_index)];
            } else {
                self.eval2_soam[usize::from(self.eval2_temp & 0x1F)] = self.legacy_data;
            }
            if self.eval2_temp & 3 == 0 {
                if self.eval2_temp_overflow {
                    if self.eval2_main & 3 != 0 || y >= height {
                        self.eval2_main = (self.eval2_main & 3)
                            | (((self.eval2_main >> 2).wrapping_add(1) & 0x3F) << 2);
                        if self.eval2_found {
                            self.eval2_main &= 0xFC;
                        } else {
                            self.eval2_main =
                                (self.eval2_main & 0xFC) | (self.eval2_main.wrapping_add(1) & 3);
                        }
                        if self.eval2_main >> 2 == 0 {
                            self.eval2_main_overflow = true;
                        }
                        return;
                    }
                    self.eval2_found = true;
                    self.status |= STATUS_OVERFLOW;
                } else if y >= height {
                    self.eval2_main &= 0xFC;
                    self.eval2_main = (self.eval2_main & 3)
                        | (((self.eval2_main >> 2).wrapping_add(1) & 0x3F) << 2);
                    if self.eval2_main >> 2 == 0 {
                        self.eval2_main_overflow = true;
                    }
                    return;
                }
                self.advance_eval2_temp();
                self.eval2_main = self.eval2_main.wrapping_add(1);
                if self.eval2_main == 0 {
                    self.eval2_main_overflow = true;
                }
            } else if self.eval2_found && y >= height {
                self.eval2_main =
                    (self.eval2_main & 3) | (((self.eval2_main >> 2).wrapping_add(1) & 0x3F) << 2);
                self.eval2_main &= 0xFC;
                if self.eval2_main >> 2 == 0 {
                    self.eval2_main_overflow = true;
                }
            } else {
                self.advance_eval2_temp();
                self.eval2_main = self.eval2_main.wrapping_add(1);
                if self.eval2_main == 0 {
                    self.eval2_main_overflow = true;
                }
                if self.eval2_temp & 3 == 0 && y >= height {
                    self.eval2_main &= 0xFC;
                }
            }
            return;
        }
        if lx <= 320 {
            let d = lx - 257;
            let index = ((d >> 3) << 2) + core::cmp::min(d & 7, 3);
            self.eval2_main = 0;
            self.eval2_temp = 0;
            self.eval2_main_overflow = false;
            self.eval2_temp_overflow = false;
            self.eval2_mode = 0;
            self.eval2_copy = 0;
            self.eval2_found = false;
            self.legacy_data = self.eval2_soam[usize::from(index & 0x1F)];
            return;
        }
        self.legacy_data = self.eval2_soam[0];
    }

    fn inc_main_index(&mut self) -> bool {
        let index = (self.oam_addr >> 2) & 0x3F;
        let next = (index + 1) & 0x3F;
        self.oam_addr = (self.oam_addr & 3) | (next << 2);
        next == 0
    }
    fn inc_main_timing(&mut self) {
        self.oam_addr = (self.oam_addr & 0xFC) | (self.oam_addr.wrapping_add(1) & 3);
    }
    fn inc_main(&mut self) -> bool {
        self.oam_addr = self.oam_addr.wrapping_add(1);
        self.oam_addr == 0
    }
    fn inc_temp(&mut self) -> bool {
        self.oam_temp = (self.oam_temp + 1) & 0x1F;
        self.oam_temp == 0
    }
    fn eval_cycle_ares(&mut self) {
        let lx = self.dot;
        if lx == 0 {
            return;
        }
        if lx <= 64 {
            if lx & 1 == 1 {
                self.oam_data = 0xFF;
            } else {
                self.secondary_oam[usize::from(self.oam_temp & 0x1F)] = self.oam_data;
                self.oam_temp = (self.oam_temp + 1) & 0x1F;
                if lx == 64 {
                    self.oam_temp = 0;
                }
            }
            return;
        }
        if lx <= 256 {
            if lx & 1 == 1 {
                self.oam_data = self.oam[usize::from(self.oam_addr)];
                return;
            }
            if self.oam_main_overflow {
                if self.inc_main_index() {
                    self.oam_main_overflow = true;
                }
                if self.oam_temp_overflow {
                    self.oam_data = self.secondary_oam[usize::from(self.oam_temp & 0x1F)];
                }
                return;
            }
            let ly = if self.scanline == 261 {
                -1i32
            } else {
                i32::from(self.scanline)
            };
            let y = (ly - i32::from(self.oam_data)) as u32;
            if self.oam_temp_overflow {
                self.oam_data = self.secondary_oam[usize::from(self.oam_temp & 0x1F)];
            } else {
                self.secondary_oam[usize::from(self.oam_temp & 0x1F)] = self.oam_data;
            }
            let height = u32::from(self.sprite_height());
            if self.oam_temp & 3 == 0 {
                if self.oam_temp_overflow {
                    if y >= height {
                        if self.inc_main_index() {
                            self.oam_main_overflow = true;
                        }
                        self.inc_main_timing();
                        return;
                    }
                    self.status |= STATUS_OVERFLOW;
                } else {
                    if y >= height {
                        self.oam_addr &= 0xFC;
                        if self.inc_main_index() {
                            self.oam_main_overflow = true;
                        }
                        return;
                    }
                    let slot = usize::from(self.oam_temp >> 2) & 7;
                    self.oam_id[slot] = if lx == 66 { 0 } else { 1 };
                }
                if self.inc_temp() {
                    if !self.oam_temp_overflow {
                        self.oam2_overflow = true;
                    }
                    self.oam_temp_overflow = true;
                }
                if self.inc_main() {
                    self.oam_main_overflow = true;
                }
            } else {
                if self.inc_temp() {
                    if !self.oam_temp_overflow {
                        self.oam2_overflow = true;
                    }
                    self.oam_temp_overflow = true;
                }
                if self.inc_main() {
                    self.oam_main_overflow = true;
                }
                if self.oam_temp & 3 == 0 && y >= height {
                    self.oam_addr &= 0xFC;
                }
            }
            return;
        }
        if lx <= 320 {
            let d = lx - 257;
            let index = ((d >> 3) << 2) + core::cmp::min(d & 7, 3);
            self.oam_addr = 0;
            self.oam_temp = 0;
            self.oam_main_overflow = false;
            self.oam_temp_overflow = false;
            self.oam_data = self.secondary_oam[usize::from(index & 0x1F)];
            return;
        }
        self.oam_data = self.secondary_oam[0];
    }

    #[allow(dead_code)]
    fn eval_cycle(&mut self, line: u16) {
        if self.dot & 1 == 1 {
            self.oam_fetch = self.oam[usize::from((self.eval_hi << 2) | self.eval_lo)];
            return;
        }
        let height = self.sprite_height();
        let mut hi = self.eval_hi;
        let mut lo = self.eval_lo;
        if self.oam_eval_done {
            hi = (hi + 1) & 0x3F;
        } else {
            let y = u16::from(self.oam_fetch);
            if !self.spr_in_range {
                self.spr_in_range = line.wrapping_sub(y) < height;
            }
            if self.secondary_oamaddr < 0x20 {
                let index = usize::from(self.secondary_oamaddr) & 0x1F;
                self.secondary_oam[index] = self.oam_fetch;
                if self.spr_in_range {
                    if lo == 0 && self.next_count < 8 {
                        if self.dot == 66 {
                            self.next_zero = Some(self.next_count);
                        }
                        self.next_sprites[self.next_count] = hi;
                        self.next_count += 1;
                        if self.next_count == 8 {
                            self.oam2_overflow = true;
                        }
                    }
                    lo += 1;
                    self.secondary_oamaddr += 1;
                    if hi == 0 {
                        self.spr_zero_in_range = true;
                    }
                    if lo == 4 {
                        self.spr_in_range = false;
                        lo = 0;
                        hi = (hi + 1) & 0x3F;
                        self.oam_eval_done |= hi == 0;
                    }
                } else {
                    hi = (hi + 1) & 0x3F;
                    self.oam_eval_done |= hi == 0;
                }
            } else {
                let index = usize::from(self.secondary_oamaddr) & 0x1F;
                self.oam_fetch = self.secondary_oam[index];
                if self.spr_in_range {
                    self.status |= STATUS_OVERFLOW;
                    lo += 1;
                    if lo == 4 {
                        lo = 0;
                        hi = (hi + 1) & 0x3F;
                    }
                    match self.overflow_count {
                        0 => self.overflow_count = 3,
                        _ => {
                            self.overflow_count -= 1;
                            if self.overflow_count == 0 {
                                self.oam_eval_done = true;
                                lo = 0;
                            }
                        }
                    }
                } else {
                    hi = (hi + 1) & 0x3F;
                    lo = (lo + 1) & 0x03;
                    self.oam_eval_done |= hi == 0;
                }
            }
        }
        self.eval_hi = hi;
        self.eval_lo = lo;
        self.oam_addr = (hi << 2) | (lo & 0x03);
    }

    /// Latches one sprite's pattern for the next scanline (called during the
    /// sprite-fetch window).
    fn latch_sprite(&mut self, slot: usize) {
        let (y, tile, attr, x, id) = if self.oam2_overflow {
            let byte = self.secondary_oam[0];
            (
                u16::from(byte),
                byte,
                byte,
                byte,
                if slot == 0 { 0 } else { 1 },
            )
        } else {
            let base = slot * 4;
            (
                u16::from(self.secondary_oam[base]),
                self.secondary_oam[base + 1],
                self.secondary_oam[base + 2],
                self.secondary_oam[base + 3],
                self.oam_id[slot],
            )
        };
        self.spr_x[slot] = x;
        self.spr_attr[slot] = attr;
        self.spr_id[slot] = id;
        let height = self.sprite_height();
        let mut row = self.scanline.wrapping_sub(y) & (height - 1);
        if attr & 0x80 != 0 {
            row ^= height - 1;
        }
        let (bank, mut tile) = if height == 16 {
            ((u16::from(tile & 1)) << 12, tile & 0xFE)
        } else {
            (if self.ctrl & 0x10 != 0 { 0x1000 } else { 0 }, tile)
        };
        if height == 16 && row >= 8 {
            tile = tile.wrapping_add(1);
            row -= 8;
        }
        let address = bank + u16::from(tile) * 16 + row;
        let mut low = self.chr.get(usize::from(address)).copied().unwrap_or(0);
        let mut high = self.chr.get(usize::from(address) + 8).copied().unwrap_or(0);
        if attr & 0x40 != 0 {
            low = low.reverse_bits();
            high = high.reverse_bits();
        }
        self.spr_lo[slot] = low;
        self.spr_hi[slot] = high;
    }

    /// Shifts the background shift registers one pixel.
    fn shift_background(&mut self) {
        self.bg_lo <<= 1;
        // The high pattern plane shifts in a constant 1 (the PPU's "serial
        // in" input).
        self.bg_hi = (self.bg_hi << 1) | 1;
        self.bg_attr_lo <<= 1;
        self.bg_attr_hi <<= 1;
    }

    /// Fetches one byte of the current background tile.
    fn fetch_background(&mut self, dot: u16) {
        match dot & 7 {
            1 => {
                let full = 0x2000 | (self.v & 0x0FFF);
                let address = if self.hybrid_fetch {
                    self.hybrid_fetch = false;
                    let hybrid = (full & 0xFF00) | u16::from(self.hybrid_low);
                    if std::env::var("HYB").is_ok() {
                        eprintln!(
                            "HYB line={} dot={} full={:04X} low={:02X} -> {:04X}",
                            self.scanline, self.dot, full, self.hybrid_low, hybrid
                        );
                    }
                    hybrid
                } else {
                    full
                };
                self.last_fetch_low = address as u8;
                self.nt_byte = self.read_vram(address);
            }
            3 => {
                let address =
                    0x23C0 | (self.v & 0x0C00) | ((self.v >> 4) & 0x38) | ((self.v >> 2) & 0x07);
                self.last_fetch_low = address as u8;
                self.at_byte = self.read_vram(address);
            }
            5 => {
                let address = self.pattern_address(self.nt_byte, false);
                self.last_fetch_low = address as u8;
                let corrupt = match self.ale_countdown {
                    Some(0) => true,
                    Some(n) => {
                        self.ale_countdown = Some(n - 1);
                        false
                    }
                    None => false,
                };
                if corrupt {
                    self.ale_countdown = None;
                    let bus_address = (address & 0xFF00) | 0x00FF;
                    self.pat_lo = self.chr.get(usize::from(bus_address)).copied().unwrap_or(0);
                } else {
                    self.pat_lo = self.chr.get(usize::from(address)).copied().unwrap_or(0);
                }
            }
            7 => {
                let address = self.pattern_address(self.nt_byte, true);
                self.last_fetch_low = address as u8;
                self.pat_hi = self.chr.get(usize::from(address)).copied().unwrap_or(0);
            }
            _ => {}
        }
    }

    /// CHR address for `tile` at the current fine Y.
    fn pattern_address(&self, tile: u8, high: bool) -> u16 {
        let fine_y = (self.v >> 12) & 0x07;
        let base = if self.ctrl & 0x10 != 0 { 0x1000 } else { 0 };
        base + u16::from(tile) * 16 + fine_y + if high { 8 } else { 0 }
    }

    /// Loads the fetched tile into the low byte of the shift registers at the
    /// end of an 8-dot fetch group.
    fn load_background(&mut self, dot: u16) {
        if dot & 7 != 0 {
            return;
        }
        self.bg_lo = (self.bg_lo & 0xFF00) | u16::from(self.pat_lo);
        self.bg_hi = (self.bg_hi & 0xFF00) | u16::from(self.pat_hi);
        let shift = ((self.v >> 4) & 4) | (self.v & 2);
        let attr = (self.at_byte >> shift) & 0x03;
        let low: u8 = if attr & 1 != 0 { 0xFF } else { 0 };
        let high: u8 = if attr & 2 != 0 { 0xFF } else { 0 };
        self.bg_attr_lo = (self.bg_attr_lo & 0xFF00) | u16::from(low);
        self.bg_attr_hi = (self.bg_attr_hi & 0xFF00) | u16::from(high);
    }

    /// Returns the CHR byte for `tile` at the current fine Y.
    /// Composes one pixel from the current shift-register state.
    fn render_pixel_dot(&mut self, x: u16) {
        let shift = 15 - u16::from(self.fine_x);
        let bit = ((self.bg_lo >> shift) & 1) | (((self.bg_hi >> shift) & 1) << 1);
        let pal = ((self.bg_attr_lo >> shift) & 1) | (((self.bg_attr_hi >> shift) & 1) << 1);
        let mut bg = 0u8;
        if self.mask & 0x08 != 0 && !(x < 8 && self.mask & 0x02 == 0) && bit as u8 != 0 {
            bg = (pal as u8) * 4 + bit as u8;
        }
        let mut color_index = bg;
        let rendering = self.mask & 0x18 != 0;
        let mut picked: Option<(u8, u8)> = None;
        for slot in 0..8 {
            if self.spr_id[slot] == 64 {
                continue;
            }
            if self.spr_x[slot] == 0 {
                self.spr_counting[slot] = false;
            }
            if self.spr_counting[slot] {
                self.spr_x[slot] = self.spr_x[slot].saturating_sub(1);
                continue;
            }
            if !rendering {
                continue;
            }
            let sbit = ((self.spr_lo[slot] >> 7) & 1) | (((self.spr_hi[slot] >> 7) & 1) << 1);
            self.spr_lo[slot] <<= 1;
            self.spr_hi[slot] <<= 1;
            if self.mask & 0x10 == 0 || (x < 8 && self.mask & 0x04 == 0) || sbit == 0 {
                continue;
            }
            if picked.is_none() {
                picked = Some((sbit, self.spr_attr[slot]));
                if self.spr_id[slot] == 0 && bg != 0 && x != 255 {
                    self.status |= STATUS_SPRITE0;
                }
            }
        }
        if let Some((sbit, attr)) = picked {
            let behind = attr & 0x20 != 0;
            if bg == 0 || !behind {
                color_index = 0x10 + (attr & 0x03) * 4 + sbit;
            }
        }
        if color_index == 0 {
            color_index = self.palette[0];
        }
        let color = self.palette[usize::from(color_index & 0x1F)];
        let [r, g, b] = NES_PALETTE[usize::from(color & 0x3F)];
        let i = (usize::from(self.scanline) * 256 + usize::from(x)) * 4;
        self.frame[i] = r;
        self.frame[i + 1] = g;
        self.frame[i + 2] = b;
        self.frame[i + 3] = 255;
    }

    fn increment_coarse_x(&mut self) {
        if (self.v & 0x001F) == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400;
        } else {
            self.v += 1;
        }
    }

    fn increment_y(&mut self) {
        if (self.v & 0x7000) != 0x7000 {
            self.v += 0x1000;
        } else {
            self.v &= !0x7000;
            let mut y = (self.v >> 5) & 0x1F;
            if y == 29 {
                y = 0;
                self.v ^= 0x0800;
            } else if y == 31 {
                y = 0;
            } else {
                y += 1;
            }
            self.v = (self.v & !0x03E0) | (y << 5);
        }
    }

    fn copy_horizontal(&mut self) {
        self.v = (self.v & !0x041F) | (self.t & 0x041F);
    }

    fn copy_vertical(&mut self) {
        self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
    }

    /// Returns the accumulated RGBA frame.
    ///
    /// Visible scanlines are composed as they are reached so mid-frame scroll
    /// and mask changes take effect on the correct line.
    #[must_use]
    pub fn render_frame(&self) -> Vec<u8> {
        self.frame.clone()
    }
}

/// Approximate NTSC palette; exact colours are a later accuracy entry.
const NES_PALETTE: [[u8; 3]; 64] = [
    [84, 84, 84],
    [0, 30, 116],
    [8, 16, 144],
    [48, 0, 136],
    [68, 0, 100],
    [92, 0, 48],
    [84, 4, 0],
    [60, 24, 0],
    [32, 42, 0],
    [8, 58, 0],
    [0, 64, 0],
    [0, 60, 0],
    [0, 50, 60],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [152, 150, 152],
    [8, 76, 196],
    [48, 50, 236],
    [92, 30, 228],
    [136, 20, 176],
    [160, 20, 100],
    [152, 34, 32],
    [120, 60, 0],
    [84, 90, 0],
    [40, 114, 0],
    [8, 124, 0],
    [0, 118, 40],
    [0, 102, 120],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [236, 238, 236],
    [76, 154, 236],
    [120, 124, 236],
    [176, 98, 236],
    [228, 84, 236],
    [236, 88, 180],
    [236, 106, 100],
    [212, 136, 32],
    [160, 170, 0],
    [116, 196, 0],
    [76, 208, 32],
    [56, 204, 108],
    [56, 180, 204],
    [60, 60, 60],
    [0, 0, 0],
    [0, 0, 0],
    [236, 238, 236],
    [168, 204, 236],
    [188, 188, 236],
    [212, 178, 236],
    [236, 174, 236],
    [236, 174, 212],
    [236, 180, 176],
    [228, 196, 144],
    [204, 210, 120],
    [180, 222, 120],
    [168, 226, 144],
    [152, 226, 180],
    [160, 214, 228],
    [160, 162, 160],
    [0, 0, 0],
    [0, 0, 0],
];

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu() -> Ppu {
        Ppu::new(Timing::NTSC, vec![0; 0x2000], true, false)
    }

    #[test]
    fn vblank_sets_and_clears() {
        let mut ppu = ppu();
        ppu.ctrl = 0x80;
        ppu.scanline = 241;
        ppu.dot = 0;
        ppu.step_dot();
        assert_eq!(ppu.status & STATUS_VBLANK, 0);
        ppu.step_dot();
        assert_ne!(ppu.status & STATUS_VBLANK, 0);
        assert!(ppu.nmi_line);
        let value = ppu.read_register(2);
        assert_ne!(value & STATUS_VBLANK, 0);
        assert!(!ppu.nmi_line);
    }

    #[test]
    fn pre_render_clears_flags() {
        let mut ppu = ppu();
        ppu.status = STATUS_VBLANK | STATUS_SPRITE0 | STATUS_OVERFLOW;
        ppu.nmi_line = true;
        ppu.scanline = 261;
        ppu.dot = 0;
        ppu.step_dot();
        ppu.step_dot();
        assert_eq!(
            ppu.status & (STATUS_VBLANK | STATUS_SPRITE0 | STATUS_OVERFLOW),
            0
        );
        assert!(!ppu.nmi_line);
    }

    #[test]
    fn address_register_latches() {
        let mut ppu = ppu();
        ppu.write_register(6, 0x21);
        ppu.write_register(6, 0x08);
        assert_eq!(ppu.v, 0x2108);
        ppu.write_register(6, 0x3F);
        ppu.write_register(6, 0x00);
        assert_eq!(ppu.v, 0x3F00);
    }

    #[test]
    fn palette_mirrors_universal_backdrop() {
        let mut ppu = ppu();
        ppu.write_register(6, 0x3F);
        ppu.write_register(6, 0x10);
        ppu.write_register(7, 0x2A);
        ppu.write_register(6, 0x3F);
        ppu.write_register(6, 0x00);
        assert_eq!(ppu.read_register(7), 0x2A);
    }
}
