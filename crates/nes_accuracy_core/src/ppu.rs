//! 2C02 PPU: registers, timing, vblank/NMI and memory.
//!
//! A2 starts with the timing and register contract used by the `vbl_nmi`
//! suites; rendering, sprite evaluation and the odd-frame skip are added next.

use std::cell::RefCell;
use std::rc::Rc;

use crate::cart::{
    A12Irq, Aux77, Bandai153, Chr169, Chr512, CpromChr, HybridChr, Mirroring, Mmc3A12, Mmc5Aux,
    N163Nt, NtMapping, OekaChr, Qtram, Vrc7Chr,
};
use crate::clock::Timing;

/// PPU status bits.
pub const STATUS_VBLANK: u8 = 0x80;
/// Sprite-zero hit.
pub const STATUS_SPRITE0: u8 = 0x40;
/// Sprite overflow.
pub const STATUS_OVERFLOW: u8 = 0x20;

/// CHR latch state for MMC2/MMC4 (mappers 9 and 10).
///
/// The four registers select a 4 KiB bank per pattern table and latch:
/// `[latch0 $FD, latch0 $FE, latch1 $FD, latch1 $FE]`. A pattern fetch whose
/// tile low byte is `$FD` or `$FE` selects the matching bank for the rest of
/// the table, which is how Punch-Out!! swaps background and sprite tiles
/// mid-frame.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Mmc2Chr {
    rom: Vec<u8>,
    banks: [u8; 4],
    latch: [bool; 2],
}

impl Mmc2Chr {
    fn new(rom: Vec<u8>, banks: [u8; 4]) -> Self {
        Self {
            rom,
            banks,
            latch: [false; 2],
        }
    }

    fn read(&self, address: u16) -> u8 {
        let table = usize::from(address & 0x1000 != 0);
        let register = if self.latch[table] {
            self.banks[table * 2 + 1]
        } else {
            self.banks[table * 2]
        };
        // The register is 5 bits wide; the ROM may carry fewer banks, so the
        // unconnected high address lines wrap (e.g. 64 KiB CHR = 16 banks).
        let banks = (self.rom.len() / 0x1000).max(1);
        let bank = usize::from(register) % banks;
        let index = bank * 0x1000 + usize::from(address & 0x0FFF);
        self.rom.get(index).copied().unwrap_or(0)
    }

    fn note_fetch(&mut self, address: u16) {
        // The tile number low byte occupies bits 4..=11 of the pattern address.
        let tile = (address >> 4) as u8;
        if tile == 0xFD || tile == 0xFE {
            let table = usize::from(address & 0x1000 != 0);
            self.latch[table] = tile == 0xFE;
        }
    }
}

/// 2C02 state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// PPU data state-machine countdown (0 = idle).
    data_read: u8,
    /// `v` captured when the `$2007` access started.
    data_read_address: u16,
    /// Rendering state captured at the `$2007` access.
    data_read_render: bool,
    /// Byte most recently read by a background/sprite fetch.
    fetch_byte: u8,
    /// Dots the current fetch byte stays on the bus (reads span two dots).
    fetch_hold: u8,
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
    /// Nametable VRAM (4 KiB; four-screen carts use all of it).
    #[serde(with = "serde_big_array::BigArray")]
    pub vram: [u8; 0x1000],
    /// Palette RAM (32 bytes).
    pub palette: [u8; 32],
    /// Expanded RGB for each palette entry (rebuilt on palette writes).
    palette_rgb: [[u8; 3]; 32],
    /// Object attribute memory.
    #[serde(with = "serde_big_array::BigArray")]
    pub oam: [u8; 256],
    /// CHR memory (8 KiB window; mappers can replace this later).
    pub chr: Vec<u8>,
    /// Per-slot 1 KiB CHR-RAM bank map for banked boards (JY ASIC).
    chr_ram_map: Option<[u8; 8]>,
    /// MMC2/MMC4 latch-banked CHR, when the cartridge uses it.
    mmc2: Option<Mmc2Chr>,
    /// Shared hybrid CHR state (mappers 165/195).
    hybrid: Option<Rc<RefCell<HybridChr>>>,
    /// Shared MMC5 state (ExRAM/nametables/CHR map).
    mmc5_aux: Option<Rc<RefCell<Mmc5Aux>>>,
    /// Shared Namco 163 nametable state (mapper 19).
    n163_nt: Option<Rc<RefCell<N163Nt>>>,
    /// Shared banked CHR-RAM state (VRC7).
    vrc7_chr: Option<Rc<RefCell<Vrc7Chr>>>,
    /// Shared Oeka Kids CHR state (mapper 96).
    oeka_chr: Option<Rc<RefCell<OekaChr>>>,
    /// Shared CHR state for mapper 512.
    chr512: Option<Rc<RefCell<Chr512>>>,
    /// Shared CHR/nametable state for mapper 77.
    aux77: Option<Rc<RefCell<Aux77>>>,
    /// Extended-attribute byte for the tile currently being fetched (MMC5
    /// mode 1).
    ext_attr: Option<u8>,
    /// Full CHR ROM used for ROM nametables (Sunsoft-4), if any.
    nt_rom: Vec<u8>,
    /// Two 1 KiB CHR nametable bank indices when ROM nametables are enabled.
    nt_banks: Option<[u16; 2]>,
    /// Per-quadrant ROM-nametable CHR pages (mapper 559).
    nt_banks4: Option<[u16; 4]>,
    nt_quads: Option<[NtMapping; 4]>,
    qtram: Option<Rc<RefCell<Qtram>>>,
    /// PPU dots remaining in the post-reset warm-up window; while non-zero,
    /// writes to `$2000`/`$2001`/`$2005`/`$2006` are ignored (NES model).
    warmup_dots: u32,
    /// Whether CHR is writable RAM.
    pub chr_ram: bool,
    /// Nametable mirroring.
    pub mirroring: Mirroring,
    /// Per-quadrant CIRAM page selection from the mapper, if any.
    nt_map: Option<[u8; 4]>,
    /// Shared A12-edge IRQ counter (mapper 563).
    a12: Option<Rc<RefCell<A12Irq>>>,
    /// Shared MMC3 A12 edge detector.
    mmc3_a12: Option<Rc<RefCell<Mmc3A12>>>,
    /// Absolute PPU dot counter, used by the MMC3 A12 low-time filter.
    dots: u64,
    /// UM6578 enhanced-mode flag (mapper 405).
    um6578: bool,
    /// UM6578 10 KiB video RAM.
    um_vram: Vec<u8>,
    /// UM6578 `$2008` colour/tile select.
    um_ctrl: u8,
    /// UM6578 64-entry palette.
    #[serde(with = "serde_big_array::BigArray")]
    um_palette: [u8; 64],
    /// Shared CHR-RAM state for mapper 169's computer modes.
    chr169: Option<Rc<RefCell<Chr169>>>,
    /// Shared 16 KiB CHR-RAM for CPROM (mapper 13).
    cprom_chr: Option<Rc<RefCell<CpromChr>>>,
    /// Mapper 153's PPU-driven outer PRG bank state.
    bandai153: Option<Rc<RefCell<Bandai153>>>,
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
    /// Display palette preset (does not affect emulated values).
    palette_variant: PaletteVariant,
    /// User table for [`PaletteVariant::Custom`] (64 RGB triplets, indexed by
    /// the 6-bit NES colour).
    custom_palette: Vec<[u8; 3]>,
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
    /// Bitmask of sprite slots holding a sprite for the current line.
    spr_active: u8,
    /// Precomputed RGB for the rest of the current background span.
    span_px: [[u8; 3]; 8],
    /// Dots remaining in the precomputed span (0 = use the per-dot path).
    span_len: u8,
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
            data_read: 0,
            data_read_address: 0,
            data_read_render: false,
            fetch_byte: 0,
            fetch_hold: 0,
            io_bus: 0,
            io_bus_age: 0,
            scanline: 0,
            dot: 40,
            frames: 0,
            // NTSC warms up for 29,658 CPU cycles, PAL/Dendy for 33,132;
            // the PPU advances three dots per CPU cycle.
            warmup_dots: (if timing.cpu_hz() > 1_700_000 {
                29_658u32
            } else {
                33_132u32
            }) * 3,
            vram: [0; 0x1000],
            palette: [0; 32],
            // Palette RAM powers on to index 0, so the backdrop starts as
            // NES_PALETTE[0] rather than black.
            palette_rgb: [NES_PALETTE[0]; 32],
            oam: [0; 256],
            chr,
            chr_ram_map: None,
            nt_map: None,
            a12: None,
            mmc3_a12: None,
            dots: 0,
            um6578: false,
            um_vram: vec![0; 0x10000],
            um_ctrl: 0,
            um_palette: [0; 64],
            chr169: None,
            cprom_chr: None,
            bandai153: None,
            mmc2: None,
            hybrid: None,
            mmc5_aux: None,
            n163_nt: None,
            vrc7_chr: None,
            oeka_chr: None,
            chr512: None,
            aux77: None,
            ext_attr: None,
            nt_rom: Vec::new(),
            nt_banks: None,
            nt_banks4: None,
            nt_quads: None,
            qtram: None,
            chr_ram,
            mirroring: if vertical_mirroring {
                Mirroring::Vertical
            } else {
                Mirroring::Horizontal
            },
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
            palette_variant: PaletteVariant::Canonical,
            custom_palette: Vec::new(),
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
            spr_active: 0,
            span_px: [[0; 3]; 8],
            span_len: 0,
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

    /// Shares the mapper's hybrid CHR state (mappers 165/195).
    pub fn set_hybrid_chr(&mut self, hybrid: Rc<RefCell<HybridChr>>) {
        self.hybrid = Some(hybrid);
    }

    /// Shares the mapper's MMC5 state (ExRAM/nametables/CHR map).
    pub fn set_mmc5_aux(&mut self, aux: Rc<RefCell<Mmc5Aux>>) {
        self.mmc5_aux = Some(aux);
    }

    /// Shares the mapper's Namco 163 nametable state (mapper 19).
    pub fn set_n163_nt(&mut self, nt: Rc<RefCell<N163Nt>>) {
        self.n163_nt = Some(nt);
    }

    /// Shares the mapper's banked CHR-RAM state (VRC7).
    pub fn set_vrc7_chr(&mut self, chr: Rc<RefCell<Vrc7Chr>>) {
        self.vrc7_chr = Some(chr);
    }

    /// Installs the Oeka Kids shared CHR state (mapper 96).
    pub fn set_oeka_chr(&mut self, chr: Rc<RefCell<OekaChr>>) {
        self.oeka_chr = Some(chr);
    }

    /// Installs the mapper-512 shared CHR state.
    pub fn set_chr512(&mut self, chr: Rc<RefCell<Chr512>>) {
        self.chr512 = Some(chr);
    }

    /// Installs the mapper-77 shared CHR/nametable state.
    pub fn set_aux77(&mut self, aux: Rc<RefCell<Aux77>>) {
        self.aux77 = Some(aux);
    }

    /// Installs the full CHR ROM used for ROM nametables.
    pub fn set_rom_nametable_rom(&mut self, rom: Vec<u8>) {
        self.nt_rom = rom;
    }

    /// Enables ROM nametables with the two 1 KiB bank indices, or disables
    /// them with `None` (CIRAM).
    pub fn set_rom_nametable_banks(&mut self, banks: Option<[u16; 2]>) {
        self.nt_banks = banks;
    }

    /// Enables per-quadrant ROM nametables (mapper 559).
    pub fn set_rom_nametable_quadrants(&mut self, banks: Option<[u16; 4]>) {
        self.nt_banks4 = banks;
    }

    /// Installs a mixed CIRAM/CHR-ROM nametable mapping (J.Y. ASIC).
    pub fn set_nametable_quadrants(&mut self, mapping: Option<[NtMapping; 4]>) {
        self.nt_quads = mapping;
    }

    /// Installs the Q-Ta shadow nametable/CHR state.
    pub fn set_qtram(&mut self, qt: Rc<RefCell<Qtram>>) {
        self.qtram = Some(qt);
    }

    /// Replaces the CHR window with latch-banked MMC2/MMC4 ROM.
    pub fn set_mmc2_chr(&mut self, rom: Vec<u8>, banks: [u8; 4]) {
        self.mmc2 = Some(Mmc2Chr::new(rom, banks));
    }

    /// Updates the four MMC2/MMC4 bank registers without touching the latches.
    pub fn set_mmc2_banks(&mut self, banks: [u8; 4]) {
        if let Some(mmc2) = &mut self.mmc2 {
            mmc2.banks = banks;
        }
    }

    /// Whether latch-banked CHR is active.
    #[must_use]
    pub const fn has_mmc2_chr(&self) -> bool {
        self.mmc2.is_some()
    }

    /// Current MMC2/MMC4 bank registers, if active.
    #[must_use]
    pub fn mmc2_banks(&self) -> Option<[u8; 4]> {
        self.mmc2.as_ref().map(|mmc2| mmc2.banks)
    }

    /// Current MMC2/MMC4 latch selection (`false` = `$FD` bank).
    #[must_use]
    pub fn mmc2_latch(&self) -> Option<[bool; 2]> {
        self.mmc2.as_ref().map(|mmc2| mmc2.latch)
    }

    /// Test-only nametable read hook.
    #[cfg(test)]
    pub(crate) fn debug_read_vram(&self, address: u16) -> u8 {
        self.read_vram(address)
    }

    /// Test-only nametable write hook.
    #[cfg(test)]
    pub(crate) fn debug_write_vram(&mut self, address: u16, value: u8) {
        self.write_vram(address, value);
    }

    /// CHR read hook used by the debugger front-end.
    pub(crate) fn debug_chr_byte(&self, address: u16) -> u8 {
        self.chr_byte(address)
    }

    /// Test-only background CHR read hook.
    #[cfg(test)]
    pub(crate) fn debug_bg_chr_byte(&self, address: u16) -> u8 {
        self.chr_byte_background(address)
    }

    /// Test-only framebuffer pixel hook.
    #[cfg(test)]
    pub(crate) fn debug_pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * 256 + x) * 4;
        [
            self.frame[i],
            self.frame[i + 1],
            self.frame[i + 2],
            self.frame[i + 3],
        ]
    }

    /// Test-only expanded palette hook.
    #[cfg(test)]
    pub(crate) fn debug_palette(&self, index: usize) -> [u8; 4] {
        let [r, g, b] = self.palette_rgb[index];
        [r, g, b, 255]
    }

    /// Physical index into the 8 KiB CHR-RAM buffer for a PPU address.
    ///
    /// Boards with banked CHR-RAM (the JY ASIC) map each 1 KiB slot through
    /// their registers; flat boards keep the identity.
    fn chr_ram_index(&self, address: u16) -> usize {
        let slot = (usize::from(address) >> 10) & 0x07;
        let bank = match self.chr_ram_map {
            Some(map) => usize::from(map[slot]) & 0x07,
            None => slot,
        };
        bank * 0x400 + usize::from(address & 0x03FF)
    }

    /// Installs the mapper's CHR-RAM slot map (`None` = identity).
    pub fn set_chr_ram_map(&mut self, map: Option<[u8; 8]>) {
        self.chr_ram_map = map;
    }

    /// Installs the mapper's per-quadrant CIRAM selection.
    pub fn set_nametable_map(&mut self, map: Option<[u8; 4]>) {
        self.nt_map = map;
    }

    /// Installs the mapper's shared A12-edge IRQ counter.
    pub fn set_a12_irq(&mut self, aux: Rc<RefCell<A12Irq>>) {
        self.a12 = Some(aux);
    }

    /// Attaches the shared MMC3 A12 edge detector.
    pub fn set_mmc3_a12(&mut self, aux: Rc<RefCell<Mmc3A12>>) {
        self.mmc3_a12 = Some(aux);
    }

    fn observe_mmc3_a12(&self, address: u16) {
        if let Some(aux) = &self.mmc3_a12 {
            aux.borrow_mut().observe(address, self.dots);
        }
    }

    /// Marks the PPU as a UM6578 enhanced famiclone.
    pub fn set_um6578(&mut self, enabled: bool) {
        self.um6578 = enabled;
    }

    /// Installs mapper 169's shared CHR-RAM state.
    pub fn set_chr169(&mut self, aux: Rc<RefCell<Chr169>>) {
        self.chr169 = Some(aux);
    }

    /// Installs CPROM's shared 16 KiB CHR-RAM state.
    pub fn set_cprom_chr(&mut self, aux: Rc<RefCell<CpromChr>>) {
        self.cprom_chr = Some(aux);
    }

    /// Installs mapper 153's shared outer-bank state.
    pub fn set_bandai153(&mut self, aux: Rc<RefCell<Bandai153>>) {
        self.bandai153 = Some(aux);
    }

    /// Writes one byte from a cartridge DMA transfer to the PPU bus.
    pub fn dma_write(&mut self, address: u16, value: u8) {
        self.write_vram(address, value);
    }

    /// Reads one byte of the active CHR window.
    fn chr_byte(&self, address: u16) -> u8 {
        if let Some(aux) = &self.bandai153 {
            aux.borrow_mut().note_address(address);
        }
        if let Some(aux) = &self.cprom_chr {
            return aux.borrow().read(address);
        }
        if let Some(aux) = &self.chr169 {
            return aux.borrow_mut().read(address);
        }
        if let Some(aux) = &self.a12 {
            aux.borrow_mut().observe(address);
        }
        self.observe_mmc3_a12(address);
        if let Some(aux) = &self.aux77 {
            return aux.borrow().chr_read(address);
        }
        if let Some(aux) = &self.chr512 {
            return aux.borrow().read(address);
        }
        if let Some(aux) = &self.oeka_chr {
            let mut aux = aux.borrow_mut();
            aux.note_address(address);
            return aux.read(address);
        }
        if let Some(aux) = &self.vrc7_chr {
            return aux.borrow().read(address);
        }
        if let Some(aux) = &self.mmc5_aux {
            return aux.borrow().chr_read(address, &self.chr);
        }
        if let Some(hybrid) = &self.hybrid {
            return hybrid.borrow().read(address);
        }
        match &self.mmc2 {
            Some(mmc2) => mmc2.read(address),
            None if self.chr_ram && self.chr_ram_map.is_some() => {
                let index = self.chr_ram_index(address);
                self.chr.get(index).copied().unwrap_or(0)
            }
            None => self
                .chr
                .get(usize::from(address) % self.chr.len().max(1))
                .copied()
                .unwrap_or(0),
        }
    }

    /// Feeds a pattern fetch to the MMC2/MMC4 latch.
    fn note_chr_fetch(&mut self, address: u16) {
        if let Some(hybrid) = &self.hybrid {
            hybrid.borrow_mut().note_fetch(address);
        }
        if let Some(mmc2) = &mut self.mmc2 {
            mmc2.note_fetch(address);
        }
    }

    /// Ends the post-reset warm-up window immediately.
    pub fn end_warmup(&mut self) {
        self.warmup_dots = 0;
    }

    /// Advances one PPU dot.
    pub fn step_dot(&mut self) {
        self.dots = self.dots.wrapping_add(1);
        self.warmup_dots = self.warmup_dots.saturating_sub(1);
        if let Some(value) = self.pending_mask {
            if self.mask_delay == 0 {
                self.mask = value;
                self.pending_mask = None;
                self.refresh_palette_rgb();
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
        let rendered_line = self.scanline < 240 || self.scanline == self.pre_render_line();
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
            let mut mask = 0u8;
            let ids = self.spr_id;
            for (slot, id) in ids.iter().enumerate() {
                if *id != 64 {
                    mask |= 1 << slot;
                }
            }
            self.spr_active = mask;
            self.sprites = self.next_sprites;
            self.sprite_count = self.next_count;
            if self.scanline < 240 || self.scanline == self.pre_render_line() {
                self.oam_id = [64; 8];
            }
        }
        if self.mask & 0x18 != 0
            && (self.scanline < 240 || self.scanline == self.pre_render_line())
            && (257..=320).contains(&self.dot)
        {
            if self.dot == 257 {
                self.fetch_addr = 0;
            } else if self.dot.is_multiple_of(2) && !self.oam2_overflow {
                self.fetch_addr = self.fetch_addr.wrapping_add(1);
            }
            let slot = ((self.dot - 257) / 8) as usize;
            let k = (self.dot - 257) % 8;
            match k {
                0 => {
                    // The first sprite slot's dummy nametable read still sees
                    // the line-end `v` plus its pending coarse-X step, before
                    // dot 257's horizontal reset.
                    let pending = if self.dot == 257 { 1 } else { 0 };
                    self.fetch_byte =
                        self.read_vram(0x2000 | (self.v.wrapping_add(pending) & 0x0FFF));
                    self.fetch_hold = 2;
                }
                2 => {
                    self.fetch_byte = self.read_vram(0x2000 | (self.v.wrapping_sub(1) & 0x0FFF));
                    self.fetch_hold = 2;
                }
                4 => {
                    self.latch_sprite(slot);
                    self.fetch_byte = self.spr_lo[slot];
                    self.fetch_hold = 2;
                }
                6 => {
                    self.fetch_byte = self.spr_hi[slot];
                    self.fetch_hold = 2;
                }
                _ => {}
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
            let ids = self.spr_id;
            for (slot, id) in ids.iter().enumerate() {
                if *id != 64 {
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
        if self.dot == 257 && self.mask & 0x18 != 0 && self.scanline <= self.pre_render_line() {
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
        let rendered_line = self.scanline < 240 || self.scanline == self.pre_render_line();
        let rendering = self.mask & 0x18 != 0;
        if rendered_line && (1..=256).contains(&self.dot) {
            if self.scanline < 240 {
                self.render_span_dot(self.dot);
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
        } else if rendered_line && rendering && (337..=340).contains(&self.dot) {
            // Dummy fetch: two nametable reads; the attribute slot reads the
            // nametable as well on hardware.
            if self.dot & 1 == 1 {
                self.fetch_byte = self.read_vram(0x2000 | (self.v & 0x0FFF));
                self.fetch_hold = 2;
            }
        }

        // Live VRAM address updates for background fetches. These only happen
        // on visible lines and the pre-render line, never in vblank.
        if (self.scanline < 240 || self.scanline == self.pre_render_line()) && self.mask & 0x18 != 0
        {
            match self.dot {
                8 | 16 | 24 | 32 | 40 | 48 | 56 | 64 | 72 | 80 | 88 | 96 | 104 | 112 | 120
                | 128 | 136 | 144 | 152 | 160 | 168 | 176 | 184 | 192 | 200 | 208 | 216 | 224
                | 232 | 240 | 248 | 328 | 336 => self.increment_coarse_x(),
                256 => self.increment_y(),
                257 => self.copy_horizontal(),
                _ => {}
            }
            if self.scanline == self.pre_render_line() && (280..=304).contains(&self.dot) {
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
        if self.scanline == self.pre_render_line() && self.dot == self.vblank_clear_dot {
            self.status &= !(STATUS_VBLANK | STATUS_SPRITE0 | STATUS_OVERFLOW);
            self.nmi_line = false;
            self.nmi_suppressed = false;
            self.vblank_suppressed = false;
        }

        if self.data_read > 0 {
            self.data_read -= 1;
            if self.data_read == 0 {
                self.read_buffer = if self.data_read_render && self.fetch_hold > 0 {
                    self.fetch_byte
                } else {
                    let address = self.data_read_address;
                    // Palette reads return the palette entry but load the
                    // buffer from the nametable underneath it ($2Fxx).
                    let buffer_address = if address >= 0x3F00 {
                        address & 0x2FFF
                    } else {
                        address
                    };
                    self.read_vram(buffer_address)
                };
                if self.data_read_render {
                    self.increment_vram_address(true);
                }
            }
        }
        self.fetch_hold = self.fetch_hold.saturating_sub(1);

        self.dot += 1;
        // The pre-render line is one dot shorter on odd frames when rendering
        // is enabled. The decision is sampled at dot 339, the dot before the
        // skipped one.
        if self.scanline == self.pre_render_line() && self.dot == 339 {
            self.rendering_at_339 = self.mask & 0x18 != 0;
        }
        if self.timing.region == crate::clock::Region::Ntsc
            && self.scanline == self.pre_render_line()
            && self.dot == 340
            && self.frames & 1 == 1
            && self.rendering_at_339
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
    #[inline]
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
                if self.scanline == self.pre_render_line() && self.dot == self.vblank_clear_dot {
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
                } else if (self.scanline < 240 || self.scanline == self.pre_render_line())
                    && self.mask & 0x18 != 0
                {
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
                if let Some(hybrid) = &self.hybrid {
                    hybrid.borrow_mut().note_read(address);
                }
                self.data_read_address = address;
                self.data_read_render = (self.scanline < 240
                    || self.scanline == self.pre_render_line())
                    && self.mask & 0x18 != 0;
                self.data_read = 4;
                let value = if address >= 0x3F00 {
                    let mut palette = self.read_palette(address) & 0x3F;
                    if self.mask & 0x01 != 0 {
                        // Greyscale mode zeros the low four palette bits on reads.
                        palette &= 0x30;
                    }
                    self.read_buffer = self.read_vram(address & 0x2FFF);
                    palette | (self.io_bus & 0xC0)
                } else {
                    self.read_buffer
                };
                self.io_bus = value;
                self.io_bus_age = 0;
                if !self.data_read_render {
                    // Forced blank: nothing else is using `v`, so the increment
                    // can land at the read. During rendering it is deferred so
                    // the still-running fetch pipeline is not disturbed.
                    self.increment_vram_address(false);
                    self.observe_mmc3_a12(self.v);
                }
                value
            }
            _ => self.io_bus,
        }
    }

    /// Writes a PPU register in `$2000..=$2007`.
    pub fn write_register(&mut self, index: u16, value: u8) {
        if self.um6578 {
            match index {
                0x2008 => {
                    self.um_ctrl = value;
                    return;
                }
                0x2040..=0x207F => {
                    self.um_palette[usize::from(index - 0x2040)] = value;
                    return;
                }
                _ => {}
            }
        }
        self.io_bus = value;
        self.io_bus_age = 0;
        // The 2C02 ignores these writes for about one frame after reset.
        if self.warmup_dots > 0 && matches!(index & 7, 0 | 1 | 5) {
            return;
        }
        match index & 7 {
            0 => {
                let was_enabled = self.ctrl & 0x80 != 0;
                self.ctrl = value;
                self.t = (self.t & !0x0C00) | (u16::from(value & 0x03) << 10);
                if value & 0x80 != 0 && !was_enabled && self.status & STATUS_VBLANK != 0 {
                    let on_clear_dot = self.scanline == self.pre_render_line()
                        && self.dot == self.vblank_clear_dot;
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
                self.span_len = 0;
                // PPU register writes reach their destination a couple of dots
                // after the bus write; model the smallest delay. Pre-render
                // writes stay immediate because the odd-frame skip decision is
                // sampled at dot 339 with the calibrated phase.
                if self.scanline == self.pre_render_line() {
                    self.mask = value;
                    self.refresh_palette_rgb();
                } else {
                    self.pending_mask = Some(value);
                    // PPU register writes reach their destination two dots
                    // after the bus write.
                    self.mask_delay = 2;
                }
            }
            3 => self.oam_addr = value,
            4 => {
                if (self.scanline < 240 || self.scanline == self.pre_render_line())
                    && self.mask & 0x18 != 0
                {
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
                    self.observe_mmc3_a12(self.v);
                    if (self.scanline < 240 || self.scanline == self.pre_render_line())
                        && self.mask & 0x18 != 0
                    {
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
                let render = (self.scanline < 240 || self.scanline == self.pre_render_line())
                    && self.mask & 0x18 != 0;
                self.increment_vram_address(render);
                self.observe_mmc3_a12(self.v);
            }
            _ => {}
        }
    }

    /// Pre-render scanline for the configured region (261 NTSC, 311 PAL).
    fn pre_render_line(&self) -> u16 {
        self.timing.scanlines - 1
    }

    fn nametable_index(&self, address: u16) -> usize {
        if let Some(aux) = &self.oeka_chr {
            aux.borrow_mut().note_address(address);
        }
        let index = usize::from(address & 0x0FFF);
        let table = index / 0x0400;
        let offset = index % 0x0400;
        let mapped = if let Some(map) = self.nt_map {
            usize::from(map[table] & 0x01)
        } else {
            match self.mirroring {
                Mirroring::Vertical => table % 2,
                Mirroring::Horizontal => table / 2,
                Mirroring::SingleLower => 0,
                Mirroring::SingleUpper => 1,
                Mirroring::FourScreen => table,
            }
        };
        mapped * 0x0400 + offset
    }

    /// Selects the CHR bank for a nametable address in ROM-nametable mode.
    fn nametable_bank(&self, address: u16, banks: [u16; 2]) -> u16 {
        let table = usize::from(address >> 10 & 3);
        match self.mirroring {
            Mirroring::Vertical | Mirroring::FourScreen => banks[table % 2],
            Mirroring::Horizontal => banks[usize::from(table >= 2)],
            Mirroring::SingleLower => banks[0],
            Mirroring::SingleUpper => banks[1],
        }
    }

    fn read_nametable(&self, address: u16) -> u8 {
        if let Some(qt) = &self.qtram {
            qt.borrow_mut().note_read(0x2000 | (address & 0x0FFF));
        }
        if let Some(aux) = &self.bandai153 {
            aux.borrow_mut().note_address(address);
        }
        if let Some(aux) = &self.chr169 {
            let mut aux = aux.borrow_mut();
            aux.note_address(0x2000 | (address & 0x0FFF));
            if aux.nt_split() {
                return aux.nt_read(address);
            }
        }
        if let Some(aux) = &self.a12 {
            aux.borrow_mut().observe(address);
        }
        self.observe_mmc3_a12(address);
        if let Some(aux) = &self.aux77 {
            return aux.borrow().nt_read(address);
        }
        if let Some(aux) = &self.chr512 {
            if let Some(value) = aux.borrow().nametable_read(address) {
                return value;
            }
        }
        // Namespace 19 maps each quadrant to CIRAM or a CHR-ROM page; both
        // live outside the normal mirroring path.
        if let Some(nt) = &self.n163_nt {
            return nt.borrow().read(0x2000 | (address & 0x0FFF));
        }
        if let Some(aux) = &self.mmc5_aux {
            let aux = aux.borrow();
            if let Some(value) = aux.nametable_override(address) {
                return value;
            }
            if let Some(index) = aux.nametable_index(address) {
                return self.vram[index];
            }
        }
        if let Some(quads) = self.nt_quads {
            let table = usize::from(address >> 10 & 3);
            let offset = usize::from(address & 0x3FF);
            return match quads[table] {
                NtMapping::Ciram(page) => self.vram[usize::from(page & 1) * 0x400 + offset],
                NtMapping::Rom(bank) => {
                    let bank = usize::from(bank) % (self.nt_rom.len() / 0x400).max(1);
                    self.nt_rom.get(bank * 0x400 + offset).copied().unwrap_or(0)
                }
            };
        }
        if let Some(banks) = self.nt_banks4 {
            if !self.nt_rom.is_empty() {
                let table = usize::from(address >> 10 & 3);
                let bank = usize::from(banks[table]) % (self.nt_rom.len() / 0x400).max(1);
                let offset = bank * 0x400 + usize::from(address & 0x3FF);
                return self.nt_rom[offset];
            }
        }
        if let Some(banks) = self.nt_banks {
            if !self.nt_rom.is_empty() {
                let bank = usize::from(self.nametable_bank(address, banks))
                    % (self.nt_rom.len() / 0x400).max(1);
                let offset = bank * 0x400 + usize::from(address & 0x3FF);
                return self.nt_rom[offset];
            }
        }
        self.vram[self.nametable_index(address)]
    }

    /// Increments `v` after a `$2007` access. During rendering this performs a
    /// coarse-X and Y increment simultaneously instead of the normal +1/+32.
    fn increment_vram_address(&mut self, render_glitch: bool) {
        if render_glitch {
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

    /// Selects the display palette preset and rebuilds the RGB cache.
    pub fn set_palette_variant(&mut self, variant: PaletteVariant) {
        self.palette_variant = variant;
        self.refresh_palette_rgb();
    }

    /// Soft-reset the `$2000`/`$2001`-derived register state, as the
    /// MiSTer "NES" PPU reset behaviour does. VRAM, OAM and palette RAM are
    /// left intact.
    pub fn soft_reset_registers(&mut self) {
        self.ctrl = 0;
        self.mask = 0;
        self.span_len = 0;
        self.refresh_palette_rgb();
    }

    /// Supplies the 64-entry table used by [`PaletteVariant::Custom`].
    pub fn set_palette_table(&mut self, table: [[u8; 3]; 64]) {
        self.custom_palette = table.to_vec();
        if matches!(self.palette_variant, PaletteVariant::Custom) {
            self.refresh_palette_rgb();
        }
    }

    /// The 32 palette entries as RGB, after emphasis, for debuggers.
    #[must_use]
    pub fn debug_palette_rgb(&self) -> [[u8; 3]; 32] {
        self.palette_rgb
    }

    /// Side-effect-free PPU address-space read for debuggers.
    #[must_use]
    pub fn debug_peek(&self, address: u16) -> u8 {
        let address = address & 0x3FFF;
        if address >= 0x3F00 {
            self.read_palette(address)
        } else {
            self.read_vram(address)
        }
    }

    /// PPU address-space write for debuggers.
    pub fn debug_poke(&mut self, address: u16, value: u8) {
        self.write_vram(address & 0x3FFF, value);
    }

    /// Side-effect-free read of `$2000-$2007`.
    #[must_use]
    pub fn debug_register_peek(&self, index: u16) -> u8 {
        match index & 7 {
            0 => self.ctrl,
            1 => self.mask,
            2 => self.status,
            3 => self.oam_addr,
            4 => self.oam[usize::from(self.oam_addr)],
            7 => self.read_buffer,
            _ => 0,
        }
    }

    fn read_vram(&self, address: u16) -> u8 {
        if self.um6578 {
            return if (0x3F00..0x3F40).contains(&address) {
                self.um_palette[usize::from(address - 0x3F00)]
            } else {
                // 10 KiB internal RAM plus the optional 32 KiB external
                // CHR-RAM share the PPU's 64 KiB address space.
                self.um_vram.get(usize::from(address)).copied().unwrap_or(0)
            };
        }
        if address < 0x2000 {
            self.chr_byte(address)
        } else if address < 0x3F00 {
            self.read_nametable(address)
        } else {
            self.read_palette(address)
        }
    }

    fn write_vram(&mut self, address: u16, value: u8) {
        if self.um6578 {
            if (0x3F00..0x3F40).contains(&address) {
                self.um_palette[usize::from(address - 0x3F00)] = value;
            } else if let Some(byte) = self.um_vram.get_mut(usize::from(address)) {
                *byte = value;
            }
            return;
        }
        if (0x2000..0x3F00).contains(&address) {
            if let Some(aux) = &self.aux77 {
                aux.borrow_mut().nt_write(address, value);
                return;
            }
            if let Some(aux) = &self.chr512 {
                if aux.borrow_mut().nametable_write(address, value) {
                    return;
                }
            }
        }
        if let Some(aux) = &self.cprom_chr {
            if address < 0x2000 {
                aux.borrow_mut().write(address, value);
                return;
            }
        }
        if let Some(aux) = &self.chr169 {
            if address < 0x2000 {
                aux.borrow_mut().write(address, value);
                return;
            }
            if (0x2000..0x3000).contains(&address) && aux.borrow().nt_split() {
                aux.borrow_mut().nt_write(address, value);
                return;
            }
        }
        if address < 0x2000 {
            if let Some(aux) = &self.aux77 {
                aux.borrow_mut().chr_write(address, value);
                return;
            }
            if let Some(aux) = &self.chr512 {
                aux.borrow_mut().write(address, value);
                return;
            }
            if let Some(aux) = &self.oeka_chr {
                aux.borrow_mut().write(address, value);
                return;
            }
            if let Some(aux) = &self.vrc7_chr {
                aux.borrow_mut().write(address, value);
                return;
            }
            if let Some(aux) = &self.mmc5_aux {
                if aux.borrow().is_ram() {
                    let offset = aux.borrow().chr_offset(address);
                    if let Some(byte) = self.chr.get_mut(offset) {
                        *byte = value;
                    }
                }
            } else if let Some(hybrid) = &self.hybrid {
                hybrid.borrow_mut().write(address, value);
            } else if self.chr_ram {
                let index = self.chr_ram_index(address);
                if let Some(byte) = self.chr.get_mut(index) {
                    *byte = value;
                }
            }
        } else if address < 0x3F00 {
            if let Some(nt) = &self.n163_nt {
                // CHR-ROM quadrants absorb the write; CIRAM quadrants store.
                nt.borrow_mut().write(0x2000 | (address & 0x0FFF), value);
                return;
            }
            if let Some(aux) = &self.mmc5_aux {
                let mut aux = aux.borrow_mut();
                if aux.nametable_write(address, value) {
                    return;
                }
                if let Some(index) = aux.nametable_index(address) {
                    self.vram[index] = value;
                    return;
                }
            }
            if let Some(quads) = self.nt_quads {
                let table = usize::from(address >> 10 & 3);
                if let NtMapping::Ciram(page) = quads[table] {
                    let offset = usize::from(address & 0x3FF);
                    self.vram[usize::from(page & 1) * 0x400 + offset] = value;
                }
                return;
            }
            // ROM nametables are read-only.
            if self.nt_banks.is_none() && self.nt_banks4.is_none() {
                let index = self.nametable_index(address);
                self.vram[index] = value;
            }
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

    /// Rebuilds the 32-entry RGB cache for the current PPUMASK emphasis.
    fn refresh_palette_rgb(&mut self) {
        let emph = (self.mask >> 5) & 0x07;
        for index in 0..32 {
            let colour = self.palette[index] & 0x3F;
            let base = if matches!(self.palette_variant, PaletteVariant::Custom) {
                self.custom_palette
                    .get(usize::from(colour))
                    .copied()
                    .unwrap_or(NES_PALETTE[usize::from(colour)])
            } else if let Some(table) = crate::palette::generated_table(self.palette_variant) {
                table[usize::from(colour)]
            } else {
                palette_colour(self.palette_variant, NES_PALETTE[usize::from(colour)])
            };
            self.palette_rgb[index] = if emph == 0 {
                base
            } else {
                let delta = emph_delta(colour, emph);
                [
                    emph_apply(base[0], delta[0]),
                    emph_apply(base[1], delta[1]),
                    emph_apply(base[2], delta[2]),
                ]
            };
        }
        self.span_len = 0;
    }

    fn write_palette(&mut self, address: u16, value: u8) {
        let index = Self::palette_index(address);
        self.palette[index] = value & 0x3F;
        self.refresh_palette_rgb();
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
                let ly = if self.scanline == self.pre_render_line() {
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
            let ly = if self.scanline == self.pre_render_line() {
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
            let ly = if self.scanline == self.pre_render_line() {
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
                if self.scanline == self.pre_render_line() {
                    // Sprite evaluation does not run on the pre-render line,
                    // so the fetch uses whatever secondary OAM still holds.
                    // The first non-empty entry is treated as sprite zero.
                    if self.secondary_oam[base] == 0xFF {
                        64
                    } else if slot == 0 {
                        0
                    } else {
                        1
                    }
                } else {
                    self.oam_id[slot]
                },
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
            // 8x8 sprites use PPUCTRL bit 3, unlike the background (bit 4).
            (if self.ctrl & 0x08 != 0 { 0x1000 } else { 0 }, tile)
        };
        if height == 16 && row >= 8 {
            tile = tile.wrapping_add(1);
            row -= 8;
        }
        let address = bank + u16::from(tile) * 16 + row;
        if let Some(aux) = &self.oeka_chr {
            // Sprite fetches are preceded by a dummy nametable fetch at
            // $2000, which latches the Oeka inner bank bits to 0.
            aux.borrow_mut().note_address(0x2000);
        }
        if let Some(qt) = &self.qtram {
            qt.borrow_mut().sprite = true;
        }
        let mut low = self.chr_byte(address);
        let mut high = self.chr_byte(address + 8);
        if let Some(qt) = &self.qtram {
            qt.borrow_mut().sprite = false;
        }
        self.note_chr_fetch(address);
        self.note_chr_fetch(address + 8);
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

    /// Background fetch-group index for a dot on a rendered line.
    ///
    /// Groups 0-31 are the visible tiles; 32-33 are the two prefetched tiles
    /// that are displayed at the start of the next line.
    fn background_group(dot: u16) -> u16 {
        if dot <= 256 {
            dot.saturating_sub(1) / 8
        } else {
            32 + dot.saturating_sub(321) / 8
        }
    }

    /// Fetches one byte of the current background tile.
    fn fetch_background(&mut self, dot: u16) {
        let group = Self::background_group(dot);
        match dot & 7 {
            1 => {
                let full = 0x2000 | (self.v & 0x0FFF);
                let address = if self.hybrid_fetch {
                    self.hybrid_fetch = false;
                    (full & 0xFF00) | u16::from(self.hybrid_low)
                } else {
                    full
                };
                self.last_fetch_low = address as u8;
                let split = self
                    .mmc5_aux
                    .as_ref()
                    .and_then(|aux| aux.borrow().split_tile(group, self.scanline));
                self.nt_byte = split.unwrap_or_else(|| self.read_vram(address));
                self.ext_attr = if split.is_none() {
                    self.mmc5_aux
                        .as_ref()
                        .and_then(|aux| aux.borrow().extended_attr(self.v))
                } else {
                    None
                };
                self.fetch_byte = self.nt_byte;
                self.fetch_hold = 2;
            }
            3 => {
                let address =
                    0x23C0 | (self.v & 0x0C00) | ((self.v >> 4) & 0x38) | ((self.v >> 2) & 0x07);
                self.last_fetch_low = address as u8;
                let split_attr = self
                    .mmc5_aux
                    .as_ref()
                    .and_then(|aux| aux.borrow().split_attr(group, self.scanline));
                self.at_byte = if let Some(ext) = self.ext_attr {
                    ext
                } else {
                    split_attr.unwrap_or_else(|| self.read_vram(address))
                };
                self.fetch_byte = self.at_byte;
                self.fetch_hold = 2;
            }
            5 => {
                let (address, physical) = self.background_pattern_address(group, false);
                self.last_fetch_low = address as u8;
                let corrupt = match self.ale_countdown {
                    Some(0) => true,
                    Some(n) => {
                        self.ale_countdown = Some(n - 1);
                        false
                    }
                    None => false,
                };
                let clean = if physical {
                    self.split_chr_byte(address)
                } else {
                    self.chr_byte_background(address as u16)
                };
                if corrupt {
                    self.ale_countdown = None;
                    let bus_address = (address & 0xFF00) | 0x00FF;
                    self.pat_lo = if physical {
                        self.split_chr_byte(bus_address)
                    } else {
                        self.chr_byte_background(bus_address as u16)
                    };
                } else {
                    self.pat_lo = clean;
                }
                self.note_chr_fetch(address as u16);
                // The `$2007` state-machine buffer sees the clean plane byte
                // even when the rendering fetch collides with the ALE.
                self.fetch_byte = clean;
                self.fetch_hold = 2;
            }
            7 => {
                let (address, physical) = self.background_pattern_address(group, true);
                self.last_fetch_low = address as u8;
                self.pat_hi = if physical {
                    self.split_chr_byte(address)
                } else {
                    self.chr_byte_background(address as u16)
                };
                self.note_chr_fetch(address as u16);
                self.fetch_byte = self.pat_hi;
                self.fetch_hold = 2;
            }
            _ => {}
        }
    }

    /// CHR address for a pattern fetch, using the split bank/fine Y when the
    /// current fetch group is inside the split region.
    fn split_pattern_address(&self, group: u16, high: bool) -> Option<usize> {
        self.mmc5_aux.as_ref().and_then(|aux| {
            aux.borrow()
                .split_pattern(group, self.scanline, self.nt_byte, high)
        })
    }

    /// CHR address and physical-address flag for a background pattern fetch,
    /// applying MMC5 extended attributes first, then split mode, then normal
    /// CHR banking.
    fn background_pattern_address(&self, group: u16, high: bool) -> (usize, bool) {
        if let (Some(ext), Some(aux)) = (self.ext_attr, &self.mmc5_aux) {
            let fine_y = ((self.v >> 12) & 0x07) as u8;
            let address =
                aux.borrow().extended_pattern(ext, self.nt_byte, fine_y) | (usize::from(high) * 8);
            return (address, true);
        }
        if let Some(address) = self.split_pattern_address(group, high) {
            return (address, true);
        }
        (usize::from(self.pattern_address(self.nt_byte, high)), false)
    }

    /// Background CHR read: MMC5 8x16 mode uses its background bank set.
    fn chr_byte_background(&self, address: u16) -> u8 {
        if let Some(aux) = &self.mmc5_aux {
            return aux.borrow().bg_chr_read(address, &self.chr);
        }
        self.chr_byte(address)
    }

    /// CHR address for a split fetch, where `address` is already the physical
    /// (banked) CHR address rather than a PPU-space address.
    fn split_chr_byte(&self, address: usize) -> u8 {
        match &self.mmc5_aux {
            Some(aux) => aux.borrow().chr_physical(address, &self.chr),
            None => self.chr_byte(address as u16),
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
        let group = Self::background_group(dot);
        let split_attr = self
            .mmc5_aux
            .as_ref()
            .and_then(|aux| aux.borrow().split_attr_field(group, self.scanline));
        let shift = ((self.v >> 4) & 4) | (self.v & 2);
        let attr = if let Some(pal) = self.ext_attr.map(Mmc5Aux::extended_palette) {
            pal
        } else {
            split_attr.unwrap_or_else(|| (self.at_byte >> shift) & 0x03)
        };
        let low: u8 = if attr & 1 != 0 { 0xFF } else { 0 };
        let high: u8 = if attr & 2 != 0 { 0xFF } else { 0 };
        self.bg_attr_lo = (self.bg_attr_lo & 0xFF00) | u16::from(low);
        self.bg_attr_hi = (self.bg_attr_hi & 0xFF00) | u16::from(high);
    }

    /// Returns the CHR byte for `tile` at the current fine Y.
    /// Composes one pixel from the current shift-register state.
    /// Composes a dot, precomputing a whole 8-dot background span at each
    /// fetch-group boundary when no sprite is present and the mask is stable.
    fn render_span_dot(&mut self, dot: u16) {
        if self.mask & 0x18 == 0 {
            self.span_len = 0;
            self.render_backdrop_dot(dot - 1);
            return;
        }
        if dot & 7 == 1 {
            if self.spr_active == 0 && dot + 7 <= 256 && self.mask & 0x08 != 0 {
                let base_x = dot - 1;
                let shift0 = 15 - u16::from(self.fine_x);
                for (j, px) in self.span_px.iter_mut().enumerate() {
                    let shift = shift0 - j as u16;
                    let bit = ((self.bg_lo >> shift) & 1) | (((self.bg_hi >> shift) & 1) << 1);
                    let pal =
                        ((self.bg_attr_lo >> shift) & 1) | (((self.bg_attr_hi >> shift) & 1) << 1);
                    let x = base_x + j as u16;
                    let mut color_index = 0u8;
                    if bit as u8 != 0 && !(x < 8 && self.mask & 0x02 == 0) {
                        color_index = (pal as u8) * 4 + bit as u8;
                    }
                    *px = if color_index == 0 {
                        // Backdrop is palette RAM index 0; palette[0] holds a
                        // NES colour index, not a palette RAM index.
                        self.palette_rgb[0]
                    } else {
                        self.palette_rgb[usize::from(color_index & 0x1F)]
                    };
                }
                self.span_len = 8;
            } else {
                self.span_len = 0;
            }
        }
        let x = dot - 1;
        if self.span_len > 0 {
            let [r, g, b] = self.span_px[(x & 7) as usize];
            let i = (usize::from(self.scanline) * 256 + usize::from(x)) * 4;
            self.frame[i] = r;
            self.frame[i + 1] = g;
            self.frame[i + 2] = b;
            self.frame[i + 3] = 255;
            self.span_len -= 1;
        } else {
            self.render_pixel_dot(x);
        }
    }

    /// Composes a pixel on a line whose rendering is disabled: the output is
    /// the backdrop colour, but the sprite X counters keep running.
    fn render_backdrop_dot(&mut self, x: u16) {
        let mut slots = self.spr_active;
        while slots != 0 {
            let slot = slots.trailing_zeros() as usize;
            slots &= slots - 1;
            if self.spr_x[slot] == 0 {
                self.spr_counting[slot] = false;
            }
            if self.spr_counting[slot] {
                self.spr_x[slot] = self.spr_x[slot].saturating_sub(1);
            }
        }
        let [r, g, b] = self.palette_rgb[0];
        let i = (usize::from(self.scanline) * 256 + usize::from(x)) * 4;
        self.frame[i] = r;
        self.frame[i + 1] = g;
        self.frame[i + 2] = b;
        self.frame[i + 3] = 255;
    }

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
        let mut picked_bit = 0u8;
        let mut picked_attr = 0u8;
        let mut picked = false;
        let sprite_mask_on = self.mask & 0x10 != 0 && !(x < 8 && self.mask & 0x04 == 0);
        let mut slots = self.spr_active;
        while slots != 0 {
            let slot = slots.trailing_zeros() as usize;
            slots &= slots - 1;
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
            if !sprite_mask_on || sbit == 0 {
                continue;
            }
            if !picked {
                picked = true;
                picked_bit = sbit;
                picked_attr = self.spr_attr[slot];
                if self.spr_id[slot] == 0 && bg != 0 && x != 255 {
                    self.status |= STATUS_SPRITE0;
                }
            }
        }
        if picked {
            let behind = picked_attr & 0x20 != 0;
            if bg == 0 || !behind {
                color_index = 0x10 + (picked_attr & 0x03) * 4 + picked_bit;
            }
        }
        if color_index == 0 {
            // Backdrop colour (palette RAM index 0).
            color_index = 0;
        }
        let [r, g, b] = self.palette_rgb[usize::from(color_index & 0x1F)];
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
        if self.um6578 {
            return self.render_um_frame();
        }
        self.frame.clone()
    }

    /// Renders any deferred video mode into the reusable frame buffer.
    /// Normal 2C02 output is already composed dot by dot, so this is a no-op.
    pub fn present(&mut self) {
        if self.um6578 {
            self.frame = self.render_um_frame();
        }
    }

    /// The most recently presented RGBA8 frame.
    #[must_use]
    pub fn video(&self) -> &[u8] {
        &self.frame
    }

    /// Renders one frame of the UM6578 enhanced video mode: word name tables,
    /// optional 4-bits-per-pixel background and the 64-entry palette.
    fn render_um_frame(&self) -> Vec<u8> {
        let mut frame = vec![0u8; 256 * 240 * 4];
        let backdrop = |index: usize| {
            palette_colour(
                self.palette_variant,
                NES_PALETTE[usize::from(self.um_palette[index & 0x3F]) & 0x3F],
            )
        };
        // Palette entries whose low five bits are all ones are transparent;
        // the SH6578 draws the universal backdrop instead (MAME's rule).
        let transparent = |index: usize| self.um_palette[index & 0x3F] & 0x1F == 0x1F;
        if self.mask & 0x08 == 0 {
            let [r, g, b] = backdrop(0);
            for pixel in frame.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[r, g, b, 255]);
            }
            return frame;
        }
        let bg4 = self.um_ctrl & 0x80 != 0;
        let single = self.um_ctrl & 0x01 != 0;
        let scroll = self.t;
        let scroll_x = usize::from(scroll & 0x1F);
        let scroll_y = usize::from((scroll >> 5) & 0x1F);
        let fine_y = usize::from((scroll >> 12) & 0x07);
        let base = usize::from((scroll & 0x0C00) << 1);
        let sprite_base = (usize::from(self.um_ctrl >> 2) & 0x03) << 12;
        for y in 0..240usize {
            let world_y = y + fine_y;
            let mut line_base = base;
            if (scroll_y + world_y / 8) & 32 != 0 {
                line_base ^= 0x1000;
            }
            let tile_y = (scroll_y + world_y / 8) & 31;
            let row = world_y & 7;
            let mut sprite_colour = [0u8; 256];
            let mut sprite_front = [false; 256];
            let mut drawn = 0usize;
            for index in 0..64usize {
                if drawn == 8 {
                    break;
                }
                let top = usize::from(self.oam[index * 4]) + 1;
                if y < top || y >= top + 8 {
                    continue;
                }
                drawn += 1;
                let attr = self.oam[index * 4 + 2];
                let sprite_row = if attr & 0x80 != 0 {
                    7 - (y - top)
                } else {
                    y - top
                };
                let tile = usize::from(self.oam[index * 4 + 1]);
                let address = (sprite_base + tile * 16 + sprite_row) & 0x3FFF;
                // Sprite patterns keep the 2C02 two-plane layout.
                let low = self.um_vram.get(address).copied().unwrap_or(0);
                let high = self.um_vram.get(address + 8).copied().unwrap_or(0);
                let start = usize::from(self.oam[index * 4 + 3]);
                let front = attr & 0x20 == 0;
                let palette_base = 0x10 + usize::from(attr & 0x03) * 4;
                for column in 0..8usize {
                    let screen_x = start + column;
                    if screen_x >= 256 {
                        break;
                    }
                    let source = if attr & 0x40 != 0 { 7 - column } else { column };
                    let shift = 7 - source;
                    let bit = u16::from((low >> shift) & 1) | (u16::from((high >> shift) & 1) << 1);
                    if bit == 0 {
                        continue;
                    }
                    if sprite_colour[screen_x] == 0 || (front && !sprite_front[screen_x]) {
                        sprite_colour[screen_x] = (palette_base + usize::from(bit)) as u8;
                        sprite_front[screen_x] = front;
                    }
                }
            }
            for x in 0..256usize {
                let world_x = x + usize::from(self.fine_x);
                let tile_column = scroll_x + world_x / 8;
                let mut index = line_base + (tile_column & 31) * 2;
                if tile_column & 32 != 0 {
                    index ^= 0x800;
                }
                index += tile_y * 64;
                if single {
                    index = (index & 0x7FF) + 0x2000;
                } else {
                    index &= 0x1FFF;
                }
                let low = self.um_vram.get(index).copied().unwrap_or(0);
                let high = self.um_vram.get(index + 1).copied().unwrap_or(0);
                let tile = usize::from(low) | (usize::from(high & 0x0F) << 8);
                let attribute = usize::from(high >> 4);
                let shift = 7 - (world_x & 7);
                let (background, background_opaque) = if bg4 {
                    // VT03 layout: four 8-byte planes, one bit per pixel.
                    let pattern = (tile & !1) << 4;
                    let mut value = 0usize;
                    for plane in 0..4 {
                        let byte = self
                            .um_vram
                            .get(pattern + plane * 8 + row)
                            .copied()
                            .unwrap_or(0);
                        value |= usize::from((byte >> shift) & 1) << plane;
                    }
                    ((attribute & 0x03) * 16 + value, value != 0)
                } else {
                    let pattern = tile << 4;
                    let low = self.um_vram.get(pattern + row).copied().unwrap_or(0);
                    let high = self.um_vram.get(pattern + row + 8).copied().unwrap_or(0);
                    let value =
                        usize::from((low >> shift) & 1) | (usize::from((high >> shift) & 1) << 1);
                    ((attribute & 0x0F) * 4 + value, value != 0)
                };
                let index = if sprite_colour[x] != 0 && (sprite_front[x] || !background_opaque) {
                    usize::from(sprite_colour[x])
                } else {
                    background
                };
                let index = if transparent(index) { 0 } else { index };
                let [r, g, b] = backdrop(index);
                let offset = (y * 256 + x) * 4;
                frame[offset] = r;
                frame[offset + 1] = g;
                frame[offset + 2] = b;
                frame[offset + 3] = 255;
            }
        }
        frame
    }
}

/// Display palette variant, serialized with the machine state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PaletteVariant {
    /// Canonical 2C02 table (oracle parity).
    #[default]
    Canonical,
    /// Warmer, slightly softer consumer-TV look.
    Consumer,
    /// Saturated, gamma-corrected RGB-monitor look.
    Rgb,
    /// Luminance-only output.
    Monochrome,
    /// User-supplied 64-entry table.
    Custom,
    /// Generated high-saturation preset.
    Vivid,
    /// Generated low-saturation, warm preset.
    Soft,
    /// Generated PAL-like hue/level preset.
    Pal,
    /// Generated neutral studio-monitor preset.
    Studio,
}

/// Applies a display palette preset to a canonical 2C02 colour.
fn palette_colour(variant: PaletteVariant, base: [u8; 3]) -> [u8; 3] {
    let r = f32::from(base[0]);
    let g = f32::from(base[1]);
    let b = f32::from(base[2]);
    let clamp = |value: f32| value.clamp(0.0, 255.0) as u8;
    match variant {
        PaletteVariant::Canonical => base,
        PaletteVariant::Consumer => {
            // Slightly gamma-lifted, warm white point and softer blue.
            let lift = |value: f32| 255.0 * (value / 255.0).powf(0.92);
            [clamp(lift(r) * 1.06), clamp(lift(g)), clamp(lift(b) * 0.92)]
        }
        PaletteVariant::Rgb => {
            // Saturation boost around Rec.601 luma, then gamma-corrected.
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let saturate = |value: f32| {
                let value = luma + (value - luma) * 1.35;
                255.0 * (value / 255.0).powf(0.9)
            };
            [clamp(saturate(r)), clamp(saturate(g)), clamp(saturate(b))]
        }
        PaletteVariant::Monochrome => {
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            [clamp(luma); 3]
        }
        PaletteVariant::Custom
        | PaletteVariant::Vivid
        | PaletteVariant::Soft
        | PaletteVariant::Pal
        | PaletteVariant::Studio => base,
    }
}

/// Canonical 2C02 RGB palette. Kept identical to the reference emulator so a
/// trace can be compared byte-for-byte against its frames.
const NES_PALETTE: [[u8; 3]; 64] = [
    [0x6A, 0x6D, 0x6A],
    [0x00, 0x13, 0x80],
    [0x1E, 0x00, 0x8A],
    [0x39, 0x00, 0x7A],
    [0x55, 0x00, 0x56],
    [0x5A, 0x00, 0x18],
    [0x4F, 0x10, 0x00],
    [0x3D, 0x1C, 0x00],
    [0x25, 0x3A, 0x00],
    [0x00, 0x4E, 0x00],
    [0x00, 0x46, 0x00],
    [0x00, 0x4A, 0x18],
    [0x00, 0x40, 0x5A],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xB9, 0xBC, 0xB9],
    [0x18, 0x4B, 0xCF],
    [0x4B, 0x24, 0xE8],
    [0x7C, 0x12, 0xE0],
    [0xAB, 0x13, 0xB5],
    [0xB7, 0x21, 0x64],
    [0xAB, 0x37, 0x18],
    [0x8B, 0x5A, 0x00],
    [0x5C, 0x7A, 0x00],
    [0x20, 0x90, 0x00],
    [0x00, 0x8F, 0x00],
    [0x00, 0x8C, 0x42],
    [0x00, 0x7D, 0x8A],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF],
    [0x64, 0xA0, 0xFF],
    [0x84, 0x79, 0xFF],
    [0xAC, 0x68, 0xFF],
    [0xDA, 0x60, 0xFF],
    [0xE2, 0x6B, 0xC5],
    [0xDC, 0x83, 0x4C],
    [0xC3, 0x9A, 0x18],
    [0x9C, 0xB0, 0x00],
    [0x60, 0xC0, 0x00],
    [0x30, 0xC8, 0x3C],
    [0x28, 0xC5, 0x8C],
    [0x3C, 0xB7, 0xC9],
    [0x4C, 0x4C, 0x4C],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF],
    [0xC8, 0xDD, 0xFF],
    [0xD5, 0xCC, 0xFF],
    [0xE5, 0xC7, 0xFF],
    [0xF5, 0xC5, 0xFF],
    [0xFA, 0xC9, 0xE6],
    [0xF8, 0xD2, 0xBD],
    [0xEF, 0xDA, 0x99],
    [0xE1, 0xE1, 0x88],
    [0xC8, 0xE7, 0x88],
    [0xB0, 0xEA, 0x9C],
    [0xA4, 0xEB, 0xBD],
    [0xAA, 0xE5, 0xE2],
    [0xB0, 0xB0, 0xB0],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
];

/// Documented 2C02 composite levels `[attenuated][low, high][row]` (volts).
const EMPH_LEVELS: [[[f64; 4]; 2]; 2] = [
    [[0.228, 0.312, 0.552, 0.880], [0.616, 0.840, 1.100, 1.100]],
    [[0.192, 0.256, 0.448, 0.712], [0.500, 0.676, 0.896, 0.896]],
];
const EMPH_BLACK: f64 = 0.312;
const EMPH_WHITE: f64 = 1.100;

fn emph_phase_high(hue: u8, phase: u8) -> bool {
    let k = (phase + hue) % 12;
    (1..=6).contains(&k)
}

fn emph_level(colour: u8, emph: u8, phase: u8) -> f64 {
    let hue = colour & 0x0F;
    let row = usize::from((colour >> 4) & 0x03);
    if hue >= 0x0E {
        return EMPH_LEVELS[0][0][1];
    }
    let attenuated = (emph & 1 != 0 && emph_phase_high(0x0C, phase))
        || (emph & 2 != 0 && emph_phase_high(0x04, phase))
        || (emph & 4 != 0 && emph_phase_high(0x08, phase));
    let set = &EMPH_LEVELS[usize::from(attenuated)];
    let (low, high) = (set[0][row], set[1][row]);
    match hue {
        0x00 => high,
        0x0D => low,
        _ if emph_phase_high(hue, phase) => high,
        _ => low,
    }
}

fn emph_decode(colour: u8, emph: u8) -> [f64; 3] {
    let (mut y, mut u, mut v) = (0.0, 0.0, 0.0);
    for phase in 0u8..12 {
        let sample = (emph_level(colour, emph, phase) - EMPH_BLACK) / (EMPH_WHITE - EMPH_BLACK);
        let angle = std::f64::consts::PI * (f64::from(phase) + 0.5 - 8.0) / 6.0;
        y += sample;
        u -= sample * angle.cos();
        v += sample * angle.sin();
    }
    let (y, u, v) = (y / 12.0, 2.0 * u / 12.0, 2.0 * v / 12.0);
    [
        y + 1.139_883 * v,
        y - 0.394_642 * u - 0.580_622 * v,
        y + 2.032_062 * u,
    ]
}

/// Change emphasis makes to a base colour, in 8-bit units (rounded).
fn emph_delta(colour: u8, emph: u8) -> [i16; 3] {
    if emph == 0 || colour & 0x0F >= 0x0E {
        return [0, 0, 0];
    }
    let on = emph_decode(colour, emph);
    let off = emph_decode(colour, 0);
    let delta = |c: usize| (255.0 * (on[c] - off[c])).round() as i16;
    [delta(0), delta(1), delta(2)]
}

fn emph_apply(base: u8, delta: i16) -> u8 {
    (i16::from(base) + delta).clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu() -> Ppu {
        let mut ppu = Ppu::new(Timing::NTSC, vec![0; 0x2000], true, false);
        ppu.end_warmup();
        ppu
    }

    #[test]
    fn palette_presets_transform_the_canonical_table() {
        let base = [0x6A, 0x6D, 0x6A];
        assert_eq!(palette_colour(PaletteVariant::Canonical, base), base);
        assert_eq!(palette_colour(PaletteVariant::Rgb, [0, 0, 0]), [0, 0, 0]);
        let mono = palette_colour(PaletteVariant::Monochrome, [200, 40, 10]);
        assert_eq!(mono[0], mono[1]);
        assert_eq!(mono[1], mono[2]);
        let warm = palette_colour(PaletteVariant::Consumer, [120, 120, 120]);
        assert!(warm[0] > warm[2], "consumer palette is warm");
        let saturated = palette_colour(PaletteVariant::Rgb, [200, 40, 10]);
        assert!(saturated[0] > 200, "RGB palette boosts saturation");
    }

    #[test]
    fn generated_palettes_change_the_display_table() {
        let mut ppu = Ppu::new(Timing::NTSC, vec![0; 0x2000], false, false);
        let canonical = ppu.debug_palette_rgb();
        ppu.set_palette_variant(PaletteVariant::Vivid);
        let vivid = ppu.debug_palette_rgb();
        assert_ne!(canonical, vivid);
        ppu.set_palette_variant(PaletteVariant::Studio);
        let studio = ppu.debug_palette_rgb();
        assert_ne!(vivid, studio);
        ppu.set_palette_variant(PaletteVariant::Canonical);
        assert_eq!(ppu.debug_palette_rgb(), canonical, "canonical is restored");
    }

    #[test]
    fn custom_palette_table_is_used_by_the_custom_variant() {
        let mut ppu = Ppu::new(Timing::NTSC, vec![0; 0x2000], false, false);
        let mut table = [[0u8; 3]; 64];
        table[0] = [1, 2, 3];
        table[0x0F] = [4, 5, 6];
        ppu.set_palette_table(table);
        ppu.set_palette_variant(PaletteVariant::Custom);
        assert_eq!(ppu.debug_palette_rgb()[0], [1, 2, 3]);
        // Palette RAM entry 0 maps to NES colour 0 at power-on.
        ppu.palette[0] = 0x0F;
        ppu.refresh_palette_rgb();
        assert_eq!(ppu.debug_palette_rgb()[0], [4, 5, 6]);
    }

    #[test]
    fn um6578_renders_4bpp_background_and_sprites() {
        let mut ppu = ppu();
        ppu.set_um6578(true);
        ppu.mask = 0x1E;
        ppu.ctrl = 0x80;
        // 4bpp background, single nametable at $2000, sprites at $0000.
        ppu.um_ctrl = 0x81;
        ppu.t = 0;
        ppu.fine_x = 0;
        ppu.um_palette[0] = 0x0F;
        ppu.um_palette[16] = 0x0F;
        ppu.um_palette[17] = 0x30;
        ppu.um_palette[21] = 0x16;
        // One even 4bpp tile (tile $010), attribute palette 1.
        ppu.um_vram[0x2000] = 0x10;
        ppu.um_vram[0x2001] = 0x10;
        ppu.um_vram[0x0100] = 0x80;
        ppu.um_vram[0x0101] = 0x80;
        // Sprite 0: tile 5, palette 1, front, at x=4/y=1; its row 0 has
        // one pixel in plane 0.
        ppu.oam[0] = 0;
        ppu.oam[1] = 5;
        ppu.oam[2] = 0x01;
        ppu.oam[3] = 4;
        ppu.um_vram[0x0050] = 0x80;
        let frame = ppu.render_um_frame();
        let pixel = |x: usize, y: usize| {
            let offset = (y * 256 + x) * 4;
            [frame[offset], frame[offset + 1], frame[offset + 2]]
        };
        assert_eq!(pixel(0, 0), NES_PALETTE[0x30]);
        assert_eq!(pixel(4, 1), NES_PALETTE[0x16]);

        // A behind-background sprite yields to an opaque background pixel
        // but still draws where the background is transparent.
        ppu.oam[2] = 0x21;
        ppu.oam[3] = 0;
        let frame = ppu.render_um_frame();
        let offset = 256 * 4;
        assert_eq!(
            &frame[offset..offset + 3],
            &NES_PALETTE[0x30],
            "a behind-background sprite yields to an opaque background pixel"
        );
        // Move the sprite over a transparent background pixel: it draws.
        ppu.oam[3] = 4;
        let frame = ppu.render_um_frame();
        let offset = (256 + 4) * 4;
        assert_eq!(&frame[offset..offset + 3], &NES_PALETTE[0x16]);
    }

    #[test]
    fn warmup_masks_control_and_scroll_but_not_address() {
        let mut ppu = Ppu::new(Timing::NTSC, vec![0; 0x2000], true, false);
        assert!(ppu.warmup_dots > 0);
        ppu.write_register(0x2000, 0x80);
        ppu.write_register(0x2001, 0x1E);
        assert_eq!(ppu.ctrl, 0);
        assert_eq!(ppu.mask, 0);
        ppu.write_register(0x2005, 0x12);
        assert!(!ppu.write_latch, "the $2005 latch must not toggle");
        // $2006 is not masked: games that probe CHR early rely on it.
        ppu.write_register(0x2006, 0x20);
        ppu.write_register(0x2006, 0x40);
        assert_eq!(ppu.v, 0x2040);
        ppu.end_warmup();
        ppu.write_register(0x2000, 0x80);
        assert_eq!(ppu.ctrl, 0x80);
    }

    #[test]
    fn palette_read_loads_the_buffer_from_the_nametable_underneath() {
        let mut ppu = ppu();
        ppu.mask = 0;
        ppu.scanline = 0;
        ppu.dot = 0;
        // Put $5A in the nametable byte under palette $3F00.
        ppu.v = 0x2F00;
        ppu.write_register(7, 0x5A);
        // A palette read returns the palette entry but buffers $2F00.
        ppu.v = 0x3F00;
        let palette = ppu.read_register(7);
        assert_eq!(palette & 0x3F, 0);
        for _ in 0..4 {
            ppu.step_dot();
        }
        assert_eq!(ppu.read_buffer, 0x5A);
        // The next normal read returns the buffered nametable byte.
        ppu.v = 0x0000;
        assert_eq!(ppu.read_register(7), 0x5A);
    }

    #[test]
    fn ppu_data_read_uses_the_full_14_bit_address() {
        let mut ppu = ppu();
        ppu.chr[0x1EC0] = 0x5A;
        ppu.chr[0x0EC0] = 0x11;
        ppu.v = 0x1EC0;
        ppu.mask = 0;
        ppu.read_register(7);
        for _ in 0..4 {
            ppu.step_dot();
        }
        assert_eq!(ppu.read_buffer, 0x5A);
    }

    #[test]
    fn eight_by_eight_sprites_use_ppuctrl_bit3() {
        let mut ppu = ppu();
        ppu.chr[0x1000] = 0xAA;
        ppu.chr[0x0000] = 0x55;
        ppu.ctrl = 0x08; // sprite pattern table 1, background table 0
        ppu.scanline = 0;
        ppu.secondary_oam[0..4].copy_from_slice(&[0, 0, 0, 0]);
        ppu.oam_id[0] = 0;
        ppu.latch_sprite(0);
        assert_eq!(ppu.spr_lo[0], 0xAA);
    }

    #[test]
    fn single_and_four_screen_mirroring() {
        let mut ppu = ppu();
        ppu.mirroring = Mirroring::SingleLower;
        ppu.write_vram(0x2000, 0x11);
        assert_eq!(ppu.read_vram(0x2400), 0x11);
        assert_eq!(ppu.read_vram(0x2C00), 0x11);

        ppu.mirroring = Mirroring::SingleUpper;
        ppu.write_vram(0x2800, 0x22);
        // Single-upper maps every nametable address to the second 1 KiB.
        assert_eq!(ppu.read_vram(0x2000), 0x22);
        assert_eq!(ppu.read_vram(0x2C00), 0x22);

        ppu.mirroring = Mirroring::FourScreen;
        for (index, address) in [0x2000u16, 0x2400, 0x2800, 0x2C00].into_iter().enumerate() {
            ppu.write_vram(address, index as u8);
        }
        for (index, address) in [0x2000u16, 0x2400, 0x2800, 0x2C00].into_iter().enumerate() {
            assert_eq!(ppu.read_vram(address), index as u8);
        }
    }

    #[test]
    fn rom_nametables_read_chr_banks() {
        let mut ppu = ppu();
        let mut rom = vec![0u8; 0x8000]; // 32 x 1 KiB banks.
        for (bank, chunk) in rom.chunks_mut(0x400).enumerate() {
            chunk.fill(0x80 | bank as u8);
        }
        ppu.set_rom_nametable_rom(rom);
        ppu.mirroring = Mirroring::Vertical;
        ppu.set_rom_nametable_banks(Some([5, 9]));
        assert_eq!(ppu.read_vram(0x2000), 0x85);
        assert_eq!(ppu.read_vram(0x2400), 0x89);
        assert_eq!(ppu.read_vram(0x2800), 0x85);
        assert_eq!(ppu.read_vram(0x2C00), 0x89);
        assert_eq!(ppu.read_vram(0x23FF), 0x85);

        // ROM nametables are read-only.
        ppu.write_vram(0x2000, 0xFF);
        assert_eq!(ppu.read_vram(0x2000), 0x85);

        // Horizontal mirroring selects the two banks by half.
        ppu.mirroring = Mirroring::Horizontal;
        assert_eq!(ppu.read_vram(0x2400), 0x85);
        assert_eq!(ppu.read_vram(0x2800), 0x89);

        // Disabling returns to CIRAM.
        ppu.set_rom_nametable_banks(None);
        ppu.write_vram(0x2000, 0x42);
        assert_eq!(ppu.read_vram(0x2000), 0x42);
    }

    #[test]
    fn backdrop_uses_palette_index_zero_not_double_indirection() {
        let mut ppu = ppu();
        // Backdrop $01; palette entry 1 is white ($20). A backdrop-only scanline
        // must be the $01 colour, not the colour of palette index 1.
        ppu.palette[0] = 0x01;
        ppu.palette[1] = 0x20;
        ppu.write_palette(0x3F00, 0x01);
        ppu.write_palette(0x3F01, 0x20);
        ppu.scanline = 0;
        ppu.render_backdrop_dot(0);
        assert_eq!(&ppu.frame[0..3], &NES_PALETTE[0x01]);
        assert_ne!(&ppu.frame[0..3], &NES_PALETTE[0x20]);
    }

    #[test]
    fn mmc2_chr_registers_wrap_to_the_rom_size() {
        let mut ppu = ppu();
        let mut rom = vec![0u8; 0x10000]; // 64 KiB = 16 x 4 KiB banks.
        for (bank, chunk) in rom.chunks_mut(0x1000).enumerate() {
            chunk.fill(bank as u8);
        }
        // 5-bit register values: bank 24 wraps to bank 8 of a 16-bank ROM.
        ppu.set_mmc2_chr(rom, [24, 24, 19, 19]);
        assert_eq!(ppu.chr_byte(0x0000), 8);
        assert_eq!(ppu.chr_byte(0x1000), 19 % 16);
    }

    #[test]
    fn mmc2_latch_switches_pattern_banks_on_fd_fe_fetches() {
        let mut ppu = ppu();
        let mut rom = vec![0u8; 0x10000]; // 64 KiB = 16 x 4 KiB banks.
        for (bank, chunk) in rom.chunks_mut(0x1000).enumerate() {
            chunk.fill(0x80 | bank as u8);
        }
        ppu.set_mmc2_chr(rom, [1, 2, 5, 6]);

        // Both pattern tables start on their $FD bank.
        assert_eq!(ppu.chr_byte(0x0000), 0x81);
        assert_eq!(ppu.chr_byte(0x1000), 0x85);

        // Tile $FD keeps latch 0 on its $FD bank; tile $FE switches it.
        ppu.note_chr_fetch(0x0FD0);
        assert_eq!(ppu.mmc2_latch(), Some([false, false]));
        ppu.note_chr_fetch(0x0FE0);
        assert_eq!(ppu.mmc2_latch(), Some([true, false]));
        assert_eq!(ppu.chr_byte(0x0000), 0x82);

        // Table 1 latches independently.
        ppu.note_chr_fetch(0x1FE0);
        assert_eq!(ppu.mmc2_latch(), Some([true, true]));
        assert_eq!(ppu.chr_byte(0x1000), 0x86);
        ppu.note_chr_fetch(0x1FD0);
        assert_eq!(ppu.chr_byte(0x1000), 0x85);

        // Bank-register updates keep the current latch selection.
        ppu.set_mmc2_banks([7, 8, 9, 10]);
        assert_eq!(ppu.mmc2_banks(), Some([7, 8, 9, 10]));
        // Latch 0 is still on $FE (bank 8), latch 1 back on $FD (bank 9).
        assert_eq!(ppu.chr_byte(0x0000), 0x88);
        assert_eq!(ppu.chr_byte(0x1000), 0x89);
    }

    #[test]
    fn ppu_data_read_while_rendering_uses_the_fetch_byte() {
        let mut ppu = ppu();
        ppu.mask = 0x18;
        ppu.scanline = 10;
        // Dot 0 runs no background fetch, so the completion sees `fetch_byte`
        // exactly as the test leaves it. VRAM holds a different value so the
        // test proves which source filled the buffer.
        ppu.dot = 0;
        ppu.v = 0x2000;
        ppu.write_vram(0x2000, 0x11);
        ppu.fetch_byte = 0x7E;
        ppu.fetch_hold = 0x04;
        // The read returns the old buffer immediately.
        assert_eq!(ppu.read_register(7), 0x00);
        // Complete the deferred fill on the next dot.
        ppu.data_read = 1;
        ppu.step_dot();
        assert_eq!(
            ppu.read_buffer, 0x7E,
            "buffer fills from the fetch pipeline"
        );
        // The increment is deferred and uses the rendering glitch (fine Y
        // increments), so $2000 becomes $3001 rather than $2001.
        assert_eq!(ppu.v, 0x3001);
    }

    #[test]
    fn odd_frames_skip_one_pre_render_dot_only_while_rendering() {
        fn steps_to_wrap(frames: u64, rendering: bool) -> u32 {
            let mut ppu = ppu();
            if rendering {
                ppu.mask = 0x18;
            }
            ppu.frames = frames;
            ppu.scanline = ppu.pre_render_line();
            ppu.dot = 338;
            let mut steps = 0;
            while ppu.frames == frames {
                ppu.step_dot();
                steps += 1;
                assert!(steps < 8, "frame wrap never happened");
            }
            steps
        }
        // Even frame with rendering: 338->339, 339->340, then the line wraps.
        assert_eq!(steps_to_wrap(0, true), 3);
        // Odd frame with rendering: dot 340 is skipped, so the wrap is a dot
        // earlier.
        assert_eq!(steps_to_wrap(1, true), 2);
        // No skip when rendering is disabled, whatever the frame parity.
        assert_eq!(steps_to_wrap(1, false), 3);
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
        ppu.scanline = ppu.pre_render_line();
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
