//! iNES cartridge loading and PRG/CHR mappers (A4).
//!
//! The cartridge owns PRG-ROM, CHR-ROM/RAM, PRG-RAM and the mapper state.
//! Mappers expose the CPU/PPU address windows so the system bus stays thin.
//!
//! Scope: iNES 1.0 headers plus the NES 2.0 mapper-number extension, NROM
//! (mapper 0) and MMC1 (mapper 1). Further mappers slot into [`Mapper`].

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Nametable mirroring requested by the cartridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    /// Two vertical nametables.
    Vertical,
    /// Two horizontal nametables.
    Horizontal,
    /// Four independent nametables.
    FourScreen,
    /// Single lower nametable.
    SingleLower,
    /// Single upper nametable.
    SingleUpper,
}

/// Shared MMC5 state the PPU needs: ExRAM, nametable mapping/fill and the
/// resolved 1 KiB CHR bank per PPU slot.
///
/// The mapper updates this on its register writes, so mid-frame CHR banking
/// and ExRAM-backed nametables take effect immediately.
#[derive(Debug)]
pub struct Mmc5Aux {
    exram: [u8; 1024],
    mode: u8,
    nt_map: [u8; 4],
    fill_tile: u8,
    fill_attr: u8,
    chr_upper: u8,
    rom: Vec<u8>,
    ram: bool,
    slots: [usize; 8],
    bg_slots: [usize; 8],
    split_enabled: bool,
    split_right: bool,
    split_threshold: u8,
    split_scroll: u8,
    split_bank: u8,
}

impl Mmc5Aux {
    fn new(rom: Vec<u8>, ram: bool) -> Self {
        Self {
            exram: [0; 1024],
            mode: 3,
            nt_map: [0; 4],
            fill_tile: 0,
            fill_attr: 0,
            chr_upper: 0,
            rom,
            ram,
            slots: [0, 1, 2, 3, 4, 5, 6, 7],
            bg_slots: [0, 1, 2, 3, 4, 5, 6, 7],
            split_enabled: false,
            split_right: false,
            split_threshold: 0,
            split_scroll: 0,
            split_bank: 0,
        }
    }

    fn set_registers(
        &mut self,
        mode: u8,
        nt_map: [u8; 4],
        fill_tile: u8,
        fill_attr: u8,
        chr_upper: u8,
    ) {
        self.mode = mode;
        self.nt_map = nt_map;
        self.fill_tile = fill_tile;
        self.fill_attr = fill_attr;
        self.chr_upper = chr_upper;
    }

    fn set_slots(&mut self, slots: [usize; 8]) {
        self.slots = slots;
    }

    fn set_bg_slots(&mut self, slots: [usize; 8]) {
        self.bg_slots = slots;
    }

    fn set_split(&mut self, enabled: bool, right: bool, threshold: u8, scroll: u8, bank: u8) {
        self.split_enabled = enabled;
        self.split_right = right;
        self.split_threshold = threshold;
        self.split_scroll = scroll;
        self.split_bank = bank;
    }

    /// Split-screen column for a background fetch-group index, if the group
    /// belongs to the split region.
    ///
    /// Left splits are left-aligned (column = group); right splits are
    /// right-aligned over the 34 fetch groups, so column = group - 2.
    fn split_column(&self, group: u16) -> Option<u16> {
        if !self.split_enabled || self.mode > 1 {
            return None;
        }
        let threshold = u16::from(self.split_threshold);
        if self.split_right {
            (group >= threshold).then(|| group.wrapping_sub(2) & 0x1F)
        } else {
            (group < threshold).then_some(group & 0x1F)
        }
    }

    /// (row, column, fine Y) inside the split ExRAM nametable.
    fn split_position(&self, group: u16, scanline: u16) -> Option<(usize, u16, u8)> {
        let column = self.split_column(group)?;
        // The pre-render line prefetches the first two tiles of line 0.
        let y = if scanline >= 240 { 0 } else { scanline };
        let total = u16::from(self.split_scroll) + y;
        let row = usize::from((total >> 3) % 30);
        let fine_y = (total & 7) as u8;
        Some((row, column, fine_y))
    }

    /// Nametable tile byte from the split ExRAM nametable.
    pub(crate) fn split_tile(&self, group: u16, scanline: u16) -> Option<u8> {
        let (row, column, _) = self.split_position(group, scanline)?;
        Some(self.exram[row * 32 + usize::from(column)])
    }

    /// Attribute byte from the split ExRAM nametable.
    pub(crate) fn split_attr(&self, group: u16, scanline: u16) -> Option<u8> {
        let (row, column, _) = self.split_position(group, scanline)?;
        let offset = 0x3C0 + (row >> 2) * 8 + usize::from(column) / 4;
        Some(self.exram[offset])
    }

    /// The 2-bit palette field the PPU should use for the split tile.
    pub(crate) fn split_attr_field(&self, group: u16, scanline: u16) -> Option<u8> {
        let (row, column, _) = self.split_position(group, scanline)?;
        let attr = self.split_attr(group, scanline)?;
        let shift = ((row & 2) << 1) | (usize::from(column) & 2);
        Some((attr >> shift) & 0x03)
    }

    /// CHR address for a split-region pattern fetch.
    ///
    /// $5202 supplies A12-A19, the tile index supplies A4-A11 (which carries
    /// the PPU's A10/A11), and the split fine Y supplies A0-A2.
    pub(crate) fn split_pattern(
        &self,
        group: u16,
        scanline: u16,
        tile: u8,
        high: bool,
    ) -> Option<usize> {
        let (_, _, fine_y) = self.split_position(group, scanline)?;
        let address = (usize::from(self.split_bank) << 12)
            | ((usize::from(tile) << 4) & 0x0FF8)
            | usize::from(fine_y);
        Some(address | (usize::from(high) * 8))
    }

    /// Extended-attribute byte for the tile at `address` in mode 1.
    pub(crate) fn extended_attr(&self, address: u16) -> Option<u8> {
        (self.mode == 1).then(|| self.exram[usize::from(address & 0x3FF)])
    }

    /// CHR address for an extended-attribute background tile: the ExRAM
    /// byte supplies the 4 KiB bank (A12-A17) and palette (bits 6-7), while
    /// $5130 supplies A18-A19.
    pub(crate) fn extended_pattern(&self, attr: u8, tile: u8, fine_y: u8) -> usize {
        // ExRAM bits 0-5 select the low six bits of the 4 KiB bank; $5130
        // supplies bank bits 6-7, so the physical offset can exceed 256 KiB.
        let bank = (usize::from(self.chr_upper & 0x03) << 6) | usize::from(attr & 0x3F);
        (bank << 12) | ((usize::from(tile) << 4) & 0x0FF8) | usize::from(fine_y & 0x07)
    }

    /// Extended-attribute palette (bits 6-7 of the ExRAM byte).
    pub(crate) fn extended_palette(attr: u8) -> u8 {
        attr >> 6
    }

    /// Nametable byte supplied by ExRAM or fill mode, if this slot uses one.
    ///
    /// ExRAM is a nametable only in modes 0/1; in modes 2/3 the mapped slot
    /// reads as all zeros (`$5104` table and the `$5105` note on the wiki).
    pub(crate) fn nametable_override(&self, address: u16) -> Option<u8> {
        let slot = usize::from(address >> 10 & 3);
        let offset = usize::from(address & 0x3FF);
        match self.nt_map[slot] {
            2 => Some(if self.mode <= 1 {
                self.exram[offset]
            } else {
                0
            }),
            3 => Some(if address & 0x03C0 == 0x03C0 {
                self.fill_attr
            } else {
                self.fill_tile
            }),
            _ => None,
        }
    }

    /// CIRAM index for this slot when `$5105` maps it to page 0 or page 1.
    ///
    /// Unlike the legacy vertical/horizontal mirroring, each MMC5 nametable
    /// slot selects its own 1 KiB CIRAM page.
    pub(crate) fn nametable_index(&self, address: u16) -> Option<usize> {
        let slot = usize::from(address >> 10 & 3);
        let offset = usize::from(address & 0x3FF);
        match self.nt_map[slot] {
            0 => Some(offset),
            1 => Some(0x400 + offset),
            _ => None,
        }
    }

    pub(crate) fn nametable_write(&mut self, address: u16, value: u8) -> bool {
        let slot = usize::from(address >> 10 & 3);
        match self.nt_map[slot] {
            2 => {
                // ExRAM is PPU-writable only while it is usable as a
                // nametable, i.e. in modes 0/1.
                if self.mode <= 1 {
                    self.exram[usize::from(address & 0x3FF)] = value;
                }
                true
            }
            3 => true,
            _ => false,
        }
    }

    pub(crate) fn chr_offset(&self, address: u16) -> usize {
        let slot = usize::from(address) / 0x400;
        self.slots[slot] * 0x400 + usize::from(address & 0x3FF)
    }

    pub(crate) fn chr_read(&self, address: u16, local: &[u8]) -> u8 {
        let offset = self.chr_offset(address);
        if self.ram {
            local.get(offset).copied().unwrap_or(0)
        } else {
            self.rom.get(offset).copied().unwrap_or(0)
        }
    }

    /// Background CHR read: in 8x16 sprite mode the MMC5 uses the
    /// `$5128-$512B` background banks instead of the sprite banks.
    pub(crate) fn bg_chr_read(&self, address: u16, local: &[u8]) -> u8 {
        let slot = usize::from(address) / 0x400;
        let offset = self.bg_slots[slot] * 0x400 + usize::from(address & 0x3FF);
        if self.ram {
            local.get(offset).copied().unwrap_or(0)
        } else {
            self.rom.get(offset).copied().unwrap_or(0)
        }
    }

    /// Read a physical CHR address (already banked by the split registers).
    pub(crate) fn chr_physical(&self, address: usize, local: &[u8]) -> u8 {
        if self.ram {
            local.get(address).copied().unwrap_or(0)
        } else {
            self.rom.get(address).copied().unwrap_or(0)
        }
    }

    pub(crate) fn is_ram(&self) -> bool {
        self.ram
    }
}

/// Which hybrid CHR board the shared state models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HybridKind {
    /// Mapper 165: MMC3 registers used as two latched 4 KiB pages; page 0 is
    /// the 4 KiB CHR-RAM page.
    Mmc2,
    /// Mapper 195: MMC3 1 KiB CHR banks with a CHR-RAM window selected by the
    /// bank number written through `$2007`.
    Waixing195,
}

/// Shared hybrid CHR state: full CHR ROM, 4 KiB CHR-RAM, the current MMC3
/// bank registers and the latch/CHRB selection.
///
/// The mapper updates the registers; the PPU owns reads, writes and (for 165)
/// the `$FD`/`$FE` fetch latches.
#[derive(Debug)]
pub struct HybridChr {
    kind: HybridKind,
    rom: Vec<u8>,
    ram: Vec<u8>,
    select: u8,
    bank_data: [u8; 8],
    latch: [bool; 2],
    chrb: u8,
}

impl HybridChr {
    fn new(kind: HybridKind, rom: Vec<u8>) -> Self {
        let ram_size = if kind == HybridKind::Mmc2 {
            0x1000
        } else {
            0x8000
        };
        Self {
            kind,
            rom,
            ram: vec![0; ram_size],
            select: 0,
            bank_data: [0; 8],
            latch: [false; 2],
            chrb: 0x80,
        }
    }

    fn update_regs(&mut self, select: u8, bank_data: [u8; 8]) {
        self.select = select;
        self.bank_data = bank_data;
    }

    fn mmc3_slot_bank(&self, slot: usize) -> usize {
        let mode = self.select & 0x80;
        let raw = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        raw & 0xFF
    }

    /// First bank and size (1 KiB units) of the CHR-RAM window selected by the
    /// CHRB value written through the PPU.
    fn ram_range(&self) -> Option<(usize, usize)> {
        match self.chrb {
            0x80 => Some((0x28, 4)),
            0x82 => Some((0x00, 4)),
            0x88 => Some((0x4C, 4)),
            0x8A => Some((0x64, 4)),
            0xC0 => Some((0x46, 2)),
            0xC2 => Some((0x7C, 2)),
            0xC8 => Some((0x0A, 2)),
            0xCA => None,
            _ => Some((0x28, 4)),
        }
    }

    fn is_chrb_value(bank: usize) -> bool {
        matches!(bank, 0x80 | 0x82 | 0x88 | 0x8A | 0xC0 | 0xC2 | 0xC8 | 0xCA)
    }

    /// Maps a PPU CHR address to `(is_ram, offset)`.
    fn map(&self, address: u16) -> (bool, usize) {
        match self.kind {
            HybridKind::Mmc2 => {
                let half = usize::from(address) / 0x1000;
                let reg = if half == 0 {
                    if self.latch[0] {
                        1
                    } else {
                        0
                    }
                } else if self.latch[1] {
                    4
                } else {
                    2
                };
                let page = usize::from(self.bank_data[reg]);
                if page == 0 {
                    (true, usize::from(address) & 0x0FFF)
                } else {
                    (false, page * 0x1000 + (usize::from(address) & 0x0FFF))
                }
            }
            HybridKind::Waixing195 => {
                let slot = usize::from(address) / 0x400;
                let bank = self.mmc3_slot_bank(slot);
                // Low banks are the writable CHR pages these games upload to;
                // the CHRB window adds the ROM banks remapped to RAM. Banks in
                // the upper half are the ROM side unless selected.
                if bank < 0x80 {
                    // The writable window is addressed raw, as the games
                    // expect when they upload tiles through $2007.
                    return (true, usize::from(address) & 0x1FFF);
                }
                if let Some((start, size)) = self.ram_range() {
                    if bank >= start && bank < start + size {
                        return (
                            true,
                            (bank * 0x400 + (usize::from(address) & 0x3FF)) % self.ram.len(),
                        );
                    }
                }
                (false, bank * 0x400 + (usize::from(address) & 0x3FF))
            }
        }
    }

    /// Reads one CHR byte through the hybrid map.
    #[must_use]
    pub fn read(&self, address: u16) -> u8 {
        let (is_ram, offset) = self.map(address);
        if is_ram {
            return self.ram[offset % self.ram.len()];
        }
        self.rom.get(offset).copied().unwrap_or(0)
    }

    /// Writes one CHR byte. CHR-ROM planes are read-only; the RAM window and
    /// the 195 CHRB latch are updated.
    pub fn write(&mut self, address: u16, value: u8) {
        if self.kind == HybridKind::Waixing195 {
            let slot = usize::from(address) / 0x400;
            let bank = self.mmc3_slot_bank(slot);
            if Self::is_chrb_value(bank) {
                self.chrb = bank as u8;
            }
        }
        let (is_ram, offset) = self.map(address);
        if is_ram {
            let len = self.ram.len();
            self.ram[offset % len] = value;
        }
    }

    /// Feeds a pattern fetch to the MMC2-style latches (mapper 165).
    pub fn note_fetch(&mut self, address: u16) {
        if self.kind != HybridKind::Mmc2 {
            return;
        }
        let tile = (address >> 4) as u8;
        if tile == 0xFD || tile == 0xFE {
            let half = usize::from(address & 0x1000 != 0);
            self.latch[half] = tile == 0xFE;
        }
    }

    /// Feeds a `$2007` CHR read; games can toggle the latch this way.
    pub fn note_read(&mut self, address: u16) {
        if address < 0x2000 {
            self.note_fetch(address);
        }
    }

    /// Current latch state (165), for tests.
    #[must_use]
    pub fn latch(&self) -> [bool; 2] {
        self.latch
    }

    /// Current CHRB value (195), for tests.
    #[must_use]
    pub fn chrb(&self) -> u8 {
        self.chrb
    }
}

/// A cartridge mapper.
pub trait Mapper: fmt::Debug {
    /// Reads a PRG address in `$4020..=$FFFF`.
    fn prg_read(&self, address: u16) -> u8;
    /// Reads an address in `$4020..=$5FFF` with side effects (FDS disk status).
    fn read_io(&mut self, address: u16) -> u8 {
        self.prg_read(address)
    }
    /// Writes a PRG address in `$4020..=$FFFF`.
    fn prg_write(&mut self, address: u16, value: u8);
    /// Reads a CHR address in `$0000..=$1FFF`.
    fn chr_read(&self, address: u16) -> u8;
    /// Writes a CHR address in `$0000..=$1FFF`.
    fn chr_write(&mut self, address: u16, value: u8);
    /// Current nametable mirroring.
    fn mirroring(&self) -> Mirroring;
    /// Mapper number.
    fn number(&self) -> u16;
    /// Battery-backed PRG-RAM contents, if the mapper exposes any.
    fn prg_ram(&self) -> &[u8] {
        &[]
    }

    /// Full CHR ROM for mappers whose pattern banks switch during rendering
    /// (MMC2/MMC4). The PPU owns the latch state in that case.
    fn chr_rom(&self) -> Option<&[u8]> {
        None
    }

    /// Current 4 KiB CHR bank indices as
    /// `[latch0 $FD, latch0 $FE, latch1 $FD, latch1 $FE]`.
    fn chr_banks(&self) -> Option<[u8; 4]> {
        None
    }

    /// Whether this mapper can drive nametables from CHR ROM.
    fn has_rom_nametables(&self) -> bool {
        false
    }

    /// Two 1 KiB CHR bank indices for ROM nametables, when enabled.
    fn rom_nametable_banks(&self) -> Option<[u16; 2]> {
        None
    }

    /// Shared hybrid CHR state for mappers that mix ROM and RAM pages.
    fn hybrid_chr(&self) -> Option<Rc<RefCell<HybridChr>>> {
        None
    }

    /// Cartridge-backed CPU window inside `$4020-$5FFF` (PRG-RAM boards).
    fn low_window(&self) -> Option<(u16, u16)> {
        None
    }

    /// Shared MMC5 state (ExRAM/nametables/CHR map) for the PPU.
    fn mmc5_aux(&self) -> Option<Rc<RefCell<Mmc5Aux>>> {
        None
    }

    /// Shared banked CHR-RAM state for the PPU (VRC7).
    fn vrc7_chr(&self) -> Option<Rc<RefCell<Vrc7Chr>>> {
        None
    }

    /// Inserts an FDS disk side (`Some`) or ejects (`None`). No-op elsewhere.
    fn insert_disk_side(&mut self, _side: Option<usize>) {}

    /// Number of sides in an FDS image (0 for non-FDS boards).
    fn disk_side_count(&self) -> usize {
        0
    }

    /// Currently inserted FDS side.
    fn current_disk_side(&self) -> usize {
        0
    }

    /// Raw `.fds` image with pending writes flushed (empty for non-FDS).
    fn disk_image(&mut self) -> Vec<u8> {
        Vec::new()
    }

    /// Notification of a fully decoded PPUCTRL write (MMC5 8x16 mode).
    fn set_ppu_ctrl(&mut self, _value: u8) {}

    /// Expansion-audio contribution for the current CPU cycle (0.0 for boards
    /// without expansion audio).
    fn audio_mix(&self) -> f32 {
        0.0
    }

    /// Mapper IRQ output (level).
    fn irq_line(&self) -> bool {
        false
    }

    /// Called once per rendered scanline (PPU A12 rise).
    fn clock_scanline(&mut self) {}

    /// Called once per CPU cycle (for mappers with CPU-cycle IRQ counters).
    fn clock_cpu(&mut self) {}

    /// Acknowledges a pending mapper IRQ (for mappers that clear on read).
    fn acknowledge_irq(&mut self) {}
}

/// A loaded CPU-visible cartridge image.
#[derive(Debug)]
pub struct Cartridge {
    chr_is_ram: bool,
    mapper: Box<dyn Mapper>,
    header: Header,
}

/// Parsed header fields retained after loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Mapper number (NES 2.0 high bits included).
    pub mapper: u16,
    /// NES 2.0 submapper.
    pub submapper: u8,
    /// PRG-ROM size in bytes.
    pub prg_size: usize,
    /// CHR-ROM size in bytes (0 means CHR-RAM).
    pub chr_size: usize,
    /// Battery-backed PRG-RAM.
    pub battery: bool,
    /// Four-screen wiring.
    pub four_screen: bool,
    /// NES 2.0 header.
    pub nes2: bool,
    /// PRG-RAM size in bytes.
    pub prg_ram_size: usize,
    /// Initial nametable mirroring from the header.
    pub mirroring: Mirroring,
}

impl Cartridge {
    /// Builds a cartridge from a raw FDS disk image and its 8 KiB BIOS.
    pub fn from_fds(disk: &[u8], bios: &[u8]) -> Result<Self, CartError> {
        if bios.len() < 0x2000 {
            return Err(CartError::Truncated {
                expected: 0x2000,
                actual: bios.len(),
            });
        }
        Ok(Self {
            chr_is_ram: true,
            mapper: Box::new(Fds::new(disk, bios)),
            header: Header {
                mapper: 20,
                submapper: 0,
                prg_size: 0,
                chr_size: 0,
                battery: false,
                four_screen: false,
                nes2: false,
                prg_ram_size: 0,
                mirroring: Mirroring::Horizontal,
            },
        })
    }

    /// Parses an iNES / NES 2.0 image.
    pub fn from_ines(bytes: &[u8]) -> Result<Self, CartError> {
        let header = parse_header(bytes)?;
        let mut offset = 16;
        if bytes[6] & 0x04 != 0 {
            // Trainer, 512 bytes.
            offset += 512;
        }
        let prg_end = offset + header.prg_size;
        let chr_end = prg_end + header.chr_size;
        if bytes.len() < chr_end {
            return Err(CartError::Truncated {
                expected: chr_end,
                actual: bytes.len(),
            });
        }
        let prg_rom = bytes[offset..prg_end].to_vec();
        let chr = bytes[prg_end..chr_end].to_vec();
        let chr_is_ram = header.chr_size == 0;
        let chr = if chr_is_ram {
            let size = match header.mapper {
                13 => 0x4000,
                28 => 0x8000,
                _ => 0x2000,
            };
            // Start the writable window from the ROM image where one exists
            // (hybrid mappers 165/195); plain CHR-RAM carts have none.
            let mut ram = vec![0; size];
            let keep = chr.len().min(size);
            ram[..keep].copy_from_slice(&chr[..keep]);
            ram
        } else {
            chr
        };
        let prg_ram = vec![0; header.prg_ram_size.max(0x2000)];
        let mapper: Box<dyn Mapper> = match header.mapper {
            0 => Box::new(Nrom::new(prg_rom, chr, header)),
            1 => Box::new(Mmc1::new(prg_rom, chr, prg_ram, header)),
            2 => Box::new(UxRom::new(prg_rom, chr, header)),
            3 => Box::new(CnRom::new(prg_rom, chr, header)),
            4 => Box::new(Mmc3::new(prg_rom, chr, header)),
            9 => Box::new(Mmc2::new(prg_rom, chr, prg_ram, header, 9)),
            10 => Box::new(Mmc2::new(prg_rom, chr, prg_ram, header, 10)),
            68 => Box::new(Sunsoft4::new(prg_rom, chr, prg_ram, header)),
            136 => Box::new(Sachen136::new(prg_rom, chr, header)),
            156 => Box::new(Daou156::new(prg_rom, chr, header)),
            199 => Box::new(Mmc3_199::new(prg_rom, chr, header)),
            253 => Box::new(Waixing253::new(prg_rom, chr, header)),
            249 => Box::new(Mmc3_249::new(prg_rom, chr, header)),
            234 => Box::new(Maxi15::new(prg_rom, chr, header)),
            164 => Box::new(Waixing164::new(prg_rom, chr, header)),
            222 => Box::new(Ctc222::new(prg_rom, chr, header)),
            193 => Box::new(NtdTc112::new(prg_rom, chr, header)),
            172 => Box::new(Sachen136::new_172(prg_rom, chr, header)),
            197 => Box::new(Mmc3_197::new(prg_rom, chr, header)),
            195 => Box::new(Waixing195::new(prg_rom, chr, header)),
            165 => Box::new(Mmc3::new_hybrid(
                prg_rom,
                chr,
                header,
                165,
                HybridKind::Mmc2,
            )),
            45 => Box::new(Mmc3::new_as(prg_rom, chr, header, 45)),
            116 => Box::new(Mmc3::new_as(prg_rom, chr, header, 116)),
            118 => Box::new(Mmc3::new_txsrom(prg_rom, chr, header)),
            196 => Box::new(Mmc3::new_196(prg_rom, chr, header)),
            5 => Box::new(Mmc5::new(prg_rom, chr, header)),
            11 => Box::new(ColorDreams::new(prg_rom, chr, header)),
            13 => Box::new(Cprom::new(prg_rom, chr, header)),
            30 => Box::new(Unrom512::new(prg_rom, chr, header)),
            18 => Box::new(Ss88006::new(prg_rom, chr, header)),
            19 => Box::new(Namco163::new(prg_rom, chr, prg_ram, header)),
            28 => Box::new(Action53::new(prg_rom, chr, header)),
            35 | 90 | 209 | 211 => Box::new(JyAsic::new(prg_rom, chr, prg_ram, header)),
            69 => Box::new(Fme7::new(prg_rom, chr, header)),
            142 => Box::new(Ks7032::new(prg_rom, chr, header)),
            143 => Box::new(Prot143::new(prg_rom, chr, header)),
            148 => Box::new(Nina148::new(prg_rom, chr, header)),
            150 => Box::new(Sachen150::new(prg_rom, chr, header)),
            152 => Box::new(Bus152::new(prg_rom, chr, header)),
            226 => Box::new(Multicart226::new(prg_rom, chr, header)),
            240 => Box::new(Gnrom240::new(prg_rom, chr, prg_ram, header)),
            178 => Box::new(Fs305::new(prg_rom, chr, prg_ram, header)),
            16 => Box::new(BandaiFcg::new(prg_rom, chr, prg_ram, header, 16)),
            159 => Box::new(BandaiFcg::new(prg_rom, chr, prg_ram, header, 159)),
            112 => Box::new(Txc112::new(prg_rom, chr, header)),
            235 => Box::new(Multicart235::new(prg_rom, chr, header)),
            241 => Box::new(BxRomWram::new(prg_rom, chr, prg_ram, header)),
            286 => Box::new(Benshieng286::new(prg_rom, chr, header)),
            58 | 213 => Box::new(Multicart58::new(prg_rom, chr, header)),
            132 => Box::new(TxcLatch::new(prg_rom, chr, header, 132)),
            173 => Box::new(TxcLatch::new(prg_rom, chr, header, 173)),
            177 => Box::new(Hengge177::new(prg_rom, chr, prg_ram, header)),
            185 => Box::new(CnRom185::new(prg_rom, chr, header)),
            189 => Box::new(Mmc3_32k::new(prg_rom, chr, header)),
            224 => Box::new(Coolboy::new(prg_rom, chr, prg_ram, header, 0x5000)),
            268 => {
                let base = if header.submapper == 0 {
                    0x6000
                } else {
                    0x5000
                };
                Box::new(Coolboy::new(prg_rom, chr, prg_ram, header, base))
            }
            176 => Box::new(Mmc3_176::new(prg_rom, chr, prg_ram, header)),
            212 => Box::new(Multicart212::new(prg_rom, chr, header)),
            215 => Box::new(Mmc3_215::new(prg_rom, chr, prg_ram, header)),
            114 => Box::new(Mmc3_114::new(prg_rom, chr, header)),
            115 => Box::new(Mmc3_115::new(prg_rom, chr, header)),
            138 => Box::new(Sachen8259::new(prg_rom, chr, header, 138, 0)),
            139 => Box::new(Sachen8259::new(prg_rom, chr, header, 139, 2)),
            141 => Box::new(Sachen8259::new(prg_rom, chr, header, 141, 1)),
            232 => Box::new(Quattro232::new(prg_rom, chr, header)),
            210 => Box::new(Namco210::new(prg_rom, chr, prg_ram, header)),
            121 => Box::new(Mmc3_121::new(prg_rom, chr, header)),
            111 => Box::new(Gtrom::new(prg_rom, chr, header)),
            83 => Box::new(Cony83::new(prg_rom, chr, prg_ram, header)),
            32 => Box::new(Irem32::new(prg_rom, chr, header)),
            33 => Box::new(Taito33::new(prg_rom, chr, header)),
            36 => Box::new(Txc36::new(prg_rom, chr, header)),
            42 => Box::new(Fds42::new(prg_rom, chr, header)),
            48 => Box::new(Taito48::new(prg_rom, chr, header)),
            85 => Box::new(Vrc7::new(prg_rom, chr, prg_ram, header)),
            86 => Box::new(Jf13::new(prg_rom, chr, header)),
            75 => Box::new(Vrc1::new(prg_rom, chr, header)),
            80 => Box::new(Taito80::new(prg_rom, chr, header, false, 80)),
            207 => Box::new(Taito80::new(prg_rom, chr, header, true, 207)),
            227 => Box::new(Multicart242::new(prg_rom, chr, header, true, 227)),
            242 => Box::new(Multicart242::new(prg_rom, chr, header, false, 242)),
            64 => Box::new(Rambo1::new(prg_rom, chr, header, false, 64)),
            158 => Box::new(Rambo1::new(prg_rom, chr, header, true, 158)),
            184 => Box::new(Sunsoft1::new(prg_rom, chr, header)),
            59 => Box::new(Multicart59::new(prg_rom, chr, header)),
            108 => Box::new(Fds108::new(prg_rom, chr, header)),
            319 => Box::new(Hp898f::new(prg_rom, chr, header)),
            34 => Box::new(Nina::new(prg_rom, chr, header)),
            79 | 146 => Box::new(Nina79::new(prg_rom, chr, header)),
            87 | 101 => Box::new(J87::new(prg_rom, chr, header)),
            91 => Box::new(JyClone::new(prg_rom, chr, header)),
            113 => Box::new(Hes113::new(prg_rom, chr, header)),
            162 => Box::new(Fs304::new(prg_rom, chr, prg_ram, header)),
            163 => Box::new(Nanjing163::new(prg_rom, chr, header)),
            558 => Box::new(Fs558::new(prg_rom, chr, prg_ram, header)),
            76 | 88 | 95 | 154 | 206 => Box::new(DxRom::new(prg_rom, chr, header)),
            66 => Box::new(GxRom::new(prg_rom, chr, header)),
            71 => Box::new(Camerica::new(prg_rom, chr, header)),
            7 => Box::new(AxRom::new(prg_rom, chr, header)),
            21 | 22 | 23 | 25 => Box::new(Vrc::new(prg_rom, chr, prg_ram, header)),
            24 => Box::new(Vrc6::new(prg_rom, chr, prg_ram, header, 24)),
            26 => Box::new(Vrc6::new(prg_rom, chr, prg_ram, header, 26)),
            other => return Err(CartError::UnsupportedMapper(other)),
        };
        Ok(Self {
            chr_is_ram,
            mapper,
            header,
        })
    }

    /// Parsed header.
    #[must_use]
    pub const fn header(&self) -> Header {
        self.header
    }

    /// Whether CHR is writable RAM.
    #[must_use]
    pub const fn chr_is_ram(&self) -> bool {
        self.chr_is_ram
    }

    /// CPU read in `$4020..=$FFFF` without side effects.
    #[must_use]
    pub fn cpu_read(&self, address: u16) -> u8 {
        self.mapper.prg_read(address)
    }

    /// CPU read with side effects (FDS `$4030`/`$4031` acknowledgement).
    pub fn cpu_read_io(&mut self, address: u16) -> u8 {
        self.mapper.read_io(address)
    }

    /// CPU write in `$4020..=$FFFF`.
    pub fn cpu_write(&mut self, address: u16, value: u8) {
        self.mapper.prg_write(address, value);
    }

    /// PPU read in `$0000..=$1FFF`.
    #[must_use]
    pub fn ppu_read(&self, address: u16) -> u8 {
        self.mapper.chr_read(address)
    }

    /// PPU write in `$0000..=$1FFF`.
    pub fn ppu_write(&mut self, address: u16, value: u8) {
        self.mapper.chr_write(address, value);
    }

    /// Current mirroring.
    #[must_use]
    pub fn mirroring(&self) -> Mirroring {
        self.mapper.mirroring()
    }

    /// Mapper number.
    #[must_use]
    pub fn mapper(&self) -> u16 {
        self.mapper.number()
    }

    /// Full CHR ROM for latch-banked mappers (MMC2/MMC4).
    #[must_use]
    pub fn chr_rom(&self) -> Option<&[u8]> {
        self.mapper.chr_rom()
    }

    /// Current CHR latch banks for latch-banked mappers (MMC2/MMC4).
    #[must_use]
    pub fn chr_banks(&self) -> Option<[u8; 4]> {
        self.mapper.chr_banks()
    }

    /// Whether the mapper can drive nametables from CHR ROM.
    #[must_use]
    pub fn has_rom_nametables(&self) -> bool {
        self.mapper.has_rom_nametables()
    }

    /// Two 1 KiB CHR bank indices for ROM nametables, when enabled.
    #[must_use]
    pub fn rom_nametable_banks(&self) -> Option<[u16; 2]> {
        self.mapper.rom_nametable_banks()
    }

    /// Shared hybrid CHR state (mappers 165/195).
    #[must_use]
    pub fn hybrid_chr(&self) -> Option<Rc<RefCell<HybridChr>>> {
        self.mapper.hybrid_chr()
    }

    /// Cartridge-backed CPU window inside `$4020-$5FFF`.
    #[must_use]
    pub fn low_window(&self) -> Option<(u16, u16)> {
        self.mapper.low_window()
    }

    /// Shared MMC5 state for the PPU.
    #[must_use]
    pub fn mmc5_aux(&self) -> Option<Rc<RefCell<Mmc5Aux>>> {
        self.mapper.mmc5_aux()
    }

    /// Shared banked CHR-RAM state for the PPU (VRC7).
    #[must_use]
    pub fn vrc7_chr(&self) -> Option<Rc<RefCell<Vrc7Chr>>> {
        self.mapper.vrc7_chr()
    }

    /// Inserts an FDS disk side (`Some`) or ejects (`None`).
    pub fn insert_disk_side(&mut self, side: Option<usize>) {
        self.mapper.insert_disk_side(side);
    }

    /// Number of sides in an FDS image (0 for non-FDS boards).
    #[must_use]
    pub fn disk_side_count(&self) -> usize {
        self.mapper.disk_side_count()
    }

    /// Currently inserted FDS side.
    #[must_use]
    pub fn current_disk_side(&self) -> usize {
        self.mapper.current_disk_side()
    }

    /// Returns the raw `.fds` image with pending write-mode changes flushed
    /// back into every side (no-op/empty for non-FDS cartridges).
    pub fn save_disk_image(&mut self) -> Vec<u8> {
        self.mapper.disk_image()
    }

    /// Notifies the mapper of a fully decoded PPUCTRL write.
    pub fn set_ppu_ctrl(&mut self, value: u8) {
        self.mapper.set_ppu_ctrl(value);
    }

    /// Expansion-audio contribution for the current CPU cycle.
    #[must_use]
    pub fn audio_mix(&self) -> f32 {
        self.mapper.audio_mix()
    }

    /// Battery-backed PRG-RAM contents.
    #[must_use]
    pub fn prg_ram(&self) -> &[u8] {
        self.mapper.prg_ram()
    }

    /// Mapper IRQ output (level).
    #[must_use]
    pub fn irq_line(&self) -> bool {
        self.mapper.irq_line()
    }

    /// Clocks the mapper once for a rendered scanline (PPU A12 rise).
    pub fn clock_scanline(&mut self) {
        self.mapper.clock_scanline();
    }

    /// Acknowledges a pending mapper IRQ.
    pub fn acknowledge_irq(&mut self) {
        self.mapper.acknowledge_irq();
    }

    /// Clocks the mapper once for a CPU cycle.
    pub fn clock_cpu(&mut self) {
        self.mapper.clock_cpu();
    }
}

/// Parses an iNES / NES 2.0 header without loading the ROM data.
pub fn parse_header(bytes: &[u8]) -> Result<Header, CartError> {
    if bytes.len() < 16 {
        return Err(CartError::Truncated {
            expected: 16,
            actual: bytes.len(),
        });
    }
    if &bytes[0..4] != b"NES\x1a" {
        return Err(CartError::BadMagic);
    }
    let flags6 = bytes[6];
    let flags7 = bytes[7];
    let nes2 = flags7 & 0x0C == 0x08;
    let mut mapper = u16::from(flags7 & 0xF0) | u16::from(flags6 >> 4);
    let mut prg_size = usize::from(bytes[4]) * 16 * 1024;
    let mut chr_size = usize::from(bytes[5]) * 8 * 1024;
    let mut prg_ram_size = 8 * 1024;
    let mut submapper = 0;
    if nes2 {
        mapper |= u16::from(bytes[8] & 0x0F) << 8;
        submapper = bytes[8] >> 4;
        prg_size = nes2_size(bytes[4], bytes[9] & 0x0F, 16);
        chr_size = nes2_size(bytes[5], bytes[9] >> 4, 8);
        // Byte 10: low nibble = volatile PRG-RAM, high nibble = battery PRG-RAM,
        // both in 8 KiB units.
        let ram_units = u32::from(bytes[10] & 0x0F) + u32::from(bytes[10] >> 4);
        prg_ram_size = ((ram_units as usize) * 8 * 1024).max(0x2000);
    }
    Ok(Header {
        mapper,
        submapper,
        prg_size,
        chr_size,
        battery: flags6 & 0x02 != 0,
        four_screen: flags6 & 0x08 != 0,
        nes2,
        prg_ram_size,
        mirroring: if flags6 & 0x08 != 0 {
            Mirroring::FourScreen
        } else if flags6 & 0x01 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        },
    })
}

fn nes2_size(low: u8, high: u8, unit_kib: usize) -> usize {
    let unit = unit_kib * 1024;
    match high {
        0 => usize::from(low) * unit,
        0x0F => {
            let exponent = usize::from((low >> 2) & 3);
            let multiplier = usize::from(low & 3) * 2 + 1;
            (1usize << exponent) * multiplier * unit
        }
        _ => (1usize << high) * (usize::from(low) * 2 + 1) * unit,
    }
}

/// NROM (mapper 0).
#[derive(Debug)]
pub struct Nrom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    mirroring: Mirroring,
}

impl Nrom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Nrom {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let offset = usize::from(address - 0x8000);
        self.prg[offset % self.prg.len()]
    }

    fn prg_write(&mut self, _address: u16, _value: u8) {}

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        0
    }
}

/// MMC1 (mapper 1).
#[derive(Debug)]
pub struct Mmc1 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    shift: u8,
    count: u8,
    control: u8,
    chr0: u8,
    chr1: u8,
    prg_bank: u8,
    four_screen: bool,
    battery: bool,
}

impl Mmc1 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            shift: 0,
            count: 0,
            control: match header.mirroring {
                Mirroring::Vertical => 0x0E,
                Mirroring::Horizontal => 0x0F,
                Mirroring::SingleLower => 0x0C,
                Mirroring::SingleUpper => 0x0D,
                Mirroring::FourScreen => 0x0C,
            },
            chr0: 0,
            chr1: 0,
            prg_bank: 0,
            four_screen: header.four_screen,
            battery: header.battery,
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x4000).max(1)
    }

    fn prg_offset(&self, address: u16) -> usize {
        let offset = usize::from(address - 0x8000);
        let banks = self.prg_banks();
        let mode = (self.control >> 2) & 3;
        let bank = usize::from(self.prg_bank & 0x0F);
        let (first, second) = match mode {
            0 | 1 => {
                let b = bank & !1;
                (b, b + 1)
            }
            2 => (0, bank),
            _ => (bank, banks - 1),
        };
        if offset < 0x4000 {
            (first % banks) * 0x4000 + offset
        } else {
            (second % banks) * 0x4000 + (offset - 0x4000)
        }
    }

    fn chr_offset(&self, address: u16) -> usize {
        let index = usize::from(address) & 0x1FFF;
        let mode = (self.control >> 4) & 1;
        let banks = (self.chr.len() / 0x1000).max(1);
        if mode == 0 {
            let bank = usize::from(self.chr0 & 0x1E);
            ((bank % banks) * 0x1000) + index
        } else if index < 0x1000 {
            (usize::from(self.chr0) % banks) * 0x1000 + index
        } else {
            (usize::from(self.chr1) % banks) * 0x1000 + (index - 0x1000)
        }
    }

    fn write_register(&mut self, address: u16, value: u8) {
        match address & 0xE000 {
            0x8000 => self.control = value & 0x1F,
            0xA000 => self.chr0 = value,
            0xC000 => self.chr1 = value,
            _ => self.prg_bank = value,
        }
    }
}

impl Mapper for Mmc1 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x6000 {
            return 0;
        }
        if address < 0x8000 {
            let offset = usize::from(address - 0x6000);
            return if self.battery || !self.prg_ram.is_empty() {
                self.prg_ram[offset % self.prg_ram.len()]
            } else {
                0
            };
        }
        self.prg[self.prg_offset(address)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            if address >= 0x6000 && !self.prg_ram.is_empty() {
                let offset = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[offset] = value;
            }
            return;
        }
        if value & 0x80 != 0 {
            self.shift = 0;
            self.count = 0;
            self.control |= 0x0C;
            return;
        }
        self.shift |= (value & 1) << self.count;
        self.count += 1;
        if self.count == 5 {
            let register = self.shift & 0x1F;
            self.write_register(address, register);
            self.shift = 0;
            self.count = 0;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr.get(self.chr_offset(address)).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = self.chr_offset(address);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.four_screen {
            return Mirroring::FourScreen;
        }
        match self.control & 3 {
            0 => Mirroring::SingleLower,
            1 => Mirroring::SingleUpper,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        }
    }

    fn number(&self) -> u16 {
        1
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Sunsoft-4 (mapper 68): 2 KiB CHR banks, a 16 KiB switchable PRG bank and
/// mapper-controlled mirroring.
///
/// | Address | Function |
/// |---|---|
/// | `$8000`/`$9000`/`$A000`/`$B000` | CHR 2 KiB banks for PPU `$0000`/`$0800`/`$1000`/`$1800` |
/// | `$C000`/`$D000` | CHR nametable banks (ROM nametable mode) |
/// | `$E000` | mirroring (bits 1:0), ROM nametables (bit 4) |
/// | `$F000` | 16 KiB PRG bank (bits 3:0), PRG-RAM enable (bit 4) |
///
/// `$8000-$BFFF` is the switchable 16 KiB bank; `$C000-$FFFF` is the fixed
/// last 16 KiB bank. ROM nametable banking is recorded but the PPU currently
/// renders from CIRAM, so titles that rely on it show CIRAM contents.
#[derive(Debug)]
pub struct Sunsoft4 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    chr_banks: [u8; 4],
    nametable_banks: [u8; 2],
    prg_bank: u8,
    mirroring: Mirroring,
    rom_nametables: bool,
    wram_enable: bool,
}

impl Sunsoft4 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            chr_banks: [0, 1, 2, 3],
            nametable_banks: [0, 1],
            prg_bank: 0,
            mirroring: header.mirroring,
            rom_nametables: false,
            wram_enable: false,
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x4000).max(1)
    }

    fn chr_offset(&self, address: u16) -> usize {
        let index = usize::from(address) & 0x1FFF;
        let slot = index / 0x800;
        let banks = (self.chr.len() / 0x800).max(1);
        let bank = usize::from(self.chr_banks[slot]) % banks;
        bank * 0x800 + index % 0x800
    }
}

impl Mapper for Sunsoft4 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x6000 {
            return 0;
        }
        if address < 0x8000 {
            if !self.wram_enable || self.prg_ram.is_empty() {
                return 0;
            }
            let offset = usize::from(address - 0x6000) % self.prg_ram.len();
            return self.prg_ram[offset];
        }
        let banks = self.prg_banks();
        let bank = if address < 0xC000 {
            usize::from(self.prg_bank & 0x0F) % banks
        } else {
            banks - 1
        };
        self.prg
            .get(bank * 0x4000 + usize::from(address & 0x3FFF))
            .copied()
            .unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            if address >= 0x6000 && self.wram_enable && !self.prg_ram.is_empty() {
                let offset = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[offset] = value;
            }
            return;
        }
        match address & 0xF000 {
            0x8000 => self.chr_banks[0] = value & 0x3F,
            0x9000 => self.chr_banks[1] = value & 0x3F,
            0xA000 => self.chr_banks[2] = value & 0x3F,
            0xB000 => self.chr_banks[3] = value & 0x3F,
            0xC000 => self.nametable_banks[0] = value & 0x7F,
            0xD000 => self.nametable_banks[1] = value & 0x7F,
            0xE000 => {
                self.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                };
                self.rom_nametables = value & 0x10 != 0;
            }
            _ => {
                self.prg_bank = value & 0x0F;
                self.wram_enable = value & 0x10 != 0;
            }
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr.get(self.chr_offset(address)).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = self.chr_offset(address);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        68
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn chr_rom(&self) -> Option<&[u8]> {
        Some(&self.chr)
    }

    fn has_rom_nametables(&self) -> bool {
        true
    }

    fn rom_nametable_banks(&self) -> Option<[u16; 2]> {
        if self.rom_nametables {
            // D7 is not connected and reads as 1, so the nametable banks live
            // in the upper half of the CHR ROM.
            Some([
                0x80 | u16::from(self.nametable_banks[0] & 0x7F),
                0x80 | u16::from(self.nametable_banks[1] & 0x7F),
            ])
        } else {
            None
        }
    }
}

/// Waixing FS303 (mapper 195): MMC3 plus 4 KiB of PRG-RAM at `$5000-$5FFF`.
///
/// The CHR side mixes ROM and RAM banks under a latch written through the
/// PPU; the accuracy core currently exposes a writable 8 KiB CHR window, which
/// covers the games that upload their tiles. The ROM half is a follow-up.
#[derive(Debug)]
pub struct Waixing195 {
    inner: Mmc3,
    ram4k: [u8; 0x1000],
}

impl Waixing195 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            inner: Mmc3::new_hybrid(prg, chr, header, 195, HybridKind::Waixing195),
            ram4k: [0; 0x1000],
        }
    }
}

impl Mapper for Waixing195 {
    fn prg_read(&self, address: u16) -> u8 {
        if (0x5000..=0x5FFF).contains(&address) {
            return self.ram4k[usize::from(address - 0x5000)];
        }
        self.inner.prg_read(address)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x5000..=0x5FFF).contains(&address) {
            self.ram4k[usize::from(address - 0x5000)] = value;
            return;
        }
        self.inner.prg_write(address, value);
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.inner.chr_read(address)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        self.inner.chr_write(address, value);
    }

    fn mirroring(&self) -> Mirroring {
        self.inner.mirroring()
    }

    fn number(&self) -> u16 {
        195
    }

    fn prg_ram(&self) -> &[u8] {
        self.inner.prg_ram()
    }

    fn hybrid_chr(&self) -> Option<Rc<RefCell<HybridChr>>> {
        self.inner.hybrid_chr()
    }

    fn low_window(&self) -> Option<(u16, u16)> {
        Some((0x5000, 0x5FFF))
    }

    fn irq_line(&self) -> bool {
        self.inner.irq_line()
    }

    fn clock_scanline(&mut self) {
        self.inner.clock_scanline();
    }

    fn acknowledge_irq(&mut self) {
        self.inner.acknowledge_irq();
    }
}

/// MMC3 clone with 512 KiB CHR wiring (mapper 197).
///
/// PRG, mirroring and IRQ are plain MMC3; the CHR address lines are shifted so
/// the MMC3's 1 KiB banks become 2/4 KiB ROM banks. Each submapper wires the
/// register selectors differently.
#[derive(Debug)]
pub struct Mmc3_197 {
    inner: Mmc3,
    sub: u8,
}

impl Mmc3_197 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        let sub = if header.submapper <= 2 {
            header.submapper
        } else {
            0
        };
        Self {
            inner: Mmc3::new_as(prg, chr, header, 197),
            sub,
        }
    }

    /// MMC3 1 KiB register value for a real PPU 1 KiB slot.
    fn mmc3_bank(&self, slot: usize) -> usize {
        let d = &self.inner.bank_data;
        let bank = match (self.sub, slot) {
            (0, 0..=1) => d[0] & 0xFE,
            (0, 2..=3) => (d[0] & 0xFE) | 1,
            (0, 4..=5) => d[2],
            (0, _) => d[3],
            (1, 0..=3) => d[1] & 0xFE,
            (1, 4..=5) => d[4],
            (1, _) => d[5],
            (2, 0..=1) => d[0] & 0xFE,
            (2, 2..=3) => d[1] | 1,
            (2, 4..=5) => d[2],
            (2, _) => d[5],
            _ => 0,
        };
        usize::from(bank)
    }

    fn chr_offset(&self, address: u16) -> usize {
        let slot = usize::from(address) / 0x400;
        let banks = (self.inner.chr.len() / 0x400).max(1);
        let bank = ((self.mmc3_bank(slot) << 1) | (slot & 1)) % banks;
        bank * 0x400 + usize::from(address & 0x3FF)
    }
}

impl Mapper for Mmc3_197 {
    fn prg_read(&self, address: u16) -> u8 {
        self.inner.prg_read(address)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        self.inner.prg_write(address, value);
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.inner
            .chr
            .get(self.chr_offset(address))
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = self.chr_offset(address);
        if let Some(byte) = self.inner.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.inner.mirroring()
    }

    fn number(&self) -> u16 {
        197
    }

    fn prg_ram(&self) -> &[u8] {
        self.inner.prg_ram()
    }

    fn irq_line(&self) -> bool {
        self.inner.irq_line()
    }

    fn clock_scanline(&mut self) {
        self.inner.clock_scanline();
    }

    fn acknowledge_irq(&mut self) {
        self.inner.acknowledge_irq();
    }
}

/// NTDEC TC-112 (mapper 193): CHR/PRG banking through `$6000-$7FFF`.
///
/// `$6000` selects a 4 KiB CHR bank for `$0000-$0FFF`, `$6001`/`$6002` select
/// 2 KiB banks for `$1000-$17FF`/`$1800-$1FFF`, `$6003` a 16 KiB PRG bank at
/// `$8000`, and `$6004` the mirroring. There is no PRG-RAM.
#[derive(Debug)]
pub struct NtdTc112 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_banks: [u8; 3],
    prg_bank: u8,
    mirroring: Mirroring,
}

impl NtdTc112 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            chr_banks: [0; 3],
            prg_bank: 0,
            mirroring: header.mirroring,
        }
    }

    fn chr_offset(&self, address: u16) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = match slot {
            0..=3 => (usize::from(self.chr_banks[0]) >> 2) * 4 + (slot & 3),
            4 | 5 => (usize::from(self.chr_banks[1]) >> 1) * 2 + (slot & 1),
            _ => (usize::from(self.chr_banks[2]) >> 1) * 2 + (slot & 1),
        } % banks;
        bank * 0x400 + usize::from(address & 0x3FF)
    }
}

impl Mapper for NtdTc112 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x2000).max(1);
        let slot = usize::from((address - 0x8000) / 0x2000);
        let bank = match slot {
            0 => usize::from(self.prg_bank & 0x0F),
            1 => banks.saturating_sub(3),
            2 => banks.saturating_sub(2),
            _ => banks - 1,
        } % banks;
        self.prg
            .get(bank * 0x2000 + usize::from(address & 0x1FFF))
            .copied()
            .unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if !(0x6000..=0x7FFF).contains(&address) {
            return;
        }
        match address & 0x0007 {
            0 => self.chr_banks[0] = value,
            1 => self.chr_banks[1] = value,
            2 => self.chr_banks[2] = value,
            3 => self.prg_bank = value & 0x0F,
            4 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr.get(self.chr_offset(address)).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = self.chr_offset(address);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        193
    }
}

/// Sachen 3011 / JV001 (mapper 136): a six-bit latch with an adder and
/// inverter, used for copy protection and CHR/PRG banking.
///
/// `$4100-$4103` are the latch interface; writing `$8000-$FFFF` latches the
/// register to the output. CHR uses an 8 KiB bank from output bits 0-2 and PRG
/// a 32 KiB bank from output bit 4.
#[derive(Debug)]
pub struct Sachen136 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    input: u8,
    register: u8,
    output: u8,
    mode: bool,
    invert: bool,
    mirroring: Mirroring,
    number: u16,
    reversed: bool,
    mirroring_write: bool,
}

impl Sachen136 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self::new_as(prg, chr, header, 136, false, false)
    }

    fn new_172(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self::new_as(prg, chr, header, 172, true, true)
    }

    fn new_as(
        prg: Vec<u8>,
        chr: Vec<u8>,
        header: Header,
        number: u16,
        reversed: bool,
        mirroring_write: bool,
    ) -> Self {
        Self {
            prg,
            chr,
            input: 0,
            register: 0,
            output: 0,
            mode: false,
            invert: false,
            mirroring: header.mirroring,
            number,
            reversed,
            mirroring_write,
        }
    }

    fn reverse6(value: u8) -> u8 {
        value.reverse_bits() >> 2
    }

    fn chr_bank(&self) -> usize {
        if self.reversed {
            usize::from((self.output >> 4) & 0x03)
        } else {
            usize::from(self.output & 0x07)
        }
    }
}

impl Mapper for Sachen136 {
    fn prg_read(&self, address: u16) -> u8 {
        if address & 0xE100 == 0x4100 {
            let value = if self.invert {
                self.register ^ 0x30
            } else {
                self.register
            };
            return if self.reversed {
                Self::reverse6(value)
            } else {
                value
            };
        }
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = usize::from((self.output >> 4) & 0x01) % banks;
        self.prg
            .get(bank * 0x8000 + usize::from(address & 0x7FFF))
            .copied()
            .unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address & 0xE100 == 0x4100 {
            match address & 0x0003 {
                0 => {
                    if self.mode {
                        self.register =
                            (self.register & 0x30) | (self.register.wrapping_add(1) & 0x0F);
                    } else {
                        let low = if self.invert {
                            (self.input & 0x0F) ^ 0x0F
                        } else {
                            self.input & 0x0F
                        };
                        self.register = (self.input & 0x30) | low;
                    }
                }
                1 => {
                    self.invert = if self.reversed {
                        value & 0x20 != 0
                    } else {
                        value & 1 != 0
                    }
                }
                2 => {
                    self.input = if self.reversed {
                        Self::reverse6(value)
                    } else {
                        value & 0x3F
                    }
                }
                _ => {
                    self.mode = if self.reversed {
                        value & 0x20 != 0
                    } else {
                        value & 1 != 0
                    }
                }
            }
            return;
        }
        if address >= 0x8000 {
            self.output = self.register;
            if self.mirroring_write {
                self.mirroring = if self.invert {
                    Mirroring::Vertical
                } else {
                    Mirroring::Horizontal
                };
            }
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = self.chr_bank() % banks;
        self.chr
            .get(bank * 0x2000 + usize::from(address & 0x1FFF))
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = self.chr_bank() % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.number
    }
}

/// MMC2 (mapper 9) and MMC4 (mapper 10).
///
/// Both use two CHR latches that switch 4 KiB pattern banks when the PPU
/// fetches tile `$FD` or `$FE`. MMC2 banks 8 KiB of PRG at `$8000` with the
/// second-to-last and last banks fixed; MMC4 banks 16 KiB at `$8000` with the
/// last bank fixed. Register layout:
///
/// | Address | Register |
/// |---|---|
/// | `$A000` | PRG bank (low 4 bits) |
/// | `$B000` | CHR latch 0, `$FD` bank |
/// | `$C000` | CHR latch 0, `$FE` bank |
/// | `$D000` | CHR latch 1, `$FD` bank |
/// | `$E000` | CHR latch 1, `$FE` bank |
/// | `$F000` | mirroring (bit 0: 1 vertical, 0 horizontal) |
#[derive(Debug)]
pub struct Mmc2 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_bank: u8,
    chr_banks: [u8; 4],
    mirroring: Mirroring,
    number: u16,
}

impl Mmc2 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header, number: u16) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            prg_bank: 0,
            chr_banks: [0; 4],
            mirroring: match header.mirroring {
                Mirroring::Vertical => Mirroring::Vertical,
                _ => Mirroring::Horizontal,
            },
            number,
        }
    }

    fn prg_offset(&self, address: u16) -> usize {
        let offset = usize::from(address - 0x8000);
        if self.number == 9 {
            let banks = (self.prg.len() / 0x2000).max(1);
            let bank = if offset < 0x2000 {
                usize::from(self.prg_bank & 0x0F) % banks
            } else {
                // The top 24 KiB is fixed to the last three 8 KiB banks:
                // $A000/$C000/$E000 see banks N-3/N-2/N-1. Punch-Out!! keeps
                // its reset code in N-3 and the vectors at the end of N-1.
                let fixed = banks.saturating_sub(3) + (offset - 0x2000) / 0x2000;
                fixed.min(banks - 1)
            };
            bank * 0x2000 + (offset & 0x1FFF)
        } else {
            let banks = (self.prg.len() / 0x4000).max(1);
            let bank = if offset < 0x4000 {
                usize::from(self.prg_bank & 0x0F) % banks
            } else {
                banks - 1
            };
            bank * 0x4000 + (offset & 0x3FFF)
        }
    }

    fn chr_offset(&self, address: u16) -> usize {
        // The mapper-side view has no latch history; the PPU drives the real
        // fetch latches and reads `chr_rom` through its own window.
        let table = usize::from(address & 0x1000 != 0);
        let banks = (self.chr.len() / 0x1000).max(1);
        let bank = usize::from(self.chr_banks[table * 2]) % banks;
        bank * 0x1000 + usize::from(address & 0x0FFF)
    }
}

impl Mapper for Mmc2 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x6000 {
            return 0;
        }
        if address < 0x8000 {
            if self.prg_ram.is_empty() {
                return 0;
            }
            let offset = usize::from(address - 0x6000) % self.prg_ram.len();
            return self.prg_ram[offset];
        }
        self.prg.get(self.prg_offset(address)).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            if address >= 0x6000 && !self.prg_ram.is_empty() {
                let offset = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[offset] = value;
            }
            return;
        }
        match address & 0xF000 {
            0xA000 => self.prg_bank = value & 0x0F,
            0xB000 => self.chr_banks[0] = value & 0x1F,
            0xC000 => self.chr_banks[1] = value & 0x1F,
            0xD000 => self.chr_banks[2] = value & 0x1F,
            0xE000 => self.chr_banks[3] = value & 0x1F,
            0xF000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Vertical
                } else {
                    Mirroring::Horizontal
                };
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr.get(self.chr_offset(address)).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = self.chr_offset(address);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.number
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn chr_rom(&self) -> Option<&[u8]> {
        Some(&self.chr)
    }

    fn chr_banks(&self) -> Option<[u8; 4]> {
        Some(self.chr_banks)
    }
}

/// UxROM (mapper 2): 16 KiB switchable at `$8000`, last bank fixed.
#[derive(Debug)]
pub struct UxRom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: usize,
    mirroring: Mirroring,
}

impl UxRom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            mirroring: header.mirroring,
        }
    }

    fn banks(&self) -> usize {
        (self.prg.len() / 0x4000).max(1)
    }
}

impl Mapper for UxRom {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = self.banks();
        let bank = if address < 0xC000 {
            self.bank % banks
        } else {
            banks - 1
        };
        let offset = usize::from(address & 0x3FFF);
        self.prg[bank * 0x4000 + offset]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            self.bank = usize::from(value) % self.banks();
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        2
    }
}

/// CNROM (mapper 3): fixed PRG, switchable 8 KiB CHR.
#[derive(Debug)]
pub struct CnRom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: usize,
    mirroring: Mirroring,
}

impl CnRom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            mirroring: header.mirroring,
        }
    }

    fn chr_banks(&self) -> usize {
        (self.chr.len() / 0x2000).max(1)
    }
}

impl Mapper for CnRom {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        self.prg
            .get(usize::from(address - 0x8000) % self.prg.len())
            .copied()
            .unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            self.bank = usize::from(value) % self.chr_banks();
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let offset = self.bank * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = self.bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        3
    }
}

/// AxROM (mapper 7): 32 KiB switchable PRG, single-screen mirroring.
#[derive(Debug)]
pub struct AxRom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: usize,
    mirroring: Mirroring,
}

impl AxRom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            mirroring: if header.four_screen {
                Mirroring::FourScreen
            } else {
                Mirroring::SingleLower
            },
        }
    }

    fn banks(&self) -> usize {
        (self.prg.len() / 0x8000).max(1)
    }
}

impl Mapper for AxRom {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let offset = self.bank * 0x8000 + usize::from(address - 0x8000);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            // AxROM boards have bus conflicts: the value is ANDed with the ROM
            // byte currently visible at the write address.
            let offset = self.bank * 0x8000 + usize::from(address - 0x8000);
            let visible = self.prg.get(offset).copied().unwrap_or(0xFF);
            let value = value & visible;
            self.bank = usize::from(value & 0x07) % self.banks();
            self.mirroring = if value & 0x10 != 0 {
                Mirroring::SingleUpper
            } else {
                Mirroring::SingleLower
            };
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        7
    }
}

/// MMC3 (mapper 4): 8 KiB PRG / 1 KiB CHR banking and mirroring.
///
/// IRQ generation is not wired to the PPU yet; split-screen games that need it
/// are a known A4 gap.
#[derive(Debug)]
pub struct Mmc3 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    mapper: u16,
    txsrom: bool,
    remap_a0: bool,
    select: u8,
    bank_data: [u8; 8],
    mirroring: Mirroring,
    ram_enabled: bool,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
    hybrid: Option<Rc<RefCell<HybridChr>>>,
}

impl Mmc3 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self::new_as(prg, chr, header, 4)
    }

    fn new_hybrid(
        prg: Vec<u8>,
        chr: Vec<u8>,
        header: Header,
        mapper: u16,
        kind: HybridKind,
    ) -> Self {
        let hybrid = Rc::new(RefCell::new(HybridChr::new(kind, chr.clone())));
        let mut mmc3 = Self::new_as(prg, chr, header, mapper);
        mmc3.hybrid = Some(hybrid);
        mmc3
    }

    fn new_196(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        let mut mmc3 = Self::new_as(prg, chr, header, 196);
        mmc3.remap_a0 = true;
        mmc3
    }

    fn new_txsrom(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        let mut mmc3 = Self::new_as(prg, chr, header, 118);
        mmc3.txsrom = true;
        mmc3
    }

    fn new_as(prg: Vec<u8>, chr: Vec<u8>, header: Header, mapper: u16) -> Self {
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x2000)],
            mapper,
            txsrom: false,
            remap_a0: false,
            select: 0,
            // MMC3 power-on register values (Mesen ResetMmc3): R0..R7.
            bank_data: [0, 2, 4, 5, 6, 7, 0, 1],
            mirroring: header.mirroring,
            ram_enabled: true,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
            hybrid: None,
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn prg_slot(&self, index: usize) -> usize {
        let banks = self.prg_banks();
        let last = banks - 1;
        let second_last = banks.saturating_sub(2);
        let mode = self.select & 0x40;
        let slot = match (mode, index) {
            (0, 0) => usize::from(self.bank_data[6]),
            (0, 1) => usize::from(self.bank_data[7]),
            (0, 2) => second_last,
            (0, _) => last,
            (_, 0) => second_last,
            (_, 1) => usize::from(self.bank_data[7]),
            (_, 2) => usize::from(self.bank_data[6]),
            (_, _) => last,
        };
        slot % banks
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let mode = self.select & 0x80;
        let raw = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        raw % banks
    }
}

impl Mapper for Mmc3 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x6000 {
            return 0;
        }
        if address < 0x8000 {
            // `$A001` bit 7 gates *writes* only; PRG-RAM reads stay mapped.
            if self.prg_ram.is_empty() {
                return 0;
            }
            return self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()];
        }
        let slot = usize::from((address - 0x8000) / 0x2000);
        let bank = self.prg_slot(slot);
        self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            if address >= 0x6000 && self.ram_enabled && !self.prg_ram.is_empty() {
                let offset = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[offset] = value;
            }
            return;
        }
        let address = if self.remap_a0 {
            let a0 = u16::from((address & 0x000E) != 0 && address & 1 == 0);
            (address & !1) | a0
        } else {
            address
        };
        match address & 0xE001 {
            0x8000 => {
                self.select = value;
                if std::env::var("PRGLOG").is_ok() && self.mapper == 195 {
                    eprintln!("select={value:02X}");
                }
                if let Some(hybrid) = &self.hybrid {
                    hybrid.borrow_mut().update_regs(self.select, self.bank_data);
                }
            }
            0x8001 => {
                let index = usize::from(self.select & 0x07);
                self.bank_data[index] = value;
                if std::env::var("PRGLOG").is_ok() && self.mapper == 195 {
                    eprintln!("bank[{index}]={value:02X}");
                }
                if let Some(hybrid) = &self.hybrid {
                    hybrid.borrow_mut().update_regs(self.select, self.bank_data);
                }
            }
            0xA000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                }
            }
            0xA001 => self.ram_enabled = value & 0x80 != 0,
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        if let Some(hybrid) = &self.hybrid {
            return hybrid.borrow().read(address);
        }
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(hybrid) = &self.hybrid {
            hybrid.borrow_mut().write(address, value);
            return;
        }
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.txsrom {
            if self.bank_data[0] & 0x80 != 0 {
                Mirroring::SingleUpper
            } else {
                Mirroring::SingleLower
            }
        } else {
            self.mirroring
        }
    }

    fn number(&self) -> u16 {
        self.mapper
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn hybrid_chr(&self) -> Option<Rc<RefCell<HybridChr>>> {
        self.hybrid.clone()
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if self.irq_counter == 0 || self.irq_reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_pending = true;
        }
    }
}

/// Waixing FS309 (mapper 199): MMC3 plus four extra registers.
///
/// `$8001` writes with select bit 3 set land in the extra registers instead of
/// the normal MMC3 bank registers. The extra registers then force 8 KiB PRG
/// slots 2/3 (`$C000`/`$E000`) and 1 KiB CHR slots 1/3, while slots 0/2 come
/// from the normal R0/R1 registers. `$A000` selects one of four mirroring
/// modes. All local images use 8 KiB CHR-RAM.
#[derive(Debug)]
pub struct Mmc3_199 {
    inner: Mmc3,
    ex: [u8; 4],
    /// 4 KiB PRG-RAM window at `$5000-$5FFF` (as on the Waixing FS303/FS309
    /// class boards). The local FS309 games copy an NMI handler there and jump
    /// to it, so reads/writes must hit this RAM instead of open bus.
    low_ram: [u8; 0x1000],
}

impl Mmc3_199 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            inner: Mmc3::new_as(prg, chr, header, 199),
            ex: [0xFE, 0xFF, 1, 3],
            low_ram: [0; 0x1000],
        }
    }

    fn prg8_banks(&self) -> usize {
        (self.inner.prg.len() / 0x2000).max(1)
    }

    fn chr1_banks(&self) -> usize {
        (self.inner.chr.len() / 0x400).max(1)
    }

    fn chr_bank_199(&self, slot: usize) -> usize {
        let raw = match slot {
            0 => usize::from(self.inner.bank_data[0] & 0xFE),
            1 => usize::from(self.ex[2]),
            2 => usize::from(self.inner.bank_data[1] & 0xFE),
            3 => usize::from(self.ex[3]),
            _ => return self.inner.chr_bank(slot),
        };
        raw % self.chr1_banks()
    }
}

impl Mapper for Mmc3_199 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x5000..=0x5FFF => self.low_ram[usize::from(address - 0x5000)],
            0xC000..=0xDFFF => {
                let bank = usize::from(self.ex[0]) % self.prg8_banks();
                self.inner.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xE000..=0xFFFF => {
                let bank = usize::from(self.ex[1]) % self.prg8_banks();
                self.inner.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => self.inner.prg_read(address),
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x5000..=0x5FFF).contains(&address) {
            self.low_ram[usize::from(address - 0x5000)] = value;
            return;
        }
        if address >= 0x8000 {
            let decoded = address & 0xE001;
            if decoded == 0xA000 {
                self.inner.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                };
                return;
            }
            if decoded == 0x8001 && self.inner.select & 0x08 != 0 {
                self.ex[usize::from(self.inner.select & 0x03)] = value;
                return;
            }
        }
        self.inner.prg_write(address, value);
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank_199(slot);
        self.inner.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank_199(slot);
        if let Some(byte) = self
            .inner
            .chr
            .get_mut(bank * 0x400 + usize::from(address & 0x3FF))
        {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.inner.mirroring
    }

    fn number(&self) -> u16 {
        199
    }

    fn prg_ram(&self) -> &[u8] {
        &self.inner.prg_ram
    }

    fn low_window(&self) -> Option<(u16, u16)> {
        Some((0x5000, 0x5FFF))
    }

    fn irq_line(&self) -> bool {
        self.inner.irq_line()
    }

    fn clock_scanline(&mut self) {
        self.inner.clock_scanline();
    }

    fn acknowledge_irq(&mut self) {
        self.inner.acknowledge_irq();
    }
}

/// Dragon Ninja / CTC-31 (mapper 222): eight 1 KiB CHR banks and two
/// switchable 8 KiB PRG banks with the last 16 KiB fixed, plus an IRQ that
/// counts A12 rises (one per rendered scanline) and fires after 240.
#[derive(Debug)]
pub struct Ctc222 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_banks: [u8; 2],
    chr_banks: [u8; 8],
    mirroring: Mirroring,
    irq_counter: u16,
    irq_pending: bool,
}

impl Ctc222 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_banks: [0, 1],
            chr_banks: [0; 8],
            mirroring: header.mirroring,
            irq_counter: 0,
            irq_pending: false,
        }
    }

    fn prg8_banks(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_banks(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }
}

impl Mapper for Ctc222 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = self.prg8_banks();
        let bank = match address {
            0x8000..=0x9FFF => usize::from(self.prg_banks[0]) % banks,
            0xA000..=0xBFFF => usize::from(self.prg_banks[1]) % banks,
            0xC000..=0xDFFF => banks.saturating_sub(2),
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xF003 {
            0x8000 => self.prg_banks[0] = value,
            0x9000 => {
                self.mirroring = if value & 0x01 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xA000 => self.prg_banks[1] = value,
            0xB000 => self.chr_banks[0] = value,
            0xB002 => self.chr_banks[1] = value,
            0xC000 => self.chr_banks[2] = value,
            0xC002 => self.chr_banks[3] = value,
            0xD000 => self.chr_banks[4] = value,
            0xD002 => self.chr_banks[5] = value,
            0xE000 => self.chr_banks[6] = value,
            0xE002 => self.chr_banks[7] = value,
            0xF000 => {
                self.irq_counter = u16::from(value);
                self.irq_pending = false;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) >> 10 & 0x07;
        let bank = usize::from(self.chr_banks[slot]) % self.chr_banks();
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, _address: u16, _value: u8) {
        // CHR-ROM board.
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        222
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        // Each A12 rise (one per rendered scanline) advances the counter; after
        // 240 it fires and restarts. A zero reload disables counting.
        if self.irq_counter != 0 {
            self.irq_counter += 1;
            if self.irq_counter >= 240 {
                self.irq_pending = true;
                self.irq_counter = 0;
            }
        }
    }
}

/// Waixing educational cartridges (mapper 164): a 32 KiB PRG bank selected by
/// two nibble writes in `$5000-$51FF`, plus 8 KiB CHR-RAM.
#[derive(Debug)]
pub struct Waixing164 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: u8,
    mirroring: Mirroring,
}

impl Waixing164 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            // Mesen powers the board with the last 32 KiB bank selected.
            prg_bank: 0x0F,
            mirroring: header.mirroring,
        }
    }

    fn banks(&self) -> usize {
        (self.prg.len() / 0x8000).max(1)
    }
}

impl Mapper for Waixing164 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let bank = usize::from(self.prg_bank) % self.banks();
        self.prg[bank * 0x8000 + usize::from(address - 0x8000)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0x7300 {
            0x5000 => self.prg_bank = (self.prg_bank & 0xF0) | (value & 0x0F),
            0x5100 => self.prg_bank = (self.prg_bank & 0x0F) | ((value & 0x0F) << 4),
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr[usize::from(address) & 0x1FFF]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        self.chr[usize::from(address) & 0x1FFF] = value;
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        164
    }
}

/// Maxi 15 multicart (mapper 234): two latches at `$FF80-$FF9F` and
/// `$FFE8-$FFF8`, 32 KiB PRG / 8 KiB CHR pages, bus conflicts, and a
/// CNROM/NINA-03 dual banking mode selected by bit 6 of the first latch.
///
/// Register reads are side-effecting (the ROM byte on the bus is latched), as
/// the menu uses dummy reads to select a game.
#[derive(Debug)]
pub struct Maxi15 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    regs: [u8; 2],
    mirroring: Mirroring,
}

impl Maxi15 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, _header: Header) -> Self {
        Self {
            prg,
            chr,
            regs: [0, 0],
            // Power-on regs are zero, which selects vertical mirroring (Mesen
            // calls UpdateState in InitMapper).
            mirroring: Mirroring::Vertical,
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x8000).max(1)
    }

    fn chr_banks(&self) -> usize {
        (self.chr.len() / 0x2000).max(1)
    }

    fn prg_page(&self) -> usize {
        let page = if self.regs[0] & 0x40 != 0 {
            // NINA-03 mode.
            usize::from((self.regs[0] & 0x0E) | (self.regs[1] & 0x01))
        } else {
            usize::from(self.regs[0] & 0x0F)
        };
        page % self.prg_banks()
    }

    fn chr_page(&self) -> usize {
        let page = if self.regs[0] & 0x40 != 0 {
            usize::from((self.regs[0] << 2) & 0x38) | (usize::from(self.regs[1] >> 4) & 0x07)
        } else {
            usize::from((self.regs[0] << 2) & 0x3C) | (usize::from(self.regs[1] >> 4) & 0x03)
        };
        page % self.chr_banks()
    }

    fn update(&mut self) {
        self.mirroring = if self.regs[0] & 0x80 != 0 {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        };
    }

    /// The ROM byte currently on the PRG bus at `address` (bus conflicts and
    /// register reads both latch this).
    fn prg_byte(&self, address: u16) -> u8 {
        let bank = self.prg_page();
        self.prg[bank * 0x8000 + usize::from(address.wrapping_sub(0x8000))]
    }

    fn latch(&mut self, address: u16, value: u8) {
        if (0xFF80..=0xFF9F).contains(&address) {
            if self.regs[0] & 0x3F == 0 {
                self.regs[0] = value;
                self.update();
            }
        } else if (0xFFE8..=0xFFF8).contains(&address) {
            self.regs[1] = value & 0x71;
            self.update();
        }
    }
}

impl Mapper for Maxi15 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        self.prg_byte(address)
    }

    fn read_io(&mut self, address: u16) -> u8 {
        if (0xFF80..=0xFF9F).contains(&address) || (0xFFE8..=0xFFF8).contains(&address) {
            let value = self.prg_byte(address);
            self.latch(address, value);
            value
        } else {
            self.prg_read(address)
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0xFF80..=0xFF9F).contains(&address) || (0xFFE8..=0xFFF8).contains(&address) {
            // Bus conflict: the cartridge drives the ROM byte, ANDing the write.
            let value = value & self.prg_byte(address);
            self.latch(address, value);
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let bank = self.chr_page();
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, _address: u16, _value: u8) {
        // Maxi 15 boards use CHR-ROM; nothing is writable.
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        234
    }
}

/// Waixing/Myth Struggle board (mapper 249): MMC3 plus a `$5000` register that
/// scrambles the CHR and PRG page numbers when bit 1 is set.
#[derive(Debug)]
pub struct Mmc3_249 {
    inner: Mmc3,
    ex: u8,
}

impl Mmc3_249 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            inner: Mmc3::new_as(prg, chr, header, 249),
            ex: 0,
        }
    }

    fn prg8_banks(&self) -> usize {
        (self.inner.prg.len() / 0x2000).max(1)
    }

    fn chr1_banks(&self) -> usize {
        (self.inner.chr.len() / 0x400).max(1)
    }

    /// The board's 8-bit CHR/PRG page permutation (Mesen `MMC3_249`).
    fn permute(page: usize) -> usize {
        (page & 0x03)
            | ((page >> 1) & 0x04)
            | ((page >> 4) & 0x08)
            | ((page >> 2) & 0x10)
            | ((page << 3) & 0x20)
            | ((page << 2) & 0xC0)
    }

    fn scramble_prg(&self, page: usize) -> usize {
        if self.ex & 0x02 == 0 {
            return page;
        }
        let scrambled = if page < 0x20 {
            (page & 0x01) | ((page >> 3) & 0x02) | ((page >> 1) & 0x04) | ((page << 2) & 0x18)
        } else {
            Self::permute(page - 0x20)
        };
        scrambled % self.prg8_banks()
    }

    fn scramble_chr(&self, page: usize) -> usize {
        if self.ex & 0x02 == 0 {
            return page % self.chr1_banks();
        }
        Self::permute(page) % self.chr1_banks()
    }
}

impl Mapper for Mmc3_249 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return self.inner.prg_read(address);
        }
        let slot = usize::from((address - 0x8000) / 0x2000);
        let bank = self.scramble_prg(self.inner.prg_slot(slot));
        self.inner.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address == 0x5000 {
            self.ex = value;
            return;
        }
        self.inner.prg_write(address, value);
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) / 0x400;
        let bank = self.scramble_chr(self.inner.chr_bank(slot));
        self.inner.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address) / 0x400;
        let bank = self.scramble_chr(self.inner.chr_bank(slot));
        if let Some(byte) = self
            .inner
            .chr
            .get_mut(bank * 0x400 + usize::from(address & 0x3FF))
        {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.inner.mirroring()
    }

    fn number(&self) -> u16 {
        249
    }

    fn prg_ram(&self) -> &[u8] {
        &self.inner.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.inner.irq_line()
    }

    fn clock_scanline(&mut self) {
        self.inner.clock_scanline();
    }

    fn acknowledge_irq(&mut self) {
        self.inner.acknowledge_irq();
    }
}

/// Waixing T9552 / VRC4-derived board (mapper 253).
///
/// 8 KiB PRG pages at `$8000`/`$A000` (`$8010`/`$A010`) with the last two pages
/// fixed at `$C000`/`$E000`; eight 1 KiB CHR pages whose 16-bit bank registers
/// are assembled from low/high nibble writes across `$B000-$E00C`; four
/// mirroring modes at `$9400`; and a CPU-clock IRQ with a /114 prescaler
/// (`$F000`/`$F004` reload, `$F008` enable). A 2 KiB CHR-RAM page pair is
/// selected while a CHR low nibble is 4 or 5 and the game has not forced
/// CHR-ROM with the `$C8`/`$88` signature on slot 0.
#[derive(Debug)]
pub struct Waixing253 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_lo: [u8; 8],
    chr_hi: [u8; 8],
    chr_ram: [u8; 0x800],
    force_chr_rom: bool,
    prg_banks: [u8; 2],
    mirroring: Mirroring,
    irq_reload: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_scaler: u16,
    irq_pending: bool,
}

impl Waixing253 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            chr_lo: [0; 8],
            chr_hi: [0; 8],
            chr_ram: [0; 0x800],
            force_chr_rom: false,
            prg_banks: [0, 1],
            mirroring: header.mirroring,
            irq_reload: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_scaler: 0,
            irq_pending: false,
        }
    }

    fn prg8_banks(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr1_banks(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }

    fn chr_page(&self, slot: usize) -> usize {
        usize::from(self.chr_lo[slot]) | (usize::from(self.chr_hi[slot]) << 8)
    }

    fn chr_is_ram(&self, slot: usize) -> bool {
        !self.force_chr_rom && matches!(self.chr_lo[slot], 4 | 5)
    }

    fn write_chr_nibble(&mut self, address: u16, value: u8) {
        let slot = usize::from(((((address & 0x08) | (address >> 8)) >> 3) + 2) & 0x07);
        let shift = address & 0x04;
        let mask = (0xF0u16 >> shift) as u8;
        self.chr_lo[slot] = (self.chr_lo[slot] & mask) | (value << shift);
        if shift != 0 {
            self.chr_hi[slot] = value >> 4;
        }
        if slot == 0 {
            if self.chr_lo[0] == 0xC8 {
                self.force_chr_rom = false;
            } else if self.chr_lo[0] == 0x88 {
                self.force_chr_rom = true;
            }
        }
    }
}

impl Mapper for Waixing253 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = self.prg8_banks();
        let bank = match address {
            0x8000..=0x9FFF => usize::from(self.prg_banks[0]) % banks,
            0xA000..=0xBFFF => usize::from(self.prg_banks[1]) % banks,
            0xC000..=0xDFFF => banks.saturating_sub(2),
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x8010 => self.prg_banks[0] = value,
            0xA010 => self.prg_banks[1] = value,
            0x9400 => {
                self.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                };
            }
            0xF000 => {
                self.irq_reload = (self.irq_reload & 0xF0) | (value & 0x0F);
                self.irq_pending = false;
            }
            0xF004 => {
                self.irq_reload = (self.irq_reload & 0x0F) | (value << 4);
                self.irq_pending = false;
            }
            0xF008 => {
                self.irq_counter = self.irq_reload;
                self.irq_enabled = value & 0x02 != 0;
                self.irq_scaler = 0;
                self.irq_pending = false;
            }
            0xB000..=0xE00C => self.write_chr_nibble(address, value),
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) >> 10 & 0x07;
        let page = self.chr_page(slot);
        let offset = usize::from(address & 0x3FF);
        if self.chr_is_ram(slot) {
            self.chr_ram[(page & 0x01) * 0x400 + offset]
        } else {
            let bank = page % self.chr1_banks();
            self.chr[bank * 0x400 + offset]
        }
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address) >> 10 & 0x07;
        if self.chr_is_ram(slot) {
            let page = self.chr_page(slot);
            self.chr_ram[(page & 0x01) * 0x400 + usize::from(address & 0x3FF)] = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        253
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if !self.irq_enabled {
            return;
        }
        self.irq_scaler += 1;
        if self.irq_scaler >= 114 {
            self.irq_scaler = 0;
            self.irq_counter = self.irq_counter.wrapping_add(1);
            if self.irq_counter == 0 {
                self.irq_counter = self.irq_reload;
                self.irq_pending = true;
            }
        }
    }
}

/// Board variant for the VRC2/VRC4 family. The mapper chip is the same; the
/// PCBs wire different CPU address lines to the chip's register select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VrcVariant {
    Vrc2a,
    Vrc2b,
    Vrc2c,
    Vrc4a,
    Vrc4b,
    Vrc4c,
    Vrc4d,
    Vrc4e,
    Vrc4f,
}

impl VrcVariant {
    const fn is_vrc4(self) -> bool {
        matches!(
            self,
            Self::Vrc4a | Self::Vrc4b | Self::Vrc4c | Self::Vrc4d | Self::Vrc4e | Self::Vrc4f
        )
    }
}

/// VRC2/VRC4 (mappers 21/22/23/25): 8 KiB PRG and 1 KiB CHR banking.
///
/// `$8000`/`$A000` are switchable 8 KiB PRG banks; `$C000` is fixed to the
/// second-to-last bank and `$E000` to the last (VRC4 swap mode exchanges
/// `$8000` and `$C000`). CHR is eight 1 KiB banks written as low/high nibble
/// pairs, and the VRC4 IRQ runs on the CPU cycle clock.
#[derive(Debug)]
pub struct Vrc {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_banks: [usize; 2],
    chr_banks: [u16; 8],
    mirroring: Mirroring,
    variant: VrcVariant,
    swap_mode: bool,
    wram_enable: bool,
    vrc2_latch: u8,
    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_pending: bool,
    number: u16,
}

impl Vrc {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        let variant = match header.mapper {
            22 => VrcVariant::Vrc2a,
            23 => match header.submapper {
                1 => VrcVariant::Vrc4f,
                3 => VrcVariant::Vrc2b,
                _ => VrcVariant::Vrc4e,
            },
            25 => match header.submapper {
                2 => VrcVariant::Vrc4d,
                3 => VrcVariant::Vrc2c,
                _ => VrcVariant::Vrc4b,
            },
            _ => {
                if header.submapper == 2 {
                    VrcVariant::Vrc4c
                } else {
                    VrcVariant::Vrc4a
                }
            }
        };
        Self {
            prg,
            chr,
            prg_ram,
            prg_banks: [0, 1],
            chr_banks: [0, 1, 2, 3, 4, 5, 6, 7],
            mirroring: header.mirroring,
            variant,
            swap_mode: false,
            wram_enable: false,
            vrc2_latch: 0,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            number: header.mapper,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }

    /// Sub-register index selected by the board's address-line wiring.
    fn register_index(&self, address: u16) -> usize {
        match self.variant {
            VrcVariant::Vrc2a | VrcVariant::Vrc2c => {
                usize::from((address & 1) << 1 | ((address >> 1) & 1))
            }
            VrcVariant::Vrc4a => usize::from((address >> 1) & 3),
            VrcVariant::Vrc4c => usize::from((address >> 6) & 3),
            VrcVariant::Vrc4d => usize::from(((address >> 3) & 1) | (((address >> 2) & 1) << 1)),
            VrcVariant::Vrc4e => usize::from(((address >> 2) & 1) | (((address >> 3) & 1) << 1)),
            VrcVariant::Vrc2b | VrcVariant::Vrc4b | VrcVariant::Vrc4f => usize::from(address & 3),
        }
    }
}

impl Mapper for Vrc {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x6000 {
            return 0;
        }
        if address < 0x8000 {
            if self.variant.is_vrc4() {
                if !self.wram_enable || self.prg_ram.is_empty() {
                    return 0;
                }
                let offset = usize::from(address - 0x6000) % self.prg_ram.len();
                return self.prg_ram[offset];
            }
            // VRC2 exposes a one-bit latch in $6000-$6FFF and open bus above.
            if address < 0x7000 {
                return 0x60 | self.vrc2_latch;
            }
            return 0;
        }
        let banks = self.prg_bank_count();
        let slot = usize::from((address - 0x8000) / 0x2000);
        let bank = match slot {
            0 => {
                if self.swap_mode {
                    banks.saturating_sub(2)
                } else {
                    self.prg_banks[0]
                }
            }
            1 => self.prg_banks[1],
            2 => {
                if self.swap_mode {
                    self.prg_banks[0]
                } else {
                    banks.saturating_sub(2)
                }
            }
            _ => banks - 1,
        } % banks;
        self.prg
            .get(bank * 0x2000 + usize::from(address & 0x1FFF))
            .copied()
            .unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x6000 {
            return;
        }
        if address < 0x8000 {
            if !self.variant.is_vrc4() {
                if address < 0x7000 {
                    self.vrc2_latch = value & 1;
                }
                return;
            }
            if self.wram_enable && !self.prg_ram.is_empty() {
                let offset = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[offset] = value;
            }
            return;
        }
        let page = (address >> 12) & 0x0F;
        let reg = self.register_index(address);
        match page {
            0x8 => {
                self.prg_banks[0] = usize::from(value & 0x1F);
            }
            0x9 => match (self.variant.is_vrc4(), reg) {
                (true, 0) | (false, _) => {
                    self.mirroring = match value & 3 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::SingleLower,
                        _ => Mirroring::SingleUpper,
                    };
                }
                (true, 2) => {
                    self.wram_enable = value & 1 != 0;
                    self.swap_mode = value & 2 != 0;
                }
                _ => {}
            },
            0xA => {
                self.prg_banks[1] = usize::from(value & 0x1F);
            }
            0xB..=0xE => {
                let index = usize::from(page - 0xB) * 2 + (reg >> 1);
                if reg & 1 == 0 {
                    self.chr_banks[index] =
                        (self.chr_banks[index] & 0x1F0) | u16::from(value & 0x0F);
                } else {
                    self.chr_banks[index] =
                        (self.chr_banks[index] & 0x00F) | (u16::from(value & 0x1F) << 4);
                }
            }
            0xF => match reg {
                0 => self.irq_latch = (self.irq_latch & 0xF0) | (value & 0x0F),
                1 => self.irq_latch = (self.irq_latch & 0x0F) | ((value & 0x0F) << 4),
                2 => self.irq_enabled = value & 0x02 != 0,
                3 => {
                    self.irq_pending = false;
                    self.irq_enabled = value & 0x01 != 0;
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = self.chr_bank_count();
        let slot = usize::from(address) / 0x400;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr
            .get(bank * 0x400 + usize::from(address & 0x3FF))
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = self.chr_bank_count();
        let slot = usize::from(address) / 0x400;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.number
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if self.irq_pending {
            return;
        }
        if self.irq_counter == 0 {
            self.irq_counter = self.irq_latch;
            if self.irq_enabled {
                self.irq_pending = true;
            }
        } else {
            self.irq_counter -= 1;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// MMC5 length-counter table (7-bit counts, lowest bit always zero).
const MMC5_LENGTH: [u8; 32] = [
    0x0A, 0xFE, 0x14, 0x02, 0x28, 0x04, 0x50, 0x06, 0xA0, 0x08, 0x3C, 0x0A, 0x0E, 0x0C, 0x1A, 0x0E,
    0x0C, 0x10, 0x18, 0x12, 0x30, 0x14, 0x60, 0x16, 0xC0, 0x18, 0x48, 0x1A, 0x10, 0x1C, 0x20, 0x1E,
];

/// APU-style pulse duty sequences (ones-complement for duty 2/3).
const MMC5_DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [0, 1, 0, 0, 1, 1, 1, 1],
];

/// One MMC5 pulse channel.
#[derive(Debug, Default, Clone)]
struct Mmc5Pulse {
    duty: u8,
    halt: bool,
    constant: bool,
    volume: u8,
    timer_period: u16,
    timer: u16,
    step: u8,
    length: u8,
    envelope_start: bool,
    envelope_divider: u8,
    envelope_volume: u8,
}

impl Mmc5Pulse {
    fn write(&mut self, register: u16, value: u8) {
        match register {
            0 => {
                self.duty = value >> 6;
                self.halt = value & 0x20 != 0;
                self.constant = value & 0x10 != 0;
                self.volume = value & 0x0F;
            }
            2 => self.timer_period = (self.timer_period & 0x0700) | u16::from(value),
            3 => {
                self.timer_period = (self.timer_period & 0x00FF) | (u16::from(value & 0x07) << 8);
                let index = usize::from(value >> 3) & 0x1F;
                self.length = MMC5_LENGTH[index];
                self.envelope_start = true;
                self.step = 0;
            }
            _ => {}
        }
    }

    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.step = (self.step + 1) & 0x07;
        } else {
            self.timer -= 1;
        }
    }

    /// Envelope and length run at the fixed ~240 Hz MMC5 rate.
    fn clock_frame(&mut self) {
        let period = self.volume;
        if self.envelope_start {
            self.envelope_start = false;
            self.envelope_divider = period;
            self.envelope_volume = 15;
        } else if self.envelope_divider == 0 {
            self.envelope_divider = period;
            if self.envelope_volume > 0 {
                self.envelope_volume -= 1;
            } else if self.halt {
                self.envelope_volume = 15;
            }
        } else {
            self.envelope_divider -= 1;
        }
        if !self.halt && self.length > 0 {
            self.length -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.length == 0 {
            return 0;
        }
        let volume = if self.constant {
            self.volume
        } else {
            self.envelope_volume
        };
        if MMC5_DUTY[usize::from(self.duty & 0x03)][usize::from(self.step)] != 0 {
            volume
        } else {
            0
        }
    }
}

/// MMC5 expansion audio: two pulses and the 8-bit PCM DAC.
#[derive(Debug, Default, Clone)]
struct Mmc5Audio {
    pulse1: Mmc5Pulse,
    pulse2: Mmc5Pulse,
    pcm: u8,
    frame_divider: u32,
    /// `$5010` bit 0: 1 = PCM read mode, 0 = write mode.
    pcm_read_mode: bool,
    /// `$5010` bit 7: PCM IRQ enable.
    pcm_irq_enable: bool,
    /// Set when a zero DAC update is seen; cleared by reading `$5010`.
    pcm_irq_trip: bool,
}

impl Mmc5Audio {
    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x5003 => self.pulse1.write(address - 0x5000, value),
            0x5004..=0x5007 => self.pulse2.write(address - 0x5004, value),
            0x5010 => {
                self.pcm_read_mode = value & 0x01 != 0;
                self.pcm_irq_enable = value & 0x80 != 0;
            }
            // Direct writes are ignored in PCM read mode; the DAC is fed by
            // PRG reads in that mode.
            0x5011 if !self.pcm_read_mode => self.dac_update(value),
            _ => {}
        }
    }

    /// A DAC update from `$5011` (write mode) or a `$8000-$BFFF` read (read
    /// mode). A zero value leaves the DAC unchanged and trips the PCM IRQ.
    fn dac_update(&mut self, value: u8) {
        if value == 0 {
            self.pcm_irq_trip = true;
        } else {
            self.pcm = value;
            self.pcm_irq_trip = false;
        }
    }

    /// `$5010` read: bit 7 is the latched IRQ, bit 0 reports MMC5A. Reading
    /// acknowledges (clears) the trip.
    fn read_mode_irq(&mut self) -> u8 {
        let irq = u8::from(self.pcm_irq_trip && self.pcm_irq_enable);
        self.pcm_irq_trip = false;
        (irq << 7) | 0x01
    }

    fn pcm_read_mode(&self) -> bool {
        self.pcm_read_mode
    }

    fn irq_active(&self) -> bool {
        self.pcm_irq_trip && self.pcm_irq_enable
    }

    fn clock(&mut self) {
        self.pulse1.clock_timer();
        self.pulse2.clock_timer();
        self.frame_divider += 1;
        // ~241 Hz: the MMC5 clocks envelope and length at M2/7424.
        if self.frame_divider >= 7424 {
            self.frame_divider = 0;
            self.pulse1.clock_frame();
            self.pulse2.clock_frame();
        }
    }

    fn mix(&self) -> f32 {
        let pulses = u16::from(self.pulse1.output()) + u16::from(self.pulse2.output());
        // PCM is silenced while the chip sources samples back to the CPU in
        // read mode. The scale matches the reference: pulses share the 2A03
        // DAC gain (650 per pulse unit), the 7-bit PCM is ~half that (40 per
        // unit), biased to centre the AC signal.
        let pcm = if self.pcm_read_mode {
            0
        } else {
            u16::from(self.pcm & 0x7F)
        };
        let raw = i32::from(pulses) * 650 + i32::from(pcm) * 40 - 12_290;
        raw as f32 / 65_536.0
    }
}

/// MMC5 (mapper 5), partial: PRG/CHR banking, mirroring and scanline IRQ.
///
/// ExRAM, split-screen mode, the multiplier and expansion audio are not
/// implemented yet.
#[derive(Debug)]
pub struct Mmc5 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    exram: [u8; 1024],
    prg_mode: u8,
    chr_mode: u8,
    nt_map: [u8; 4],
    fill_tile: u8,
    fill_attr: u8,
    aux: Option<Rc<RefCell<Mmc5Aux>>>,
    mirroring: Mirroring,
    prg_banks: [u8; 4],
    chr_banks: [u8; 12],
    ppu_ctrl: u8,
    mul_a: u8,
    mul_b: u8,
    protect1: u8,
    protect2: u8,
    exram_mode: u8,
    prg_ram_bank: u8,
    chr_upper: u8,
    split_enabled: bool,
    split_right: bool,
    split_threshold: u8,
    split_scroll: u8,
    split_bank: u8,
    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_pending: bool,
    audio: Mmc5Audio,
}

impl Mmc5 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        let aux = Rc::new(RefCell::new(Mmc5Aux::new(
            chr.clone(),
            header.chr_size == 0,
        )));
        let mmc5 = Self {
            prg,
            chr,
            prg_ram: vec![0; 64 * 1024],
            exram: [0; 1024],
            nt_map: [0; 4],
            fill_tile: 0,
            fill_attr: 0,
            aux: Some(aux),
            // MMC5 power-on: PRG mode 3, $5114-$5116 = 0 and $5117 = $FF,
            // so the last bank is visible at $E000 and the reset vector works.
            prg_mode: 3,
            chr_mode: 3,
            mirroring: header.mirroring,
            prg_banks: [0, 0, 0, 0xFF],
            chr_banks: [0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0],
            ppu_ctrl: 0,
            mul_a: 0,
            mul_b: 0,
            protect1: 0x01,
            protect2: 0x02,
            exram_mode: 0x03,
            prg_ram_bank: 0,
            chr_upper: 0,
            split_enabled: false,
            split_right: false,
            split_threshold: 0,
            split_scroll: 0,
            split_bank: 0,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            audio: Mmc5Audio::default(),
        };
        mmc5.refresh_aux();
        mmc5
    }

    fn refresh_aux(&self) {
        let Some(aux) = &self.aux else {
            return;
        };
        let mut aux = aux.borrow_mut();
        aux.set_registers(
            self.exram_mode,
            self.nt_map,
            self.fill_tile,
            self.fill_attr,
            self.chr_upper,
        );
        aux.set_split(
            self.split_enabled,
            self.split_right,
            self.split_threshold,
            self.split_scroll,
            self.split_bank,
        );
        let mut slots = [0usize; 8];
        let mut bg_slots = [0usize; 8];
        for (slot, value) in slots.iter_mut().enumerate() {
            *value = self.chr_bank(slot);
        }
        if self.ppu_ctrl & 0x20 != 0 {
            // 8x16 sprites: $5128-$512B apply to background tiles. Each
            // register covers the corresponding slot in both pattern tables.
            for (slot, value) in bg_slots.iter_mut().enumerate() {
                let register = 8 + (slot & 0x03);
                let bank = ((usize::from(self.chr_upper) << 8)
                    | usize::from(self.chr_banks[register]))
                    % self.chr_bank_count();
                *value = bank;
            }
        } else {
            bg_slots = slots;
        }
        aux.set_slots(slots);
        aux.set_bg_slots(bg_slots);
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }

    fn prg_offset(&self, address: u16) -> usize {
        let banks = self.prg_bank_count();
        let slot = usize::from((address - 0x8000) / 0x2000);
        let bank = match self.prg_mode & 3 {
            // Mode 0: 32 KiB from $5117.
            0 => (usize::from(self.prg_banks[3]) & !3) + slot,
            // Mode 1: 16 KiB from $5115, 16 KiB from $5117.
            1 => {
                if slot < 2 {
                    (usize::from(self.prg_banks[1]) & !1) + slot
                } else {
                    (usize::from(self.prg_banks[3]) & !1) + (slot - 2)
                }
            }
            // Mode 2: 16 KiB from $5115, $C000 from $5116, $E000 from $5117.
            2 => {
                if slot < 2 {
                    (usize::from(self.prg_banks[1]) & !1) + slot
                } else {
                    usize::from(self.prg_banks[slot])
                }
            }
            // Mode 3: $5114-$5117.
            _ => usize::from(self.prg_banks[slot]),
        } % banks;
        bank * 0x2000 + usize::from(address & 0x1FFF)
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = self.chr_bank_count();
        let base = match self.chr_mode & 3 {
            0 => (usize::from(self.chr_banks[0]) & !7) + slot,
            1 => {
                if slot < 4 {
                    (usize::from(self.chr_banks[0]) & !3) + slot
                } else {
                    (usize::from(self.chr_banks[4]) & !3) + (slot - 4)
                }
            }
            2 => {
                let group = slot / 2;
                (usize::from(self.chr_banks[group * 2]) & !1) + (slot & 1)
            }
            _ => usize::from(self.chr_banks[slot]),
        };
        ((usize::from(self.chr_upper) << 8) | (base & 0xFF)) % banks
    }

    /// PRG-RAM window for mode 3 registers with bit 7 set.
    fn ram_window(&self, address: u16) -> Option<usize> {
        let slot = usize::from((address - 0x8000) / 0x2000);
        if (self.prg_mode & 3) != 3 || slot == 3 {
            return None;
        }
        let value = self.prg_banks[slot];
        if value & 0x80 != 0 {
            return None;
        }
        Some(usize::from(value & 0x07) * 0x2000 + usize::from(address & 0x1FFF))
    }

    fn write_register(&mut self, address: u16, value: u8) {
        match address {
            0x5100 => self.prg_mode = value & 3,
            0x5101 => {
                self.chr_mode = value & 3;
                self.refresh_aux();
            }
            0x5102 => self.protect1 = value & 0x03,
            0x5103 => self.protect2 = value & 0x03,
            0x5104 => {
                self.exram_mode = value & 0x03;
                self.refresh_aux();
            }
            0x5105 => {
                self.nt_map = [
                    value & 3,
                    (value >> 2) & 3,
                    (value >> 4) & 3,
                    (value >> 6) & 3,
                ];
                let nt = self.nt_map;
                self.mirroring = if nt[0] == nt[2] && nt[1] == nt[3] && nt[0] != nt[1] {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
                self.refresh_aux();
            }
            0x5106 => {
                self.fill_tile = value;
                self.refresh_aux();
            }
            0x5107 => {
                self.fill_attr = value;
                self.refresh_aux();
            }
            0x5113 => self.prg_ram_bank = value & 0x07,
            0x5200 => {
                self.split_enabled = value & 0x80 != 0;
                self.split_right = value & 0x40 != 0;
                self.split_threshold = value & 0x1F;
                self.refresh_aux();
            }
            0x5201 => {
                self.split_scroll = value;
                self.refresh_aux();
            }
            0x5202 => {
                self.split_bank = value;
                self.refresh_aux();
            }
            0x5114..=0x5117 => {
                let index = usize::from(address - 0x5114);
                self.prg_banks[index] = value;
            }
            0x5120..=0x512B => {
                let index = usize::from(address - 0x5120);
                self.chr_banks[index] = value;
                self.refresh_aux();
            }
            0x5205 => self.mul_a = value,
            0x5206 => self.mul_b = value,
            0x5130 => {
                self.chr_upper = value & 0x03;
                self.refresh_aux();
            }
            0x5000..=0x5007 | 0x5010 | 0x5011 => self.audio.write(address, value),
            0x5203 => self.irq_latch = value,
            0x5204 => {
                if value & 0x80 != 0 {
                    self.irq_enabled = true;
                    self.irq_counter = self.irq_latch;
                    self.irq_pending = false;
                } else {
                    self.irq_enabled = false;
                    self.irq_pending = false;
                }
            }
            0x5C00..=0x5FFF => self.exram[usize::from(address - 0x5C00)] = value,
            _ => {}
        }
    }
}

impl Mapper for Mmc5 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x5204 => {
                // In-frame is not tracked without PPU integration; expose only
                // the pending IRQ flag.
                (u8::from(self.irq_pending)) << 6
            }
            0x5205 => (u16::from(self.mul_a) * u16::from(self.mul_b)) as u8,
            0x5206 => ((u16::from(self.mul_a) * u16::from(self.mul_b)) >> 8) as u8,
            0x5C00..=0x5FFF => match self.exram_mode {
                0 | 1 => 0,
                _ => {
                    let index = usize::from(address - 0x5C00);
                    if let Some(aux) = &self.aux {
                        aux.borrow().exram[index]
                    } else {
                        self.exram[index]
                    }
                }
            },
            0x5000..=0x5FFF => 0,
            0x6000..=0x7FFF => {
                let offset =
                    usize::from(self.prg_ram_bank) * 0x2000 + usize::from(address - 0x6000);
                self.prg_ram[offset % self.prg_ram.len()]
            }
            0x8000..=0xFFFF => {
                if let Some(ram) = self.ram_window(address) {
                    return self.prg_ram[ram % self.prg_ram.len()];
                }
                self.prg[self.prg_offset(address)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x5FFF => {
                if (0x5C00..=0x5FFF).contains(&address) {
                    if self.exram_mode != 3 {
                        let index = usize::from(address - 0x5C00);
                        self.exram[index] = value;
                        if let Some(aux) = &self.aux {
                            aux.borrow_mut().exram[index] = value;
                        }
                    }
                } else {
                    self.write_register(address, value);
                }
            }
            0x6000..=0x7FFF => {
                if self.protect1 == 0x02 && self.protect2 == 0x01 {
                    let index =
                        usize::from(self.prg_ram_bank) * 0x2000 + usize::from(address - 0x6000);
                    let len = self.prg_ram.len();
                    self.prg_ram[index % len] = value;
                }
            }
            0x8000..=0xFFFF => {}
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        if let Some(aux) = &self.aux {
            return aux.borrow().chr_read(address, &self.chr);
        }
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(aux) = &self.aux {
            if aux.borrow().is_ram() {
                let offset = aux.borrow().chr_offset(address);
                if let Some(byte) = self.chr.get_mut(offset) {
                    *byte = value;
                }
            }
            return;
        }
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        5
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn mmc5_aux(&self) -> Option<Rc<RefCell<Mmc5Aux>>> {
        self.aux.clone()
    }

    fn set_ppu_ctrl(&mut self, value: u8) {
        self.ppu_ctrl = value;
        self.refresh_aux();
    }

    fn read_io(&mut self, address: u16) -> u8 {
        // PCM read mode: every PRG-ROM read in `$8000-$BFFF` feeds the DAC;
        // a zero read trips the PCM IRQ until `$5010` is read.
        if (0x8000..=0xBFFF).contains(&address) && self.audio.pcm_read_mode() {
            let value = self.prg_read(address);
            self.audio.dac_update(value);
            return value;
        }
        if address == 0x5010 {
            return self.audio.read_mode_irq();
        }
        self.prg_read(address)
    }

    fn low_window(&self) -> Option<(u16, u16)> {
        Some((0x5000, 0x5FFF))
    }

    fn irq_line(&self) -> bool {
        self.irq_pending || self.audio.irq_active()
    }

    fn clock_scanline(&mut self) {
        if self.irq_enabled {
            if self.irq_counter == 0 {
                self.irq_pending = true;
            } else {
                self.irq_counter -= 1;
                if self.irq_counter == 0 {
                    self.irq_pending = true;
                }
            }
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }

    fn clock_cpu(&mut self) {
        self.audio.clock();
    }

    fn audio_mix(&self) -> f32 {
        self.audio.mix()
    }
}

/// Daou Infosys (mapper 156): 16 KiB PRG banking, eight 1 KiB CHR banks with
/// 16-bit bank registers split across low/high writes, 8 KiB work RAM and a
/// runtime vertical/horizontal mirroring select.
#[derive(Debug)]
pub struct Daou156 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_bank: u8,
    chr_lo: [u8; 8],
    chr_hi: [u8; 8],
    mirroring: Mirroring,
}

impl Daou156 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x2000)],
            prg_bank: 0,
            chr_lo: [0; 8],
            chr_hi: [0; 8],
            // DAOU/DIS23C01 powers on single-screen (lower nametable);
            // `$C014` selects vertical/horizontal at runtime.
            mirroring: Mirroring::SingleLower,
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x4000).max(1)
    }

    fn chr_banks(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }
}

impl Mapper for Daou156 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xBFFF => {
                let bank = usize::from(self.prg_bank) % self.prg_banks();
                self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
            }
            0xC000..=0xFFFF => {
                let bank = self.prg_banks() - 1;
                self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            // 16 CHR-bank-nibble registers at `$C000-$C00F`. The 1 KiB slot is
            // `(addr & 3) + (addr >= $C008 ? 4 : 0)`; bit 2 selects the high
            // nibble. This is the Mesen2 decode, not a flat lo/hi split.
            0xC000..=0xC00F => {
                let slot = usize::from((address & 0x03) + if address >= 0xC008 { 4 } else { 0 });
                if address & 0x04 != 0 {
                    self.chr_hi[slot] = value;
                } else {
                    self.chr_lo[slot] = value;
                }
            }
            0xC010 => self.prg_bank = value,
            0xC014 => {
                self.mirroring = if value & 0x01 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) >> 10 & 0x07;
        let bank = (usize::from(self.chr_lo[slot]) | (usize::from(self.chr_hi[slot]) << 8))
            % self.chr_banks();
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        // CHR-ROM boards ignore writes; CHR-RAM headers are handled by the
        // shared CHR-RAM path.
        if self.chr.is_empty() {
            return;
        }
        let slot = usize::from(address) >> 10 & 0x07;
        let bank = (usize::from(self.chr_lo[slot]) | (usize::from(self.chr_hi[slot]) << 8))
            % self.chr_banks();
        if let Some(byte) = self
            .chr
            .get_mut(bank * 0x400 + usize::from(address & 0x3FF))
        {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        156
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Color Dreams (mapper 11): 32 KiB PRG + 8 KiB CHR banking, bus conflicts.
#[derive(Debug)]
pub struct ColorDreams {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: usize,
    chr_bank: usize,
    mirroring: Mirroring,
}

impl ColorDreams {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for ColorDreams {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let offset = (self.prg_bank % banks) * 0x8000 + usize::from(address - 0x8000);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            return;
        }
        let offset = (self.prg_bank % (self.prg.len() / 0x8000).max(1)) * 0x8000
            + usize::from(address - 0x8000);
        let visible = self.prg.get(offset).copied().unwrap_or(0xFF);
        let value = value & visible;
        self.prg_bank = usize::from((value >> 4) & 3);
        self.chr_bank = usize::from(value & 3);
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        11
    }
}

/// CPROM (mapper 13): fixed 32 KiB PRG, 16 KiB CHR-RAM with a switchable
/// upper 4 KiB bank.
#[derive(Debug)]
pub struct Cprom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_bank: usize,
    mirroring: Mirroring,
}

impl Cprom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Cprom {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        self.prg
            .get(usize::from(address - 0x8000) % self.prg.len())
            .copied()
            .unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            self.chr_bank = usize::from((value >> 4) & 3);
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr.get(self.chr_offset(address)).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = self.chr_offset(address);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        13
    }
}

impl Cprom {
    fn chr_offset(&self, address: u16) -> usize {
        let index = usize::from(address) & 0x1FFF;
        if index < 0x1000 {
            index
        } else {
            (self.chr_bank % 4) * 0x1000 + (index - 0x1000)
        }
    }
}

/// GxROM (mapper 66): 32 KiB PRG + 8 KiB CHR banking.
#[derive(Debug)]
pub struct GxRom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: usize,
    chr_bank: usize,
    mirroring: Mirroring,
}

impl GxRom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for GxRom {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let offset = (self.prg_bank % banks) * 0x8000 + usize::from(address - 0x8000);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            self.prg_bank = usize::from((value >> 4) & 3);
            self.chr_bank = usize::from(value & 3);
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        66
    }
}

/// Camerica (mapper 71): 16 KiB switchable + fixed last, CHR-RAM.
#[derive(Debug)]
pub struct Camerica {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: usize,
    mirroring: Mirroring,
}

impl Camerica {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            mirroring: header.mirroring,
        }
    }

    fn banks(&self) -> usize {
        (self.prg.len() / 0x4000).max(1)
    }
}

impl Mapper for Camerica {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = self.banks();
        let bank = if address < 0xC000 {
            self.bank % banks
        } else {
            banks - 1
        };
        let offset = bank * 0x4000 + usize::from(address & 0x3FFF);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            return;
        }
        self.bank = usize::from(value & 0x0F) % self.banks();
        self.mirroring = if value & 0x10 != 0 {
            Mirroring::SingleUpper
        } else {
            Mirroring::SingleLower
        };
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        71
    }
}

/// Mapper 34: BNROM (32 KiB PRG banking, CHR-RAM) and NINA-001
/// (32 KiB PRG + 4 KiB CHR banking with registers in `$7FFD-$7FFF`).
#[derive(Debug)]
pub struct Nina {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    nina001: bool,
    prg_bank: usize,
    chr_banks: [usize; 2],
    mirroring: Mirroring,
}

impl Nina {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        let nina001 = chr.len() > 0x2000;
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x2000)],
            nina001,
            prg_bank: 0,
            chr_banks: [0, 0],
            mirroring: header.mirroring,
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x8000).max(1)
    }
}

impl Mapper for Nina {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let offset =
                    (self.prg_bank % self.prg_banks()) * 0x8000 + usize::from(address - 0x8000);
                self.prg.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if self.nina001 {
            match address {
                0x7FFD => self.prg_bank = usize::from(value & 1),
                0x7FFE => self.chr_banks[0] = usize::from(value & 0x0F),
                0x7FFF => self.chr_banks[1] = usize::from(value & 0x0F),
                0x6000..=0x7FFF => {
                    let index = usize::from(address - 0x6000) % self.prg_ram.len();
                    self.prg_ram[index] = value;
                }
                _ => {}
            }
        } else if address >= 0x8000 {
            // BNROM has bus conflicts: AND with the visible ROM byte.
            let offset =
                (self.prg_bank % self.prg_banks()) * 0x8000 + usize::from(address - 0x8000);
            let visible = self.prg.get(offset).copied().unwrap_or(0xFF);
            self.prg_bank = usize::from((value & visible) & 0x0F) % self.prg_banks();
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let index = usize::from(address) & 0x1FFF;
        if self.nina001 {
            let bank = if index < 0x1000 {
                self.chr_banks[0]
            } else {
                self.chr_banks[1]
            };
            let offset = (bank % (self.chr.len() / 0x1000).max(1)) * 0x1000 + (index & 0x0FFF);
            self.chr.get(offset).copied().unwrap_or(0)
        } else {
            self.chr.get(index).copied().unwrap_or(0)
        }
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let index = usize::from(address) & 0x1FFF;
        if self.nina001 {
            let bank = if index < 0x1000 {
                self.chr_banks[0]
            } else {
                self.chr_banks[1]
            };
            let offset = (bank % (self.chr.len() / 0x1000).max(1)) * 0x1000 + (index & 0x0FFF);
            if let Some(byte) = self.chr.get_mut(offset) {
                *byte = value;
            }
        } else if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        34
    }
}

/// NINA-03/NINA-06 (mapper 79) and the identical Sachen 3015 (mapper 146).
/// A single control register selects a 32 KiB PRG bank and an 8 KiB CHR bank.
#[derive(Debug)]
pub struct Nina79 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: usize,
    chr_bank: usize,
    mirroring: Mirroring,
}

impl Nina79 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Nina79 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let offset = (self.prg_bank % banks) * 0x8000 + usize::from(address - 0x8000);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        // Control register: 010x xxx1 xxxx xxxx ($4100-$41FF and mirrors).
        if address & 0xE100 == 0x4100 {
            self.prg_bank = usize::from((value >> 4) & 0x07);
            self.chr_bank = usize::from(value & 0x07);
        } else if address >= 0x8000 {
            // Errata: some mislabeled CNROM games write here for CHR banking.
            self.chr_bank = usize::from(value & 0x07);
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        79
    }
}

/// DxROM / Namco 118 / Tengen MIMIC-1 (mappers 206, 76, 88, 95, 154): a
/// simplified MMC3 with no IRQ. `$C000`/`$E000` are always the last two 8 KiB
/// banks and CHR always uses the left 2 KiB / right 1 KiB arrangement.
#[derive(Debug)]
pub struct DxRom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    select: u8,
    banks: [u8; 8],
    mirroring: Mirroring,
}

impl DxRom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x2000)],
            select: 0,
            banks: [0; 8],
            mirroring: header.mirroring,
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn prg_slot(&self, slot: usize) -> usize {
        let banks = self.prg_banks();
        let last = banks - 1;
        match slot {
            0 => usize::from(self.banks[6]) % banks,
            1 => usize::from(self.banks[7]) % banks,
            2 => banks.saturating_sub(2),
            _ => last,
        }
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let raw = match slot {
            0 | 1 => usize::from(self.banks[0] & 0xFE) + (slot & 1),
            2 | 3 => usize::from(self.banks[1] & 0xFE) + (slot & 1),
            4 => usize::from(self.banks[2]),
            5 => usize::from(self.banks[3]),
            6 => usize::from(self.banks[4]),
            _ => usize::from(self.banks[5]),
        };
        raw % banks
    }
}

impl Mapper for DxRom {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let slot = usize::from((address - 0x8000) / 0x2000);
                let bank = self.prg_slot(slot);
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            0x8000..=0x9FFF => {
                if address & 1 == 0 {
                    self.select = value & 0x07;
                } else {
                    self.banks[usize::from(self.select & 0x07)] = value;
                }
            }
            0xA000..=0xBFFF if address & 1 == 0 => {
                // Mapper 88 exposes mirroring here; harmless for 206.
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        206
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// UNROM-512 (mapper 30): 16 KiB switchable PRG at `$8000`, fixed last
/// 16 KiB at `$C000`, 8 KiB CHR-RAM and optional one-screen mirroring.
#[derive(Debug)]
pub struct Unrom512 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: usize,
    chr_bank: usize,
    one_screen: bool,
    one_screen_upper: bool,
    bus_conflicts: bool,
    fixed_mirroring: Mirroring,
}

impl Unrom512 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        // The iNES header reuses the four-screen bit to request a switchable
        // one-screen arrangement on this board.
        let one_screen = header.four_screen;
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            one_screen,
            one_screen_upper: false,
            bus_conflicts: header.submapper == 2 || (header.submapper == 0 && !header.battery),
            fixed_mirroring: if one_screen {
                Mirroring::SingleLower
            } else {
                header.mirroring
            },
        }
    }

    fn prg_banks(&self) -> usize {
        (self.prg.len() / 0x4000).max(1)
    }

    fn chr_banks(&self) -> usize {
        (self.chr.len() / 0x2000).max(1)
    }
}

impl Mapper for Unrom512 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = self.prg_banks();
        let bank = if address < 0xC000 {
            self.prg_bank % banks
        } else {
            banks - 1
        };
        let offset = bank * 0x4000 + usize::from(address & 0x3FFF);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            return;
        }
        let value = if self.bus_conflicts {
            let banks = self.prg_banks();
            let offset = (self.prg_bank % banks) * 0x4000 + usize::from(address & 0x3FFF);
            value & self.prg.get(offset).copied().unwrap_or(0xFF)
        } else {
            value
        };
        self.prg_bank = usize::from(value & 0x1F);
        self.chr_bank = usize::from((value >> 5) & 0x03);
        if self.one_screen {
            self.one_screen_upper = value & 0x80 != 0;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = self.chr_banks();
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = self.chr_banks();
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.one_screen {
            if self.one_screen_upper {
                Mirroring::SingleUpper
            } else {
                Mirroring::SingleLower
            }
        } else {
            self.fixed_mirroring
        }
    }

    fn number(&self) -> u16 {
        30
    }
}

/// Nanjing FC-001 (mapper 163): 32 KiB switchable PRG, 8 KiB CHR-RAM and a
/// small mode/feedback register set.
#[derive(Debug)]
pub struct Nanjing163 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    low: u8,
    high: u8,
    mode: u8,
    bank: usize,
    feedback: bool,
    mirroring: Mirroring,
}

impl Nanjing163 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x2000)],
            low: 0,
            high: 0,
            // Reset clears the mode, so A15/A16 are forced high: bank 3.
            mode: 0,
            bank: 3,
            feedback: false,
            mirroring: header.mirroring,
        }
    }

    fn swap_bits(value: u8) -> u8 {
        (value & !0x03) | ((value & 0x01) << 1) | ((value & 0x02) >> 1)
    }

    fn recompute(&mut self) {
        let mut bank = usize::from(self.low & 0x0F) | (usize::from(self.high & 0x03) << 4);
        // Bit 2 selects register banking (games write `$5300 = $04`); without
        // it A15/A16 are forced high, so power-on sees 32 KiB bank 3.
        if self.mode & 0x04 == 0 {
            bank |= 0x03;
        }
        self.bank = bank;
    }
}

impl Mapper for Nanjing163 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x5000..=0x5FFF => {
                if address & 0xFF00 == 0x5500 && self.feedback {
                    0x04
                } else {
                    0
                }
            }
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let offset = (self.bank % banks) * 0x8000 + usize::from(address - 0x8000);
                self.prg.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x5FFF => match address & 0xFF00 {
                0x5000 => {
                    let value = if self.mode & 0x01 != 0 {
                        Self::swap_bits(value)
                    } else {
                        value
                    };
                    self.low = value & 0x0F;
                    self.recompute();
                }
                0x5100 => {
                    if address & 1 == 0 {
                        if value & 0x01 == 0 {
                            self.feedback = value & 0x02 != 0;
                        }
                    } else if value & 0x02 != 0 {
                        self.feedback = !self.feedback;
                    }
                }
                0x5200 => {
                    let value = if self.mode & 0x01 != 0 {
                        Self::swap_bits(value)
                    } else {
                        value
                    };
                    self.high = value & 0x03;
                    self.recompute();
                }
                0x5300 => {
                    self.mode = value;
                    self.recompute();
                }
                _ => {}
            },
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        163
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Sachen SA-015/SA-630 (mapper 150): index/data register pair at
/// `$4100`/`$4101` selecting 32 KiB PRG, 8 KiB CHR and mirroring.
#[derive(Debug)]
pub struct Sachen150 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    regs: [u8; 8],
    index: u8,
    mirroring: Mirroring,
}

impl Sachen150 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            regs: [0; 8],
            index: 0,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank(&self) -> usize {
        usize::from(self.regs[5] & 0x03)
    }

    fn chr_bank(&self) -> usize {
        (usize::from(self.regs[4] & 0x01) << 2) | usize::from(self.regs[6] & 0x03)
    }

    fn update_mirroring(&mut self) {
        self.mirroring = match (self.regs[7] >> 2) & 0x03 {
            0 => Mirroring::SingleLower,
            1 => Mirroring::Horizontal,
            2 => Mirroring::Vertical,
            _ => Mirroring::SingleUpper,
        };
    }
}

impl Mapper for Sachen150 {
    fn prg_read(&self, address: u16) -> u8 {
        if address & 0xC101 == 0x4101 {
            return self.regs[usize::from(self.index & 0x07)];
        }
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let offset = (self.prg_bank() % banks) * 0x8000 + usize::from(address - 0x8000);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address & 0xC101 == 0x4100 {
            self.index = value & 0x07;
        } else if address & 0xC101 == 0x4101 {
            self.regs[usize::from(self.index & 0x07)] = value & 0x07;
            self.update_mirroring();
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank() % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank() % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        150
    }
}

/// One YM2149 tone channel.
#[derive(Debug, Default, Clone)]
struct YmTone {
    phase: u16,
    output: bool,
}

/// The YM2149 noise generator (17-bit LFSR, taps at bits 16 and 13).
#[derive(Debug, Clone)]
struct YmNoise {
    phase: u16,
    lfsr: u32,
}

impl Default for YmNoise {
    fn default() -> Self {
        Self { phase: 0, lfsr: 1 }
    }
}

impl YmNoise {
    fn clock(&mut self, period: u16) {
        if self.phase >= period.max(1) {
            self.phase = 0;
            let bit = (self.lfsr ^ (self.lfsr >> 3)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 16);
        } else {
            self.phase += 1;
        }
    }

    fn output(&self) -> bool {
        self.lfsr & 1 != 0
    }
}

/// The YM2149 envelope generator.
#[derive(Debug, Default, Clone)]
struct YmEnvelope {
    period: u16,
    phase: u16,
    step: u8,
    value: u8,
    attack: bool,
    alternate: bool,
    hold: bool,
    continue_: bool,
    running: bool,
}

impl YmEnvelope {
    fn reset(&mut self, shape: u8) {
        self.continue_ = shape & 0x08 != 0;
        self.attack = shape & 0x04 != 0;
        self.alternate = shape & 0x02 != 0;
        self.hold = shape & 0x01 != 0;
        self.phase = 0;
        self.step = 0;
        // Attack=1 ramps up from 0; attack=0 ramps down from 31.
        self.value = if self.attack { 0 } else { 31 };
        self.running = true;
    }

    fn clock(&mut self) {
        if !self.running {
            return;
        }
        if self.phase < self.period.max(1) {
            self.phase += 1;
            return;
        }
        self.phase = 0;
        if self.attack {
            if self.value < 31 {
                self.value += 1;
            } else {
                self.boundary();
            }
        } else if self.value > 0 {
            self.value -= 1;
        } else {
            self.boundary();
        }
        self.step = self.step.wrapping_add(1);
    }

    fn boundary(&mut self) {
        if !self.continue_ {
            self.running = false;
        } else if self.alternate {
            self.attack = !self.attack;
            if self.hold {
                self.running = self.alternate;
            }
        } else if self.hold {
            self.running = false;
        } else {
            // Continuous sawtooth: wrap to the opposite end.
            self.value = if self.attack { 0 } else { 31 };
        }
    }

    fn level(&self) -> u8 {
        self.value
    }
}

/// YM2149 16-step logarithmic DAC for fixed volumes (hardware table).
const SUNSOFT5B_LOG_VOL: [i32; 16] = [
    0, 15, 21, 30, 42, 59, 84, 119, 168, 237, 335, 473, 668, 944, 1333, 1882,
];
/// The same DAC at the 32-step envelope resolution (`[2v+1] == LOG_VOL[v]`).
const SUNSOFT5B_LOG_VOL32: [i32; 32] = [
    0, 0, 13, 15, 18, 21, 25, 30, 35, 42, 50, 59, 71, 84, 100, 119, 141, 168, 199, 237, 282, 335,
    398, 473, 562, 668, 794, 944, 1122, 1333, 1584, 1882,
];

/// Sunsoft 5B (YM2149F) expansion audio used by FME-7 boards such as Gimmick!.
#[derive(Debug, Default, Clone)]
struct Sunsoft5b {
    registers: [u8; 16],
    selected: u8,
    tone: [YmTone; 3],
    noise: YmNoise,
    envelope: YmEnvelope,
    prescaler: u8,
}

impl Sunsoft5b {
    fn select(&mut self, value: u8) {
        if value & 0xF0 == 0 {
            self.selected = value & 0x0F;
        }
    }

    fn write(&mut self, value: u8) {
        let register = usize::from(self.selected);
        if register >= 0x0E {
            return;
        }
        self.registers[register] = value;
        match register {
            0x0D => self.envelope.reset(value),
            0x0B => self.envelope.period = (self.envelope.period & 0xFF00) | u16::from(value),
            0x0C => {
                self.envelope.period = (self.envelope.period & 0x00FF) | (u16::from(value) << 8);
            }
            _ => {}
        }
    }

    fn tone_period(&self, channel: usize) -> u16 {
        let low = u16::from(self.registers[channel * 2]);
        let high = u16::from(self.registers[channel * 2 + 1] & 0x0F);
        (high << 8) | low
    }

    fn noise_period(&self) -> u16 {
        u16::from(self.registers[0x06] & 0x1F)
    }

    /// Advances one CPU cycle; the tone/noise/envelope counters run at
    /// CPU/16.
    fn tick(&mut self) {
        self.prescaler += 1;
        if self.prescaler < 16 {
            return;
        }
        self.prescaler = 0;
        for channel in 0..3 {
            let period = self.tone_period(channel).max(1);
            let tone = &mut self.tone[channel];
            if tone.phase >= period {
                tone.phase = 0;
                tone.output = !tone.output;
            } else {
                tone.phase += 1;
            }
        }
        self.noise.clock(self.noise_period());
        self.envelope.clock();
    }

    fn channel_output(&self, channel: usize) -> i32 {
        let mixer = self.registers[0x07];
        let tone_disabled = mixer & (1 << channel) != 0;
        let noise_disabled = mixer & (1 << (channel + 3)) != 0;
        let tone = self.tone[channel].output;
        let noise = self.noise.output();
        let signal = match (tone_disabled, noise_disabled) {
            (true, true) => true,
            (false, true) => tone,
            (true, false) => noise,
            (false, false) => tone & noise,
        };
        let control = self.registers[0x08 + channel];
        // The YM2149 DAC is logarithmic; fixed volume indexes the 16-step
        // table and the envelope the 32-step one (the same curve).
        let volume = if control & 0x10 != 0 {
            SUNSOFT5B_LOG_VOL32[usize::from(self.envelope.level()) & 0x1F]
        } else {
            SUNSOFT5B_LOG_VOL[usize::from(control & 0x0F)]
        };
        if signal {
            volume
        } else {
            0
        }
    }

    fn mix(&self) -> f32 {
        let sum: i32 = (0..3).map(|channel| self.channel_output(channel)).sum();
        // Scale to the documented 5B level (2549/138) and normalize by the
        // bus's /65536 external-audio contract.
        (sum * 2549 / 138) as f32 / 65_536.0
    }
}

/// Sunsoft FME-7 / 5A / 5B (mapper 69): 8 KiB PRG and 1 KiB CHR banking,
/// switchable nametables, a CPU-cycle IRQ counter and YM2149 audio on 5B
/// boards such as Gimmick!.
#[derive(Debug)]
pub struct Fme7 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    command: u8,
    chr_banks: [u8; 8],
    prg_banks: [u8; 3],
    ram_bank: u8,
    mirroring: Mirroring,
    irq_enabled: bool,
    irq_line: bool,
    irq_counter: u16,
    irq_reload: u16,
    audio: Sunsoft5b,
}

impl Fme7 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x20000)],
            command: 0,
            chr_banks: [0; 8],
            prg_banks: [0; 3],
            ram_bank: 0,
            mirroring: header.mirroring,
            irq_enabled: false,
            irq_line: false,
            irq_counter: 0,
            irq_reload: 0,
            audio: Sunsoft5b::default(),
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn write_parameter(&mut self, value: u8) {
        match self.command & 0x0F {
            0x0..=0x07 => self.chr_banks[usize::from(self.command & 0x07)] = value,
            0x08 => {
                if value & 0x40 != 0 {
                    self.ram_bank = value & 0x3F;
                } else {
                    self.prg_banks[0] = value & 0x3F;
                }
            }
            0x09 => self.prg_banks[0] = value & 0x3F,
            0x0A => self.prg_banks[1] = value & 0x3F,
            0x0B => self.prg_banks[2] = value & 0x3F,
            0x0C => {
                self.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                }
            }
            0x0D => {
                self.irq_enabled = value & 0x80 != 0;
                if value & 0x01 != 0 {
                    self.irq_line = false;
                }
                self.irq_reload = if value & 0x01 != 0 { 0x0001 } else { 0xFFFF };
            }
            0x0E => self.irq_counter = (self.irq_counter & 0xFF00) | u16::from(value),
            0x0F => self.irq_counter = (self.irq_counter & 0x00FF) | (u16::from(value) << 8),
            _ => {}
        }
    }
}

impl Mapper for Fme7 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = self.prg_bank_count();
        match address {
            0x6000..=0x7FFF => {
                let offset = usize::from(self.ram_bank) * 0x2000 + usize::from(address - 0x6000);
                self.prg_ram
                    .get(offset % self.prg_ram.len())
                    .copied()
                    .unwrap_or(0)
            }
            0x8000..=0x9FFF => {
                self.prg[(usize::from(self.prg_banks[0]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xA000..=0xBFFF => {
                self.prg[(usize::from(self.prg_banks[1]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xC000..=0xDFFF => {
                self.prg[(usize::from(self.prg_banks[2]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xE000..=0xFFFF => self.prg[(banks - 1) * 0x2000 + usize::from(address & 0x1FFF)],
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x7FFF => {
                let offset = usize::from(self.ram_bank) * 0x2000 + usize::from(address - 0x6000);
                let len = self.prg_ram.len();
                self.prg_ram[offset % len] = value;
            }
            0x8000..=0x9FFF => self.command = value & 0x0F,
            0xA000..=0xBFFF => self.write_parameter(value),
            0xC000..=0xDFFF => self.audio.select(value),
            0xE000..=0xFFFF => self.audio.write(value),
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        69
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_line
    }

    fn clock_cpu(&mut self) {
        self.audio.tick();
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0 {
            self.irq_counter = self.irq_reload;
            self.irq_line = true;
        } else {
            self.irq_counter -= 1;
        }
    }

    fn audio_mix(&self) -> f32 {
        self.audio.mix()
    }
}

/// Sachen SA-008-A / Tengen 800008 (mapper 148): mapper 79's latch moved to
/// `$8000-$FFFF` with bus conflicts.
#[derive(Debug)]
pub struct Nina148 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: usize,
    chr_bank: usize,
    mirroring: Mirroring,
}

impl Nina148 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Nina148 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let offset = (self.prg_bank % banks) * 0x8000 + usize::from(address - 0x8000);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            return;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let visible = self
            .prg
            .get((self.prg_bank % banks) * 0x8000 + usize::from(address - 0x8000))
            .copied()
            .unwrap_or(0xFF);
        let value = value & visible;
        self.prg_bank = usize::from((value >> 4) & 0x07);
        self.chr_bank = usize::from(value & 0x07);
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (self.chr_bank % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        148
    }
}

/// Jaleco SS 88006 (mapper 18): VRC4-like 8 KiB PRG / 1 KiB CHR banking with
/// a CPU-cycle IRQ counter. Banks are built from nibble pairs.
#[derive(Debug)]
pub struct Ss88006 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_banks: [u8; 3],
    chr_banks: [u8; 8],
    irq_reload: u16,
    irq_counter: u16,
    irq_size_16: bool,
    irq_enabled: bool,
    irq_pending: bool,
    ram_enabled: bool,
    mirroring: Mirroring,
}

impl Ss88006 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x2000)],
            prg_banks: [0; 3],
            chr_banks: [0; 8],
            irq_reload: 0,
            irq_counter: 0,
            irq_size_16: true,
            irq_enabled: false,
            irq_pending: false,
            ram_enabled: true,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }
}

impl Mapper for Ss88006 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = self.prg_bank_count();
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0x9FFF => {
                self.prg[(usize::from(self.prg_banks[0]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xA000..=0xBFFF => {
                self.prg[(usize::from(self.prg_banks[1]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xC000..=0xDFFF => {
                self.prg[(usize::from(self.prg_banks[2]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xE000..=0xFFFF => self.prg[(banks - 1) * 0x2000 + usize::from(address & 0x1FFF)],
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            if address >= 0x6000 && self.ram_enabled {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            return;
        }
        let low = value & 0x0F;
        let high = value & 0x0F;
        match address & 0xF003 {
            0x8000 => self.prg_banks[0] = (self.prg_banks[0] & 0x30) | low,
            0x8001 => self.prg_banks[0] = (self.prg_banks[0] & 0x0F) | (high << 4),
            0x8002 => self.prg_banks[1] = (self.prg_banks[1] & 0x30) | low,
            0x8003 => self.prg_banks[1] = (self.prg_banks[1] & 0x0F) | (high << 4),
            0x9000 => self.prg_banks[2] = (self.prg_banks[2] & 0x30) | low,
            0x9001 => self.prg_banks[2] = (self.prg_banks[2] & 0x0F) | (high << 4),
            0x9002 => {
                self.ram_enabled = value & 0x01 != 0;
            }
            0xA000 => self.chr_banks[0] = (self.chr_banks[0] & 0xF0) | low,
            0xA001 => self.chr_banks[0] = (self.chr_banks[0] & 0x0F) | (high << 4),
            0xA002 => self.chr_banks[1] = (self.chr_banks[1] & 0xF0) | low,
            0xA003 => self.chr_banks[1] = (self.chr_banks[1] & 0x0F) | (high << 4),
            0xB000 => self.chr_banks[2] = (self.chr_banks[2] & 0xF0) | low,
            0xB001 => self.chr_banks[2] = (self.chr_banks[2] & 0x0F) | (high << 4),
            0xB002 => self.chr_banks[3] = (self.chr_banks[3] & 0xF0) | low,
            0xB003 => self.chr_banks[3] = (self.chr_banks[3] & 0x0F) | (high << 4),
            0xC000 => self.chr_banks[4] = (self.chr_banks[4] & 0xF0) | low,
            0xC001 => self.chr_banks[4] = (self.chr_banks[4] & 0x0F) | (high << 4),
            0xC002 => self.chr_banks[5] = (self.chr_banks[5] & 0xF0) | low,
            0xC003 => self.chr_banks[5] = (self.chr_banks[5] & 0x0F) | (high << 4),
            0xD000 => self.chr_banks[6] = (self.chr_banks[6] & 0xF0) | low,
            0xD001 => self.chr_banks[6] = (self.chr_banks[6] & 0x0F) | (high << 4),
            0xD002 => self.chr_banks[7] = (self.chr_banks[7] & 0xF0) | low,
            0xD003 => self.chr_banks[7] = (self.chr_banks[7] & 0x0F) | (high << 4),
            0xE000 => self.irq_reload = (self.irq_reload & 0xFFF0) | u16::from(low),
            0xE001 => self.irq_reload = (self.irq_reload & 0xFF0F) | (u16::from(low) << 4),
            0xE002 => self.irq_reload = (self.irq_reload & 0xF0FF) | (u16::from(low) << 8),
            0xE003 => self.irq_reload = (self.irq_reload & 0x0FFF) | (u16::from(low) << 12),
            0xF000 => {
                self.irq_counter = self.irq_reload;
                self.irq_pending = false;
                self.irq_enabled = true;
            }
            0xF001 => self.irq_size_16 = value & 0x08 == 0,
            0xF002 => {
                self.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                }
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = self.chr_bank_count();
        let slot = usize::from(address) / 0x400;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = self.chr_bank_count();
        let slot = usize::from(address) / 0x400;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        18
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0 {
            self.irq_counter = if self.irq_size_16 {
                self.irq_reload
            } else {
                self.irq_reload & 0xFF
            };
            self.irq_pending = true;
        } else {
            self.irq_counter -= 1;
        }
    }
}

/// Cartridge loading errors.
#[derive(Debug)]
pub enum CartError {
    /// Missing/incorrect `NES\x1a` magic.
    BadMagic,
    /// Image shorter than the header claims.
    Truncated {
        /// Required length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// Mapper not implemented yet.
    UnsupportedMapper(u16),
}

impl fmt::Display for CartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not an iNES image"),
            Self::Truncated { expected, actual } => {
                write!(
                    f,
                    "image truncated: expected {expected} bytes, got {actual}"
                )
            }
            Self::UnsupportedMapper(mapper) => write!(f, "mapper {mapper} is not implemented"),
        }
    }
}

impl std::error::Error for CartError {}

/// Waixing FS305 / Nanjing NJ0430 (mapper 178): 8 KiB CHR(-RAM) window,
/// 16/32 KiB PRG with a low inner bank and a high outer bank, plus 8 KiB
/// banked PRG-RAM.
#[derive(Debug)]
pub struct Fs305 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    mode: u8,
    low: u8,
    high: u8,
    ram_bank: u8,
    mirroring: Mirroring,
}

impl Fs305 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            mode: 0,
            low: 0,
            high: 0,
            ram_bank: 0,
            mirroring: header.mirroring,
        }
    }

    /// Builds the 8 KiB PRG bank index for `address` in `$8000..=$FFFF`.
    fn prg_bank(&self, address: u16) -> usize {
        let a14 = usize::from((address >> 14) & 1);
        let low = usize::from(self.low & 0x07);
        let high = usize::from(self.high) << 3;
        match (self.mode >> 2) & 0x03 {
            // NROM-256/BNROM: PRG A14 follows the CPU address.
            0 => (high | (low & !1)) | a14,
            // UNROM: fixed upper half at `$C000`.
            1 if a14 == 1 => high | 0x07,
            1 => high | low,
            // NROM-128: same 16 KiB bank in both halves.
            2 => high | (low & !1),
            // UNROM with selectable bit 0 in the fixed half.
            _ if a14 == 1 => high | 0x06 | (low & 0x01),
            _ => high | low,
        }
    }
}

impl Mapper for Fs305 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => {
                let offset = usize::from(self.ram_bank) * 0x2000 + usize::from(address - 0x6000);
                self.prg_ram[offset % self.prg_ram.len()]
            }
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let offset =
                    (self.prg_bank(address) % banks) * 0x2000 + usize::from(address & 0x1FFF);
                self.prg.get(offset).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x4800..=0x4FFF => match address & 0x03 {
                0 => {
                    self.mode = value;
                    self.mirroring = match value & 0x03 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::SingleLower,
                        _ => Mirroring::SingleUpper,
                    };
                }
                1 => self.low = value & 0x07,
                2 => self.high = value,
                _ => self.ram_bank = value,
            },
            0x6000..=0x7FFF => {
                let offset = usize::from(self.ram_bank) * 0x2000 + usize::from(address - 0x6000);
                let len = self.prg_ram.len();
                self.prg_ram[offset % len] = value;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        178
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Action 53 (mapper 28): multicart mapper with inner/outer PRG bank
/// registers and an 8 KiB window over 32 KiB of CHR-RAM.
#[derive(Debug)]
pub struct Action53 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    select: u8,
    chr_bank: u8,
    inner: u8,
    mode: u8,
    outer: u8,
    mirror: u8,
}

impl Action53 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        let banks16 = (prg.len() / 0x4000).max(1);
        let last = banks16 - 1;
        Self {
            prg,
            chr,
            select: 0,
            chr_bank: 0,
            inner: (last & 0x0F) as u8,
            mode: 0,
            outer: (last >> 4) as u8,
            mirror: match header.mirroring {
                Mirroring::Vertical => 2,
                Mirroring::Horizontal => 3,
                Mirroring::SingleUpper => 1,
                _ => 0,
            },
        }
    }

    /// Reference Action 53 bank formula, returning a 16 KiB bank index.
    fn prg_bank(&self, address: u16) -> usize {
        let a14 = usize::from((address >> 14) & 1);
        let mut bank_mode = usize::from(self.mode >> 2);
        let outer = usize::from(self.outer) << 1;
        let mut current = usize::from(self.inner);
        if (bank_mode ^ a14) & 0x03 == 0x02 {
            bank_mode = 0;
        }
        if bank_mode & 0x02 == 0 {
            current = (current << 1) | a14;
        }
        let mask = [0x01usize, 0x03, 0x07, 0x0F][(bank_mode >> 2) & 0x03];
        (current & mask) | (outer & !mask)
    }
}

impl Mapper for Action53 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x4000).max(1);
        let offset = (self.prg_bank(address) % banks) * 0x4000 + usize::from(address & 0x3FFF);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x5FFF => self.select = value,
            0x8000..=0xFFFF => {
                let supervisor = self.select & 0x80 != 0;
                let index = self.select & 0x01;
                if supervisor {
                    if index == 0 {
                        self.mode = value;
                        self.mirror = value & 0x03;
                    } else {
                        self.outer = value;
                    }
                } else if index == 0 {
                    self.chr_bank = value & 0x03;
                    if self.mirror < 2 {
                        self.mirror = (self.mirror & 0x02) | ((value >> 4) & 0x01);
                    }
                } else {
                    self.inner = value & 0x0F;
                    if self.mirror < 2 {
                        self.mirror = (self.mirror & 0x02) | ((value >> 4) & 0x01);
                    }
                }
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        match self.mirror & 0x03 {
            0 => Mirroring::SingleLower,
            1 => Mirroring::SingleUpper,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        }
    }

    fn number(&self) -> u16 {
        28
    }
}

/// Namco 129/163 (mapper 19): 8 KiB PRG banks, 1 KiB CHR banks that can also
/// select the 2 KiB nametable RAM, a 15-bit CPU-cycle IRQ counter and the
/// 128-byte internal chip RAM.
#[derive(Debug)]
pub struct Namco163 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    chip_ram: [u8; 128],
    chip_addr: u8,
    chr_banks: [u8; 8],
    nt_banks: [u8; 4],
    prg_banks: [u8; 3],
    nt_ram: [u8; 0x800],
    write_protect: u8,
    irq_counter: u16,
    irq_enabled: bool,
    irq_pending: bool,
    chr_ram_low_disabled: bool,
    chr_ram_high_disabled: bool,
    mirroring: Mirroring,
    chip_inc: bool,
    sound_disabled: bool,
    audio_divider: u8,
    audio_channel: usize,
    audio_channels: [i16; 8],
}

impl Namco163 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            chip_ram: [0; 128],
            chip_addr: 0,
            chr_banks: [0; 8],
            nt_banks: [0; 4],
            prg_banks: [0; 3],
            nt_ram: [0; 0x800],
            // Power-on state is write-enabled until the game programs $F800.
            write_protect: 0x40,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            chr_ram_low_disabled: false,
            chr_ram_high_disabled: false,
            mirroring: header.mirroring,
            chip_inc: false,
            sound_disabled: false,
            audio_divider: 0,
            audio_channel: 0,
            audio_channels: [0; 8],
        }
    }

    /// Updates one N163 wave channel, cycling through the enabled set every
    /// 15 CPU cycles.
    fn audio_update(&mut self) {
        if self.sound_disabled {
            self.audio_channels = [0; 8];
            return;
        }
        let control = self.chip_ram[0x7F];
        let count = usize::from((control >> 4) & 0x07) + 1;
        let index = 7 - (self.audio_channel % count);
        self.audio_channel = (self.audio_channel + 1) % count;
        let base = 0x40 + index * 8;
        let frequency = (u32::from(self.chip_ram[base + 4] & 0x03) << 16)
            | (u32::from(self.chip_ram[base + 2]) << 8)
            | u32::from(self.chip_ram[base]);
        let length = 256 - u32::from(self.chip_ram[base + 4] & 0xFC);
        let mut phase = (u32::from(self.chip_ram[base + 5]) << 16)
            | (u32::from(self.chip_ram[base + 3]) << 8)
            | u32::from(self.chip_ram[base + 1]);
        phase = (phase + frequency) % (length << 16);
        self.chip_ram[base + 1] = phase as u8;
        self.chip_ram[base + 3] = (phase >> 8) as u8;
        self.chip_ram[base + 5] = (phase >> 16) as u8;

        let offset = u32::from(self.chip_ram[base + 6]);
        let sample_address = ((phase >> 16) + offset) & 0xFF;
        let byte = self.chip_ram[(sample_address >> 1) as usize & 0x7F];
        let sample = if sample_address & 1 == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        };
        let volume = i16::from(self.chip_ram[base + 7] & 0x0F);
        self.audio_channels[index] = (i16::from(sample) - 8) * volume;
    }

    /// Number of active N163 wave channels (1..=8).
    fn audio_channel_count(&self) -> usize {
        usize::from((self.chip_ram[0x7F] >> 4) & 0x07) + 1
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }

    fn ram_write_enabled(&self, address: u16) -> bool {
        self.write_protect & 0xF0 == 0x40
            && self.write_protect & (1 << ((address - 0x6000) / 0x800)) == 0
    }

    fn chr_bank(&self, address: u16) -> usize {
        let slot = usize::from(address / 0x400) & 0x07;
        let raw = usize::from(self.chr_banks[slot]);
        if raw >= 0xE0 {
            let disabled = if address < 0x1000 {
                self.chr_ram_low_disabled
            } else {
                self.chr_ram_high_disabled
            };
            if disabled {
                let banks = self.chr_bank_count();
                banks.saturating_sub(0x20) + ((raw & 0x1F).min(banks - 1))
            } else {
                // Even pages read page A of the nametable RAM, odd page B.
                0xE0 + (raw & 0x01)
            }
        } else {
            raw
        }
    }
}

impl Mapper for Namco163 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x4800..=0x4FFF => self.chip_ram[usize::from(self.chip_addr) & 0x7F],
            0x5000..=0x57FF => self.irq_counter as u8,
            0x5800..=0x5FFF => {
                ((self.irq_counter >> 8) as u8 & 0x7F) | if self.irq_enabled { 0x80 } else { 0 }
            }
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0x9FFF => {
                let banks = self.prg_bank_count();
                let bank = usize::from(self.prg_banks[0]) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xA000..=0xBFFF => {
                let banks = self.prg_bank_count();
                let bank = usize::from(self.prg_banks[1]) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xC000..=0xDFFF => {
                let banks = self.prg_bank_count();
                let bank = usize::from(self.prg_banks[2]) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xE000..=0xFFFF => {
                let bank = self.prg_bank_count() - 1;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x4800..=0x4FFF => {
                let index = usize::from(self.chip_addr) & 0x7F;
                self.chip_ram[index] = value;
                if self.chip_inc && index < 0x7F {
                    self.chip_addr = (index + 1) as u8;
                }
            }
            0x5000..=0x57FF => {
                self.irq_counter = (self.irq_counter & 0x7F00) | u16::from(value);
                self.irq_pending = false;
            }
            0x5800..=0x5FFF => {
                self.irq_counter = (self.irq_counter & 0x00FF) | (u16::from(value & 0x7F) << 8);
                self.irq_enabled = value & 0x80 != 0;
                self.irq_pending = false;
            }
            0x6000..=0x7FFF => {
                if self.ram_write_enabled(address) {
                    let index = usize::from(address - 0x6000) % self.prg_ram.len();
                    self.prg_ram[index] = value;
                }
            }
            0x8000..=0xBFFF => {
                let slot = usize::from((address - 0x8000) / 0x800) & 0x07;
                self.chr_banks[slot] = value;
                if slot >= 4 {
                    let nt = slot - 4;
                    self.nt_banks[nt] = value;
                }
            }
            0xC000..=0xDFFF => {
                let slot = usize::from((address - 0xC000) / 0x800) & 0x03;
                self.nt_banks[slot] = value;
            }
            0xE000..=0xE7FF => {
                self.prg_banks[0] = value & 0x3F;
                self.sound_disabled = value & 0x40 != 0;
            }
            0xE800..=0xEFFF => {
                self.prg_banks[1] = value & 0x3F;
                self.chr_ram_low_disabled = value & 0x40 != 0;
                self.chr_ram_high_disabled = value & 0x80 != 0;
            }
            0xF000..=0xF7FF => self.prg_banks[2] = value & 0x3F,
            0xF800..=0xFFFF => {
                self.chip_addr = value & 0x7F;
                self.chip_inc = value & 0x80 != 0;
                self.write_protect = value;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let bank = self.chr_bank(address);
        if bank >= 0xE0 {
            let page = bank & 0x01;
            let offset = page * 0x400 + usize::from(address & 0x3FF);
            self.nt_ram[offset % self.nt_ram.len()]
        } else {
            let banks = self.chr_bank_count();
            let offset = (bank % banks) * 0x400 + usize::from(address & 0x3FF);
            self.chr.get(offset).copied().unwrap_or(0)
        }
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let bank = self.chr_bank(address);
        if bank >= 0xE0 {
            let offset = (bank & 0x01) * 0x400 + usize::from(address & 0x3FF);
            self.nt_ram[offset % self.nt_ram.len()] = value;
        } else {
            let banks = self.chr_bank_count();
            let offset = (bank % banks) * 0x400 + usize::from(address & 0x3FF);
            if let Some(byte) = self.chr.get_mut(offset) {
                *byte = value;
            }
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        19
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        self.audio_divider += 1;
        if self.audio_divider >= 15 {
            self.audio_divider = 0;
            self.audio_update();
        }
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter < 0x7FFF {
            self.irq_counter += 1;
            if self.irq_counter == 0x7FFF {
                self.irq_pending = true;
            }
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }

    fn audio_mix(&self) -> f32 {
        if self.sound_disabled {
            return 0.0;
        }
        let count = self.audio_channel_count();
        // `audio_update` maps the cycled channel to `7 - (n % count)`, so the
        // active set is the top `count` slots.
        let sum: i32 = self.audio_channels[8 - count..]
            .iter()
            .map(|&value| i32::from(value))
            .sum();
        // Per-channel range is -120..=105; average over the channel count and
        // scale to the documented N163 level (`NAMCO163_MIX_SCALE` = 261),
        // normalized by the bus's /65536.
        (sum / count as i32) as f32 * 261.0 / 65536.0
    }
}

/// J.Y. Company ASIC (mappers 35/90/209/211): flexible PRG/CHR banking,
/// 4-way nametable arrangement, hardware multiplier and a prescaled IRQ
/// counter with selectable clock source.
#[derive(Debug)]
pub struct JyAsic {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_banks: [u8; 4],
    chr_low: [u8; 8],
    chr_high: [u8; 8],
    nt_banks: [u8; 4],
    mode: u8,
    nt_arrange: u8,
    ppu_conf: u8,
    outer: u8,
    irq_enable: bool,
    irq_mode: u8,
    irq_counter: u8,
    irq_prescaler: u8,
    irq_pending: bool,
    mult_a: u8,
    mult_b: u8,
    product: u16,
    accumulator: u8,
    test: u8,
    mirroring: Mirroring,
}

impl JyAsic {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            prg_banks: [0; 4],
            chr_low: [0; 8],
            chr_high: [0; 8],
            nt_banks: [0; 4],
            mode: 0,
            nt_arrange: 0,
            ppu_conf: 0,
            outer: 0,
            irq_enable: false,
            irq_mode: 0,
            irq_counter: 0,
            irq_prescaler: 0,
            irq_pending: false,
            mult_a: 0,
            mult_b: 0,
            product: 0,
            accumulator: 0,
            test: 0,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }

    fn outer_prg(&self) -> usize {
        usize::from((self.outer >> 1) & 0x03) * 64
    }

    fn prg_bank(&self, address: u16) -> usize {
        let base = self.outer_prg();
        let last = self.mode & 0x04 != 0;
        let a13 = usize::from((address >> 13) & 1);
        let a14 = usize::from((address >> 14) & 1);
        let within = match self.mode & 0x03 {
            // 32 KiB banking at $8000-$FFFF.
            0 => {
                let sel = if last {
                    usize::from(self.prg_banks[3]) & 0x0F
                } else {
                    0x0F
                };
                sel * 4 + a13 + a14 * 2
            }
            // 16 KiB banking: $8000-$BFFF and $C000-$FFFF.
            1 => {
                let sel = if address < 0xC000 {
                    usize::from(self.prg_banks[1]) & 0x1F
                } else if last {
                    usize::from(self.prg_banks[3]) & 0x1F
                } else {
                    0x1F
                };
                sel * 2 + a13
            }
            // 8 KiB banking.
            _ => {
                let slot = usize::from((address >> 13) & 0x03);
                let raw = if slot < 3 {
                    self.prg_banks[slot]
                } else if last {
                    self.prg_banks[3]
                } else {
                    0x3F
                };
                let raw = if self.mode & 0x03 == 3 {
                    reverse_seven_bits(raw)
                } else {
                    raw
                };
                usize::from(raw) & 0x3F
            }
        };
        base + within
    }

    fn rom_at_6000(&self) -> usize {
        let base = self.outer_prg();
        let sel = usize::from(self.prg_banks[3]) & 0x3F;
        base + match self.mode & 0x03 {
            0 => (sel & 0x0F) * 4 + 3,
            1 => (sel & 0x1F) * 2 + 1,
            _ => sel,
        }
    }

    fn chr_bank(&self, address: u16) -> usize {
        let slot = usize::from(address / 0x400) & 0x07;
        let mode = (self.mode >> 3) & 0x03;
        let (index, scale, sub) = match mode {
            0 => (0, 8, slot),
            1 => (slot / 4, 4, slot & 0x03),
            2 => (slot / 2, 2, slot & 0x01),
            _ => (slot, 1, 0),
        };
        let outer =
            (usize::from((self.outer >> 3) & 0x03) << 9) | (usize::from(self.outer & 0x01) << 8);
        let low = usize::from(self.chr_low[index]);
        let high = usize::from(self.chr_high[index] & 0x03);
        outer + ((high << 8) | low) * scale + sub
    }

    fn irq_tick(&mut self) {
        if !self.irq_enable {
            return;
        }
        let direction = (self.irq_mode >> 4) & 0x03;
        if direction == 0 || direction == 3 {
            return;
        }
        let mask = if self.irq_mode & 0x04 != 0 {
            0x07
        } else {
            0xFF
        };
        if self.irq_prescaler >= mask {
            self.irq_prescaler = 0;
            if direction == 1 {
                self.irq_counter = self.irq_counter.wrapping_add(1);
                if self.irq_counter == 0 {
                    self.irq_pending = true;
                }
            } else {
                self.irq_counter = self.irq_counter.wrapping_sub(1);
                if self.irq_counter == 0xFF {
                    self.irq_pending = true;
                }
            }
        } else {
            self.irq_prescaler += 1;
        }
    }
}

fn reverse_seven_bits(value: u8) -> u8 {
    let mut result = 0u8;
    for bit in 0..7 {
        if value & (1 << bit) != 0 {
            result |= 1 << (6 - bit);
        }
    }
    result
}

impl Mapper for JyAsic {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x5000..=0x57FF => 0,
            0x5800..=0x5801 => {
                if address == 0x5800 {
                    self.product as u8
                } else {
                    (self.product >> 8) as u8
                }
            }
            0x5802..=0x5803 => {
                if address == 0x5802 {
                    self.accumulator
                } else {
                    self.test
                }
            }
            0x5804..=0x5FFF => 0,
            0x6000..=0x7FFF => {
                if self.mode & 0x80 != 0 {
                    let banks = self.prg_bank_count();
                    let bank = self.rom_at_6000() % banks;
                    self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
                } else {
                    self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()]
                }
            }
            0x8000..=0xFFFF => {
                let banks = self.prg_bank_count();
                let bank = self.prg_bank(address) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x57FF => {}
            0x5800..=0x5801 => {
                if address == 0x5800 {
                    self.mult_a = value;
                } else {
                    self.mult_b = value;
                }
                self.product = u16::from(self.mult_a) * u16::from(self.mult_b);
            }
            0x5802..=0x5803 => {
                if address == 0x5802 {
                    self.accumulator = self.accumulator.wrapping_add(value);
                } else {
                    self.accumulator = 0;
                    self.test = value;
                }
            }
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            0x8000..=0x87FF if address & 0x800 == 0 => {
                self.prg_banks[usize::from(address & 0x03)] = value;
            }
            0x9000..=0xA7FF if address & 0x800 == 0 => {
                let slot = usize::from(address & 0x07);
                if address < 0xA000 {
                    self.chr_low[slot] = value;
                } else {
                    self.chr_high[slot] = value;
                }
            }
            0xB000..=0xB7FF if address & 0x800 == 0 => {
                self.nt_banks[usize::from(address & 0x03)] = value;
            }
            0xC000..=0xC1FF if address & 1 == 0 => {
                self.irq_enable = value & 0x01 != 0;
                self.irq_pending = false;
                self.irq_prescaler = 0;
            }
            0xC000..=0xC1FF if address & 1 == 1 => self.irq_mode = value,
            0xD000..=0xD7FF if address & 0x800 == 0 => match address & 0x03 {
                0 => {
                    self.mode = value;
                    self.mirroring = match self.nt_arrange & 0x03 {
                        0 => Mirroring::Horizontal,
                        1 => Mirroring::Vertical,
                        2 => Mirroring::SingleLower,
                        _ => Mirroring::SingleUpper,
                    };
                }
                1 => {
                    self.nt_arrange = value;
                    self.mirroring = match value & 0x03 {
                        0 => Mirroring::Horizontal,
                        1 => Mirroring::Vertical,
                        2 => Mirroring::SingleLower,
                        _ => Mirroring::SingleUpper,
                    };
                }
                2 => self.ppu_conf = value,
                _ => self.outer = value,
            },
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = self.chr_bank_count();
        let bank = self.chr_bank(address) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if self.ppu_conf & 0x01 == 0 {
            return;
        }
        let banks = self.chr_bank_count();
        let bank = self.chr_bank(address) % banks;
        let offset = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        90
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if self.irq_mode & 0x03 == 0 {
            self.irq_tick();
        }
    }

    fn clock_scanline(&mut self) {
        let count = match self.irq_mode & 0x03 {
            1 => 8,
            2 => 170,
            _ => 0,
        };
        for _ in 0..count {
            self.irq_tick();
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// J87 (mappers 87/101): a 32 KiB fixed PRG board with a single 8 KiB CHR
/// bank register at `$6000-$7FFF`. Mapper 101 is the same board with every
/// CHR byte's bit order reversed in the dump.
#[derive(Debug)]
pub struct J87 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_bank: u8,
    reversed: bool,
    mirroring: Mirroring,
}

impl J87 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            chr_bank: 0,
            reversed: header.mapper == 101,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for J87 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let offset = usize::from(address - 0x8000) % self.prg.len().max(1);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x6000..=0x7FFF).contains(&address) {
            self.chr_bank = value & 0x03;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        let byte = self.chr.get(offset).copied().unwrap_or(0);
        if self.reversed {
            byte.reverse_bits()
        } else {
            byte
        }
    }

    fn chr_write(&mut self, _address: u16, _value: u8) {}

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        if self.reversed {
            101
        } else {
            87
        }
    }
}

/// J.Y. Company clone (mapper 91, submapper 0): two 8 KiB PRG banks plus a
/// fixed last bank, four 2 KiB CHR banks, an outer 128 KiB PRG / 512 KiB CHR
/// register and a fixed 64-rise PPU A12 IRQ.
#[derive(Debug)]
pub struct JyClone {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_banks: [u8; 2],
    chr_banks: [u8; 4],
    outer_prg: u8,
    outer_chr: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_pending: bool,
    mirroring: Mirroring,
}

impl JyClone {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_banks: [0; 2],
            chr_banks: [0; 4],
            outer_prg: 0,
            outer_chr: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }
}

impl Mapper for JyClone {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = self.prg_bank_count();
        let outer = usize::from(self.outer_prg) << 4;
        let bank = match address {
            0x8000..=0x9FFF => outer | (usize::from(self.prg_banks[0]) & 0x0F),
            0xA000..=0xBFFF => outer | (usize::from(self.prg_banks[1]) & 0x0F),
            0xC000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x6003 => self.chr_banks[usize::from(address & 0x03)] = value,
            0x7000..=0x7001 => self.prg_banks[usize::from(address & 0x01)] = value,
            0x7006 => {
                self.irq_pending = false;
                self.irq_enabled = false;
            }
            0x7007 => {
                self.irq_counter = 0;
                self.irq_pending = false;
                self.irq_enabled = true;
            }
            0x8000..=0x9FFF => {
                self.outer_prg = value & 0x06;
                self.outer_chr = value & 0x01;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x800).max(1);
        let slot = usize::from(address / 0x800) & 0x03;
        let outer = usize::from(self.outer_chr) << 8;
        let bank = (outer | usize::from(self.chr_banks[slot])) % banks;
        self.chr[bank * 0x800 + usize::from(address & 0x7FF)]
    }

    fn chr_write(&mut self, _address: u16, _value: u8) {}

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        91
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        // The counter is clocked by unfiltered PPU A12 (eight rises per line).
        self.irq_counter = self.irq_counter.saturating_add(8);
        if self.irq_counter >= 64 {
            self.irq_counter -= 64;
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Bandai FCG / LZ93D50 (mapper 16): 16 KiB PRG banks, eight 1 KiB CHR
/// banks, mirroring control and a 16-bit CPU-cycle down-counter IRQ.
#[derive(Debug)]
pub struct BandaiFcg {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    chr_banks: [u8; 8],
    prg_bank: u8,
    number: u16,
    irq_latch: u16,
    irq_counter: u16,
    irq_enabled: bool,
    irq_pending: bool,
    mirroring: Mirroring,
}

impl BandaiFcg {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header, number: u16) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            chr_banks: [0; 8],
            prg_bank: 0,
            number,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x4000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }
}

impl Mapper for BandaiFcg {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xBFFF => {
                let banks = self.prg_bank_count();
                let bank = usize::from(self.prg_bank) % banks;
                self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
            }
            0xC000..=0xFFFF => {
                let bank = self.prg_bank_count() - 1;
                self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x6000..=0x7FFF).contains(&address) {
            let index = usize::from(address - 0x6000) % self.prg_ram.len();
            self.prg_ram[index] = value;
        }
        if !(0x6000..=0x7FFF).contains(&address) && address < 0x8000 {
            return;
        }
        match address & 0x0F {
            0x00..=0x07 => self.chr_banks[usize::from(address & 0x07)] = value,
            0x08 => self.prg_bank = value & 0x0F,
            0x09 => {
                self.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                };
            }
            0x0A => {
                self.irq_enabled = value & 0x01 != 0;
                self.irq_pending = false;
                self.irq_counter = self.irq_latch;
                if self.irq_enabled && self.irq_counter == 0 {
                    self.irq_pending = true;
                }
            }
            0x0B => {
                self.irq_latch = (self.irq_latch & 0xFF00) | u16::from(value);
                self.irq_counter = self.irq_latch;
            }
            0x0C => {
                self.irq_latch = (self.irq_latch & 0x00FF) | (u16::from(value) << 8);
                self.irq_counter = self.irq_latch;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address / 0x400) & 0x07;
        let banks = self.chr_bank_count();
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address / 0x400) & 0x07;
        let banks = self.chr_bank_count();
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.number
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        } else {
            self.irq_counter -= 1;
            if self.irq_counter == 0 {
                self.irq_pending = true;
            }
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// TXC 05-00002-010 (mapper 112): MMC3-like 8 KiB PRG and mixed 2/1 KiB CHR
/// banking with a scrambled register layout.
#[derive(Debug)]
pub struct Txc112 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    select: u8,
    prg_banks: [u8; 2],
    chr_banks: [u8; 8],
    mirroring: Mirroring,
}

impl Txc112 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            select: 0,
            prg_banks: [0, 1],
            chr_banks: [0, 2, 4, 5, 6, 7, 0, 1],
            mirroring: header.mirroring,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }
}

impl Mapper for Txc112 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = self.prg_bank_count();
        let bank = match address {
            0x8000..=0x9FFF => usize::from(self.prg_banks[0]),
            0xA000..=0xBFFF => usize::from(self.prg_banks[1]),
            0xC000..=0xDFFF => banks.saturating_sub(2),
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xE001 {
            0x8000 => self.select = value & 0x07,
            0xA000 => match self.select {
                0 => self.prg_banks[0] = value,
                1 => self.prg_banks[1] = value,
                2..=7 => self.chr_banks[usize::from(self.select)] = value,
                _ => {}
            },
            0xE000 => {
                self.mirroring = if value & 0x01 == 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = self.chr_bank_count();
        let (bank, offset) = if address < 0x1000 {
            let slot = usize::from(address / 0x800) + 2;
            (
                usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1),
                usize::from(address & 0x3FF),
            )
        } else {
            let slot = usize::from(address / 0x400);
            (
                usize::from(self.chr_banks[slot]),
                usize::from(address & 0x3FF),
            )
        };
        self.chr[(bank % banks) * 0x400 + offset]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = self.chr_bank_count();
        let (bank, offset) = if address < 0x1000 {
            let slot = usize::from(address / 0x800) + 2;
            (
                usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1),
                usize::from(address & 0x3FF),
            )
        } else {
            let slot = usize::from(address / 0x400);
            (
                usize::from(self.chr_banks[slot]),
                usize::from(address & 0x3FF),
            )
        };
        let index = (bank % banks) * 0x400 + offset;
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        112
    }
}

/// BxROM with 8 KiB WRAM (mapper 241): 32 KiB PRG banking at `$8000-$FFFF`
/// without bus conflicts, plus battery-backed PRG-RAM.
#[derive(Debug)]
pub struct BxRomWram {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    bank: u8,
    mirroring: Mirroring,
}

impl BxRomWram {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for BxRomWram {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = usize::from(self.bank) % banks;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            0x4800..=0x5FFF | 0x8000..=0xFFFF => self.bank = value & 0x3F,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        241
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Simple NROM/CNROM multicart (mappers 58/213): a single latch at
/// `$8000-$FFFF` selects a 32 KiB PRG bank, an 8 KiB CHR bank, PRG size mode
/// and mirroring.
#[derive(Debug)]
pub struct Multicart58 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: u8,
    chr_bank: u8,
    nrom128: bool,
    mirroring: Mirroring,
}

impl Multicart58 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            nrom128: false,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Multicart58 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = usize::from(self.prg_bank) % banks;
        let offset = if self.nrom128 {
            bank * 0x8000 + usize::from(address & 0x3FFF)
        } else {
            bank * 0x8000 + usize::from(address & 0x7FFF)
        };
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            return;
        }
        self.prg_bank = value & 0x07;
        self.chr_bank = (value >> 3) & 0x07;
        self.nrom128 = value & 0x40 != 0;
        self.mirroring = if value & 0x80 == 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        58
    }
}

/// CNROM with the 8 KiB CHR configuration (mapper 185). Bus conflicts apply
/// because the latch is written through the PRG window.
#[derive(Debug)]
pub struct CnRom185 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl CnRom185 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for CnRom185 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let offset = usize::from(address - 0x8000) % self.prg.len().max(1);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            return;
        }
        let visible = self
            .prg
            .get(usize::from(address - 0x8000) % self.prg.len().max(1))
            .copied()
            .unwrap_or(0xFF);
        self.chr_bank = value & visible & 0x03;
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, _address: u16, _value: u8) {}

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        185
    }
}

/// Hengge BxROM (mapper 177): 32 KiB PRG banking, mirroring control and
/// 8 KiB banked PRG-RAM.
#[derive(Debug)]
pub struct Hengge177 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    bank: u8,
    ram_bank: u8,
    mirroring: Mirroring,
}

impl Hengge177 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            bank: 0,
            ram_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Hengge177 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => {
                let offset = usize::from(self.ram_bank) * 0x2000 + usize::from(address - 0x6000);
                self.prg_ram[offset % self.prg_ram.len()]
            }
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = usize::from(self.bank) % banks;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x7FFF => {
                let offset = usize::from(self.ram_bank) * 0x2000 + usize::from(address - 0x6000);
                let len = self.prg_ram.len();
                self.prg_ram[offset % len] = value;
            }
            0x8000..=0xFFFF => {
                self.bank = value & 0x1F;
                self.mirroring = if value & 0x20 == 0 {
                    Mirroring::Vertical
                } else {
                    Mirroring::Horizontal
                };
                self.ram_bank = (value >> 6) & 0x03;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        177
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// TXC 05-00002-010 latch (mappers 132 and 173): a three-bit shift/invert
/// register pair used for CHR (and, on 132, PRG) banking plus copy
/// protection.
#[derive(Debug)]
pub struct TxcLatch {
    prg: Vec<u8>,
    chr: Vec<u8>,
    mapper: u16,
    p: u8,
    r: u8,
    s: bool,
    invert: bool,
    increment: bool,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl TxcLatch {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header, mapper: u16) -> Self {
        Self {
            prg,
            chr,
            mapper,
            p: 0,
            r: 0,
            s: false,
            invert: false,
            increment: false,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }

    fn output(&self) -> u8 {
        self.r ^ if self.invert { 0x07 } else { 0 }
    }

    fn apply(&mut self) {
        let r = self.output();
        if self.mapper == 132 {
            self.prg_bank = r & 0x01;
            self.chr_bank = ((r >> 1) & 0x01) << 1 | ((r >> 2) & 0x01);
        } else {
            let a13 = (r >> 1) & 0x01;
            let a14 = if self.invert { 0 } else { 1 };
            let a15 = r & 0x01;
            self.chr_bank = (a15 << 2) | (a14 << 1) | a13;
        }
    }
}

impl Mapper for TxcLatch {
    fn prg_read(&self, address: u16) -> u8 {
        if address & 0xE103 == 0x4100 {
            return (if self.s ^ self.invert { 0x08 } else { 0 }) | self.output();
        }
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = if self.mapper == 132 {
            usize::from(self.prg_bank) % banks
        } else {
            0
        };
        // 16 KiB PRG images mirror across the whole 32 KiB window.
        self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xE103 {
            0x4100 => {
                if self.increment {
                    self.r = self.r.wrapping_add(1) & 0x07;
                } else {
                    self.r = if self.invert { !self.p & 0x07 } else { self.p };
                }
            }
            0x4101 => self.invert = value & 0x01 != 0,
            0x4102 => {
                self.s = value & 0x08 != 0;
                self.p = value & 0x07;
            }
            0x4103 => self.increment = value & 0x01 != 0,
            0x8000 => self.apply(),
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        self.chr.get(offset).copied().unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let offset = (usize::from(self.chr_bank) % banks) * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(offset) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.mapper
    }
}

/// MMC3 clone with 32 KiB PRG banking (mapper 189): CHR, mirroring and IRQ
/// come from the MMC3 register set, while PRG is a single 32 KiB latch at
/// `$4100-$5FFF`.
#[derive(Debug)]
pub struct Mmc3_32k {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    select: u8,
    bank_data: [u8; 8],
    bank32: u8,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

impl Mmc3_32k {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram: vec![0; header.prg_ram_size.max(0x2000)],
            select: 0,
            bank_data: [0; 8],
            bank32: 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let mode = self.select & 0x80;
        let raw = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        raw % banks
    }
}

impl Mapper for Mmc3_32k {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = usize::from(self.bank32 & 0x07) % banks;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x6000 {
            if address & 0xE100 == 0x4100 {
                self.bank32 = (value >> 4) & 0x07;
            }
            return;
        }
        if address < 0x8000 {
            let index = usize::from(address - 0x6000) % self.prg_ram.len();
            self.prg_ram[index] = value;
            return;
        }
        match address & 0xE001 {
            0x8000 => self.select = value,
            0x8001 => {
                let index = usize::from(self.select & 0x07);
                self.bank_data[index] = value;
            }
            0xA000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        189
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// CoolBoy / MINDKIDS AA6023 (mappers 224/268): an oversize MMC3 clone with
/// outer PRG/CHR bank registers in a selectable `$5000-$5FFF` or
/// `$6000-$6FFF` window.
#[derive(Debug)]
pub struct Coolboy {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    base: u16,
    select: u8,
    bank_data: [u8; 8],
    outer0: u8,
    outer1: u8,
    outer3: u8,
    outer4: u8,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

impl Coolboy {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header, base: u16) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            base,
            select: 0,
            bank_data: [0; 8],
            outer0: 0,
            outer1: 0,
            outer3: 0,
            outer4: 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn outer_bits(&self) -> usize {
        let eee = usize::from(self.outer0 & 0x07);
        let cc = usize::from((self.outer0 >> 4) & 0x03);
        let a17 = eee & 0x01;
        let a18 = (eee >> 1) & 0x01;
        let a19 = (eee >> 2) & 0x01;
        let a20 = usize::from((self.outer1 >> 4) & 0x01);
        let a21 = usize::from((self.outer1 >> 2) & 0x01);
        let a22 = usize::from((self.outer1 >> 3) & 0x01);
        let a23 = cc & 0x01;
        let a24 = (cc >> 1) & 0x01;
        (a17 << 4)
            | (a18 << 5)
            | (a19 << 6)
            | (a20 << 7)
            | (a21 << 8)
            | (a22 << 9)
            | (a23 << 10)
            | (a24 << 11)
    }

    /// Applies the outer masks to an MMC3-selected 8 KiB bank.
    fn map_bank(&self, mmc3: usize) -> usize {
        let outer = self.outer_bits();
        let from_mmc3_17 = self.outer0 & 0x80 == 0;
        let from_mmc3_18 = self.outer1 & 0x80 == 0;
        let from_mmc3_19 = self.outer1 & 0x40 != 0;
        let from_mmc3_20 = self.outer1 & 0x20 != 0;
        let mut bank = mmc3 & 0x0F;
        bank |= if from_mmc3_17 {
            (mmc3 >> 4) & 0x01
        } else {
            (outer >> 4) & 0x01
        } << 4;
        bank |= if from_mmc3_18 {
            (mmc3 >> 5) & 0x01
        } else {
            (outer >> 5) & 0x01
        } << 5;
        bank |= if from_mmc3_19 {
            (mmc3 >> 6) & 0x01
        } else {
            (outer >> 6) & 0x01
        } << 6;
        bank |= if from_mmc3_20 {
            (mmc3 >> 7) & 0x01
        } else {
            (outer >> 7) & 0x01
        } << 7;
        bank | (outer & !0xFF)
    }

    fn prg_slot(&self, slot: usize) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        let last = banks - 1;
        let second_last = banks.saturating_sub(2);
        let mode = self.select & 0x40;
        let raw = match (mode, slot) {
            (0, 0) => usize::from(self.bank_data[6]),
            (0, 1) => usize::from(self.bank_data[7]),
            (0, 2) => second_last,
            (0, _) => last,
            (_, 0) => second_last,
            (_, 1) => usize::from(self.bank_data[7]),
            (_, 2) => usize::from(self.bank_data[6]),
            (_, _) => last,
        };
        self.map_bank(raw) % banks
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let mode = self.select & 0x80;
        let raw = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        raw % banks
    }

    fn is_outer(&self, address: u16) -> bool {
        address & 0xF800 == self.base
    }
}

impl Mapper for Coolboy {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let slot = usize::from((address - 0x8000) / 0x2000);
                let bank = self.prg_slot(slot);
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if self.is_outer(address) {
            match address & 0x07 {
                0 => self.outer0 = value,
                1 => self.outer1 = value,
                3 => self.outer3 = value,
                4 => self.outer4 = value,
                _ => {}
            }
            return;
        }
        if address < 0x6000 {
            return;
        }
        if address < 0x8000 {
            let index = usize::from(address - 0x6000) % self.prg_ram.len();
            self.prg_ram[index] = value;
            return;
        }
        match address & 0xE001 {
            0x8000 => self.select = value,
            0x8001 => {
                let index = usize::from(self.select & 0x07);
                self.bank_data[index] = value;
            }
            0xA000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        if self.base == 0x6000 {
            268
        } else {
            224
        }
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Enhanced MMC3 8025 (mapper 176): standard MMC3 banking plus a mode
/// register, PRG/CHR base registers and NROM/UNROM compatibility modes.
#[derive(Debug)]
pub struct Mmc3_176 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    select: u8,
    bank_data: [u8; 8],
    mode: u8,
    prg_base: u8,
    chr_base: u8,
    unrom_latch: u8,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
    ram_config: bool,
    outer_regs: bool,
    wram_bank: usize,
    wram_enable: bool,
}

impl Mmc3_176 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, mut prg_ram: Vec<u8>, header: Header) -> Self {
        // FS005 (submapper 2) exposes 32 KiB of banked WRAM.
        prg_ram.resize(0x8000, 0);
        Self {
            prg,
            chr,
            prg_ram,
            select: 0,
            bank_data: [0; 8],
            mode: 0,
            prg_base: 0,
            chr_base: 0,
            unrom_latch: 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
            ram_config: false,
            outer_regs: true,
            wram_bank: 0,
            wram_enable: true,
        }
    }

    fn wram_offset(&self, address: u16) -> usize {
        if self.ram_config {
            (self.wram_bank * 0x2000 + usize::from(address - 0x6000)) % self.prg_ram.len()
        } else {
            usize::from(address - 0x6000) % self.prg_ram.len()
        }
    }

    /// $5000-$5FFF read: outer bank registers, or the upper half of WRAM bank 2
    /// while the FS005 configuration register has them disabled.
    fn paged_read(&self, address: u16) -> u8 {
        if self.outer_regs {
            0
        } else {
            self.prg_ram[usize::from(address) % self.prg_ram.len()]
        }
    }

    fn paged_write(&mut self, address: u16, value: u8) {
        if !self.outer_regs {
            let index = usize::from(address) % self.prg_ram.len();
            self.prg_ram[index] = value;
        }
    }

    fn prg_mode(&self) -> u8 {
        self.mode & 0x07
    }

    /// Combines the MMC3 8 KiB bank with the PRG base register for modes 0-2.
    fn combine(&self, mmc3: usize) -> usize {
        match self.prg_mode() {
            0 => ((usize::from(self.prg_base) & 0x60) << 1) | (mmc3 & 0x3F),
            1 => ((usize::from(self.prg_base) & 0x70) << 1) | (mmc3 & 0x1F),
            2 => ((usize::from(self.prg_base) & 0x78) << 1) | (mmc3 & 0x0F),
            _ => mmc3,
        }
    }

    fn fixed_low(&self) -> usize {
        match self.prg_mode() {
            0 => 0x3E,
            1 => 0x1E,
            2 => 0x0E,
            _ => 0,
        }
    }

    fn fixed_high(&self) -> usize {
        match self.prg_mode() {
            0 => 0x3F,
            1 => 0x1F,
            2 => 0x0F,
            _ => 0,
        }
    }

    fn prg_bank(&self, address: u16) -> usize {
        let mode = self.prg_mode();
        if mode >= 3 {
            let a13 = usize::from((address >> 13) & 1);
            return match mode {
                3 => (usize::from(self.prg_base) << 1) | a13,
                4 => {
                    ((usize::from(self.prg_base) & 0x7E) << 1) | usize::from((address >> 13) & 0x03)
                }
                _ if address < 0xC000 => {
                    ((usize::from(self.prg_base) & 0x78) << 1)
                        | (usize::from(self.unrom_latch & 0x07) << 1)
                        | a13
                }
                _ => ((usize::from(self.prg_base) & 0x78) << 1) | (0x07 << 1) | a13,
            };
        }
        let slot = usize::from((address - 0x8000) / 0x2000);
        let mmc3_mode = self.select & 0x40;
        let raw = match (mmc3_mode, slot) {
            (0, 0) => usize::from(self.bank_data[6]),
            (0, 1) => usize::from(self.bank_data[7]),
            (0, 2) => self.fixed_low(),
            (0, _) => self.fixed_high(),
            (_, 0) => self.fixed_low(),
            (_, 1) => usize::from(self.bank_data[7]),
            (_, 2) => usize::from(self.bank_data[6]),
            (_, _) => self.fixed_high(),
        };
        self.combine(raw)
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let mode = self.select & 0x80;
        let mmc3 = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        let base = usize::from(self.chr_base);
        let outer = if self.mode & 0x08 == 0 {
            (base & 0xE0) << 3
        } else {
            (base & 0xF0) << 3
        };
        let local = if self.mode & 0x08 == 0 {
            mmc3 & 0xFF
        } else {
            mmc3 & 0x7F
        };
        outer | local
    }
}

impl Mapper for Mmc3_176 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x5000..=0x5FFF => self.paged_read(address),
            0x6000..=0x7FFF => {
                if !self.wram_enable {
                    return 0;
                }
                self.prg_ram[self.wram_offset(address)]
            }
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let bank = self.prg_bank(address) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x5000..=0x5FFF).contains(&address) {
            if self.outer_regs {
                match address & 0x03 {
                    0 => self.mode = value,
                    1 => self.prg_base = value & 0x7F,
                    2 => self.chr_base = value,
                    _ => {}
                }
            } else {
                self.paged_write(address, value);
            }
            return;
        }
        if (0x6000..=0x7FFF).contains(&address) {
            if self.wram_enable {
                let index = self.wram_offset(address);
                self.prg_ram[index] = value;
            }
            return;
        }
        if address < 0x8000 {
            return;
        }
        if self.prg_mode() == 5 {
            self.unrom_latch = value & 0x07;
            return;
        }
        match address & 0xE003 {
            0x8000 => self.select = value,
            0x8001 => {
                let index = usize::from(self.select & 0x07);
                self.bank_data[index] = value;
            }
            0xA000 => {
                self.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                };
            }
            0xA001 => {
                // FS005 RAM Configuration Register (bit 5 enables it).
                if value & 0x20 != 0 {
                    self.ram_config = true;
                    self.wram_bank = usize::from(value & 0x03);
                    self.outer_regs = value & 0x40 != 0;
                    self.wram_enable = value & 0x80 != 0;
                } else {
                    self.ram_config = false;
                    self.outer_regs = true;
                    self.wram_enable = true;
                }
            }
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        176
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Sugar Softec MMC3 clone (mapper 215): standard MMC3 registers behind a
/// selectable address/data scramble, plus NROM/outer-bank overrides.
#[derive(Debug)]
pub struct Mmc3_215 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    select: u8,
    bank_data: [u8; 8],
    mode: u8,
    outer: u8,
    scramble: u8,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

const SCRAMBLE_ADDR: [[u16; 8]; 8] = [
    [
        0x8000, 0x8001, 0xA000, 0xA001, 0xC000, 0xC001, 0xE000, 0xE001,
    ],
    [
        0xA001, 0xA000, 0x8000, 0xC000, 0x8001, 0xC001, 0xE000, 0xE001,
    ],
    [
        0x8000, 0x8001, 0xA000, 0xA001, 0xC000, 0xC001, 0xE000, 0xE001,
    ],
    [
        0xC001, 0x8000, 0x8001, 0xA000, 0xA001, 0xE001, 0xE000, 0xC000,
    ],
    [
        0xA001, 0x8001, 0x8000, 0xC000, 0xA000, 0xC001, 0xE000, 0xE001,
    ],
    [
        0x8000, 0x8001, 0xA000, 0xA001, 0xC000, 0xC001, 0xE000, 0xE001,
    ],
    [
        0x8000, 0x8001, 0xA000, 0xA001, 0xC000, 0xC001, 0xE000, 0xE001,
    ],
    [
        0x8000, 0x8001, 0xA000, 0xA001, 0xC000, 0xC001, 0xE000, 0xE001,
    ],
];

const SCRAMBLE_DATA: [[u8; 8]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7],
    [0, 2, 6, 1, 7, 3, 4, 5],
    [0, 5, 4, 1, 7, 2, 6, 3],
    [0, 6, 3, 7, 5, 2, 4, 1],
    [0, 2, 5, 3, 6, 1, 7, 4],
    [0, 1, 2, 3, 4, 5, 6, 7],
    [0, 1, 2, 3, 4, 5, 6, 7],
    [0, 1, 2, 3, 4, 5, 6, 7],
];

impl Mmc3_215 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            select: 0,
            bank_data: [0; 8],
            mode: 0,
            outer: 0x0F,
            scramble: 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn real_address(&self, address: u16) -> u16 {
        let pattern = usize::from(self.scramble & 0x07);
        let base = match address & 0xE001 {
            0x8000 => 0,
            0x8001 => 1,
            0xA000 => 2,
            0xA001 => 3,
            0xC000 => 4,
            0xC001 => 5,
            0xE000 => 6,
            _ => 7,
        };
        SCRAMBLE_ADDR[pattern][base]
    }

    fn unscramble_data(&self, value: u8) -> u8 {
        let pattern = usize::from(self.scramble & 0x07);
        let low = SCRAMBLE_DATA[pattern][usize::from(value & 0x07)];
        (value & 0xF8) | low
    }

    fn prg_bank(&self, address: u16) -> usize {
        if self.mode & 0x40 != 0 {
            // NROM mode: ignore the MMC3 PRG banks.
            let a14 = usize::from((address >> 14) & 1);
            let mut bank16 = usize::from(self.mode & 0x0F);
            if self.mode & 0x10 != 0 {
                bank16 = (bank16 & 0x0E) | a14;
            }
            let mut outer = (usize::from(self.outer) & 0x03) << 4;
            if self.mode & 0x20 != 0 {
                outer |= usize::from((self.outer >> 4) & 0x01) << 3;
            }
            return ((outer | bank16) * 2) + usize::from((address >> 13) & 1);
        }
        let slot = usize::from((address - 0x8000) / 0x2000);
        let mmc3_mode = self.select & 0x40;
        let banks = (self.prg.len() / 0x2000).max(1);
        let last = banks - 1;
        let second_last = banks.saturating_sub(2);
        let raw = match (mmc3_mode, slot) {
            (0, 0) => usize::from(self.bank_data[6]),
            (0, 1) => usize::from(self.bank_data[7]),
            (0, 2) => second_last,
            (0, _) => last,
            (_, 0) => second_last,
            (_, 1) => usize::from(self.bank_data[7]),
            (_, 2) => usize::from(self.bank_data[6]),
            (_, _) => last,
        };
        let mut bank = (usize::from(self.outer) & 0x03) << 5 | (raw & 0x1F);
        if self.mode & 0x20 == 0 {
            bank |= raw & 0x20;
        } else {
            bank |= usize::from((self.outer >> 4) & 0x01) << 4;
        }
        bank
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let mode = self.select & 0x80;
        let mmc3 = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        let outer = (usize::from(self.outer) & 0x0C) << 6;
        let mut bank = outer | (mmc3 & 0x7F);
        if self.mode & 0x20 != 0 {
            bank |= usize::from((self.outer >> 5) & 0x01) << 7;
        } else {
            bank |= mmc3 & 0x80;
        }
        bank
    }
}

impl Mapper for Mmc3_215 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let bank = self.prg_bank(address) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x5007 => {
                match address & 0x07 {
                    0 => self.mode = value,
                    1 => self.outer = value,
                    _ => {}
                }
                if address & 0x07 == 7 {
                    self.scramble = value & 0x07;
                }
            }
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            0x8000..=0xFFFF => {
                let real = self.real_address(address);
                let value = if real == 0x8000 {
                    self.unscramble_data(value)
                } else {
                    value
                };
                match real {
                    0x8000 => self.select = value,
                    0x8001 => {
                        let index = usize::from(self.select & 0x07);
                        self.bank_data[index] = value;
                    }
                    0xA000 => {
                        self.mirroring = if value & 1 != 0 {
                            Mirroring::Horizontal
                        } else {
                            Mirroring::Vertical
                        };
                    }
                    0xC000 => self.irq_latch = value,
                    0xC001 => {
                        self.irq_counter = 0;
                        self.irq_reload = true;
                    }
                    0xE000 => {
                        self.irq_enabled = false;
                        self.irq_pending = false;
                    }
                    0xE001 => self.irq_enabled = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        215
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Kasheng SFC-02B/-03/-004 (mapper 115): MMC3 clone with a 256 KiB outer
/// CHR bank and an NROM-style PRG override register.
#[derive(Debug)]
pub struct Mmc3_115 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    select: u8,
    bank_data: [u8; 8],
    mode: u8,
    chr_outer: u8,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

impl Mmc3_115 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            select: 0,
            bank_data: [0; 8],
            mode: 0,
            chr_outer: 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn prg_bank(&self, address: u16) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        if self.mode & 0x40 != 0 {
            let mut bank16 =
                (usize::from(self.mode & 0x0F)) | (usize::from((self.mode >> 5) & 0x01) << 4);
            if self.mode & 0x10 != 0 {
                bank16 = (bank16 & 0x0E) | usize::from((address >> 14) & 1);
                return ((bank16 * 2) + usize::from((address >> 13) & 1)) % banks;
            }
            return ((bank16 * 2) + usize::from((address >> 13) & 1)) % banks;
        }
        let slot = usize::from((address - 0x8000) / 0x2000);
        let mmc3_mode = self.select & 0x40;
        let last = banks - 1;
        let second_last = banks.saturating_sub(2);
        let raw = match (mmc3_mode, slot) {
            (0, 0) => usize::from(self.bank_data[6]),
            (0, 1) => usize::from(self.bank_data[7]),
            (0, 2) => second_last,
            (0, _) => last,
            (_, 0) => second_last,
            (_, 1) => usize::from(self.bank_data[7]),
            (_, 2) => usize::from(self.bank_data[6]),
            (_, _) => last,
        };
        (raw | (usize::from((self.mode >> 5) & 0x01) << 5)) % banks
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let mode = self.select & 0x80;
        let mmc3 = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        mmc3 | (usize::from(self.chr_outer & 0x01) << 8)
    }
}

impl Mapper for Mmc3_115 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => 0,
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let bank = self.prg_bank(address) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xE003 {
            0x6000 => self.mode = value,
            0x6001 => self.chr_outer = value & 0x01,
            0x6002 => {}
            0x8000 => self.select = value,
            0x8001 => {
                let index = usize::from(self.select & 0x07);
                self.bank_data[index] = value;
            }
            0xA000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        115
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Sachen 8259 (mappers 138/139/141): register-pair banking with four 2 KiB
/// CHR windows, an outer CHR bank and a 32 KiB PRG bank.
#[derive(Debug)]
pub struct Sachen8259 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    mapper: u16,
    shift: u8,
    select: u8,
    regs: [u8; 8],
    mirroring: Mirroring,
}

impl Sachen8259 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header, mapper: u16, shift: u8) -> Self {
        Self {
            prg,
            chr,
            mapper,
            shift,
            select: 0,
            regs: [0; 8],
            mirroring: header.mirroring,
        }
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let raw = (usize::from(self.regs[4] & 0x07) << 3) | usize::from(self.regs[slot] & 0x07);
        raw << self.shift
    }
}

impl Mapper for Sachen8259 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = usize::from(self.regs[5]) % banks;
        self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xC101 {
            0x4100 => self.select = value & 0x07,
            0x4101 => {
                let index = usize::from(self.select);
                self.regs[index] = value & 0x07;
                if index == 7 {
                    self.mirroring = match (value >> 1) & 0x03 {
                        0 => Mirroring::Vertical,
                        1 => Mirroring::Horizontal,
                        2 => Mirroring::FourScreen,
                        _ => Mirroring::SingleUpper,
                    };
                }
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x800).max(1);
        let slot = usize::from(address / 0x800) & 0x03;
        let bank = self.chr_bank(slot) % banks;
        self.chr[bank * 0x800 + usize::from(address & 0x7FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x800).max(1);
        let slot = usize::from(address / 0x800) & 0x03;
        let bank = self.chr_bank(slot) % banks;
        let index = bank * 0x800 + usize::from(address & 0x7FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.mapper
    }
}

/// Camerica Quattro (mapper 232): mapper 71 plus a 64 KiB outer block
/// register. `$8000-$BFFF` selects the outer block, `$C000-$FFFF` the inner
/// 16 KiB bank; `$C000-$FFFF` is fixed to the last bank of the block.
#[derive(Debug)]
pub struct Quattro232 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    outer: u8,
    inner: u8,
    mirroring: Mirroring,
}

impl Quattro232 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            outer: 0,
            inner: 0,
            mirroring: header.mirroring,
        }
    }

    fn bank(&self, address: u16) -> usize {
        let base = usize::from(self.outer & 0x03) << 2;
        if address < 0xC000 {
            base | (usize::from(self.inner) & 0x03)
        } else {
            base | 0x03
        }
    }
}

impl Mapper for Quattro232 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x4000).max(1);
        let bank = self.bank(address) % banks;
        self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x8000..=0xBFFF => self.outer = (value >> 3) & 0x03,
            0xC000..=0xFFFF => self.inner = value & 0x0F,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        232
    }
}

/// Namco 175/340 (mapper 210): a cost-reduced Namco 163 with 8 KiB PRG and
/// 1 KiB CHR banking, no IRQ or expansion audio.
#[derive(Debug)]
pub struct Namco210 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    chr_banks: [u8; 8],
    prg_banks: [u8; 3],
    select_mirroring: bool,
    mirroring: Mirroring,
}

impl Namco210 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            chr_banks: [0; 8],
            prg_banks: [0; 3],
            // Battery-backed boards are the hardwired-mirroring Namco 175.
            select_mirroring: !header.battery,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }
}

impl Mapper for Namco210 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0x9FFF => {
                let banks = self.prg_bank_count();
                let bank = usize::from(self.prg_banks[0]) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xA000..=0xBFFF => {
                let banks = self.prg_bank_count();
                let bank = usize::from(self.prg_banks[1]) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xC000..=0xDFFF => {
                let banks = self.prg_bank_count();
                let bank = usize::from(self.prg_banks[2]) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xE000..=0xFFFF => {
                let bank = self.prg_bank_count() - 1;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            0x8000..=0xBFFF => {
                let slot = usize::from((address - 0x8000) / 0x800) & 0x07;
                self.chr_banks[slot] = value;
            }
            0xE000..=0xE7FF => {
                self.prg_banks[0] = value & 0x3F;
                if self.select_mirroring {
                    self.mirroring = match (value >> 6) & 0x03 {
                        0 => Mirroring::SingleLower,
                        1 => Mirroring::Vertical,
                        2 => Mirroring::SingleUpper,
                        _ => Mirroring::Horizontal,
                    };
                }
            }
            0xE800..=0xEFFF => self.prg_banks[1] = value & 0x3F,
            0xF000..=0xF7FF => self.prg_banks[2] = value & 0x3F,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = self.chr_bank_count();
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = self.chr_bank_count();
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        210
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Kasheng A9711/A9713 (mapper 121): MMC3 clone with a read-back protection
/// array, a bit-reversed protection bank latch and an outer 256 KiB bank.
#[derive(Debug)]
pub struct Mmc3_121 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    select: u8,
    bank_data: [u8; 8],
    array_index: u8,
    prot_index: u8,
    latch: u8,
    prot_slot: Option<usize>,
    outer: u8,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

const PROT_ARRAY: [u8; 4] = [0x83, 0x83, 0x42, 0x00];
const PROT_ARRAY_ALT: [u8; 4] = [0x00, 0x02, 0x02, 0x03];

fn reverse_lower_six(value: u8) -> u8 {
    let mut result = 0u8;
    for bit in 0..6 {
        if value & (1 << bit) != 0 {
            result |= 1 << (5 - bit);
        }
    }
    (value & 0xC0) | result
}

impl Mmc3_121 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            select: 0,
            bank_data: [0; 8],
            array_index: 0,
            prot_index: 0xFF,
            latch: 0,
            prot_slot: None,
            outer: 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn prg_slot(&self, slot: usize) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        let last = banks - 1;
        let second_last = banks.saturating_sub(2);
        let mode = self.select & 0x40;
        let raw = match (mode, slot) {
            (0, 0) => usize::from(self.bank_data[6]),
            (0, 1) => usize::from(self.bank_data[7]),
            (0, 2) => second_last,
            (0, _) => last,
            (_, 0) => second_last,
            (_, 1) => usize::from(self.bank_data[7]),
            (_, 2) => usize::from(self.bank_data[6]),
            (_, _) => last,
        };
        (raw | (usize::from(self.outer >> 7) << 5)) % banks
    }

    fn override_slot(&self, slot: usize) -> Option<usize> {
        if self.prot_slot == Some(slot) {
            let banks = (self.prg.len() / 0x2000).max(1);
            let raw = usize::from(self.latch & 0x3F) | (usize::from(self.outer >> 7) << 5);
            Some(raw % banks)
        } else {
            None
        }
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let mode = self.select & 0x80;
        let raw = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        (raw | (usize::from(self.outer >> 7) << 8)) % banks
    }

    fn apply_protection(&mut self) {
        let slot = match self.prot_index {
            0x26 => Some(3),
            0x28 => Some(2),
            0x2A => Some(1),
            0x2C | 0x20 | 0x29 | 0x2B | 0x3C | 0x3F => {
                if self.latch & 0x3F != 0 {
                    Some(3)
                } else {
                    None
                }
            }
            _ => None,
        };
        self.prot_slot = slot;
    }
}

impl Mapper for Mmc3_121 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x5000..=0x5FFF => {
                let array = if address & 0x0100 != 0 {
                    PROT_ARRAY_ALT
                } else {
                    PROT_ARRAY
                };
                array[usize::from(self.array_index & 0x03)]
            }
            0x6000..=0x7FFF => 0,
            0x8000..=0xFFFF => {
                let slot = usize::from((address - 0x8000) / 0x2000);
                let bank = self
                    .override_slot(slot)
                    .unwrap_or_else(|| self.prg_slot(slot));
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            if address & 0xF180 == 0x5180 {
                self.outer = value;
            } else if (0x5000..=0x5FFF).contains(&address) {
                self.array_index = value & 0x03;
            }
            return;
        }
        match address & 0xE003 {
            0x8000 => self.select = value,
            0x8001 => {
                self.latch = reverse_lower_six(value);
                let index = usize::from(self.select & 0x07);
                self.bank_data[index] = value;
                if matches!(self.prot_index, 0x26 | 0x28 | 0x2A) {
                    self.apply_protection();
                }
            }
            0x8003 => {
                self.prot_index = value & 0x3F;
                self.select = value;
                self.apply_protection();
            }
            0xA000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        121
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// GTROM / Cheapocabra (mapper 111, CHR-RAM variant): 32 KiB PRG pages
/// selected through `$5000-$5FFF` / `$7000-$7FFF`.
#[derive(Debug)]
pub struct Gtrom {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: u8,
    mirroring: Mirroring,
}

impl Gtrom {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Gtrom {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = usize::from(self.bank & 0x0F) % banks;
        self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x5000..=0x5FFF).contains(&address) || (0x7000..=0x7FFF).contains(&address) {
            self.bank = value;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        111
    }
}

/// Cony/Yoko ASIC (mapper 83): PRG base + mode registers, four 8 KiB PRG
/// windows, eight 1 KiB CHR windows, scratch RAM and an up/down IRQ counter.
#[derive(Debug)]
pub struct Cony83 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_base: u8,
    mode: u8,
    regs8k: [u8; 4],
    chr_banks: [u8; 8],
    scratch: [u8; 4],
    irq_counter: u16,
    irq_enabled: bool,
    irq_pending: bool,
    irq_source_a12: bool,
    mirroring: Mirroring,
}

impl Cony83 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            prg_base: 0,
            mode: 0,
            regs8k: [0; 4],
            chr_banks: [0; 8],
            scratch: [0; 4],
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            irq_source_a12: false,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank_count(&self) -> usize {
        (self.prg.len() / 0x2000).max(1)
    }

    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / 0x400).max(1)
    }

    fn high_bits(&self) -> usize {
        (usize::from(self.prg_base) >> 4) << 5
    }

    fn prg_bank(&self, address: u16) -> usize {
        match (self.mode >> 2) & 0x03 {
            0 => {
                let a14 = usize::from((address >> 14) & 1);
                let bank16 = if a14 == 1 {
                    (usize::from(self.prg_base) & 0xF0) | 0x0F
                } else {
                    usize::from(self.prg_base)
                };
                (bank16 * 2) + usize::from((address >> 13) & 1)
            }
            1 => {
                let bank16 = usize::from(self.prg_base) & 0xFE;
                (bank16 * 2) + usize::from((address >> 13) & 1)
            }
            _ => {
                if address >= 0xE000 {
                    self.high_bits() | 0x1F
                } else {
                    let window = usize::from((address - 0x8000) / 0x2000);
                    self.high_bits() | (usize::from(self.regs8k[window]) & 0x1F)
                }
            }
        }
    }

    fn irq_clock(&mut self) {
        if !self.irq_enabled || self.irq_counter == 0 {
            return;
        }
        if self.mode & 0x80 != 0 {
            self.irq_counter = self.irq_counter.wrapping_sub(1);
        } else {
            self.irq_counter = self.irq_counter.wrapping_add(1);
        }
        if self.irq_counter == 0 {
            self.irq_enabled = false;
            self.irq_pending = true;
        }
    }
}

impl Mapper for Cony83 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x5100..=0x5103 => self.scratch[usize::from(address & 0x03)],
            0x5000..=0x5FFF => 0,
            0x6000..=0x7FFF => {
                if self.mode & 0x10 != 0 {
                    let banks = self.prg_bank_count();
                    let bank = (self.high_bits() | usize::from(self.regs8k[3])) % banks;
                    self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
                } else {
                    self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()]
                }
            }
            0x8000..=0xFFFF => {
                let banks = self.prg_bank_count();
                let bank = self.prg_bank(address) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5100..=0x5103 => self.scratch[usize::from(address & 0x03)] = value,
            0x5000..=0x5FFF => {}
            0x8000..=0xFFFF => {
                if address & 0x8300 == 0x8000 {
                    self.prg_base = value;
                } else if address & 0x8300 == 0x8100 {
                    self.mode = value;
                    self.mirroring = match value & 0x03 {
                        0 => Mirroring::Horizontal,
                        1 => Mirroring::Vertical,
                        2 => Mirroring::SingleLower,
                        _ => Mirroring::SingleUpper,
                    };
                } else if address & 0x8301 == 0x8200 {
                    self.irq_counter = (self.irq_counter & 0xFF00) | u16::from(value);
                    self.irq_pending = false;
                } else if address & 0x8301 == 0x8201 {
                    self.irq_counter = (self.irq_counter & 0x00FF) | (u16::from(value) << 8);
                    if self.mode & 0x40 != 0 {
                        self.irq_enabled = true;
                    }
                } else if address & 0x8310 == 0x8300 {
                    self.regs8k[usize::from(address & 0x03)] = value;
                } else if address & 0x8318 == 0x8310 {
                    self.chr_banks[usize::from(address & 0x07)] = value;
                } else if address & 0x8318 == 0x8318 {
                    self.irq_source_a12 = value != 0;
                }
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = self.chr_bank_count();
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = self.chr_bank_count();
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        83
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if !self.irq_source_a12 {
            self.irq_clock();
        }
    }

    fn clock_scanline(&mut self) {
        if self.irq_source_a12 {
            for _ in 0..8 {
                self.irq_clock();
            }
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Taito TC0190 (mapper 33): two 8 KiB PRG banks, six CHR windows and
/// mirroring, without IRQs.
#[derive(Debug)]
pub struct Taito33 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_banks: [u8; 2],
    chr_banks: [u8; 6],
    mirroring: Mirroring,
}

impl Taito33 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_banks: [0, 1],
            chr_banks: [0, 1, 4, 5, 6, 7],
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Taito33 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = match address {
            0x8000..=0x9FFF => usize::from(self.prg_banks[0]),
            0xA000..=0xBFFF => usize::from(self.prg_banks[1]),
            0xC000..=0xDFFF => banks.saturating_sub(2),
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xA003 {
            0x8000 => {
                self.prg_banks[0] = value & 0x3F;
                self.mirroring = if value & 0x40 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0x8001 => self.prg_banks[1] = value & 0x3F,
            0x8002 => self.chr_banks[0] = value,
            0x8003 => self.chr_banks[1] = value,
            0xA000..=0xA003 => self.chr_banks[2 + usize::from(address & 0x03)] = value,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = if address < 0x1000 {
            let slot = usize::from(address / 0x800);
            usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1)
        } else {
            let slot = usize::from((address - 0x1000) / 0x400) + 2;
            usize::from(self.chr_banks[slot])
        };
        self.chr[(bank % banks) * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = if address < 0x1000 {
            let slot = usize::from(address / 0x800);
            usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1)
        } else {
            let slot = usize::from((address - 0x1000) / 0x400) + 2;
            usize::from(self.chr_banks[slot])
        };
        let index = (bank % banks) * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        33
    }
}

/// Konami VRC1 (mapper 75): three 8 KiB PRG banks, two 4 KiB CHR banks with
/// high bits from the mirroring register, no IRQ.
#[derive(Debug)]
pub struct Vrc1 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_banks: [u8; 3],
    chr_low: [u8; 2],
    chr_high: u8,
    mirroring: Mirroring,
}

impl Vrc1 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_banks: [0, 1, 2],
            chr_low: [0, 0],
            chr_high: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Vrc1 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = match address {
            0x8000..=0x9FFF => usize::from(self.prg_banks[0]),
            0xA000..=0xBFFF => usize::from(self.prg_banks[1]),
            0xC000..=0xDFFF => usize::from(self.prg_banks[2]),
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x8000..=0x8FFF => self.prg_banks[0] = value & 0x0F,
            0x9000..=0x9FFF => {
                self.chr_high = value;
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xA000..=0xAFFF => self.prg_banks[1] = value & 0x0F,
            0xC000..=0xCFFF => self.prg_banks[2] = value & 0x0F,
            0xE000..=0xEFFF => self.chr_low[0] = value & 0x0F,
            0xF000..=0xFFFF => self.chr_low[1] = value & 0x0F,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x1000).max(1);
        let slot = usize::from(address / 0x1000) & 0x01;
        let high = usize::from((self.chr_high >> (slot + 1)) & 0x01) << 4;
        let bank = (high | usize::from(self.chr_low[slot] & 0x0F)) % banks;
        self.chr[bank * 0x1000 + usize::from(address & 0x0FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x1000).max(1);
        let slot = usize::from(address / 0x1000) & 0x01;
        let high = usize::from((self.chr_high >> (slot + 1)) & 0x01) << 4;
        let bank = (high | usize::from(self.chr_low[slot] & 0x0F)) % banks;
        let index = bank * 0x1000 + usize::from(address & 0x0FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        75
    }
}

/// Taito X1-005 (mapper 80): three 8 KiB PRG banks, six CHR windows,
/// mirroring, 128 bytes of internal RAM and a RAM-enable register.
#[derive(Debug)]
pub struct Taito80 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    internal_ram: [u8; 128],
    ram_enabled: bool,
    prg_banks: [u8; 3],
    chr_banks: [u8; 6],
    mirroring: Mirroring,
    single_screen: bool,
    mapper: u16,
}

impl Taito80 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header, single_screen: bool, mapper: u16) -> Self {
        Self {
            prg,
            chr,
            internal_ram: [0; 128],
            ram_enabled: false,
            prg_banks: [0, 1, 2],
            chr_banks: [0, 1, 4, 5, 6, 7],
            mirroring: header.mirroring,
            single_screen,
            mapper,
        }
    }

    fn register(address: u16) -> Option<usize> {
        if address & 0xFF00 == 0x7E00 && address & 0x70 == 0x70 {
            Some(usize::from(address & 0x0F))
        } else {
            None
        }
    }
}

impl Mapper for Taito80 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x7F00..=0x7FFF => {
                if self.ram_enabled {
                    self.internal_ram[usize::from(address & 0x7F)]
                } else {
                    0
                }
            }
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let bank = match address {
                    0x8000..=0x9FFF => usize::from(self.prg_banks[0]),
                    0xA000..=0xBFFF => usize::from(self.prg_banks[1]),
                    0xC000..=0xDFFF => usize::from(self.prg_banks[2]),
                    _ => banks - 1,
                };
                self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x7F00..=0x7FFF).contains(&address) {
            if self.ram_enabled {
                self.internal_ram[usize::from(address & 0x7F)] = value;
            }
            return;
        }
        if let Some(reg) = Self::register(address) {
            match reg {
                0..=5 => self.chr_banks[reg] = value,
                6 => {
                    self.mirroring = if value & 1 != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                }
                8 => self.ram_enabled = value != 0,
                0xA => self.prg_banks[0] = value & 0x3F,
                0xC => self.prg_banks[1] = value & 0x3F,
                0xE => self.prg_banks[2] = value & 0x3F,
                _ => {}
            }
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = if address < 0x1000 {
            let slot = usize::from(address / 0x800);
            usize::from(self.chr_banks[slot] & 0xFE) + usize::from((address >> 10) & 1)
        } else {
            let slot = usize::from((address - 0x1000) / 0x400) + 2;
            usize::from(self.chr_banks[slot])
        };
        self.chr[(bank % banks) * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = if address < 0x1000 {
            let slot = usize::from(address / 0x800);
            usize::from(self.chr_banks[slot] & 0xFE) + usize::from((address >> 10) & 1)
        } else {
            let slot = usize::from((address - 0x1000) / 0x400) + 2;
            usize::from(self.chr_banks[slot])
        };
        let index = (bank % banks) * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.single_screen {
            Mirroring::SingleLower
        } else {
            self.mirroring
        }
    }

    fn number(&self) -> u16 {
        self.mapper
    }
}

/// Address-latch multicart (mapper 242): the write address itself selects the
/// outer/inner PRG banks, mirroring and NROM/UNROM modes.
#[derive(Debug)]
pub struct Multicart242 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    latch: u16,
    outer19: bool,
    mapper: u16,
    mirroring: Mirroring,
}

impl Multicart242 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header, outer19: bool, mapper: u16) -> Self {
        Self {
            prg,
            chr,
            latch: 0,
            outer19,
            mapper,
            mirroring: header.mirroring,
        }
    }

    fn bank(&self, address: u16) -> usize {
        let s = usize::from(self.latch & 0x01);
        let o = usize::from((self.latch >> 7) & 0x01);
        let l = usize::from((self.latch >> 9) & 0x01);
        let inner = usize::from((self.latch >> 2) & 0x07);
        let mut outer = usize::from((self.latch >> 5) & 0x03);
        if self.outer19 {
            outer |= usize::from((self.latch >> 8) & 0x01) << 2;
        }
        let bank16 = (outer << 3) | inner;
        let cpu_a14 = usize::from((address >> 14) & 1);
        if address < 0xC000 {
            if s == 1 {
                bank16 & !1
            } else {
                bank16
            }
        } else if o == 1 {
            if s == 0 {
                bank16
            } else {
                (bank16 & !1) | cpu_a14
            }
        } else if l == 0 {
            outer << 3
        } else {
            (outer << 3) | 7
        }
    }
}

impl Mapper for Multicart242 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x4000).max(1);
        let bank = self.bank(address) % banks;
        self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            self.latch = address;
            self.mirroring = if address & 0x02 != 0 {
                Mirroring::Horizontal
            } else {
                Mirroring::Vertical
            };
            let _ = value;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.mapper
    }
}

/// Sunsoft-1 (mapper 184): a single CHR latch at `$6000-$7FFF` selecting the
/// two 4 KiB CHR banks; PRG is a fixed 32 KiB.
#[derive(Debug)]
pub struct Sunsoft1 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: u8,
    mirroring: Mirroring,
}

impl Sunsoft1 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Sunsoft1 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let offset = usize::from(address & 0x7FFF) % self.prg.len().max(1);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x6000..=0x7FFF).contains(&address) {
            self.bank = value;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x1000).max(1);
        let bank = if address < 0x1000 {
            usize::from(self.bank & 0x07)
        } else {
            usize::from(((self.bank >> 4) & 0x03) | 0x04)
        } % banks;
        self.chr[bank * 0x1000 + usize::from(address & 0x0FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x1000).max(1);
        let bank = if address < 0x1000 {
            usize::from(self.bank & 0x07)
        } else {
            usize::from(((self.bank >> 4) & 0x03) | 0x04)
        } % banks;
        let index = bank * 0x1000 + usize::from(address & 0x0FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        184
    }
}

/// Tengen RAMBO-1 (mapper 64): MMC3-like PRG/CHR banking with a 16-entry
/// register file, CHR A12 inversion, 1 KiB CHR mode and a scanline or
/// CPU-cycle IRQ counter.
#[derive(Debug)]
pub struct Rambo1 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    select: u8,
    bank_data: [u8; 16],
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
    irq_cycle_mode: bool,
    prescaler: u8,
    mirror_from_chr: bool,
    mapper: u16,
}

impl Rambo1 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header, mirror_from_chr: bool, mapper: u16) -> Self {
        Self {
            prg,
            chr,
            select: 0,
            bank_data: [0; 16],
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
            irq_cycle_mode: false,
            prescaler: 0,
            mirror_from_chr,
            mapper,
        }
    }

    fn chr_bank(&self, address: u16) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let c = self.select & 0x80 != 0;
        let k = self.select & 0x20 != 0;
        let slot = usize::from(address / 0x400) & 0x07;
        let (index, two_k) = match (c, k) {
            (false, false) => match slot {
                0 | 1 => (0, true),
                2 | 3 => (1, true),
                4 => (2, false),
                5 => (3, false),
                6 => (4, false),
                _ => (5, false),
            },
            (false, true) => match slot {
                0 => (0, false),
                1 => (8, false),
                2 => (1, false),
                3 => (9, false),
                4 => (2, false),
                5 => (3, false),
                6 => (4, false),
                _ => (5, false),
            },
            (true, false) => match slot {
                0 => (2, false),
                1 => (3, false),
                2 => (4, false),
                3 => (5, false),
                4 | 5 => (0, true),
                _ => (1, true),
            },
            (true, true) => match slot {
                0 => (2, false),
                1 => (3, false),
                2 => (4, false),
                3 => (5, false),
                4 => (0, false),
                5 => (8, false),
                6 => (1, false),
                _ => (9, false),
            },
        };
        let mut bank = usize::from(self.bank_data[index]);
        if two_k {
            bank = (bank & !1) | usize::from((address >> 10) & 1);
        }
        bank % banks
    }

    fn prg_bank(&self, address: u16) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        let mode = self.select & 0x40 != 0;
        let bank = match address {
            0x8000..=0x9FFF => {
                if mode {
                    usize::from(self.bank_data[0x0F])
                } else {
                    usize::from(self.bank_data[6])
                }
            }
            0xA000..=0xBFFF => usize::from(self.bank_data[7]),
            0xC000..=0xDFFF => {
                if mode {
                    usize::from(self.bank_data[6])
                } else {
                    usize::from(self.bank_data[0x0F])
                }
            }
            _ => banks - 1,
        };
        bank % banks
    }

    fn irq_clock(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }
}

impl Mapper for Rambo1 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = self.prg_bank(address) % banks;
        self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xE001 {
            0x8000 => self.select = value,
            0x8001 => {
                let index = usize::from(self.select & 0x0F);
                self.bank_data[index] = value;
            }
            0xA000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_cycle_mode = value & 1 != 0;
                self.irq_counter = 0;
                self.irq_reload = true;
                self.prescaler = 0;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let bank = self.chr_bank(address);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let bank = self.chr_bank(address);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        if self.mirror_from_chr {
            if self.bank_data[0] & 0x80 != 0 {
                Mirroring::SingleUpper
            } else {
                Mirroring::SingleLower
            }
        } else {
            self.mirroring
        }
    }

    fn number(&self) -> u16 {
        self.mapper
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if !self.irq_cycle_mode {
            return;
        }
        self.prescaler += 1;
        if self.prescaler >= 4 {
            self.prescaler = 0;
            self.irq_clock();
        }
    }

    fn clock_scanline(&mut self) {
        if !self.irq_cycle_mode {
            self.irq_clock();
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Address-latch multicart (mapper 59): the write address selects an 8 KiB
/// CHR bank, mirroring, a 16 KiB PRG bank and the NROM-128/256 mode, with a
/// lock bit and a DIP-switch read mode.
#[derive(Debug)]
pub struct Multicart59 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    latch: u16,
    locked: bool,
    mirroring: Mirroring,
}

impl Multicart59 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            latch: 0,
            locked: false,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank(&self, address: u16) -> usize {
        let bank = usize::from((self.latch >> 4) & 0x07);
        let nrom256 = (self.latch >> 7) & 0x01 == 0;
        if nrom256 {
            (bank & 0x06) | usize::from((address >> 14) & 1)
        } else {
            bank
        }
    }
}

impl Mapper for Multicart59 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        if self.latch & 0x0100 != 0 {
            return 0; // DIP-switch read mode (jumper value 0).
        }
        let banks = (self.prg.len() / 0x4000).max(1);
        let bank = self.prg_bank(address) % banks;
        self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 || self.locked {
            return;
        }
        self.latch = address;
        self.locked = address & 0x0400 != 0;
        self.mirroring = if address & 0x0008 != 0 {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        };
        let _ = value;
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.latch & 0x07) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.latch & 0x07) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        59
    }
}

/// HP-898F / KD-7/9-E (NES 2.0 mapper 319): two 16 KiB PRG windows selected
/// by a `$6004` register with mirroring, plus an 8 KiB CHR bank at `$6000`.
#[derive(Debug)]
pub struct Hp898f {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_a15: u8,
    prg_a16: u8,
    prg_a14: u8,
    cpu_a14_mode: bool,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Hp898f {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_a15: 0,
            prg_a16: 0,
            prg_a14: 0,
            cpu_a14_mode: false,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Hp898f {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x4000).max(1);
        let base = (u16::from(self.prg_a16) << 2) | (u16::from(self.prg_a15) << 1);
        let a14 = if self.cpu_a14_mode {
            (address >> 14) & 1
        } else {
            u16::from(self.prg_a14)
        };
        let bank = usize::from(base | a14) % banks;
        self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0x6004 {
            0x6004 => {
                self.prg_a15 = value & 0x01;
                self.prg_a16 = (value >> 1) & 0x01;
                self.prg_a14 = (value >> 2) & 0x01;
                self.cpu_a14_mode = value & 0x08 != 0;
                self.mirroring = if value & 0x10 != 0 {
                    Mirroring::Vertical
                } else {
                    Mirroring::Horizontal
                };
            }
            0x6000 => self.chr_bank = value & 0x0F,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        319
    }
}

/// FDS-to-cartridge conversions (mapper 108): an 8 KiB switchable bank at
/// `$6000-$7FFF`, a fixed last 32 KiB at `$8000-$FFFF`, and CHR-RAM or a
/// switchable 8 KiB CHR-ROM bank.
#[derive(Debug)]
pub struct Fds108 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_rom: bool,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Fds108 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            chr_rom: header.chr_size > 0,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Fds108 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let bank = usize::from(self.prg_bank) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = banks - 1;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0xE000 {
            if self.chr_rom {
                self.prg_bank = value;
                self.chr_bank = value;
            } else if address >= 0xF000 {
                self.prg_bank = value;
            }
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        108
    }
}

/// Shared banked CHR-RAM state for VRC7 boards (Lagrange Point uses CHR RAM
/// and switches 1 KiB banks at runtime).
#[derive(Debug)]
pub struct Vrc7Chr {
    ram: Vec<u8>,
    banks: [u8; 8],
}

impl Vrc7Chr {
    fn new(ram: Vec<u8>) -> Self {
        Self { ram, banks: [0; 8] }
    }

    fn set_banks(&mut self, banks: [u8; 8]) {
        self.banks = banks;
    }

    pub(crate) fn read(&self, address: u16) -> u8 {
        let slot = usize::from(address / 0x400) & 0x07;
        let banks = (self.ram.len() / 0x400).max(1);
        let bank = usize::from(self.banks[slot]) % banks;
        self.ram[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    pub(crate) fn write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address / 0x400) & 0x07;
        let banks = (self.ram.len() / 0x400).max(1);
        let bank = usize::from(self.banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.ram.get_mut(index) {
            *byte = value;
        }
    }
}

/// Famicom Disk System wavetable expansion audio (2C33).
///
/// The 64-step wave RAM and 32-entry modulation table are exposed through the
/// `$4040-$408A` register window; the module is clocked once per CPU cycle and
/// produces a signed sample for the APU mixer. The board/BIOS loader is a
/// separate follow-up; this device is self-contained so it can be tested and
/// attached to an FDS mapper later.
#[derive(Debug, Clone)]
pub struct FdsAudio {
    wave: [u8; 64],
    mod_table: [u8; 32],
    enabled: bool,
    wave_pos: f32,
    frequency: u16,
    master_volume: u8,
    volume_gain: u8,
    volume_speed: u8,
    volume_increase: bool,
    envelope_off: bool,
    volume_timer: u32,
    mod_pos: f32,
    mod_frequency: u16,
    mod_counter: i8,
    mod_index: u8,
    mod_unit_off: bool,
    mod_timer: u32,
    wave_write: bool,
    mod_write: bool,
    output: f32,
}

impl Default for FdsAudio {
    fn default() -> Self {
        Self {
            wave: [0; 64],
            mod_table: [0; 32],
            enabled: false,
            wave_pos: 0.0,
            frequency: 0,
            master_volume: 0,
            volume_gain: 0,
            volume_speed: 0,
            volume_increase: false,
            envelope_off: false,
            volume_timer: 0,
            mod_pos: 0.0,
            mod_frequency: 0,
            mod_counter: 0,
            mod_index: 0,
            mod_unit_off: false,
            mod_timer: 0,
            wave_write: false,
            mod_write: false,
            output: 0.0,
        }
    }
}

impl FdsAudio {
    /// Writes an FDS audio register (`$4023` or `$4040-$408A`).
    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x4023 => {
                let was = self.enabled;
                self.enabled = value & 0x02 != 0;
                if was && !self.enabled {
                    self.wave_pos = 0.0;
                    self.mod_pos = 0.0;
                    self.mod_counter = 0;
                    self.volume_gain = 0;
                    self.output = 0.0;
                }
            }
            0x4040..=0x407F => {
                if self.wave_write {
                    self.wave[usize::from(address - 0x4040) & 0x3F] = value & 0x3F;
                }
            }
            0x4080 => {
                self.envelope_off = value & 0x80 != 0;
                self.volume_increase = value & 0x40 != 0;
                self.volume_speed = value & 0x3F;
                if self.envelope_off {
                    self.volume_gain = self.volume_speed;
                }
                self.volume_timer = 0;
            }
            0x4082 => self.frequency = (self.frequency & 0x0F00) | u16::from(value),
            0x4083 => {
                self.frequency = (self.frequency & 0x00FF) | (u16::from(value & 0x0F) << 8);
                // Bit 6 disables the envelopes; bit 7 speeds them up and
                // halts the modulation accumulator.
                let disable = value & 0x40 != 0;
                self.envelope_off |= disable;
                if disable {
                    self.volume_timer = 0;
                }
                self.mod_unit_off = value & 0x80 != 0;
            }
            0x4084 => {
                self.mod_unit_off = value & 0x80 != 0;
                self.mod_timer = 0;
            }
            0x4085 => self.mod_counter = (value & 0x7F) as i8,
            0x4086 => self.mod_frequency = (self.mod_frequency & 0x0F00) | u16::from(value),
            0x4087 => {
                self.mod_frequency = (self.mod_frequency & 0x00FF) | (u16::from(value & 0x0F) << 8);
                self.mod_write = value & 0x80 != 0;
                if self.mod_write {
                    self.mod_pos = 0.0;
                }
            }
            0x4088 => {
                if self.mod_write {
                    let index = usize::from(self.mod_index >> 1) & 0x1F;
                    self.mod_table[index] = value & 0x3F;
                    self.mod_index = (self.mod_index + 1) & 0x3F;
                }
            }
            0x4089 => {
                self.master_volume = value & 0x03;
                self.wave_write = value & 0x80 != 0;
            }
            0x408A => self.volume_speed = value & 0x3F,
            _ => {}
        }
    }

    /// Reads a readable FDS audio register (`$4090-$4097`).
    #[must_use]
    pub fn read(&self, address: u16) -> u8 {
        match address {
            0x4090 => self.volume_gain,
            0x4091 => self.volume_speed,
            0x4092 => self.volume_gain,
            0x4093 => (self.frequency >> 8) as u8,
            0x4094 => self.frequency as u8,
            0x4095 => self.wave[self.wave_index()],
            0x4096 => self.mod_table[usize::from(self.mod_index >> 1) & 0x1F],
            0x4097 => self.mod_counter as u8,
            _ => 0,
        }
    }

    fn wave_index(&self) -> usize {
        let moderated = (self.wave_pos.round() as i32) + i32::from(self.mod_counter);
        (moderated & 0x3F) as usize
    }

    /// Advances one CPU cycle.
    pub fn clock(&mut self) {
        if !self.enabled {
            self.output = 0.0;
            return;
        }
        // Wave accumulator: the 12-bit frequency selects the step rate.
        self.wave_pos = (self.wave_pos + f32::from(self.frequency) / 4096.0) % 64.0;

        // Volume envelope, clocked at (speed + 1) * 8 CPU cycles.
        self.volume_timer += 1;
        let period = (u32::from(self.volume_speed) + 1) * 8;
        if self.volume_timer >= period {
            self.volume_timer = 0;
            if !self.envelope_off {
                if self.volume_increase {
                    self.volume_gain = (self.volume_gain + 1).min(32);
                } else {
                    self.volume_gain = self.volume_gain.saturating_sub(1);
                }
            }
        }

        // Modulation accumulator: a carry steps the 32-entry table twice.
        if self.mod_frequency != 0 && !self.mod_unit_off {
            self.mod_pos = (self.mod_pos + f32::from(self.mod_frequency) / 4096.0) % 1.0;
            self.mod_timer += 1;
            if self.mod_timer >= 16 {
                self.mod_timer = 0;
                let index = usize::from(self.mod_index >> 1) & 0x1F;
                let step = (self.mod_table[index] ^ 0x20).wrapping_sub(0x20) as i8;
                self.mod_counter = self.mod_counter.wrapping_add(step);
                self.mod_index = (self.mod_index + 1) & 0x3F;
            }
        }

        let sample = (f32::from(self.wave[self.wave_index()]) - 32.0) / 32.0;
        let gain = f32::from(self.volume_gain.min(32)) / 32.0;
        self.output = sample * gain * (f32::from(self.master_volume) / 3.0);
    }

    /// Signed output for the APU mixer.
    #[must_use]
    pub fn mix(&self) -> f32 {
        self.output * 0.5
    }
}

/// CPU cycles between disk byte transfers (~96.4 kHz bit rate / 8 bits).
const FDS_BYTE_CYCLES: u32 = 149;
/// Lead-in gap (zero bytes) synthesized before the first block of a side.
const FDS_LEAD_IN_GAP: usize = 200;
/// Inter-block gap (zero bytes) synthesized between consecutive blocks.
const FDS_BLOCK_GAP: usize = 100;
/// The FDS block start mark: a single 1 bit, stored little-endian as `$80`.
const FDS_START_MARK: u8 = 0x80;
/// Deterministic spin-up window after the motor turns on.
const FDS_MOTOR_SPIN_UP: u32 = 50_000;
/// Head re-seek window on a motor restart after the first spin-up.
const FDS_HEAD_RESEEK: u32 = 8_000;

/// CRC-16/KERMIT over the start mark and block payload, matching the RP2C33
/// block CRC (reflected, poly `$8408`).
fn fds_block_crc(block: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    let update = |byte: u8, crc: &mut u16| {
        *crc ^= u16::from(byte);
        for _ in 0..8 {
            let carry = *crc & 1 != 0;
            *crc >>= 1;
            if carry {
                *crc ^= 0x8408;
            }
        }
    };
    update(FDS_START_MARK, &mut crc);
    for &byte in block {
        update(byte, &mut crc);
    }
    crc
}

/// Walks a raw `.fds` side and returns `(offset, length)` for each block.
fn fds_side_blocks(side: &[u8]) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut pos = 0usize;
    let mut pending_size: Option<usize> = None;
    while pos < side.len() {
        let len = match side[pos] {
            0x01 => 56,
            0x02 => 2,
            0x03 => {
                let size = if pos + 15 <= side.len() {
                    usize::from(side[pos + 13]) | (usize::from(side[pos + 14]) << 8)
                } else {
                    0
                };
                pending_size = Some(size);
                16
            }
            0x04 => 1 + pending_size.take().unwrap_or(0),
            _ => break,
        };
        if pos + len > side.len() {
            blocks.push((pos, side.len() - pos));
            break;
        }
        blocks.push((pos, len));
        pos += len;
    }
    blocks
}

/// Synthesizes the wire image for one raw `.fds` side:
/// `[gap $00] [$80 mark] [block] [crc lo] [crc hi]` per block, with a trailing
/// gap so the head reads `$00` past the last block.
fn fds_side_wire(side: &[u8]) -> (Vec<u8>, Vec<(usize, usize, usize)>) {
    let blocks = fds_side_blocks(side);
    let mut wire = Vec::with_capacity(side.len() + blocks.len() * 32 + FDS_LEAD_IN_GAP);
    let mut map = Vec::with_capacity(blocks.len());
    for (index, &(offset, len)) in blocks.iter().enumerate() {
        let gap = if index == 0 {
            FDS_LEAD_IN_GAP
        } else {
            FDS_BLOCK_GAP
        };
        wire.resize(wire.len() + gap, 0x00);
        wire.push(FDS_START_MARK);
        let payload = wire.len();
        let block = &side[offset..offset + len];
        wire.extend_from_slice(block);
        let crc = fds_block_crc(block);
        wire.push(crc as u8);
        wire.push((crc >> 8) as u8);
        map.push((payload, offset, len));
    }
    wire.resize(wire.len() + FDS_BLOCK_GAP, 0x00);
    (wire, map)
}

/// Famicom Disk System board (mapper 20).
///
/// Maps the 8 KiB BIOS at `$E000-$FFFF`, 32 KiB of work RAM at `$6000-$DFFF`,
/// and the `$4020-$4032` disk registers / `$4040-$4097` audio window. The disk
/// is presented as a synthesized wire image (gaps, `$80` start marks and
/// per-block CRC-16), and bytes are delivered to `$4031` every [`FDS_BYTE_CYCLES`]
/// CPU cycles while a read transfer is armed — the handshake the real BIOS
/// loader polls through `$4030`/`$4032`.
#[derive(Debug)]
pub struct Fds {
    bios: Vec<u8>,
    /// Raw `.fds` bytes of every side, concatenated.
    disk: Vec<u8>,
    sides: usize,
    side: usize,
    wire: Vec<u8>,
    /// For each block of the inserted side: `(wire payload offset, raw side
    /// offset, payload length)`. Kept so write-mode transfers can refresh the
    /// per-block CRCs and persist payload bytes back to the raw image.
    wire_map: Vec<(usize, usize, usize)>,
    ram: Vec<u8>,
    chr: Vec<u8>,
    head: usize,
    insert_not_ready: u32,
    spun_up: bool,

    irq_reload: u16,
    irq_counter: u16,
    irq_enabled: bool,
    irq_repeat: bool,
    timer_irq_flag: bool,

    disk_enabled: bool,
    sound_enabled: bool,
    write_data: u8,
    read_data: u8,
    motor_on: bool,
    transfer_reset: bool,
    read_mode: bool,
    crc_control: bool,
    crc_enabled: bool,
    irq_on_transfer: bool,
    ext_con: u8,
    byte_transfer_flag: bool,
    end_of_head: bool,
    transfer_timer: u32,
    transfer_active: bool,
    skipping_gap: bool,

    mirroring: Mirroring,
    audio: FdsAudio,
}

impl Fds {
    fn new(disk: &[u8], bios: &[u8]) -> Self {
        let (sides, body) = if disk.len() >= 16 && &disk[0..4] == b"FDS\x1a" {
            (usize::from(disk[4]).max(1), &disk[16..])
        } else {
            ((disk.len() / 65500).max(1), disk)
        };
        let mut fds = Self {
            bios: bios.to_vec(),
            disk: body.to_vec(),
            sides,
            side: 0,
            wire: Vec::new(),
            wire_map: Vec::new(),
            ram: vec![0; 0x8000],
            chr: vec![0; 0x2000],
            head: 0,
            insert_not_ready: FDS_BYTE_CYCLES,
            spun_up: false,
            irq_reload: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_repeat: false,
            timer_irq_flag: false,
            disk_enabled: false,
            sound_enabled: false,
            write_data: 0,
            read_data: 0,
            motor_on: false,
            transfer_reset: true,
            read_mode: true,
            crc_control: false,
            crc_enabled: false,
            irq_on_transfer: false,
            ext_con: 0x7F,
            byte_transfer_flag: false,
            end_of_head: false,
            transfer_timer: FDS_BYTE_CYCLES,
            transfer_active: false,
            skipping_gap: true,
            mirroring: Mirroring::Vertical,
            audio: FdsAudio::default(),
        };
        fds.rebuild_wire();
        fds
    }

    fn side_bytes(&self) -> &[u8] {
        let start = (self.side * 65500).min(self.disk.len());
        let end = (start + 65500).min(self.disk.len());
        &self.disk[start..end]
    }

    fn rebuild_wire(&mut self) {
        let (wire, map) = fds_side_wire(self.side_bytes());
        self.wire = wire;
        self.wire_map = map;
        self.head = 0;
        self.end_of_head = false;
        self.skipping_gap = true;
    }

    /// Recomputes every block's CRC-16 after a write-mode transfer modified
    /// the wire image. The 2C33 regenerates CRCs in hardware; the BIOS never
    /// writes them itself.
    fn refresh_crcs(&mut self) {
        for &(payload, _, len) in &self.wire_map {
            if payload == 0 || payload + len + 2 > self.wire.len() {
                continue;
            }
            let end = payload + len;
            let crc = fds_block_crc(&self.wire[payload..end]);
            self.wire[end] = crc as u8;
            self.wire[end + 1] = (crc >> 8) as u8;
        }
    }

    /// Copies the inserted side's wire payloads back into the raw side image
    /// and returns the full side.
    fn save_current_side(&mut self) -> Vec<u8> {
        self.refresh_crcs();
        let base = self.side * 65_500;
        let mut side = self.side_bytes().to_vec();
        side.resize(65_500, 0);
        for &(payload, raw_offset, len) in &self.wire_map {
            if payload + len <= self.wire.len() && raw_offset + len <= side.len() {
                side[raw_offset..raw_offset + len]
                    .copy_from_slice(&self.wire[payload..payload + len]);
            }
        }
        if base < self.disk.len() {
            let end = (base + 65_500).min(self.disk.len());
            self.disk[base..end].copy_from_slice(&side[..end - base]);
        }
        side
    }

    /// Returns the full raw `.fds` image (fwNES header + every side), with the
    /// inserted side's pending writes flushed back into it.
    pub fn disk_image(&mut self) -> Vec<u8> {
        let current = self.save_current_side();
        let mut out = Vec::with_capacity(16 + self.disk.len());
        out.extend_from_slice(b"FDS\x1a");
        out.push(self.sides.min(255) as u8);
        out.extend(std::iter::repeat_n(0, 11));
        for side in 0..self.sides {
            let start = side * 65_500;
            let end = (start + 65_500).min(self.disk.len());
            if side == self.side {
                let mut bytes = current.clone();
                bytes.resize(65_500, 0);
                out.extend_from_slice(&bytes);
            } else {
                out.extend_from_slice(&self.disk[start..end]);
                out.extend(std::iter::repeat_n(0, 65_500 - (end - start)));
            }
        }
        out
    }

    /// Inserts a disk side (or ejects), resetting the head and opening a short
    /// not-ready window.
    pub fn set_side(&mut self, side: Option<usize>) {
        match side {
            Some(index) if index < self.sides => {
                self.side = index;
                self.insert_not_ready = FDS_MOTOR_SPIN_UP;
                self.spun_up = false;
                self.rebuild_wire();
            }
            Some(_) => {}
            None => {
                self.wire.clear();
                self.head = 0;
                self.motor_on = false;
                self.end_of_head = true;
            }
        }
        self.update_transfer_state();
    }

    fn update_transfer_state(&mut self) {
        let ready = self.insert_not_ready == 0;
        let crc_armed = self.crc_enabled && self.crc_control;
        let active = ready && crc_armed && self.disk_enabled && (!self.read_mode || self.motor_on);
        // Only a transition into an active read re-arms the gap skip; the
        // BIOS toggles CRC between individual data bytes during a block, and
        // re-arming there would swallow leading `$00` payload bytes.
        if active != self.transfer_active {
            self.transfer_active = active;
            if active {
                self.transfer_timer = FDS_BYTE_CYCLES;
                if self.read_mode {
                    self.skipping_gap = true;
                }
            }
        }
    }

    fn deliver_byte(&mut self) {
        if self.skipping_gap {
            // A resync bit-shifts past the gap (zero run) and its terminating
            // `$80` start mark without raising a byte-transfer event; the first
            // event delivers the byte that follows the mark. A non-zero,
            // non-mark byte (e.g. the previous block's CRC) is real data, so
            // stop skipping and deliver it.
            while self.head < self.wire.len() && self.wire[self.head] == 0x00 {
                self.head += 1;
            }
            if self.head < self.wire.len() && self.wire[self.head] == FDS_START_MARK {
                self.head += 1;
            }
            self.skipping_gap = false;
        }
        if self.head < self.wire.len() {
            self.read_data = self.wire[self.head];
            self.head += 1;
        } else {
            self.read_data = 0;
            self.end_of_head = true;
            self.motor_on = false;
        }
        self.byte_transfer_flag = true;
    }

    /// Write-mode transfer: lands the byte last written to `$4024` on the
    /// wire image and advances the head. The BIOS emits the full gap/mark/
    /// block/CRC stream itself, so storing the wire bytes verbatim is the
    /// faithful model; the 2C33's automatic CRC regeneration is not modelled.
    fn store_byte(&mut self) {
        if self.head < self.wire.len() {
            self.wire[self.head] = self.write_data;
            self.head += 1;
            self.refresh_crcs();
        }
        if self.head >= self.wire.len() {
            self.end_of_head = true;
            self.motor_on = false;
        }
        self.byte_transfer_flag = true;
    }

    fn write_control(&mut self, value: u8) {
        let was_motor_on = self.motor_on;
        let was_reset = self.transfer_reset;
        self.transfer_reset = value & 0x01 != 0;
        self.motor_on = value & 0x02 == 0;
        self.read_mode = value & 0x04 != 0;
        self.crc_control = value & 0x20 != 0;
        self.crc_enabled = value & 0x40 != 0;
        self.irq_on_transfer = value & 0x80 != 0;
        self.mirroring = if value & 0x08 != 0 {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        };
        if self.motor_on && !was_motor_on {
            if self.spun_up {
                if self.head == 0 {
                    self.insert_not_ready = self.insert_not_ready.max(FDS_HEAD_RESEEK);
                }
            } else {
                self.insert_not_ready = FDS_MOTOR_SPIN_UP;
                self.spun_up = true;
            }
        }
        if !self.motor_on && was_motor_on {
            // The belt-driven drive rewinds to the start of the side.
            self.head = 0;
            self.end_of_head = false;
            self.skipping_gap = true;
        }
        if self.transfer_reset && !was_reset {
            self.skipping_gap = true;
            self.transfer_timer = FDS_BYTE_CYCLES;
        }
        // Writing $4025 acknowledges disk IRQs.
        self.byte_transfer_flag = false;
        self.update_transfer_state();
    }

    fn clock_timer(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0 {
            self.timer_irq_flag = true;
            self.irq_counter = self.irq_reload;
            if !self.irq_repeat {
                self.irq_enabled = false;
            }
        } else {
            self.irq_counter -= 1;
        }
    }

    fn status_4030(&mut self) -> u8 {
        let mut value = 0u8;
        if self.timer_irq_flag {
            value |= 0x01;
        }
        if self.mirroring == Mirroring::Horizontal {
            value |= 0x08;
        }
        if self.end_of_head {
            value |= 0x40;
        }
        if self.byte_transfer_flag {
            value |= 0x80;
        }
        // Reading $4030 acknowledges the timer IRQ but not the byte-transfer
        // flag.
        self.timer_irq_flag = false;
        value
    }

    fn drive_status_4032(&self) -> u8 {
        let mut value = 0u8;
        if self.insert_not_ready > 0 || self.end_of_head || !self.motor_on {
            value |= 0x02;
        }
        value
    }
}

impl Mapper for Fds {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x4090..=0x4097 => self.audio.read(address),
            0x6000..=0xDFFF => self.ram[usize::from(address - 0x6000)],
            0xE000..=0xFFFF => self.bios[usize::from(address - 0xE000) % self.bios.len().max(1)],
            _ => 0,
        }
    }

    fn read_io(&mut self, address: u16) -> u8 {
        match address {
            0x4030 => self.status_4030(),
            0x4031 => {
                self.byte_transfer_flag = false;
                self.read_data
            }
            0x4032 => self.drive_status_4032(),
            0x4033 => {
                let battery = if self.wire.is_empty() { 0 } else { 0x80 };
                battery | (self.ext_con & 0x7F)
            }
            0x4090..=0x4097 => self.audio.read(address),
            0x6000..=0xDFFF => self.ram[usize::from(address - 0x6000)],
            0xE000..=0xFFFF => self.bios[usize::from(address - 0xE000) % self.bios.len().max(1)],
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x4020 => {
                self.irq_reload = (self.irq_reload & 0xFF00) | u16::from(value);
            }
            0x4021 => {
                self.irq_reload = (self.irq_reload & 0x00FF) | (u16::from(value) << 8);
            }
            0x4022 => {
                if !self.disk_enabled {
                    return;
                }
                self.irq_repeat = value & 0x01 != 0;
                self.irq_enabled = value & 0x02 != 0;
                if self.irq_enabled {
                    self.irq_counter = self.irq_reload;
                } else {
                    self.timer_irq_flag = false;
                }
            }
            0x4023 => {
                let was_enabled = self.disk_enabled;
                self.disk_enabled = value & 0x01 != 0;
                self.sound_enabled = value & 0x02 != 0;
                self.audio.write(0x4023, value);
                if was_enabled && !self.disk_enabled {
                    self.irq_enabled = false;
                    self.timer_irq_flag = false;
                    self.byte_transfer_flag = false;
                    self.transfer_reset = true;
                    self.motor_on = false;
                    self.read_mode = true;
                    self.crc_control = false;
                    self.crc_enabled = false;
                    self.irq_on_transfer = false;
                    self.end_of_head = false;
                    self.mirroring = Mirroring::Vertical;
                    self.ext_con = 0x7F;
                }
                self.update_transfer_state();
            }
            0x4024 => {
                if !self.disk_enabled {
                    return;
                }
                self.write_data = value;
                self.byte_transfer_flag = false;
            }
            0x4025 => {
                if self.disk_enabled {
                    self.write_control(value);
                }
            }
            0x4026 => {
                if self.disk_enabled {
                    self.ext_con = value & 0x7F;
                }
            }
            0x4040..=0x408A => {
                // Wave volume, mod counter and mod-table writes are ignored
                // while the sound registers are reset.
                if !self.sound_enabled && matches!(address, 0x4080 | 0x4085 | 0x4088) {
                    return;
                }
                self.audio.write(address, value);
            }
            0x6000..=0xDFFF => self.ram[usize::from(address - 0x6000)] = value,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr[usize::from(address) & 0x1FFF]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        self.chr[usize::from(address) & 0x1FFF] = value;
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        20
    }

    fn prg_ram(&self) -> &[u8] {
        &self.ram
    }

    fn low_window(&self) -> Option<(u16, u16)> {
        Some((0x4020, 0x4097))
    }

    fn clock_cpu(&mut self) {
        self.audio.clock();
        self.clock_timer();
        if self.insert_not_ready > 0 {
            self.insert_not_ready -= 1;
            if self.insert_not_ready == 0 {
                self.update_transfer_state();
            }
        }
        if !self.transfer_active {
            return;
        }
        if self.transfer_timer > 0 {
            self.transfer_timer -= 1;
        }
        if self.transfer_timer == 0 {
            if self.read_mode {
                self.deliver_byte();
            } else {
                self.store_byte();
            }
            self.transfer_timer = FDS_BYTE_CYCLES;
        }
    }

    fn audio_mix(&self) -> f32 {
        self.audio.mix()
    }

    fn irq_line(&self) -> bool {
        self.timer_irq_flag || (self.byte_transfer_flag && self.irq_on_transfer)
    }

    fn acknowledge_irq(&mut self) {}

    fn insert_disk_side(&mut self, side: Option<usize>) {
        self.set_side(side);
    }

    fn disk_side_count(&self) -> usize {
        self.sides
    }

    fn current_disk_side(&self) -> usize {
        self.side
    }

    fn disk_image(&mut self) -> Vec<u8> {
        Fds::disk_image(self)
    }
}

/// VRC7 built-in instrument patches/// VRC7 built-in instrument patches (approximation of the YM2413 patch
/// ROM). Index 0 is the user patch, which is built from registers $00-$07.
const VRC7_PATCHES: [[u8; 8]; 16] = [
    // Instrument 0 is the user patch written through $00-$07.
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // Custom
    [0x03, 0x21, 0x05, 0x06, 0xE8, 0x81, 0x42, 0x27], // Buzzy Bell
    [0x13, 0x41, 0x14, 0x0D, 0xD8, 0xF6, 0x23, 0x12], // Guitar
    [0x11, 0x11, 0x08, 0x08, 0xFA, 0xB2, 0x20, 0x12], // Wurly
    [0x31, 0x61, 0x0C, 0x07, 0xA8, 0x64, 0x61, 0x27], // Flute
    [0x32, 0x21, 0x1E, 0x06, 0xE1, 0x76, 0x01, 0x28], // Clarinet
    [0x02, 0x01, 0x06, 0x00, 0xA3, 0xE2, 0xF4, 0xF4], // Synth
    [0x21, 0x61, 0x1D, 0x07, 0x82, 0x81, 0x11, 0x07], // Trumpet
    [0x23, 0x21, 0x22, 0x17, 0xA2, 0x72, 0x01, 0x17], // Organ
    [0x35, 0x11, 0x25, 0x00, 0x40, 0x73, 0x72, 0x01], // Bells
    [0xB5, 0x01, 0x0F, 0x0F, 0xA8, 0xA5, 0x51, 0x02], // Vibes
    [0x17, 0xC1, 0x24, 0x07, 0xF8, 0xF8, 0x22, 0x12], // Vibraphone
    [0x71, 0x23, 0x11, 0x06, 0x65, 0x74, 0x18, 0x16], // Tutti
    [0x01, 0x02, 0xD3, 0x05, 0xC9, 0x95, 0x03, 0x02], // Fretless
    [0x61, 0x63, 0x0C, 0x00, 0x94, 0xC0, 0x33, 0xF6], // Synth Bass
    [0x21, 0x72, 0x0D, 0x00, 0xC1, 0xD5, 0x56, 0x06], // Sweep
];

/// Decodes the two operator waveform bits from patch byte `$03`
/// (`KK-Q WFFF`): Q is the carrier (bit 4), W the modulator (bit 3).
fn vrc7_waveforms(patch: &[u8; 8]) -> (bool, bool) {
    (patch[3] & 0x08 != 0, patch[3] & 0x10 != 0)
}

/// The VRC7 multi-multiplier table.
const VRC7_MULTIPLIER: [f32; 16] = [
    0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 10.0, 12.0, 12.0, 15.0, 15.0,
];

/// Envelope stage indices.
const ENV_IDLE: u8 = 0;
const ENV_ATTACK: u8 = 1;
const ENV_DECAY: u8 = 2;
const ENV_SUSTAIN: u8 = 3;
const ENV_RELEASE: u8 = 4;

#[derive(Debug, Default, Clone)]
struct Vrc7Operator {
    phase: f32,
    envelope: f32,
    stage: u8,
}

impl Vrc7Operator {
    fn key_on(&mut self) {
        self.phase = 0.0;
        self.envelope = 0.0;
        self.stage = ENV_ATTACK;
    }

    fn key_off(&mut self) {
        if self.stage != ENV_IDLE {
            self.stage = ENV_RELEASE;
        }
    }

    fn advance(&mut self, patch: &[u8; 8], offset: usize, channel_sustain: bool) {
        let rate_scale = |rate: u8, scale: f32| -> f32 {
            if rate == 0 {
                0.0
            } else {
                scale * f32::from(rate) * f32::from(rate) / 225.0
            }
        };
        match self.stage {
            ENV_ATTACK => {
                let rate = patch[4 + offset] >> 4;
                self.envelope += rate_scale(rate, 0.012);
                if self.envelope >= 1.0 {
                    self.envelope = 1.0;
                    self.stage = ENV_DECAY;
                }
            }
            ENV_DECAY => {
                let rate = patch[4 + offset] & 0x0F;
                let sustain = 1.0 - f32::from(patch[6 + offset] >> 4) / 15.0 * 0.95;
                self.envelope -= rate_scale(rate, 0.004);
                if self.envelope <= sustain {
                    self.envelope = sustain;
                    self.stage = ENV_SUSTAIN;
                }
            }
            ENV_SUSTAIN => {
                // The sustain bit keeps the operator in the sustain section;
                // without it the release starts as soon as decay finishes.
                if !channel_sustain && patch[offset] & 0x20 == 0 {
                    self.stage = ENV_RELEASE;
                }
            }
            ENV_RELEASE => {
                let rate = if channel_sustain {
                    5
                } else {
                    patch[6 + offset] & 0x0F
                };
                self.envelope -= rate_scale(rate, 0.005);
                if self.envelope <= 0.0 {
                    self.envelope = 0.0;
                    self.stage = ENV_IDLE;
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Default, Clone)]
struct Vrc7Channel {
    frequency: u16,
    octave: u8,
    instrument: u8,
    volume: u8,
    key_on: bool,
    sustain: bool,
    modulator: Vrc7Operator,
    carrier: Vrc7Operator,
}

/// VRC7 / YM2413-style two-operator FM synthesis, six channels.
#[derive(Debug, Clone)]
struct Vrc7Audio {
    custom: [u8; 8],
    registers: [u8; 64],
    selected: u8,
    channels: [Vrc7Channel; 6],
    current: usize,
    divider: u8,
    output: f32,
    silenced: bool,
}

impl Default for Vrc7Audio {
    fn default() -> Self {
        Self {
            custom: [0; 8],
            registers: [0; 64],
            selected: 0,
            channels: Default::default(),
            current: 0,
            divider: 0,
            output: 0.0,
            silenced: false,
        }
    }
}

impl Vrc7Audio {
    fn silence(&mut self) {
        self.registers = [0; 64];
        self.custom = [0; 8];
        self.channels = Default::default();
        self.output = 0.0;
        self.silenced = true;
    }

    fn write_select(&mut self, value: u8) {
        if self.silenced {
            return;
        }
        self.selected = value & 0x3F;
    }

    fn write_data(&mut self, value: u8) {
        if self.silenced {
            return;
        }
        let register = usize::from(self.selected);
        if register >= self.registers.len() {
            return;
        }
        self.registers[register] = value;
        match register {
            0x00..=0x07 => self.custom[register] = value,
            0x10..=0x15 => {
                let channel = register - 0x10;
                self.channels[channel].frequency =
                    (self.channels[channel].frequency & 0x100) | u16::from(value);
            }
            0x20..=0x25 => {
                let channel = register - 0x20;
                let key_on = value & 0x10 != 0;
                let was_on = self.channels[channel].key_on;
                self.channels[channel].frequency =
                    (self.channels[channel].frequency & 0x0FF) | (u16::from(value & 0x01) << 8);
                self.channels[channel].octave = (value >> 1) & 0x07;
                self.channels[channel].sustain = value & 0x20 != 0;
                self.channels[channel].key_on = key_on;
                if key_on && !was_on {
                    self.channels[channel].modulator.key_on();
                    self.channels[channel].carrier.key_on();
                } else if !key_on && was_on {
                    self.channels[channel].modulator.key_off();
                    self.channels[channel].carrier.key_off();
                }
            }
            0x30..=0x35 => {
                let channel = register - 0x30;
                self.channels[channel].instrument = value >> 4;
                self.channels[channel].volume = value & 0x0F;
            }
            _ => {}
        }
    }

    fn patch(&self, channel: &Vrc7Channel) -> [u8; 8] {
        if channel.instrument == 0 {
            self.custom
        } else {
            VRC7_PATCHES[usize::from(channel.instrument & 0x0F)]
        }
    }

    fn update_channel(&mut self, index: usize) {
        let patch = {
            let channel = &self.channels[index];
            self.patch(channel)
        };
        let channel = &mut self.channels[index];
        let modulator_multiplier = VRC7_MULTIPLIER[usize::from(patch[0] & 0x0F)];
        let carrier_multiplier = VRC7_MULTIPLIER[usize::from(patch[1] & 0x0F)];
        // 8286 Hz per-channel update rate: 49716 Hz * freq / 2^(19-octave)
        // divided by 6 channels.
        let scale = 6.0 / (1u32 << (19u32.saturating_sub(u32::from(channel.octave)))) as f32;
        let modulator_increment = f32::from(channel.frequency) * modulator_multiplier * scale;
        let carrier_increment = f32::from(channel.frequency) * carrier_multiplier * scale;

        channel.modulator.advance(&patch, 0, channel.sustain);
        channel.carrier.advance(&patch, 1, channel.sustain);
        channel.modulator.phase = (channel.modulator.phase + modulator_increment).fract();
        channel.carrier.phase = (channel.carrier.phase + carrier_increment).fract();

        let waveform = |phase: f32, half: bool| -> f32 {
            let value = (phase * std::f32::consts::TAU).sin();
            if half {
                value.max(0.0)
            } else {
                value
            }
        };
        let (mod_half, car_half) = vrc7_waveforms(&patch);
        let mod_level = 10f32.powf(-0.75 * f32::from(patch[2] & 0x3F) / 20.0);
        let feedback = f32::from(patch[3] & 0x07) / 7.0;
        let modulator = waveform(channel.modulator.phase, mod_half)
            * channel.modulator.envelope
            * mod_level
            * (1.0 + feedback);
        let carrier =
            waveform(channel.carrier.phase + modulator * 1.5, car_half) * channel.carrier.envelope;
        self.output = carrier * (f32::from(channel.volume) / 15.0);
    }

    fn clock(&mut self) {
        self.divider += 1;
        if self.divider < 36 {
            return;
        }
        self.divider = 0;
        if self.silenced {
            self.output = 0.0;
            return;
        }
        self.update_channel(self.current);
        self.current = (self.current + 1) % 6;
    }

    fn mix(&self) -> f32 {
        self.output * 0.5
    }
}

/// Konami VRC7 (mapper 85): 8 KiB PRG + 1 KiB CHR banking, WRAM protection,
/// switchable mirroring, a VRC cycle/scanline IRQ and the YM2413-style
/// six-channel FM expansion audio.
#[derive(Debug)]
pub struct Vrc7 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_banks: [u8; 3],
    chr_banks: [u8; 8],
    wram_enabled: bool,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_cycle_mode: bool,
    irq_pending: bool,
    scanline_prescaler: u16,
    audio: Vrc7Audio,
    chr_aux: Option<Rc<RefCell<Vrc7Chr>>>,
}

impl Vrc7 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        let chr_aux =
            (header.chr_size == 0).then(|| Rc::new(RefCell::new(Vrc7Chr::new(chr.clone()))));
        Self {
            prg,
            chr,
            prg_ram,
            prg_banks: [0; 3],
            chr_banks: [0; 8],
            wram_enabled: false,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_cycle_mode: false,
            irq_pending: false,
            scanline_prescaler: 0,
            audio: Vrc7Audio::default(),
            chr_aux,
        }
    }

    fn refresh_chr_banks(&self) {
        if let Some(aux) = &self.chr_aux {
            aux.borrow_mut().set_banks(self.chr_banks);
        }
    }

    fn clock_irq(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0xFF {
            self.irq_counter = self.irq_latch;
            self.irq_pending = true;
        } else {
            self.irq_counter = self.irq_counter.wrapping_add(1);
        }
    }
}

impl Mapper for Vrc7 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0x9FFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                self.prg[(usize::from(self.prg_banks[0]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xA000..=0xBFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                self.prg[(usize::from(self.prg_banks[1]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xC000..=0xDFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                self.prg[(usize::from(self.prg_banks[2]) % banks) * 0x2000
                    + usize::from(address & 0x1FFF)]
            }
            0xE000..=0xFFFF => {
                let bank = (self.prg.len() / 0x2000).max(1) - 1;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x6000..=0x7FFF => {
                if self.wram_enabled {
                    let index = usize::from(address - 0x6000) % self.prg_ram.len();
                    self.prg_ram[index] = value;
                }
            }
            0x8000 => self.prg_banks[0] = value & 0x3F,
            0x8008 | 0x8010 => self.prg_banks[1] = value & 0x3F,
            0x9000 => self.prg_banks[2] = value & 0x3F,
            0x9010 => self.audio.write_select(value),
            0x9030 => self.audio.write_data(value),
            0xA000 => {
                self.chr_banks[0] = value;
                self.refresh_chr_banks();
            }
            0xA008 | 0xA010 => {
                self.chr_banks[1] = value;
                self.refresh_chr_banks();
            }
            0xB000 => {
                self.chr_banks[2] = value;
                self.refresh_chr_banks();
            }
            0xB008 | 0xB010 => {
                self.chr_banks[3] = value;
                self.refresh_chr_banks();
            }
            0xC000 => {
                self.chr_banks[4] = value;
                self.refresh_chr_banks();
            }
            0xC008 | 0xC010 => {
                self.chr_banks[5] = value;
                self.refresh_chr_banks();
            }
            0xD000 => {
                self.chr_banks[6] = value;
                self.refresh_chr_banks();
            }
            0xD008 | 0xD010 => {
                self.chr_banks[7] = value;
                self.refresh_chr_banks();
            }
            0xE000 => {
                self.wram_enabled = value & 0x80 != 0;
                if value & 0x40 != 0 {
                    self.audio.silence();
                }
                self.mirroring = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                };
            }
            0xE008 | 0xE010 => {
                self.irq_latch = value;
                self.irq_counter = value;
            }
            0xF000 => {
                self.irq_enabled = value & 0x02 != 0;
                self.irq_cycle_mode = value & 0x04 != 0;
                if value & 0x01 != 0 {
                    self.irq_pending = false;
                }
            }
            0xF008 | 0xF010 => self.irq_pending = false,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        if let Some(aux) = &self.chr_aux {
            return aux.borrow().read(address);
        }
        let slot = usize::from(address / 0x400) & 0x07;
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(aux) = &self.chr_aux {
            aux.borrow_mut().write(address, value);
            return;
        }
        let slot = usize::from(address / 0x400) & 0x07;
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        85
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        self.audio.clock();
        if self.irq_cycle_mode {
            self.clock_irq();
        } else {
            self.scanline_prescaler += 1;
            if self.scanline_prescaler >= 114 {
                self.scanline_prescaler = 0;
                self.clock_irq();
            }
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }

    fn audio_mix(&self) -> f32 {
        self.audio.mix()
    }

    fn vrc7_chr(&self) -> Option<Rc<RefCell<Vrc7Chr>>> {
        self.chr_aux.clone()
    }
}

/// One VRC6 pulse channel.
#[derive(Debug, Default, Clone)]
struct Vrc6Pulse {
    volume: u8,
    duty: u8,
    mode: bool,
    freq: u16,
    enabled: bool,
    divider: u16,
    step: u8,
}

impl Vrc6Pulse {
    fn write(&mut self, register: u16, value: u8) {
        match register {
            0 => {
                self.volume = value & 0x0F;
                self.duty = (value >> 4) & 0x07;
                self.mode = value & 0x80 != 0;
            }
            1 => self.freq = (self.freq & 0x0F00) | u16::from(value),
            2 => {
                self.freq = (self.freq & 0x00FF) | (u16::from(value & 0x0F) << 8);
                self.enabled = value & 0x80 != 0;
                if !self.enabled {
                    self.step = 15;
                }
            }
            _ => {}
        }
    }

    fn clock(&mut self, halt: bool, shift: u8) {
        if halt {
            return;
        }
        if !self.enabled {
            self.step = 15;
            return;
        }
        if self.divider == 0 {
            self.divider = self.freq >> shift;
            // The 16-step duty generator counts down from 15 to 0.
            self.step = if self.step == 0 { 15 } else { self.step - 1 };
        } else {
            self.divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && (self.mode || self.step <= self.duty) {
            self.volume
        } else {
            0
        }
    }
}

/// The VRC6 sawtooth channel.
#[derive(Debug, Default, Clone)]
struct Vrc6Saw {
    rate: u8,
    freq: u16,
    enabled: bool,
    divider: u16,
    accumulator: u8,
    step: u8,
}

impl Vrc6Saw {
    fn write(&mut self, register: u16, value: u8) {
        match register {
            0 => self.rate = value & 0x3F,
            1 => self.freq = (self.freq & 0x0F00) | u16::from(value),
            2 => {
                self.freq = (self.freq & 0x00FF) | (u16::from(value & 0x0F) << 8);
                self.enabled = value & 0x80 != 0;
            }
            _ => {}
        }
    }

    fn clock(&mut self, halt: bool, shift: u8) {
        if halt {
            return;
        }
        if !self.enabled {
            self.accumulator = 0;
            return;
        }
        if self.divider == 0 {
            self.divider = self.freq >> shift;
            self.step += 1;
            if self.step >= 14 {
                // The 14th clock resets the accumulator instead of adding.
                self.step = 0;
                self.accumulator = 0;
            } else if self.step.is_multiple_of(2) {
                self.accumulator = self.accumulator.wrapping_add(self.rate);
            }
        } else {
            self.divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled {
            self.accumulator >> 3
        } else {
            0
        }
    }
}

/// VRC6 expansion audio: two pulse channels and one sawtooth.
#[derive(Debug, Default, Clone)]
struct Vrc6Audio {
    pulse1: Vrc6Pulse,
    pulse2: Vrc6Pulse,
    saw: Vrc6Saw,
    halt: bool,
    shift: u8,
}

impl Vrc6Audio {
    fn write(&mut self, address: u16, value: u8) {
        match address & 0xF000 {
            0x9000 => self.pulse1.write(address & 0x03, value),
            0xA000 => self.pulse2.write(address & 0x03, value),
            0xB000 => self.saw.write(address & 0x03, value),
            _ => {}
        }
    }

    fn write_frequency_control(&mut self, value: u8) {
        self.halt = value & 0x01 != 0;
        self.shift = if value & 0x04 != 0 {
            8
        } else if value & 0x02 != 0 {
            4
        } else {
            0
        };
    }

    fn clock(&mut self) {
        self.pulse1.clock(self.halt, self.shift);
        self.pulse2.clock(self.halt, self.shift);
        self.saw.clock(self.halt, self.shift);
    }

    fn mix(&self) -> f32 {
        // Linear 6-bit DAC: two 4-bit pulses plus the saw's high 5 bits,
        // centred on ~half the 0..=61 peak and scaled so one full-volume pulse
        // matches a full-volume 2A03 pulse (the documented VRC6 level). This
        // mirrors the reference's `((sum - 30) * 650) / 65536`.
        let sum = i32::from(self.pulse1.output())
            + i32::from(self.pulse2.output())
            + i32::from(self.saw.output());
        (sum - 30) as f32 * 650.0 / 65536.0
    }
}

/// Konami VRC6 (mappers 24/26): 16 KiB + 8 KiB PRG banking, eight 1 KiB CHR
/// banks, PPU banking/mirroring control, the VRC cycle/scanline IRQ and the
/// two-pulse + sawtooth expansion audio.
#[derive(Debug)]
pub struct Vrc6 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    mapper: u16,
    prg16: u8,
    prg8: u8,
    chr_banks: [u8; 8],
    ppu_style: u8,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_enabled: bool,
    irq_cycle_mode: bool,
    irq_pending: bool,
    scanline_prescaler: u16,
    audio: Vrc6Audio,
}

impl Vrc6 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header, mapper: u16) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            mapper,
            prg16: 0,
            prg8: 0,
            chr_banks: [0; 8],
            ppu_style: 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_cycle_mode: false,
            irq_pending: false,
            scanline_prescaler: 0,
            audio: Vrc6Audio::default(),
        }
    }

    fn banking_mode(&self) -> u8 {
        self.ppu_style & 0x03
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let mode = self.banking_mode();
        let index = match mode {
            0 | 3 => slot,
            1 => match slot {
                0 | 1 => 0,
                2 | 3 => 1,
                4 | 5 => 2,
                6 => 3,
                _ => 4,
            },
            2 => match slot {
                0 => 0,
                1 => 1,
                2 | 3 => 2,
                4 => 3,
                5 => 4,
                6 => 5,
                _ => 6,
            },
            _ => 0,
        };
        usize::from(self.chr_banks[index]) % banks
    }

    fn clock_irq(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0xFF {
            self.irq_counter = self.irq_latch;
            self.irq_pending = true;
        } else {
            self.irq_counter = self.irq_counter.wrapping_add(1);
        }
    }
}

impl Mapper for Vrc6 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xBFFF => {
                let banks = (self.prg.len() / 0x4000).max(1);
                let bank = usize::from(self.prg16) % banks;
                self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
            }
            0xC000..=0xDFFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let bank = usize::from(self.prg8) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0xE000..=0xFFFF => {
                let bank = (self.prg.len() / 0x2000).max(1) - 1;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        let address = if self.mapper == 26 {
            (address & !0x03) | ((address & 0x01) << 1) | ((address & 0x02) >> 1)
        } else {
            address
        };
        if (0x6000..=0x7FFF).contains(&address) {
            let index = usize::from(address - 0x6000) % self.prg_ram.len();
            self.prg_ram[index] = value;
            return;
        }
        match address & 0xF003 {
            0x8000 => self.prg16 = value & 0x0F,
            0xB003 => {
                self.ppu_style = value;
                self.mirroring = match (value >> 2) & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleLower,
                    _ => Mirroring::SingleUpper,
                };
            }
            0xC000 => self.prg8 = value & 0x3F,
            0xD000..=0xD003 => self.chr_banks[usize::from(address & 0x03)] = value,
            0xE000..=0xE003 => self.chr_banks[4 + usize::from(address & 0x03)] = value,
            0x9000..=0x9002 | 0xA000..=0xA002 | 0xB000..=0xB002 => {
                self.audio.write(address, value);
            }
            0x9003 => self.audio.write_frequency_control(value),
            0xF000 => {
                self.irq_latch = value;
                self.irq_counter = value;
            }
            0xF001 => {
                self.irq_enabled = value & 0x02 != 0;
                self.irq_cycle_mode = value & 0x04 != 0;
                if value & 0x01 != 0 {
                    self.irq_pending = false;
                }
            }
            0xF002 => self.irq_pending = false,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = self.chr_bank(slot);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = self.chr_bank(slot);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        self.mapper
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        self.audio.clock();
        if self.irq_cycle_mode {
            self.clock_irq();
        } else {
            self.scanline_prescaler += 1;
            if self.scanline_prescaler >= 114 {
                self.scanline_prescaler = 0;
                self.clock_irq();
            }
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }

    fn audio_mix(&self) -> f32 {
        self.audio.mix()
    }
}

/// Irem G-101 (mapper 32): two 8 KiB PRG banks with a switchable/fixed pair,
/// eight 1 KiB CHR banks and a mirroring/PRG-mode register.
#[derive(Debug)]
pub struct Irem32 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg0: u8,
    prg1: u8,
    prg_mode: bool,
    chr_banks: [u8; 8],
    mirroring: Mirroring,
}

impl Irem32 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg0: 0,
            prg1: 1,
            prg_mode: false,
            chr_banks: [0; 8],
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Irem32 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = (self.prg.len() / 0x2000).max(1);
        let second_last = banks.saturating_sub(2);
        let bank = match address {
            0x8000..=0x9FFF => {
                if self.prg_mode {
                    second_last
                } else {
                    usize::from(self.prg0)
                }
            }
            0xA000..=0xBFFF => usize::from(self.prg1),
            0xC000..=0xDFFF => {
                if self.prg_mode {
                    usize::from(self.prg0)
                } else {
                    second_last
                }
            }
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xF000 {
            0x8000 => self.prg0 = value & 0x1F,
            0x9000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
                self.prg_mode = value & 0x02 != 0;
            }
            0xA000 => self.prg1 = value & 0x1F,
            0xB000 => self.chr_banks[usize::from(address & 0x07)] = value,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address / 0x400) & 0x07;
        let bank = usize::from(self.chr_banks[slot]) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        32
    }
}

/// Jaleco JF-13 (mapper 86): GNROM-style 32 KiB PRG / 8 KiB CHR latch at
/// `$6000-$6FFF` (audio at `$7000-$7FFF` is not modelled).
#[derive(Debug)]
pub struct Jf13 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: u8,
    mirroring: Mirroring,
}

impl Jf13 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Jf13 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = usize::from((self.bank >> 4) & 0x07) % banks;
        self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if (0x6000..=0x6FFF).contains(&address) {
            self.bank = value;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.bank & 0x03) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.bank & 0x03) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        86
    }
}

/// Taito TC0690 (mapper 48): TC0190-style PRG/CHR banking plus an MMC3-like
/// IRQ with an inverted reload value.
#[derive(Debug)]
pub struct Taito48 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_banks: [u8; 2],
    chr_banks: [u8; 6],
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

impl Taito48 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_banks: [0, 1],
            chr_banks: [0, 1, 4, 5, 6, 7],
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }
}

impl Mapper for Taito48 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = match address {
            0x8000..=0x9FFF => usize::from(self.prg_banks[0]),
            0xA000..=0xBFFF => usize::from(self.prg_banks[1]),
            0xC000..=0xDFFF => banks.saturating_sub(2),
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xE003 {
            0x8000 => self.prg_banks[0] = value & 0x3F,
            0x8001 => self.prg_banks[1] = value & 0x3F,
            0x8002 => self.chr_banks[0] = value,
            0x8003 => self.chr_banks[1] = value,
            0xA000..=0xA003 => self.chr_banks[2 + usize::from(address & 0x03)] = value,
            0xC000 => self.irq_latch = value ^ 0xFF,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xC002 => self.irq_enabled = true,
            0xC003 => self.irq_pending = false,
            0xE000 => {
                self.mirroring = if value & 0x40 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = if address < 0x1000 {
            let slot = usize::from(address / 0x800);
            usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1)
        } else {
            let slot = usize::from((address - 0x1000) / 0x400) + 2;
            usize::from(self.chr_banks[slot])
        };
        self.chr[(bank % banks) * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let bank = if address < 0x1000 {
            let slot = usize::from(address / 0x800);
            usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1)
        } else {
            let slot = usize::from((address - 0x1000) / 0x400) + 2;
            usize::from(self.chr_banks[slot])
        };
        let index = (bank % banks) * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        48
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// TXC 01-22000-400 (mapper 36): a 2-bit latch/inverter ASIC selecting the
/// 32 KiB PRG bank, plus a separate 8 KiB CHR latch at `$4200`.
#[derive(Debug)]
pub struct Txc36 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    p: u8,
    r: u8,
    invert: bool,
    increment: bool,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Txc36 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            p: 0,
            r: 0,
            invert: false,
            increment: false,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Txc36 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = usize::from(self.r & 0x03) % banks;
        self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address & 0xE200 == 0x4200 {
            self.chr_bank = value & 0x0F;
            return;
        }
        match address & 0xE103 {
            0x4100 => {
                if self.increment {
                    self.r = self.r.wrapping_add(1) & 0x03;
                } else {
                    self.r = if self.invert { !self.p & 0x03 } else { self.p };
                }
            }
            0x4101 => self.invert = value & 0x10 != 0,
            0x4102 => self.p = (value >> 4) & 0x03,
            0x4103 => self.increment = value & 0x10 != 0,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        36
    }
}

/// FDS-to-cartridge conversion (mapper 42): an 8 KiB bank at `$6000-$7FFF`,
/// fixed last 32 KiB, an 8 KiB CHR bank, mirroring and a 24576-cycle IRQ.
#[derive(Debug)]
pub struct Fds42 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_ram: bool,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
    irq_counter: u16,
    irq_enabled: bool,
    irq_pending: bool,
}

impl Fds42 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            chr_ram: header.chr_size == 0,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
        }
    }
}

impl Mapper for Fds42 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => {
                let banks = (self.prg.len() / 0x2000).max(1);
                let bank = usize::from(self.prg_bank) % banks;
                self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
            }
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = banks - 1;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xE003 {
            0x8000 => self.chr_bank = value & 0x0F,
            0xE000 => self.prg_bank = value & 0x0F,
            0xE001 => {
                self.mirroring = if value & 0x08 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xE002 => {
                self.irq_enabled = value & 0x01 != 0;
                if !self.irq_enabled {
                    self.irq_counter = 0;
                    self.irq_pending = false;
                }
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        if self.chr_ram {
            return self
                .chr
                .get(usize::from(address) & 0x1FFF)
                .copied()
                .unwrap_or(0);
        }
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let offset = usize::from(address) & 0x1FFF;
        if self.chr_ram {
            if let Some(byte) = self.chr.get_mut(offset) {
                *byte = value;
            }
            return;
        }
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        let index = bank * 0x2000 + offset;
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        42
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if !self.irq_enabled {
            return;
        }
        self.irq_counter = self.irq_counter.wrapping_add(1) & 0x7FFF;
        self.irq_pending = self.irq_counter & 0x6000 == 0x6000;
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Waixing FS304 (mapper 162): 32 KiB PRG banking, 8 KiB CHR-RAM and
/// battery-backed PRG-RAM, with the `$5000`/`$5100`/`$5200`/`$5300` bank
/// composition.
#[derive(Debug)]
pub struct Fs304 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    low: u8,
    alt_low: u8,
    high: u8,
    mode: u8,
    mirroring: Mirroring,
}

impl Fs304 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            low: 0,
            alt_low: 0,
            high: 0,
            mode: 0,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank(&self) -> usize {
        let a20_19 = usize::from(self.high & 0x03);
        let a18_17 = usize::from((self.low >> 4) & 0x03);
        let a16 = if self.mode & 0x04 == 0 {
            1
        } else {
            usize::from((self.low >> 1) & 0x01)
        };
        let a15 = if self.mode & 0x01 == 0 {
            usize::from((self.alt_low >> 1) & 0x01)
        } else if self.mode & 0x04 == 0 {
            1
        } else {
            usize::from(self.low & 0x01)
        };
        (a20_19 << 4) | (a18_17 << 2) | (a16 << 1) | a15
    }
}

impl Mapper for Fs304 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = self.prg_bank() % banks;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x50FF => self.low = value,
            0x5100..=0x51FF => self.alt_low = value,
            0x5200..=0x52FF => self.high = value,
            0x5300..=0x53FF => self.mode = value,
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        162
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// HES NTD-8 (mapper 113): 32 KiB PRG / 8 KiB CHR banking with mirroring, in
/// a control register in `$4100-$5FFF`.
#[derive(Debug)]
pub struct Hes113 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Hes113 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Hes113 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x8000).max(1);
        let bank = usize::from(self.prg_bank) % banks;
        self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address & 0xE100 == 0x4100 {
            self.prg_bank = (value >> 3) & 0x07;
            self.chr_bank = (value & 0x07) | ((value >> 3) & 0x08);
            self.mirroring = if value & 0x80 != 0 {
                Mirroring::Vertical
            } else {
                Mirroring::Horizontal
            };
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        113
    }
}

/// Ka Sheng / SuperGame MMC3 clone (mapper 114): MMC3 registers behind a
/// scrambled address/index map, an NROM PRG override at `$6000` and a
/// 256 KiB outer CHR bank at `$6001`.
#[derive(Debug)]
pub struct Mmc3_114 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    select: u8,
    bank_data: [u8; 8],
    nrom: u8,
    chr_outer: u8,
    scramble: bool,
    mirroring: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

const M114_ADDR_A: [u16; 8] = [
    0xA001, 0xA000, 0x8000, 0xC000, 0x8001, 0xC001, 0xE000, 0xE001,
];
const M114_ADDR_B: [u16; 8] = [
    0xA001, 0x8001, 0x8000, 0xC001, 0xA000, 0xC000, 0xE000, 0xE001,
];
const M114_INDEX: [u8; 8] = [0, 3, 1, 5, 6, 7, 2, 4];

impl Mmc3_114 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            select: 0,
            bank_data: [0; 8],
            nrom: 0,
            chr_outer: 0,
            scramble: header.submapper == 0,
            mirroring: header.mirroring,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn real_address(&self, address: u16) -> u16 {
        let base = match address & 0xE001 {
            0x8000 => 0,
            0x8001 => 1,
            0xA000 => 2,
            0xA001 => 3,
            0xC000 => 4,
            0xC001 => 5,
            0xE000 => 6,
            _ => 7,
        };
        if self.scramble {
            M114_ADDR_A[base]
        } else {
            M114_ADDR_B[base]
        }
    }

    fn remap_index(&self, value: u8) -> u8 {
        if self.scramble {
            M114_INDEX[usize::from(value & 0x07)] | (value & 0xF8)
        } else {
            value
        }
    }

    fn prg_slot(&self, slot: usize) -> usize {
        let banks = (self.prg.len() / 0x2000).max(1);
        let last = banks - 1;
        let second_last = banks.saturating_sub(2);
        let mode = self.select & 0x40;
        let raw = match (mode, slot) {
            (0, 0) => usize::from(self.bank_data[6]),
            (0, 1) => usize::from(self.bank_data[7]),
            (0, 2) => second_last,
            (0, _) => last,
            (_, 0) => second_last,
            (_, 1) => usize::from(self.bank_data[7]),
            (_, 2) => usize::from(self.bank_data[6]),
            (_, _) => last,
        };
        raw % banks
    }

    fn chr_bank(&self, slot: usize) -> usize {
        let banks = (self.chr.len() / 0x400).max(1);
        let mode = self.select & 0x80;
        let raw = if mode == 0 {
            match slot {
                0 | 1 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                2 | 3 => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
                4 => usize::from(self.bank_data[2]),
                5 => usize::from(self.bank_data[3]),
                6 => usize::from(self.bank_data[4]),
                _ => usize::from(self.bank_data[5]),
            }
        } else {
            match slot {
                0 => usize::from(self.bank_data[2]),
                1 => usize::from(self.bank_data[3]),
                2 => usize::from(self.bank_data[4]),
                3 => usize::from(self.bank_data[5]),
                4 | 5 => usize::from(self.bank_data[0] & 0xFE) + (slot & 1),
                _ => usize::from(self.bank_data[1] & 0xFE) + (slot & 1),
            }
        };
        (raw | (usize::from(self.chr_outer & 0x01) << 8)) % banks
    }
}

impl Mapper for Mmc3_114 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x2000).max(1);
        if self.nrom & 0x40 != 0 {
            let mut bank16 = usize::from(self.nrom & 0x0F);
            if self.nrom & 0x10 != 0 {
                bank16 = (bank16 & 0x0E) | usize::from((address >> 14) & 1);
            }
            let bank = ((bank16 * 2) + usize::from((address >> 13) & 1)) % banks;
            return self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)];
        }
        let slot = usize::from((address - 0x8000) / 0x2000);
        let bank = self.prg_slot(slot);
        self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0xE001 {
            0x6000 => {
                self.nrom = value;
                return;
            }
            0x6001 => {
                self.chr_outer = value & 0x01;
                return;
            }
            _ => {}
        }
        if address < 0x8000 {
            return;
        }
        match self.real_address(address) & 0xE001 {
            0x8000 => self.select = self.remap_index(value),
            0x8001 => {
                let index = usize::from(self.select & 0x07);
                self.bank_data[index] = value;
            }
            0xA000 => {
                self.mirroring = if value & 1 != 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            0xC000 => self.irq_latch = value,
            0xC001 => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            0xE001 => self.irq_enabled = true,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let slot = usize::from(address) / 0x400;
        let bank = self.chr_bank(slot);
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        114
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_scanline(&mut self) {
        if !self.irq_enabled {
            return;
        }
        let reload = self.irq_counter == 0 || self.irq_reload;
        if reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 {
            self.irq_pending = true;
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Mapper 152: 16 KiB PRG at `$8000`, 8 KiB CHR and one-screen mirroring from
/// a single bus-conflicted latch at `$8000-$FFFF`.
#[derive(Debug)]
pub struct Bus152 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Bus152 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Bus152 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = (self.prg.len() / 0x4000).max(1);
        let bank = match address {
            0x8000..=0xBFFF => usize::from(self.prg_bank),
            0xC000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x4000 + usize::from(address & 0x3FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address < 0x8000 {
            return;
        }
        let banks = (self.prg.len() / 0x4000).max(1);
        let visible = self
            .prg
            .get((usize::from(self.prg_bank) % banks) * 0x4000 + usize::from(address & 0x3FFF))
            .copied()
            .unwrap_or(0xFF);
        let value = value & visible;
        self.prg_bank = (value >> 4) & 0x07;
        self.chr_bank = value & 0x0F;
        self.mirroring = if value & 0x80 != 0 {
            Mirroring::SingleUpper
        } else {
            Mirroring::SingleLower
        };
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        152
    }
}

/// Multicart 226: a 7-bit PRG latch at `$8000`/`$8001` with mirroring and a
/// 32 KiB/16 KiB mode; CHR is unbanked RAM.
#[derive(Debug)]
pub struct Multicart226 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    low: u8,
    high: u8,
    mode32: bool,
    mirroring: Mirroring,
}

impl Multicart226 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            low: 0,
            high: 0,
            mode32: true,
            mirroring: header.mirroring,
        }
    }

    fn bank(&self) -> usize {
        usize::from(self.low & 0x3F) | (usize::from(self.high & 0x01) << 6)
    }
}

impl Mapper for Multicart226 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        if self.mode32 {
            let banks = (self.prg.len() / 0x8000).max(1);
            let bank = self.bank() % banks;
            self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
        } else {
            let banks = (self.prg.len() / 0x4000).max(1);
            let bank = self.bank() % banks;
            self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address & 0x8001 {
            0x8000 => {
                self.low = value & 0x3F;
                self.mode32 = value & 0x80 == 0;
                self.mirroring = if value & 0x40 != 0 {
                    Mirroring::Vertical
                } else {
                    Mirroring::Horizontal
                };
            }
            0x8001 => self.high = value & 0x01,
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        226
    }
}

/// GNROM-style latch at `$4020-$5FFF` (mapper 240): 32 KiB PRG / 8 KiB CHR
/// plus PRG-RAM.
#[derive(Debug)]
pub struct Gnrom240 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    prg_bank: u8,
    chr_bank: u8,
    mirroring: Mirroring,
}

impl Gnrom240 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            prg_bank: 0,
            chr_bank: 0,
            mirroring: header.mirroring,
        }
    }

    fn is_register(address: u16) -> bool {
        address & 0xE800 == 0x4800 || address & 0xE100 == 0x4100
    }
}

impl Mapper for Gnrom240 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = usize::from(self.prg_bank) % banks;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if Self::is_register(address) {
            self.prg_bank = (value >> 4) & 0x0F;
            self.chr_bank = value & 0x0F;
            return;
        }
        if (0x6000..=0x7FFF).contains(&address) {
            let index = usize::from(address - 0x6000) % self.prg_ram.len();
            self.prg_ram[index] = value;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.chr_bank) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        240
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Kaiser KS7032 / VRC3-like (mapper 142): four 8 KiB PRG windows plus a
/// fixed last bank, 8 KiB CHR-RAM and a 16-bit cycle IRQ.
#[derive(Debug)]
pub struct Ks7032 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_banks: [u8; 4],
    select: u8,
    irq_reload: u16,
    irq_counter: u16,
    irq_enabled: bool,
    irq_pending: bool,
    mirroring: Mirroring,
}

impl Ks7032 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_banks: [0; 4],
            select: 0,
            irq_reload: 0,
            irq_counter: 0,
            irq_enabled: false,
            irq_pending: false,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Ks7032 {
    fn prg_read(&self, address: u16) -> u8 {
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = match address {
            0x6000..=0x7FFF => usize::from(self.prg_banks[3]),
            0x8000..=0x9FFF => usize::from(self.prg_banks[0]),
            0xA000..=0xBFFF => usize::from(self.prg_banks[1]),
            0xC000..=0xDFFF => usize::from(self.prg_banks[2]),
            0xE000..=0xFFFF => banks - 1,
            _ => return 0,
        };
        self.prg[(bank % banks) * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x8000..=0xBFFF => {
                let shift = u32::from(address & 0x03) * 4;
                self.irq_reload =
                    (self.irq_reload & !(0x000F << shift)) | (u16::from(value & 0x0F) << shift);
            }
            0xC000..=0xCFFF => {
                self.irq_enabled = value & 0x06 != 0;
                self.irq_counter = self.irq_reload;
                if !self.irq_enabled {
                    self.irq_pending = false;
                }
            }
            0xD000..=0xDFFF => self.irq_pending = false,
            0xE000..=0xEFFF => self.select = value & 0x07,
            0xF000..=0xFFFF => match self.select {
                1 => self.prg_banks[0] = value & 0x0F,
                2 => self.prg_banks[1] = value & 0x0F,
                3 => self.prg_banks[2] = value & 0x0F,
                4 => self.prg_banks[3] = value & 0x0F,
                _ => {}
            },
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        142
    }

    fn irq_line(&self) -> bool {
        self.irq_pending
    }

    fn clock_cpu(&mut self) {
        if !self.irq_enabled {
            return;
        }
        if self.irq_counter == 0xFFFF {
            self.irq_counter = self.irq_reload;
            self.irq_pending = true;
        } else {
            self.irq_counter = self.irq_counter.wrapping_add(1);
        }
    }

    fn acknowledge_irq(&mut self) {
        self.irq_pending = false;
    }
}

/// Yancheng YC-03-09 (NES 2.0 mapper 558): 32 KiB PRG banking from
/// `$5000`/`$5100`/`$5300`, 8 KiB CHR-RAM and PRG-RAM.
#[derive(Debug)]
pub struct Fs558 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_ram: Vec<u8>,
    low: u8,
    high: u8,
    mode: u8,
    mirroring: Mirroring,
}

impl Fs558 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, prg_ram: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_ram,
            low: 0,
            high: 0,
            mode: 0,
            mirroring: header.mirroring,
        }
    }

    fn prg_bank(&self) -> usize {
        let outer = usize::from(self.high & 0x03) << 4;
        let (a15, a16) = if self.mode & 0x04 == 0 {
            (1, 1)
        } else {
            (
                usize::from(self.low & 0x01),
                usize::from((self.low >> 1) & 0x01),
            )
        };
        outer | (usize::from((self.low >> 2) & 0x03) << 2) | (a16 << 1) | a15
    }

    fn swap_bits(value: u8, mode: u8) -> u8 {
        if mode & 0x01 == 0 {
            (value & !0x03) | ((value & 0x01) << 1) | ((value & 0x02) >> 1)
        } else {
            value
        }
    }
}

impl Mapper for Fs558 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => self.prg_ram[usize::from(address - 0x6000) % self.prg_ram.len()],
            0x8000..=0xFFFF => {
                let banks = (self.prg.len() / 0x8000).max(1);
                let bank = self.prg_bank() % banks;
                self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        match address {
            0x5000..=0x50FF => self.low = Self::swap_bits(value, self.mode),
            0x5100..=0x51FF => self.high = Self::swap_bits(value, self.mode),
            0x5200..=0x52FF => {}
            0x5300..=0x53FF => self.mode = value,
            0x6000..=0x7FFF => {
                let index = usize::from(address - 0x6000) % self.prg_ram.len();
                self.prg_ram[index] = value;
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        558
    }

    fn prg_ram(&self) -> &[u8] {
        &self.prg_ram
    }
}

/// Discrete-logic multicart (mapper 212): the write address is the latch,
/// selecting 16 or 32 KiB PRG, 8 KiB CHR and mirroring.
#[derive(Debug)]
pub struct Multicart212 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    bank: u8,
    style32: bool,
    mirroring: Mirroring,
}

impl Multicart212 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            bank: 0,
            style32: false,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Multicart212 {
    fn prg_read(&self, address: u16) -> u8 {
        match address {
            0x6000..=0x7FFF => {
                // Solder pad value; D7 is the inverse of A4.
                if address & 0x10 != 0 {
                    0x00
                } else {
                    0x80
                }
            }
            0x8000..=0xFFFF => {
                if self.style32 {
                    let banks = (self.prg.len() / 0x8000).max(1);
                    let bank = usize::from((self.bank >> 1) & 0x03) % banks;
                    self.prg[(bank * 0x8000 + usize::from(address & 0x7FFF)) % self.prg.len()]
                } else {
                    let banks = (self.prg.len() / 0x4000).max(1);
                    let bank = usize::from(self.bank & 0x07) % banks;
                    self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
                }
            }
            _ => 0,
        }
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            self.bank = (address & 0x07) as u8;
            self.style32 = address & 0x10 != 0;
            self.mirroring = if address & 0x08 != 0 {
                Mirroring::Horizontal
            } else {
                Mirroring::Vertical
            };
            let _ = value;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.bank & 0x07) % banks;
        self.chr[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x2000).max(1);
        let bank = usize::from(self.bank & 0x07) % banks;
        let index = bank * 0x2000 + usize::from(address & 0x1FFF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        212
    }
}

/// Protected NROM board (mapper 143): normal NROM PRG/CHR plus a protection
/// read in `$4100-$5FFF` that returns the inverted low address bits.
#[derive(Debug)]
pub struct Prot143 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    mirroring: Mirroring,
}

impl Prot143 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            mirroring: header.mirroring,
        }
    }
}

impl Mapper for Prot143 {
    fn prg_read(&self, address: u16) -> u8 {
        if address & 0xE100 == 0x4100 {
            return ((!address) & 0x3F) as u8;
        }
        if address < 0x8000 {
            return 0;
        }
        let offset = usize::from(address & 0x7FFF) % self.prg.len().max(1);
        self.prg.get(offset).copied().unwrap_or(0)
    }

    fn prg_write(&mut self, _address: u16, _value: u8) {}

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, _address: u16, _value: u8) {}

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        143
    }
}

/// Golden Game 150-in-1 (mapper 235): the write address latches the 32 KiB
/// page, 16 KiB half, ROM chip, mirroring and one/two-screen mode.
#[derive(Debug)]
pub struct Multicart235 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    latch: u16,
    mirroring: Mirroring,
}

impl Multicart235 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            latch: 0,
            mirroring: header.mirroring,
        }
    }

    fn page32(&self) -> usize {
        usize::from(self.latch & 0x1F) | (usize::from((self.latch >> 8) & 0x03) << 5)
    }

    fn bank16(&self, address: u16) -> usize {
        let base = self.page32() * 2;
        if self.latch & 0x0800 != 0 {
            // 16 KiB page mode: P picks the half at $8000, the other at $C000.
            let p = usize::from((self.latch >> 12) & 0x01);
            if address < 0xC000 {
                base + p
            } else {
                base + (1 - p)
            }
        } else {
            // 32 KiB page mode: lower half at $8000, upper at $C000.
            base + usize::from((address >> 14) & 1)
        }
    }
}

impl Mapper for Multicart235 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let banks = (self.prg.len() / 0x4000).max(1);
        let bank = self.bank16(address) % banks;
        self.prg[bank * 0x4000 + usize::from(address & 0x3FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        if address >= 0x8000 {
            self.latch = address;
            let one_screen = address & 0x0400 != 0;
            self.mirroring = if one_screen {
                if address & 0x2000 != 0 {
                    Mirroring::SingleUpper
                } else {
                    Mirroring::SingleLower
                }
            } else if address & 0x2000 != 0 {
                Mirroring::Vertical
            } else {
                Mirroring::Horizontal
            };
            let _ = value;
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        self.chr
            .get(usize::from(address) & 0x1FFF)
            .copied()
            .unwrap_or(0)
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        if let Some(byte) = self.chr.get_mut(usize::from(address) & 0x1FFF) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        235
    }
}

/// Benshieng BMC-BS-5 (NES 2.0 mapper 286): address-latched 8 KiB PRG and
/// 2 KiB CHR windows.
#[derive(Debug)]
pub struct Benshieng286 {
    prg: Vec<u8>,
    chr: Vec<u8>,
    prg_banks: [u8; 4],
    chr_banks: [u8; 4],
    mirroring: Mirroring,
}

impl Benshieng286 {
    fn new(prg: Vec<u8>, chr: Vec<u8>, header: Header) -> Self {
        Self {
            prg,
            chr,
            prg_banks: [0, 1, 2, 3],
            chr_banks: [0; 4],
            mirroring: header.mirroring,
        }
    }

    fn latch_bank(address: u16) -> u8 {
        ((((address >> 8) & 0x03) << 4) | (address & 0x0F)) as u8
    }
}

impl Mapper for Benshieng286 {
    fn prg_read(&self, address: u16) -> u8 {
        if address < 0x8000 {
            return 0;
        }
        let window = usize::from((address - 0x8000) / 0x2000);
        let banks = (self.prg.len() / 0x2000).max(1);
        let bank = usize::from(self.prg_banks[window]) % banks;
        self.prg[bank * 0x2000 + usize::from(address & 0x1FFF)]
    }

    fn prg_write(&mut self, address: u16, value: u8) {
        let _ = value;
        match address & 0xE000 {
            0x8000 => {
                let window = usize::from((address >> 10) & 0x03);
                self.chr_banks[window] = Self::latch_bank(address);
            }
            0xA000 => {
                let window = usize::from((address >> 10) & 0x03);
                self.prg_banks[window] = Self::latch_bank(address);
            }
            _ => {}
        }
    }

    fn chr_read(&self, address: u16) -> u8 {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address / 0x800) & 0x03;
        let bank =
            (usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1)) % banks;
        self.chr[bank * 0x400 + usize::from(address & 0x3FF)]
    }

    fn chr_write(&mut self, address: u16, value: u8) {
        let banks = (self.chr.len() / 0x400).max(1);
        let slot = usize::from(address / 0x800) & 0x03;
        let bank =
            (usize::from(self.chr_banks[slot]) * 2 + usize::from((address >> 10) & 1)) % banks;
        let index = bank * 0x400 + usize::from(address & 0x3FF);
        if let Some(byte) = self.chr.get_mut(index) {
            *byte = value;
        }
    }

    fn mirroring(&self) -> Mirroring {
        self.mirroring
    }

    fn number(&self) -> u16 {
        286
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fs305_switches_prg_modes() {
        let rom = synth_banked(178, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Mode 0 (NROM-256): PRG A14 follows the CPU address.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 1);
        cart.cpu_write(0x4801, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xC000), 3);
        cart.cpu_write(0x4802, 1);
        assert_eq!(cart.cpu_read(0x8000), 10);
        // Mode 1 (UNROM): $C000 is fixed to the last 8 KiB bank of the outer.
        cart.cpu_write(0x4800, 0x04);
        assert_eq!(cart.cpu_read(0xC000), 15);
    }

    #[test]
    fn action53_calculates_prg_bank() {
        let rom = synth_banked(28, 128, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert!(cart.chr_is_ram());
        // Supervisor register $80: 32 KiB banking, vertical mirroring.
        cart.cpu_write(0x5000, 0x80);
        cart.cpu_write(0x8000, 0x02);
        cart.cpu_write(0x5000, 0x81);
        cart.cpu_write(0x8000, 0x03);
        // The synthetic ROM fills 8 KiB banks with their indices.
        assert_eq!(cart.cpu_read(0x8000), 12);
        assert_eq!(cart.cpu_read(0xC000), 14);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        // 64 KiB outer, UNROM mode: $01 selects the switched 16 KiB bank.
        cart.cpu_write(0x5000, 0x80);
        cart.cpu_write(0x8000, 0x1E);
        cart.cpu_write(0x5000, 0x81);
        cart.cpu_write(0x8000, 0x00);
        cart.cpu_write(0x5000, 0x01);
        cart.cpu_write(0x8000, 0x03);
        assert_eq!(cart.cpu_read(0x8000), 6);
        assert_eq!(cart.cpu_read(0xC000), 2);
        // CHR-RAM window selects one of four 8 KiB banks.
        cart.cpu_write(0x5000, 0x00);
        cart.cpu_write(0x8000, 0x01);
        cart.ppu_write(0x0000, 0x5A);
        assert_eq!(cart.ppu_read(0x0000), 0x5A);
    }

    #[test]
    fn namco163_banks_and_irq() {
        let rom = synth_banked(19, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0xE000, 1);
        assert_eq!(cart.cpu_read(0x8000), 1);
        cart.cpu_write(0xE800, 2);
        assert_eq!(cart.cpu_read(0xA000), 2);
        cart.cpu_write(0xF000, 3);
        assert_eq!(cart.cpu_read(0xC000), 3);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0x8000, 5);
        assert_eq!(cart.ppu_read(0x0000), 0x85);
        // Chip RAM port.
        cart.cpu_write(0xF800, 0x40);
        cart.cpu_write(0x4800, 0x5A);
        assert_eq!(cart.cpu_read(0x4800), 0x5A);
        // 15-bit up counter fires at $7FFF.
        cart.cpu_write(0x5000, 0xFE);
        cart.cpu_write(0x5800, 0xFF);
        assert!(!cart.irq_line());
        cart.clock_cpu();
        assert!(cart.irq_line());
    }

    #[test]
    fn jy_asic_banks_and_irq() {
        let rom = synth_banked(90, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // 8 KiB PRG banking, fixed last bank at $E000.
        cart.cpu_write(0xD000, 0x02);
        cart.cpu_write(0x8000, 2);
        cart.cpu_write(0x8001, 3);
        cart.cpu_write(0x8002, 4);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 4);
        assert_eq!(cart.cpu_read(0xE000), 15);
        // 1 KiB CHR banking.
        cart.cpu_write(0xD000, 0x1A);
        cart.cpu_write(0x9000, 3);
        assert_eq!(cart.ppu_read(0x0000), 0x83);
        // IRQ: M2 clock, increase, 8-cycle prescaler.
        cart.cpu_write(0xC000, 0x01);
        cart.cpu_write(0xC001, 0x14);
        for _ in 0..2050 {
            cart.clock_cpu();
        }
        assert!(cart.irq_line());
    }

    use super::*;

    #[test]
    fn j87_selects_chr_bank() {
        let rom = synth_banked(87, 32, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.ppu_read(0x0000), 0x80);
        cart.cpu_write(0x6000, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
        let reversed = synth_banked(101, 32, 32);
        let cart = Cartridge::from_ines(&reversed).expect("cart");
        assert_eq!(cart.ppu_read(0x0000), 0x01);
    }

    #[test]
    fn jy_clone_banks_and_irq() {
        let rom = synth_banked(91, 512, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x7000, 2);
        cart.cpu_write(0x7001, 3);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 63);
        // Outer 128 KiB PRG bank 2 selects 8 KiB bank 34.
        cart.cpu_write(0x8000, 0x02);
        assert_eq!(cart.cpu_read(0x8000), 34);
        cart.cpu_write(0x6000, 4);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
        cart.cpu_write(0x7007, 0);
        for _ in 0..8 {
            cart.clock_scanline();
        }
        assert!(cart.irq_line());
    }

    #[test]
    fn bandai_fcg_banks_chr_and_irq() {
        let rom = synth_banked(16, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x6008, 2);
        // The synthetic ROM fills 8 KiB banks with their indices.
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xC000), 14);
        cart.cpu_write(0x6000, 3);
        assert_eq!(cart.ppu_read(0x0000), 0x83);
        cart.cpu_write(0x600B, 0xFE);
        cart.cpu_write(0x600C, 0x00);
        cart.cpu_write(0x600A, 0x01);
        assert!(!cart.irq_line());
        for _ in 0..0xFE {
            cart.clock_cpu();
        }
        assert!(cart.irq_line());
    }

    #[test]
    fn txc112_banks_and_mirroring() {
        let rom = synth_banked(112, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0xA000, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        cart.cpu_write(0x8000, 1);
        cart.cpu_write(0xA000, 3);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 14);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0x8000, 2);
        cart.cpu_write(0xA000, 3);
        assert_eq!(cart.ppu_read(0x0000), 0x86);
        cart.cpu_write(0xE000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn bxrom_wram_banks_32k() {
        let rom = synth_banked(241, 128, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.cpu_read(0x8000), 8);
        cart.cpu_write(0x6000, 0x5A);
        assert_eq!(cart.cpu_read(0x6000), 0x5A);
    }

    #[test]
    fn multicart58_latches_banks() {
        let rom = synth_banked(58, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 0x09);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn cnrom185_has_bus_conflicts() {
        let mut rom = synth_banked(185, 128, 32);
        rom[16] = 0xFF;
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
    }

    #[test]
    fn hengge177_banks_and_mirrors() {
        let rom = synth_banked(177, 128, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 0x21);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x6000, 0x5A);
        assert_eq!(cart.cpu_read(0x6000), 0x5A);
    }

    #[test]
    fn txc_latch_shifts_and_reads_back() {
        let rom = synth_banked(132, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x4102, 0x05);
        cart.cpu_write(0x4100, 0x00);
        assert_eq!(cart.cpu_read(0x4100), 0x05);
        cart.cpu_write(0x8000, 0x00);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
    }

    #[test]
    fn mmc3_32k_banks_prg_and_chr() {
        let rom = synth_banked(189, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x4100, 0x20);
        assert_eq!(cart.cpu_read(0x8000), 8);
        assert_eq!(cart.cpu_read(0xC000), 10);
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 3);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
        cart.cpu_write(0xA000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn coolboy_maps_outer_prg_banks() {
        let rom = synth_banked(224, 512, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 224);
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 62);
        // Force PRG A17 from the outer register.
        cart.cpu_write(0x5000, 0x81);
        assert_eq!(cart.cpu_read(0x8000), 16);
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 3);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
    }

    #[test]
    fn mmc3_176_modes_and_base_banks() {
        let rom = synth_banked(176, 512, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 6);
        cart.cpu_write(0x8001, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xC000), 62);
        assert_eq!(cart.cpu_read(0xE000), 63);
        cart.cpu_write(0xA000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        // NROM-128: 16 KiB bank from the base register, mirrored.
        cart.cpu_write(0x5000, 3);
        cart.cpu_write(0x5001, 2);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xA000), 5);
        assert_eq!(cart.cpu_read(0xE000), 5);
        // NROM-256: 32 KiB bank from the base register.
        cart.cpu_write(0x5000, 4);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xC000), 6);
    }

    #[test]
    fn mmc3_215_unscrambles_registers() {
        let rom = synth_banked(215, 512, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Pattern 1 maps $A000 to the real $8000 bank-select and unscrambles
        // its data low bits. Real index 6 is selected by writing 2.
        cart.cpu_write(0x5001, 0x00);
        cart.cpu_write(0x5007, 0x01);
        cart.cpu_write(0xA000, 2);
        // Pattern 1 maps $C000 to the real $8001 bank-data register.
        cart.cpu_write(0xC000, 4);
        // Slot 0 uses PRG R6.
        assert_eq!(cart.cpu_read(0x8000), 4);
        cart.cpu_write(0xC000, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
    }

    #[test]
    fn mmc3_115_nrom_mode_and_chr_outer() {
        let rom = synth_banked(115, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 6);
        cart.cpu_write(0x8001, 3);
        assert_eq!(cart.cpu_read(0x8000), 3);
        assert_eq!(cart.cpu_read(0xE000), 15);
        // NROM-128 mode: 16 KiB bank 2 mirrored across the 32 KiB window.
        cart.cpu_write(0x6000, 0x42);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xC000), 4);
        // CHR A18 outer bit.
        cart.cpu_write(0x6001, 1);
        assert_eq!(cart.cpu_read(0x8000), 4);
    }

    #[test]
    fn sachen8259_banks_prg_and_chr() {
        let rom = synth_banked(141, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x4100, 5);
        cart.cpu_write(0x4101, 2);
        assert_eq!(cart.cpu_read(0x8000), 8);
        cart.cpu_write(0x4100, 0);
        cart.cpu_write(0x4101, 3);
        cart.cpu_write(0x4100, 4);
        cart.cpu_write(0x4101, 1);
        assert_eq!(cart.ppu_read(0x0000), 0x8C);
        cart.cpu_write(0x4100, 7);
        cart.cpu_write(0x4101, 0x02);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn quattro232_banks_outer_and_inner() {
        let rom = synth_banked(232, 256, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 0x08);
        cart.cpu_write(0xC000, 2);
        assert_eq!(cart.cpu_read(0x8000), 12);
        assert_eq!(cart.cpu_read(0xC000), 14);
    }

    #[test]
    fn namco210_banks_prg_and_chr() {
        let rom = synth_banked(210, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0xE000, 2);
        cart.cpu_write(0xE800, 3);
        cart.cpu_write(0xF000, 4);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 4);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0x8800, 5);
        assert_eq!(cart.ppu_read(0x0400), 0x85);
        cart.cpu_write(0xE000, 0x40);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn mmc3_121_protection_and_banks() {
        let rom = synth_banked(121, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x5000, 1);
        assert_eq!(cart.cpu_read(0x5000), 0x83);
        cart.cpu_write(0x8000, 6);
        cart.cpu_write(0x8001, 4);
        assert_eq!(cart.cpu_read(0x8000), 4);
        // Protection index $26 applies the bit-reversed $8001 value to $E000.
        cart.cpu_write(0x8003, 0x26);
        assert_eq!(cart.cpu_read(0xE000), 8);
    }

    #[test]
    fn gtrom_banks_32k_pages() {
        let rom = synth_banked(111, 512, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x5000, 3);
        assert_eq!(cart.cpu_read(0x8000), 12);
        cart.cpu_write(0x7000, 5);
        assert_eq!(cart.cpu_read(0x8000), 20);
    }

    #[test]
    fn cony83_banks_via_base_and_8k_regs() {
        let rom = synth_banked(83, 256, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // 4x8 KiB mode, base 0.
        cart.cpu_write(0x8100, 0x08);
        cart.cpu_write(0x8300, 2);
        cart.cpu_write(0x8301, 3);
        cart.cpu_write(0x8302, 4);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 4);
        assert_eq!(cart.cpu_read(0xE000), 31);
        cart.cpu_write(0x8310, 5);
        assert_eq!(cart.ppu_read(0x0000), 0x85);
        cart.cpu_write(0x5100, 0x5A);
        assert_eq!(cart.cpu_read(0x5100), 0x5A);
    }

    #[test]
    fn taito33_banks_prg_and_chr() {
        let rom = synth_banked(33, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 14);
        cart.cpu_write(0x8000, 0x42);
        cart.cpu_write(0x8001, 3);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x8002, 1);
        cart.cpu_write(0xA000, 5);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
        assert_eq!(cart.ppu_read(0x1000), 0x85);
    }

    #[test]
    fn vrc1_banks_prg_and_chr() {
        let rom = synth_banked(75, 128, 256);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0xE000, 1);
        cart.cpu_write(0x9000, 0);
        assert_eq!(cart.ppu_read(0x0000), 0x84);
        cart.cpu_write(0x9000, 3);
        assert_eq!(cart.ppu_read(0x0000), 0xC4);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn taito80_banks_and_internal_ram() {
        let rom = synth_banked(80, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x7EFA, 3);
        assert_eq!(cart.cpu_read(0x8000), 3);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0x7EF0, 2);
        cart.cpu_write(0x7EF2, 5);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
        assert_eq!(cart.ppu_read(0x1000), 0x85);
        cart.cpu_write(0x7EF6, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x7EF8, 1);
        cart.cpu_write(0x7F00, 0x5A);
        assert_eq!(cart.cpu_read(0x7F00), 0x5A);
        assert_eq!(cart.cpu_read(0x7F80), 0x5A);
    }

    #[test]
    fn multicart242_latches_address() {
        let rom = synth_banked(242, 512, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8020, 0);
        assert_eq!(cart.cpu_read(0x8000), 16);
        // NROM-256: O=1, S=1; CPU A14 selects the two 16 KiB halves.
        cart.cpu_write(0x8083, 0);
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 2);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn sunsoft1_selects_two_4k_chr_banks() {
        let rom = synth_banked(184, 32, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.ppu_read(0x0000), 0x80);
        assert_eq!(cart.ppu_read(0x1000), 0x90);
        cart.cpu_write(0x6000, 0x51);
        assert_eq!(cart.ppu_read(0x0000), 0x84);
        assert_eq!(cart.ppu_read(0x1000), 0x94);
    }

    #[test]
    fn rambo1_banks_prg_chr_and_irq() {
        let rom = synth_banked(64, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 6);
        cart.cpu_write(0x8001, 2);
        cart.cpu_write(0x8000, 7);
        cart.cpu_write(0x8001, 3);
        cart.cpu_write(0x8000, 0x0F);
        cart.cpu_write(0x8001, 4);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 4);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0xA000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
        assert_eq!(cart.ppu_read(0x0400), 0x83);
        cart.cpu_write(0xC000, 1);
        cart.cpu_write(0xC001, 0);
        cart.cpu_write(0xE001, 0);
        for _ in 0..2 {
            cart.clock_scanline();
        }
        assert!(cart.irq_line());
    }

    #[test]
    fn multicart59_latches_address() {
        let rom = synth_banked(59, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8002, 0);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
        // NROM-256: bank 0 at $8000, bank 1 at $C000.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 2);
        // NROM-128: bit 7 set leaves the bank unchanged in both halves.
        cart.cpu_write(0x8090, 0);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xC000), 2);
        cart.cpu_write(0x8008, 0);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn hp898f_banks_prg_and_chr() {
        let rom = synth_banked(319, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x6004, 0x09);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xC000), 6);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x6004, 0x19);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0x6000, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
    }

    #[test]
    fn fds108_banks_6000_and_chr() {
        let rom = synth_banked(108, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0xE000, 2);
        assert_eq!(cart.cpu_read(0x6000), 2);
        assert_eq!(cart.cpu_read(0x8000), 12);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
    }

    #[test]
    fn vrc6_banks_prg_chr_and_irq() {
        let rom = synth_banked(24, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 2);
        cart.cpu_write(0xC000, 5);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xC000), 5);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0xD000, 3);
        cart.cpu_write(0xE000, 4);
        assert_eq!(cart.ppu_read(0x0000), 0x83);
        assert_eq!(cart.ppu_read(0x1000), 0x84);
        cart.cpu_write(0xB003, 0x24);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0xF000, 0xFE);
        cart.cpu_write(0xF001, 0x06);
        cart.clock_cpu();
        cart.clock_cpu();
        assert!(cart.irq_line());
    }

    #[test]
    fn irem32_banks_and_mode() {
        let rom = synth_banked(32, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 2);
        cart.cpu_write(0xA000, 3);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 14);
        cart.cpu_write(0x9000, 0x03);
        assert_eq!(cart.cpu_read(0x8000), 14);
        assert_eq!(cart.cpu_read(0xC000), 2);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0xB000, 5);
        assert_eq!(cart.ppu_read(0x0000), 0x85);
    }

    #[test]
    fn jf13_banks_gnrom() {
        let rom = synth_banked(86, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x6000, 0x51);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
    }

    #[test]
    fn taito48_banks_and_irq() {
        let rom = synth_banked(48, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 2);
        cart.cpu_write(0x8001, 3);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 14);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0x8002, 1);
        cart.cpu_write(0xA000, 5);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
        assert_eq!(cart.ppu_read(0x1000), 0x85);
        cart.cpu_write(0xE000, 0x40);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0xC000, 0xFE);
        cart.cpu_write(0xC001, 0);
        cart.cpu_write(0xC002, 0);
        cart.clock_scanline();
        cart.clock_scanline();
        assert!(cart.irq_line());
    }

    #[test]
    fn txc36_latch_and_chr() {
        let rom = synth_banked(36, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x4102, 0x10);
        cart.cpu_write(0x4100, 0);
        assert_eq!(cart.cpu_read(0x8000), 4);
        cart.cpu_write(0x4200, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
    }

    #[test]
    fn fds42_banks_and_irq() {
        let rom = synth_banked(42, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0xE000, 2);
        assert_eq!(cart.cpu_read(0x6000), 2);
        assert_eq!(cart.cpu_read(0x8000), 12);
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
        cart.cpu_write(0xE001, 0x08);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0xE002, 1);
        for _ in 0..0x6000 {
            cart.clock_cpu();
        }
        assert!(cart.irq_line());
    }

    #[test]
    fn fs304_boots_and_banks() {
        let rom = synth_banked(162, 128, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Power-on maps 32 KiB bank 2.
        assert_eq!(cart.cpu_read(0x8000), 8);
        // $5300=4 takes A16 from $5000.1 and A15 from $5100.1.
        cart.cpu_write(0x5300, 0x04);
        cart.cpu_write(0x5100, 0x02);
        assert_eq!(cart.cpu_read(0x8000), 4);
    }

    #[test]
    fn hes113_banks_prg_chr_and_mirroring() {
        let rom = synth_banked(113, 256, 128);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x4100, 0x12);
        assert_eq!(cart.cpu_read(0x8000), 8);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x4100, 0x80);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn mmc3_114_scrambles_registers() {
        let rom = synth_banked(114, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // $A000 maps to the real $8000 bank select; index 4 maps to register 6.
        cart.cpu_write(0xA000, 4);
        // $C000 maps to the real $8001 bank data.
        cart.cpu_write(0xC000, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        // NROM override maps 16 KiB bank 2 at $8000.
        cart.cpu_write(0x6000, 0x42);
        assert_eq!(cart.cpu_read(0x8000), 4);
    }

    #[test]
    fn bus152_latches_and_conflicts() {
        let mut rom = synth_banked(152, 128, 32);
        rom[16] = 0xFF;
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 0x32);
        assert_eq!(cart.cpu_read(0x8000), 6);
        assert_eq!(cart.cpu_read(0xC000), 14);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
        assert_eq!(cart.mirroring(), Mirroring::SingleLower);
    }

    #[test]
    fn multicart226_banks_32k() {
        let rom = synth_banked(226, 512, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 0x02);
        assert_eq!(cart.cpu_read(0x8000), 8);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn gnrom240_banks_gnrom() {
        let rom = synth_banked(240, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x4800, 0x21);
        assert_eq!(cart.cpu_read(0x8000), 8);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
    }

    #[test]
    fn ks7032_banks_and_irq() {
        let rom = synth_banked(142, 128, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0xE000, 1);
        cart.cpu_write(0xF000, 2);
        cart.cpu_write(0xE000, 2);
        cart.cpu_write(0xF000, 3);
        cart.cpu_write(0xE000, 4);
        cart.cpu_write(0xF000, 5);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0x6000), 5);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0x8000, 0x0E);
        cart.cpu_write(0x8001, 0x0F);
        cart.cpu_write(0x8002, 0x0F);
        cart.cpu_write(0x8003, 0x0F);
        cart.cpu_write(0xC000, 0x02);
        cart.clock_cpu();
        cart.clock_cpu();
        assert!(cart.irq_line());
    }

    #[test]
    fn fs558_boots_bank_three_and_banks() {
        let rom = synth_banked(558, 512, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Power-on: $5300=0 holds A15/A16 high, so bank 3 is mapped.
        assert_eq!(cart.cpu_read(0x8000), 12);
        // $5300=4 lets $5000 provide A15/A16; low nibble 0 -> bank 0.
        cart.cpu_write(0x5300, 0x04);
        cart.cpu_write(0x5000, 0x00);
        assert_eq!(cart.cpu_read(0x8000), 0);
    }

    #[test]
    fn multicart212_latches_address() {
        let rom = synth_banked(212, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8002, 0);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xC000), 4);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
        assert_eq!(cart.cpu_read(0x6000), 0x80);
        assert_eq!(cart.cpu_read(0x6010), 0x00);
        cart.cpu_write(0x8010, 0);
        // 32 KiB bank 0: $8000 is 8 KiB bank 0, $C000 is 8 KiB bank 2.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 2);
    }

    #[test]
    fn prot143_reads_inverted_low_bits() {
        let rom = synth_banked(143, 32, 8);
        let cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x4105), (!0x4105u16) as u8 & 0x3F);
        assert_eq!(cart.cpu_read(0x8000), 0);
    }

    #[test]
    fn multicart235_latches_page_and_half() {
        let rom = synth_banked(235, 512, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // 32 KiB page 1: 16 KiB banks 2/3 -> 8 KiB banks 4/6.
        cart.cpu_write(0x8001, 0);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.cpu_read(0xC000), 6);
        // 16 KiB page mode with P=1: upper half (bank 3 -> 8 KiB 6) first.
        cart.cpu_write(0x9801, 0);
        assert_eq!(cart.cpu_read(0x8000), 6);
        assert_eq!(cart.cpu_read(0xC000), 4);
    }

    #[test]
    fn benshieng286_latches_windows() {
        let rom = synth_banked(286, 512, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // PRG window 0 ($8000): bank 2 -> 8 KiB bank 2.
        cart.cpu_write(0xA002, 0);
        assert_eq!(cart.cpu_read(0x8000), 2);
        // CHR window 0: 2 KiB bank 1 -> 1 KiB bank 2.
        cart.cpu_write(0x8001, 0);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
    }

    fn synthetic(mapper: u8, prg_banks: u8, chr_banks: u8) -> Vec<u8> {
        let prg_len = usize::from(prg_banks) * 16 * 1024;
        let chr_len = usize::from(chr_banks) * 8 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = prg_banks;
        rom[5] = chr_banks;
        rom[6] = mapper << 4;
        rom[7] = mapper & 0xF0;
        // Fill each 16 KiB PRG bank with its index and each 8 KiB CHR bank too.
        for bank in 0..usize::from(prg_banks) * 2 {
            let start = 16 + bank * 0x2000;
            for byte in &mut rom[start..start + 0x2000] {
                *byte = bank as u8;
            }
        }
        for bank in 0..usize::from(chr_banks) {
            let start = 16 + prg_len + bank * 0x2000;
            for byte in &mut rom[start..start + 0x2000] {
                *byte = 0x80 | bank as u8;
            }
        }
        rom
    }

    #[test]
    fn parses_nrom_header() {
        let rom = synthetic(0, 2, 1);
        let cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 0);
        assert_eq!(cart.header().prg_size, 32 * 1024);
        assert_eq!(cart.header().chr_size, 8 * 1024);
        assert!(!cart.chr_is_ram());
    }

    #[test]
    fn nrom_maps_32k_prg() {
        let rom = synthetic(0, 2, 1);
        let cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 2);
        assert_eq!(cart.ppu_read(0x0000), 0x80);
    }

    #[test]
    fn nrom_mirrors_16k_prg() {
        let rom = synthetic(0, 1, 1);
        let cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 0);
    }

    #[test]
    fn chr_ram_is_writable() {
        let rom = synthetic(0, 1, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert!(cart.chr_is_ram());
        cart.ppu_write(0x0123, 0x5A);
        assert_eq!(cart.ppu_read(0x0123), 0x5A);
    }

    fn synth_banked(mapper: u16, prg_kib: usize, chr_kib: usize) -> Vec<u8> {
        let prg_len = prg_kib * 1024;
        let chr_len = chr_kib * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = (prg_len / (16 * 1024)) as u8;
        rom[5] = (chr_len / (8 * 1024)) as u8;
        rom[6] = ((mapper & 0x0F) << 4) as u8;
        rom[7] = (mapper & 0xF0) as u8;
        if mapper > 0xFF {
            rom[7] |= 0x08;
            rom[8] = ((mapper >> 8) & 0x0F) as u8;
        }
        for bank in 0..prg_len / 0x2000 {
            let start = 16 + bank * 0x2000;
            for byte in &mut rom[start..start + 0x2000] {
                *byte = bank as u8;
            }
        }
        for bank in 0..chr_len / 0x400 {
            let start = 16 + prg_len + bank * 0x400;
            for byte in &mut rom[start..start + 0x400] {
                *byte = 0x80 | (bank as u8 & 0x7F);
            }
        }
        rom
    }

    #[test]
    fn mmc2_maps_prg_and_chr_registers() {
        // 128 KiB PRG (16 x 8 KiB banks), 64 KiB CHR (16 x 4 KiB banks).
        let rom = synthetic(9, 8, 8);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 9);
        assert_eq!(cart.header().mirroring, Mirroring::Horizontal);

        // $8000 is switchable; the top 24 KiB is the last three 8 KiB banks.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xA000), 13);
        assert_eq!(cart.cpu_read(0xC000), 14);
        assert_eq!(cart.cpu_read(0xE000), 15);
        cart.cpu_write(0xA000, 3);
        assert_eq!(cart.cpu_read(0x8000), 3);

        // CHR latch registers: both tables default to their $FD bank.
        cart.cpu_write(0xB000, 2);
        cart.cpu_write(0xC000, 4);
        cart.cpu_write(0xD000, 6);
        cart.cpu_write(0xE000, 8);
        assert_eq!(cart.chr_banks(), Some([2, 4, 6, 8]));
        assert_eq!(cart.ppu_read(0x0000), 0x81);
        assert_eq!(cart.ppu_read(0x1000), 0x83);
        assert_eq!(cart.chr_rom().map(<[u8]>::len), Some(64 * 1024));

        cart.cpu_write(0xF000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0xF000, 0);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn mmc4_switches_16k_prg_banks() {
        let rom = synthetic(10, 8, 8);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 10);
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 14);

        cart.cpu_write(0xA000, 3);
        assert_eq!(cart.cpu_read(0x8000), 6);
        assert_eq!(cart.cpu_read(0xA000), 7);
        assert_eq!(cart.cpu_read(0xC000), 14);

        // CHR latch banks are shared with MMC2.
        cart.cpu_write(0xB000, 5);
        assert_eq!(cart.chr_banks(), Some([5, 0, 0, 0]));
        assert_eq!(cart.ppu_read(0x0000), 0x82);
    }

    #[test]
    fn txc_latch_mirrors_16k_prg() {
        // 16 KiB PRG / 16 KiB CHR image: the 32 KiB window must mirror instead
        // of indexing past the PRG buffer (fuzzing found this as a panic).
        let rom = synth_banked(173, 16, 16);
        let cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 173);
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xFFFC), 1);
    }

    #[test]
    fn mmc3_176_fs005_ram_config_banks_wram() {
        let rom = synth_banked(176, 128, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 176);

        // $A001 = $A1: config on, outer registers off, WRAM bank 2 selected.
        cart.cpu_write(0xA001, 0xA1);
        cart.cpu_write(0x5000, 0x5A); // lands in WRAM, not the mode register.
        assert_eq!(cart.cpu_read(0x5000), 0x5A);

        // $A001 = $E2: outer registers on, WRAM bank 2 at $6000-$7FFF.
        cart.cpu_write(0xA001, 0xE2);
        assert_eq!(cart.cpu_read(0x7000), 0x5A);

        // $5000 writes the mode register again; WRAM keeps its value.
        cart.cpu_write(0x5000, 0x03);
        cart.cpu_write(0xA001, 0xA1);
        assert_eq!(cart.cpu_read(0x5000), 0x5A);
    }

    #[test]
    fn bandai_fcg_159_reports_mapper_and_banks() {
        let rom = synth_banked(159, 64, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 159);
        cart.cpu_write(0x8008, 3);
        assert_eq!(cart.cpu_read(0x8000), 6);
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
    }

    #[test]
    fn hybrid_chr_165_latches_between_ram_and_rom() {
        let rom = synth_banked(165, 64, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 165);
        assert!(!cart.chr_is_ram());
        let hybrid = cart.hybrid_chr().expect("hybrid state");

        // CHR page 0 is RAM.
        cart.ppu_write(0x0010, 0x5A);
        assert_eq!(cart.ppu_read(0x0010), 0x5A);

        // R1 = page 4 (ROM); a $FE fetch switches latch 0 to it.
        cart.cpu_write(0x8000, 1);
        cart.cpu_write(0x8001, 4);
        hybrid.borrow_mut().note_fetch(0x0FE0);
        assert_eq!(cart.ppu_read(0x0000), 0x90);

        // A $FD `$2007` read switches back to R0 (= page 0, RAM).
        hybrid.borrow_mut().note_read(0x0FD0);
        assert_eq!(cart.ppu_read(0x0010), 0x5A);

        // R0 = page 1 (ROM) is visible while the latch is on $FD.
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 1);
        assert_eq!(cart.ppu_read(0x0000), 0x84);
    }

    #[test]
    fn hybrid_chr_195_maps_ram_and_chrb() {
        let rom = synth_banked(195, 64, 256);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 195);
        assert!(!cart.chr_is_ram());
        let hybrid = cart.hybrid_chr().expect("hybrid state");

        // Bank $28 is inside the power-on RAM window ($28-$2B).
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 0x28);
        cart.ppu_write(0x0000, 0x5A);
        assert_eq!(cart.ppu_read(0x0000), 0x5A);

        // Writing through bank $82 configures CHRB; the bank itself is ROM.
        cart.cpu_write(0x8001, 0x82);
        cart.ppu_write(0x0000, 0x11);
        assert_eq!(hybrid.borrow().chrb(), 0x82);
        assert_eq!(cart.ppu_read(0x0000), 0x82);

        // Bank 0 is now inside the $00-$03 RAM window.
        cart.cpu_write(0x8001, 0x00);
        cart.ppu_write(0x0200, 0x33);
        assert_eq!(cart.ppu_read(0x0200), 0x33);
    }

    #[test]
    fn waixing253_banks_prg_chr_mirroring_and_irq() {
        // 256 KiB PRG (32 x 8 KiB), 512 KiB CHR-ROM (512 x 1 KiB).
        let rom = synth_banked(253, 256, 512);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 253);

        // $8000/$A000 are switched, $C000/$E000 fixed to the last two banks.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xA000), 1);
        assert_eq!(cart.cpu_read(0xC000), 30);
        assert_eq!(cart.cpu_read(0xE000), 31);
        cart.cpu_write(0x8010, 5);
        cart.cpu_write(0xA010, 7);
        assert_eq!(cart.cpu_read(0x8000), 5);
        assert_eq!(cart.cpu_read(0xA000), 7);

        // CHR slot 1: low byte at $B800, high byte/nibble at $B804 (the high
        // write carries the page's upper byte in its high nibble and the low
        // byte's high nibble in its low nibble): page 0x0103.
        cart.cpu_write(0xB800, 0x03);
        cart.cpu_write(0xB804, 0x10);
        assert_eq!(cart.ppu_read(0x0400), 0x80 | (0x0103u16 & 0x7F) as u8);

        // Four mirroring modes.
        cart.cpu_write(0x9400, 0);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0x9400, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x9400, 2);
        assert_eq!(cart.mirroring(), Mirroring::SingleLower);
        cart.cpu_write(0x9400, 3);
        assert_eq!(cart.mirroring(), Mirroring::SingleUpper);

        // /114 IRQ prescaler: reload 0xFF, counter 0xFF wraps to 0 and fires.
        cart.cpu_write(0xF000, 0x0F); // low nibble
        cart.cpu_write(0xF004, 0x0F); // high nibble -> reload 0xFF
        cart.cpu_write(0xF008, 0x02);
        assert!(!cart.irq_line());
        for _ in 0..114 {
            cart.clock_cpu();
        }
        assert!(
            cart.irq_line(),
            "IRQ did not fire after one prescaler period"
        );
        cart.cpu_write(0xF008, 0x00);
        assert!(!cart.irq_line());
    }

    #[test]
    fn ctc222_banks_irq_and_mirroring() {
        // 128 KiB PRG (16 x 8 KiB), 128 KiB CHR (128 x 1 KiB).
        let rom = synth_banked(222, 128, 128);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 222);

        // Power-on: slots 0/1 are 0/1, the last 16 KiB is fixed.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xA000), 1);
        assert_eq!(cart.cpu_read(0xC000), 14);
        assert_eq!(cart.cpu_read(0xE000), 15);

        cart.cpu_write(0x8000, 5);
        cart.cpu_write(0xA000, 6);
        assert_eq!(cart.cpu_read(0x8000), 5);
        assert_eq!(cart.cpu_read(0xA000), 6);

        // CHR 1 KiB slots: $B000/$B002 select slots 0/1.
        cart.cpu_write(0xB000, 4);
        cart.cpu_write(0xB002, 5);
        assert_eq!(cart.ppu_read(0x0000), 0x80 | 4);
        assert_eq!(cart.ppu_read(0x0400), 0x80 | 5);

        // Mirroring.
        cart.cpu_write(0x9000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x9000, 0);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);

        // A12-rise IRQ: reload 1, fire on the 239th rise.
        cart.cpu_write(0xF000, 1);
        for _ in 0..238 {
            cart.clock_scanline();
        }
        assert!(!cart.irq_line());
        cart.clock_scanline();
        assert!(cart.irq_line(), "IRQ did not fire after 239 A12 rises");
        cart.cpu_write(0xF000, 0);
        assert!(!cart.irq_line());
    }

    #[test]
    fn waixing164_prg_nibbles_and_chr_ram() {
        // 1 MiB PRG (32 x 32 KiB), 8 KiB CHR-RAM.
        let rom = synth_banked(164, 1024, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 164);
        assert!(cart.chr_is_ram());

        // Power-on selects the last 32 KiB bank: 8 KiB bank 60.
        assert_eq!(cart.cpu_read(0x8000), 60);

        // $5000 sets the low nibble, $5100 the high nibble.
        cart.cpu_write(0x5000, 0x03);
        assert_eq!(cart.cpu_read(0x8000), 12); // 32 KiB bank 3 -> 8 KiB 12
        cart.cpu_write(0x5100, 0x01);
        assert_eq!(cart.cpu_read(0x8000), 76); // 32 KiB bank 19 -> 8 KiB 76

        // CHR-RAM round-trips.
        cart.ppu_write(0x0123, 0x5A);
        assert_eq!(cart.ppu_read(0x0123), 0x5A);
    }

    #[test]
    fn maxi15_234_latches_prg_chr_mirroring_and_bus_conflicts() {
        // 512 KiB PRG (16 x 32 KiB), 512 KiB CHR (64 x 8 KiB).
        let mut rom = synth_banked(234, 512, 512);
        // Make both latch addresses read $FF in every 32 KiB bank so bus
        // conflicts allow any written value.
        for bank8k in 0..(512 * 1024 / 0x2000) {
            rom[16 + bank8k * 0x2000 + 0x1F80] = 0xFF;
            rom[16 + bank8k * 0x2000 + 0x1FE8] = 0xFF;
        }
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 234);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);

        // $FF80 bit 6 selects NINA-03 mode; $FFE8 supplies the low bank bits.
        cart.cpu_write(0xFF80, 0x40);
        cart.cpu_write(0xFFE8, 0x11);
        // PRG page 1 -> 32 KiB bank 1 (8 KiB bank 4 in the synth image).
        assert_eq!(cart.cpu_read(0x8000), 4);
        // CHR page 1 -> 8 KiB bank 1, whose first 1 KiB bank is 8.
        assert_eq!(cart.ppu_read(0x0000), 0x80 | 8);

        // $FF80 bit 7 selects CNROM mode + horizontal mirroring.
        cart.cpu_write(0xFF80, 0x80);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        assert_eq!(cart.cpu_read(0x8000), 0);

        // Bus conflict: a write is ANDed with the ROM byte at the address.
        let mut rom2 = synth_banked(234, 512, 512);
        // Bus conflict with ROM byte $0E latches CNROM PRG page 14.
        for bank8k in 0..(512 * 1024 / 0x2000) {
            rom2[16 + bank8k * 0x2000 + 0x1F80] = 0x0E;
        }
        let mut other = Cartridge::from_ines(&rom2).expect("cart");
        other.cpu_write(0xFF80, 0xFF);
        assert_eq!(other.cpu_read(0x8000), 56); // 32 KiB bank 14 -> 8 KiB 56
    }

    #[test]
    fn mmc3_249_5000_register_scrambles_prg_and_chr_pages() {
        // 256 KiB PRG (32 x 8 KiB), 256 KiB CHR-ROM (256 x 1 KiB).
        let rom = synth_banked(249, 256, 256);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 249);

        // R6 = 2 -> $8000 sees bank 2 while $5000 bit 1 is clear.
        cart.cpu_write(0x8000, 0x06);
        cart.cpu_write(0x8001, 0x02);
        assert_eq!(cart.cpu_read(0x8000), 2);

        // With $5000 bit 1 set the 5-bit PRG page is permuted: 2 -> 8.
        cart.cpu_write(0x5000, 0x02);
        assert_eq!(cart.cpu_read(0x8000), 8);

        // CHR page 4 permutes to 32 ((page << 3) & 0x20).
        cart.cpu_write(0x8000, 0x00);
        cart.cpu_write(0x8001, 0x04);
        assert_eq!(cart.ppu_read(0x0000), 0x80 | 32);

        // Clearing bit 1 restores the direct page.
        cart.cpu_write(0x5000, 0x00);
        assert_eq!(cart.ppu_read(0x0000), 0x80 | 4);
        assert_eq!(cart.cpu_read(0x8000), 2);
    }

    #[test]
    fn mmc3_199_extra_registers_and_fixed_banks() {
        // 512 KiB PRG (64 x 8 KiB banks), 8 KiB CHR-RAM.
        let rom = synth_banked(199, 512, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 199);
        assert!(cart.chr_is_ram());

        // Slot 0/1 follow the MMC3 registers; slots 2/3 are the extra regs
        // (power-on 0xFE/0xFF -> last two 8 KiB banks of a 64-bank image).
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xA000), 1);
        assert_eq!(cart.cpu_read(0xC000), 62);
        assert_eq!(cart.cpu_read(0xE000), 63);

        // $8000 select bit 3 routes $8001 to the extra registers.
        cart.cpu_write(0x8000, 0x08);
        cart.cpu_write(0x8001, 0x20);
        assert_eq!(cart.cpu_read(0xC000), 0x20);
        cart.cpu_write(0x8000, 0x09);
        cart.cpu_write(0x8001, 0x21);
        assert_eq!(cart.cpu_read(0xE000), 0x21);

        // CHR slots 1/3 come from the extra registers; slot 1 defaults to 1.
        cart.ppu_write(0x0400, 0x5A);
        assert_eq!(cart.ppu_read(0x0400), 0x5A);
        cart.cpu_write(0x8000, 0x0A); // extra register 2 -> CHR slot 1
        cart.cpu_write(0x8001, 0x02);
        assert_eq!(cart.ppu_read(0x0400), 0, "slot 1 moved to the empty bank 2");

        // Four mirroring modes from $A000.
        cart.cpu_write(0xA000, 0);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0xA000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0xA000, 2);
        assert_eq!(cart.mirroring(), Mirroring::SingleLower);
        cart.cpu_write(0xA000, 3);
        assert_eq!(cart.mirroring(), Mirroring::SingleUpper);

        // Work RAM.
        cart.cpu_write(0x6000, 0x5A);
        assert_eq!(cart.cpu_read(0x6000), 0x5A);

        // FS309 low window: 4 KiB PRG-RAM at $5000-$5FFF, which the games use
        // for a copied NMI handler.
        assert_eq!(cart.low_window(), Some((0x5000, 0x5FFF)));
        cart.cpu_write(0x5C42, 0x5A);
        assert_eq!(cart.cpu_read(0x5C42), 0x5A);
        assert_eq!(cart.cpu_read(0x5FFF), 0);
    }

    #[test]
    fn daou156_banks_prg_chr_mirroring_and_ram() {
        let rom = synth_banked(156, 64, 8);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 156);

        // $8000 is the switchable 16 KiB bank, $C000 the fixed last bank.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 6); // 16 KiB bank 3 -> 8 KiB bank 6
        cart.cpu_write(0xC010, 2);
        assert_eq!(cart.cpu_read(0x8000), 4);

        // CHR slot 0: low nibble at $C000, high nibble at $C004.
        cart.cpu_write(0xC000, 0x03);
        cart.cpu_write(0xC004, 0x01); // bank 0x0103, wrapped to 3 of 8
        assert_eq!(cart.ppu_read(0x0000), 0x80 | 3);

        // $6000-$7FFF work RAM.
        cart.cpu_write(0x6000, 0x5A);
        assert_eq!(cart.cpu_read(0x6000), 0x5A);

        // Power-on single-screen, then $C014 selects H/V.
        assert_eq!(cart.mirroring(), Mirroring::SingleLower);
        cart.cpu_write(0xC014, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0xC014, 0);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn mmc5_expansion_audio_pulses_and_length() {
        let rom = synth_banked(5, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");

        // Pulse 1: 50% duty, length halt, constant volume 15. A full-volume
        // pulse swings the mixer by 15 * 650 / 65536, matching a 2A03 pulse.
        cart.cpu_write(0x5000, 0xBF);
        cart.cpu_write(0x5002, 0x00);
        cart.cpu_write(0x5003, 0x00); // length index 0 -> 10
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..64 {
            cart.clock_cpu();
            let value = cart.audio_mix();
            lo = lo.min(value);
            hi = hi.max(value);
        }
        let pulse_swing = 15.0 * 650.0 / 65_536.0;
        assert!(
            (hi - lo - pulse_swing).abs() < 1e-4,
            "pulse swing {hi} - {lo}"
        );

        // Pulse 2 adds to the same DAC sum (both channels share the duty
        // phase, so the swing doubles).
        cart.cpu_write(0x5004, 0xBF);
        cart.cpu_write(0x5006, 0x00);
        cart.cpu_write(0x5007, 0x00);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..64 {
            cart.clock_cpu();
            let value = cart.audio_mix();
            lo = lo.min(value);
            hi = hi.max(value);
        }
        assert!(
            (hi - lo - 2.0 * pulse_swing).abs() < 1e-4,
            "two-pulse swing {hi} - {lo}"
        );

        // Clearing the halt bit lets the length counter expire (~240 Hz);
        // both pulses then rest at the centred bias.
        cart.cpu_write(0x5000, 0x8F);
        cart.cpu_write(0x5003, 0x00);
        cart.cpu_write(0x5004, 0x8F);
        cart.cpu_write(0x5007, 0x00);
        for _ in 0..(7424 * 11) {
            cart.clock_cpu();
        }
        let bias = -12_290.0 / 65_536.0;
        assert!(
            (cart.audio_mix() - bias).abs() < 1e-6,
            "expired {}",
            cart.audio_mix()
        );
    }

    #[test]
    fn namco163_wavetable_audio_updates_and_disables() {
        let rom = synth_banked(19, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 19);

        // Waveform at sound-RAM 0: sixteen samples of $F (8 bytes x 2).
        cart.cpu_write(0xF800, 0x80); // address 0, auto-increment
        for _ in 0..8 {
            cart.cpu_write(0x4800, 0xFF);
        }
        // Channel 8: frequency $001000, phase 0, length 256, wave address 0,
        // volume 15, only channel 8 enabled (C=0 in the high nibble).
        cart.cpu_write(0xF800, 0xF8); // address $78, auto-increment
        for value in [0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x0F] {
            cart.cpu_write(0x4800, value);
        }

        // One channel is updated every 15 CPU cycles; a sample of $F minus
        // the -8 bias is 7, times volume 15.
        cart.cpu_write(0xF800, 0x00);
        assert_eq!(cart.cpu_read(0x4800), 0xFF, "wave sample");
        cart.cpu_write(0xF800, 0x7F);
        assert_eq!(cart.cpu_read(0x4800), 0x0F, "volume register");
        for _ in 0..15 {
            cart.clock_cpu();
        }
        let sample = cart.audio_mix();
        let expected = 105.0 * 261.0 / 65_536.0;
        assert!((sample - expected).abs() < 1e-4, "sample {sample}");

        // $E000 bit 6 disables sound.
        cart.cpu_write(0xE000, 0x40);
        for _ in 0..15 {
            cart.clock_cpu();
        }
        assert_eq!(cart.audio_mix(), 0.0);
    }

    #[test]
    fn sunsoft5b_audio_tone_noise_and_mixer() {
        let rom = synth_banked(69, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.audio_mix(), 0.0);

        let write = |cart: &mut Cartridge, register: u8, value: u8| {
            cart.cpu_write(0xC000, register);
            cart.cpu_write(0xE000, value);
        };

        // Mixer: all tones enabled, noise disabled. Tone A/B period 1, volume
        // 15; channel C silent.
        write(&mut cart, 0x07, 0x38);
        write(&mut cart, 0x00, 0x01);
        write(&mut cart, 0x01, 0x00);
        write(&mut cart, 0x02, 0x01);
        write(&mut cart, 0x03, 0x00);
        write(&mut cart, 0x08, 0x0F);
        write(&mut cart, 0x09, 0x0F);
        write(&mut cart, 0x0A, 0x00);
        let mut low = 0.0f32;
        let mut high = 0.0f32;
        for _ in 0..512 {
            cart.clock_cpu();
            let sample = cart.audio_mix();
            low = low.min(sample);
            high = high.max(sample);
        }
        assert_eq!(low, 0.0);
        let two_tone = (2 * SUNSOFT5B_LOG_VOL[15] * 2549 / 138) as f32 / 65_536.0;
        assert!((high - two_tone).abs() < 1e-4, "tone peak {high}");

        // Disabling both tone and noise holds the channel at its volume.
        write(&mut cart, 0x07, 0x3F);
        for _ in 0..64 {
            cart.clock_cpu();
        }
        assert!((cart.audio_mix() - two_tone).abs() < 1e-4);

        // Volume 0 is silent even with a constant signal.
        write(&mut cart, 0x08, 0x00);
        write(&mut cart, 0x09, 0x00);
        assert_eq!(cart.audio_mix(), 0.0);

        // Noise-only output must produce both LFSR states.
        write(&mut cart, 0x07, 0x07); // tone off on all channels, noise on A
        write(&mut cart, 0x08, 0x0F);
        write(&mut cart, 0x06, 0x01);
        let one_tone = (SUNSOFT5B_LOG_VOL[15] * 2549 / 138) as f32 / 65_536.0;
        let mut saw_low = false;
        let mut saw_high = false;
        for _ in 0..2048 {
            cart.clock_cpu();
            let sample = cart.audio_mix();
            saw_low |= sample == 0.0;
            saw_high |= (sample - one_tone).abs() < 1e-4;
        }
        assert!(saw_low && saw_high, "noise did not toggle");
    }

    #[test]
    fn fds_write_regenerates_block_crc_and_persists_to_the_image() {
        let mut side = vec![0u8; 65_500];
        side[0] = 0x01;
        side[1..15].copy_from_slice(b"*NINTENDO-HVC*");
        for (i, byte) in side.iter_mut().enumerate().take(56).skip(15) {
            *byte = 0x10 + i as u8;
        }
        side[56] = 0x02;
        side[57] = 0x00;
        let mut fds = Fds::new(&side, &vec![0xEA; 0x2000]);
        assert_eq!(fds.wire_map.len(), 2);

        // Write into the first block's payload through the write-mode path.
        let (payload, raw_offset, len) = fds.wire_map[0];
        assert_eq!(raw_offset, 0);
        assert_eq!(len, 56);
        // Offset 1 keeps the block-type byte ($01) intact so the saved side
        // still parses; the licensee string byte is a realistic write target.
        fds.head = payload + 1;
        fds.write_data = 0x99;
        fds.store_byte();
        assert_eq!(fds.wire[payload + 1], 0x99);

        // The block CRC must now cover the modified payload.
        let expected = fds_block_crc(&fds.wire[payload..payload + 56]);
        assert_eq!(fds.wire[payload + 56], expected as u8);
        assert_eq!(fds.wire[payload + 57], (expected >> 8) as u8);

        // Saving flushes the payload back into the raw side.
        let image = fds.disk_image();
        assert_eq!(&image[0..4], b"FDS\x1a");
        assert_eq!(image[4], 1);
        assert_eq!(image[16], 0x01, "block type intact");
        assert_eq!(image[17], 0x99, "written byte persisted in the raw side");

        // Reloading the saved image reproduces the payload and its CRC.
        let reloaded = Fds::new(&image[16..], &vec![0xEA; 0x2000]);
        let (p2, _, _) = reloaded.wire_map[0];
        assert_eq!(reloaded.wire[p2], 0x01);
        assert_eq!(reloaded.wire[p2 + 1], 0x99);
        let crc2 = fds_block_crc(&reloaded.wire[p2..p2 + 56]);
        assert_eq!(reloaded.wire[p2 + 56], crc2 as u8);
        assert_eq!(reloaded.wire[p2 + 57], (crc2 >> 8) as u8);
    }

    #[test]
    fn fds_write_mode_stores_bytes_in_the_wire_image() {
        let mut side = vec![0u8; 65_500];
        side[0] = 0x01;
        side[1..15].copy_from_slice(b"*NINTENDO-HVC*");
        side[56] = 0x02;
        let mut fds = Fds::new(&side, &vec![0xEA; 0x2000]);

        fds.prg_write(0x4023, 0x83); // disk + sound I/O enabled
        fds.prg_write(0x4025, 0x60); // write mode, motor on, CRC control+enable
        assert_eq!(fds.drive_status_4032() & 0x04, 0, "drive is writable");
        while fds.drive_status_4032() & 0x02 != 0 {
            fds.clock_cpu();
        }

        let head0 = fds.head;
        fds.prg_write(0x4024, 0x5A);
        let mut guard = 0;
        while fds.head == head0 && guard < 1_000 {
            fds.clock_cpu();
            guard += 1;
        }
        assert!(guard < 1_000, "write transfer never advanced");
        assert_eq!(fds.wire[head0], 0x5A);

        let head1 = fds.head;
        fds.prg_write(0x4024, 0xA5);
        guard = 0;
        while fds.head == head1 && guard < 1_000 {
            fds.clock_cpu();
            guard += 1;
        }
        assert_eq!(fds.wire[head1], 0xA5);
        assert_eq!(fds.status_4030() & 0x80, 0x80, "byte-transfer flag");
    }

    #[test]
    fn fds_multi_side_insertion_switches_the_disk_image() {
        // fwNES header + two 65,500-byte sides. Each side's disk-info block
        // carries a distinct licensee byte at offset $0F.
        let mut image = vec![0u8; 16 + 2 * 65_500];
        image[0..4].copy_from_slice(b"FDS\x1a");
        image[4] = 2;
        for (side, licensee) in [(0usize, 0x55u8), (1, 0xAA)] {
            let base = 16 + side * 65_500;
            let block = &mut image[base..base + 65_500];
            block[0] = 0x01;
            block[1..15].copy_from_slice(b"*NINTENDO-HVC*");
            block[15] = licensee;
            for (i, byte) in block.iter_mut().enumerate().take(56).skip(16) {
                *byte = 0xA0 | (i as u8 & 0x1F);
            }
            block[56] = 0x02;
            block[57] = 0x00;
        }
        let mut cart = Cartridge::from_fds(&image, &vec![0xEA; 0x2000]).expect("fds");
        assert_eq!(cart.disk_side_count(), 2);
        assert_eq!(cart.current_disk_side(), 0);

        fn read_info_block(cart: &mut Cartridge) -> Vec<u8> {
            cart.cpu_write(0x4023, 0x83);
            cart.cpu_write(0x4025, 0x64); // read mode, motor on, CRC enabled
            while cart.cpu_read_io(0x4032) & 0x02 != 0 {
                cart.clock_cpu();
            }
            for _ in 0..FDS_BYTE_CYCLES {
                cart.clock_cpu();
            }
            let mut block = Vec::with_capacity(56);
            for _ in 0..56 {
                block.push(cart.cpu_read_io(0x4031));
                for _ in 0..FDS_BYTE_CYCLES {
                    cart.clock_cpu();
                }
            }
            block
        }

        let side_a = read_info_block(&mut cart);
        assert_eq!(side_a[0], 0x01);
        assert_eq!(side_a[15], 0x55);

        cart.insert_disk_side(Some(1));
        assert_eq!(cart.current_disk_side(), 1);
        // The motor restarts from the beginning of the newly inserted side.
        cart.cpu_write(0x4025, 0x66); // stop the motor first
        let side_b = read_info_block(&mut cart);
        assert_eq!(side_b[0], 0x01);
        assert_eq!(side_b[15], 0xAA);

        // Ejecting stops the drive and reports not-ready.
        cart.insert_disk_side(None);
        assert_eq!(cart.cpu_read_io(0x4032) & 0x02, 0x02);
    }

    #[test]
    fn fds_disk_reads_a_block_and_resyncs_after_the_gap() {
        // Synthetic side: disk-info block (56 bytes), file-amount block (2),
        // then a file header (16) and one byte of file data.
        let mut side = vec![0u8; 65500];
        side[0] = 0x01;
        side[1..15].copy_from_slice(b"*NINTENDO-HVC*");
        for (i, byte) in side.iter_mut().enumerate().take(56).skip(15) {
            *byte = 0xA0 | (i as u8 & 0x1F);
        }
        side[56] = 0x02;
        side[57] = 0x02;
        side[58] = 0x03;
        side[71] = 1; // file size low
        side[74] = 0x04;
        side[75] = 0x5A;

        let mut cart = Cartridge::from_fds(&side, &vec![0xEA; 0x2000]).expect("fds");

        // Enable disk I/O and sound, then start the motor in read mode with
        // CRC enabled ($4025 = 0110_0100: bit6 CRC enable, bit5 CRC control,
        // bit2 read, bit1 motor on, bit0 no reset).
        cart.cpu_write(0x4023, 0x83);
        assert_eq!(
            cart.cpu_read_io(0x4032) & 0x02,
            0x02,
            "not ready at power-on"
        );
        cart.cpu_write(0x4025, 0x64);
        while cart.cpu_read_io(0x4032) & 0x02 != 0 {
            cart.clock_cpu();
        }
        // The first byte lands one cadence after the drive reports ready; the
        // controller has skipped the lead-in gap and $80 start mark, so it is
        // the block code.
        for _ in 0..FDS_BYTE_CYCLES {
            cart.clock_cpu();
        }
        assert_eq!(cart.cpu_read_io(0x4030) & 0x80, 0x80);
        assert_eq!(cart.cpu_read_io(0x4031), 0x01);

        // Read the rest of the info block, one cadence per byte.
        for (i, expected) in side.iter().enumerate().take(56).skip(1) {
            for _ in 0..FDS_BYTE_CYCLES {
                cart.clock_cpu();
            }
            assert_eq!(cart.cpu_read_io(0x4030) & 0x80, 0x80);
            assert_eq!(cart.cpu_read_io(0x4031), *expected, "info block byte {i}");
        }

        // Let the head run past the CRC into the inter-block gap, then assert
        // transfer reset so the next delivery re-syncs to block 2.
        for _ in 0..FDS_BYTE_CYCLES * 4 {
            cart.clock_cpu();
        }
        cart.cpu_write(0x4025, 0x65); // same, plus transfer reset
        for _ in 0..FDS_BYTE_CYCLES {
            cart.clock_cpu();
        }
        assert_eq!(cart.cpu_read_io(0x4031), 0x02, "file-amount block code");

        // Stop the motor: the drive rewinds to the start of the side, so a
        // restart re-reads block 1.
        cart.cpu_write(0x4025, 0x66);
        assert_eq!(
            cart.cpu_read_io(0x4032) & 0x02,
            0x02,
            "stopped -> not ready"
        );
        cart.cpu_write(0x4025, 0x64);
        while cart.cpu_read_io(0x4032) & 0x02 != 0 {
            cart.clock_cpu();
        }
        for _ in 0..FDS_BYTE_CYCLES {
            cart.clock_cpu();
        }
        assert_eq!(cart.cpu_read_io(0x4031), 0x01, "rewound to block 1");
    }

    #[test]
    fn fds_timer_irq_reloads_and_clears_on_status_read() {
        let mut cart = Cartridge::from_fds(&vec![0u8; 65500], &vec![0xEA; 0x2000]).expect("fds");
        cart.cpu_write(0x4023, 0x83);
        cart.cpu_write(0x4020, 0x03);
        cart.cpu_write(0x4021, 0x00);
        cart.cpu_write(0x4022, 0x03); // enable + repeat
        for _ in 0..4 {
            cart.clock_cpu();
        }
        assert!(cart.irq_line(), "timer did not fire");
        assert_eq!(cart.cpu_read_io(0x4030) & 0x01, 0x01);
        assert!(!cart.irq_line(), "reading $4030 acknowledges the timer");
        // Repeat keeps it firing.
        for _ in 0..4 {
            cart.clock_cpu();
        }
        assert!(cart.irq_line());
        // Disabling clears it.
        cart.cpu_write(0x4022, 0x00);
        assert!(!cart.irq_line());
    }

    #[test]
    fn fds_wavetable_volume_envelope_and_disable() {
        let mut audio = FdsAudio::default();
        audio.write(0x4023, 0x02); // enable sound registers
        audio.write(0x4089, 0x83); // master volume 3, wave RAM write enable
        for i in 0..64u16 {
            audio.write(0x4040 + i, if i < 32 { 63 } else { 0 });
        }
        audio.write(0x4089, 0x03); // master volume 3, writes disabled
        audio.write(0x4080, 0xBF); // envelope off, gain/speed 63
        audio.write(0x4082, 0x40);
        audio.write(0x4083, 0x00);
        audio.write(0x408A, 0x3F);

        let mut peak = 0.0f32;
        let mut trough = 0.0f32;
        for _ in 0..5000 {
            audio.clock();
            peak = peak.max(audio.mix());
            trough = trough.min(audio.mix());
        }
        assert!(peak > 0.1, "positive wave half missing: {peak}");
        assert!(trough < -0.1, "negative wave half missing: {trough}");

        // Re-arm a 32-step decreasing envelope; after ~16k cycles it is silent.
        audio.write(0x4080, 0xA0);
        audio.write(0x4080, 0x3F);
        for _ in 0..20_000 {
            audio.clock();
        }
        assert!(audio.mix().abs() < 0.01, "envelope did not decay");

        // Clearing $4023 bit 1 resets the output.
        audio.write(0x4023, 0x00);
        audio.clock();
        assert_eq!(audio.mix(), 0.0);
    }

    #[test]
    fn vrc7_built_in_patches_match_the_documented_rom() {
        // Dumped from the VRC7 instrument ROM (siliconpr0n).
        assert_eq!(
            VRC7_PATCHES[1],
            [0x03, 0x21, 0x05, 0x06, 0xE8, 0x81, 0x42, 0x27]
        );
        assert_eq!(
            VRC7_PATCHES[2],
            [0x13, 0x41, 0x14, 0x0D, 0xD8, 0xF6, 0x23, 0x12]
        );
        assert_eq!(
            VRC7_PATCHES[9],
            [0x35, 0x11, 0x25, 0x00, 0x40, 0x73, 0x72, 0x01]
        );
        assert_eq!(
            VRC7_PATCHES[15],
            [0x21, 0x72, 0x0D, 0x00, 0xC1, 0xD5, 0x56, 0x06]
        );
        // Instrument 0 is the user patch slot, not a built-in.
        assert_eq!(VRC7_PATCHES[0], [0; 8]);
    }

    #[test]
    fn vrc7_waveform_bits_are_bit3_modulator_bit4_carrier() {
        let mut patch = [0u8; 8];
        patch[3] = 0x08;
        assert_eq!(vrc7_waveforms(&patch), (true, false));
        patch[3] = 0x10;
        assert_eq!(vrc7_waveforms(&patch), (false, true));
        patch[3] = 0x18;
        assert_eq!(vrc7_waveforms(&patch), (true, true));
        // Feedback (bits 0-2) does not select a waveform.
        patch[3] = 0x07;
        assert_eq!(vrc7_waveforms(&patch), (false, false));
    }

    #[test]
    fn vrc7_banks_prg_chr_irq_and_wram() {
        let rom = synth_banked(85, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 85);
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xE000), 15);

        cart.cpu_write(0x8000, 2);
        cart.cpu_write(0x8010, 3);
        cart.cpu_write(0x9000, 4);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xA000), 3);
        assert_eq!(cart.cpu_read(0xC000), 4);

        cart.cpu_write(0xA010, 5);
        cart.cpu_write(0xD008, 7);
        assert_eq!(cart.ppu_read(0x0400), 0x85);
        assert_eq!(cart.ppu_read(0x1C00), 0x87);

        // WRAM is protected until $E000 bit 6.
        cart.cpu_write(0x6000, 0x11);
        assert_eq!(cart.cpu_read(0x6000), 0);
        cart.cpu_write(0xE000, 0x80);
        cart.cpu_write(0x6000, 0x22);
        assert_eq!(cart.cpu_read(0x6000), 0x22);
        cart.cpu_write(0xE000, 0x81);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);

        // IRQ: latch, enable, then 0x100 increments.
        cart.cpu_write(0xF000, 0x06);
        cart.cpu_write(0xF008, 0x00);
        for _ in 0..255 {
            cart.clock_cpu();
        }
        assert!(!cart.irq_line());
        cart.clock_cpu();
        assert!(cart.irq_line());
        cart.cpu_write(0xF010, 0);
        assert!(!cart.irq_line());
    }

    #[test]
    fn vrc7_opll_key_on_release_and_silence() {
        let rom = synth_banked(85, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        let write = |cart: &mut Cartridge, register: u8, value: u8| {
            cart.cpu_write(0x9010, register);
            cart.cpu_write(0x9030, value);
        };

        // Custom patch: sustain on both operators, fast attack, release 15.
        for (register, value) in [
            (0x00, 0x21),
            (0x01, 0x21),
            (0x02, 0x00),
            (0x03, 0x00),
            (0x04, 0xF0),
            (0x05, 0xF0),
            (0x06, 0x0F),
            (0x07, 0x0F),
        ] {
            write(&mut cart, register, value);
        }
        // Channel 0: frequency $080 at octave 4, custom instrument, volume 15.
        write(&mut cart, 0x10, 0x80);
        write(&mut cart, 0x20, 0x18); // trigger, octave 4
        write(&mut cart, 0x30, 0x0F);

        let mut peak = 0.0f32;
        for _ in 0..(36 * 6 * 400) {
            cart.clock_cpu();
            peak = peak.max(cart.audio_mix().abs());
        }
        assert!(peak > 0.01, "VRC7 produced no FM output: {peak}");

        // Key-off starts the release; after 500 channel updates it is silent.
        write(&mut cart, 0x20, 0x08);
        for _ in 0..(36 * 6 * 600) {
            cart.clock_cpu();
        }
        assert_eq!(cart.audio_mix(), 0.0);

        // $E000 bit 5 silences and clears the audio registers.
        cart.cpu_write(0xE000, 0x40);
        assert_eq!(cart.audio_mix(), 0.0);
        write(&mut cart, 0x10, 0x01);
        for _ in 0..1000 {
            cart.clock_cpu();
        }
        assert_eq!(cart.audio_mix(), 0.0);
    }

    #[test]
    fn vrc6_expansion_audio_pulses_and_saw() {
        let rom = synth_banked(24, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Centred linear sum: value = (p1 + p2 + saw - 30) * 650 / 65536.
        let scale = 650.0 / 65_536.0;
        assert!((cart.audio_mix() - (-30.0 * scale)).abs() < 1e-4);

        // Pulse 1: volume 15, duty 0, period 0, enabled. The 16-step duty
        // generator only outputs on step 0, so the swing is 15 units.
        cart.cpu_write(0x9000, 0x0F);
        cart.cpu_write(0x9001, 0x00);
        cart.cpu_write(0x9002, 0x80);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..64 {
            cart.clock_cpu();
            let value = cart.audio_mix();
            lo = lo.min(value);
            hi = hi.max(value);
        }
        assert!(
            (hi - lo - 15.0 * scale).abs() < 1e-4,
            "pulse swing {hi} - {lo}"
        );

        // The mode bit ignores the duty generator and holds full volume.
        cart.cpu_write(0x9000, 0x8F);
        assert!((cart.audio_mix() - (15.0 - 30.0) * scale).abs() < 1e-4);

        // Pulse 2 is independent and adds to the DAC sum.
        cart.cpu_write(0xA000, 0x8F);
        cart.cpu_write(0xA002, 0x80);
        assert!((cart.audio_mix() - (30.0 - 30.0) * scale).abs() < 1e-4);

        // Sawtooth: rate 42 gives six additions of 42 = 252, so the high five
        // bits reach 31 before the 14-step reset.
        let mut saw = Cartridge::from_ines(&rom).expect("saw cart");
        saw.cpu_write(0xB000, 42);
        saw.cpu_write(0xB001, 0x00);
        saw.cpu_write(0xB002, 0x80);
        let mut saw_max = f32::MIN;
        for _ in 0..64 {
            saw.clock_cpu();
            saw_max = saw_max.max(saw.audio_mix());
        }
        assert!(
            (saw_max - (31.0 - 30.0) * scale).abs() < 1e-4,
            "saw peak {saw_max}"
        );

        // Disabling the channel forces the output back to the centred bias.
        saw.cpu_write(0xB002, 0x00);
        assert!((saw.audio_mix() - (-30.0 * scale)).abs() < 1e-4);
    }

    #[test]
    fn mmc5_split_screen_register_and_addressing() {
        let rom = synth_banked(5, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        let aux = cart.mmc5_aux().expect("aux");

        // Left split, 8-tile threshold, scroll 3, 4 KiB bank 2.
        cart.cpu_write(0x5104, 0);
        cart.cpu_write(0x5200, 0x80 | 0x08);
        cart.cpu_write(0x5201, 3);
        cart.cpu_write(0x5202, 2);
        {
            let mut aux = aux.borrow_mut();
            // Row 0 starts at ExRAM 0.
            aux.exram[0] = 0x10;
            aux.exram[9] = 0x11;
            aux.exram[0x3C0] = 0x1B;
        }
        {
            let aux = aux.borrow();
            assert_eq!(aux.split_tile(0, 0), Some(0x10));
            assert_eq!(aux.split_tile(9, 0), None);
            // Fine Y = (scroll + y) & 7 = 3; bank 2; tile 0 -> $2003.
            assert_eq!(aux.split_pattern(0, 0, 0x10, false), Some(0x2103));
            assert_eq!(aux.split_pattern(0, 0, 0x10, true), Some(0x210B));
            // Attribute field for row 0/column 0 is bits 0-1 of the byte.
            assert_eq!(aux.split_attr_field(0, 0), Some(0x03));
        }

        // Right split is right-aligned: group 0 wraps to column 30.
        cart.cpu_write(0x5200, 0x80 | 0x40 | 0x02);
        {
            let aux = aux.borrow();
            assert_eq!(aux.split_tile(0, 0), None);
            assert_eq!(aux.split_tile(1, 0), None);
            // The split region is right-aligned over the 34 fetch groups:
            // group 2 is column 0 and group 33 is the final column 31.
            assert_eq!(aux.split_tile(2, 0), Some(aux.exram[0]));
            assert_eq!(aux.split_tile(33, 0), Some(aux.exram[31]));
        }

        // ExRAM modes 2/3 disable split.
        cart.cpu_write(0x5104, 2);
        {
            let aux = aux.borrow();
            assert_eq!(aux.split_tile(2, 0), None);
        }
        // Vertical scroll selects the row and fine Y (40 + 0 -> row 5).
        cart.cpu_write(0x5104, 0);
        cart.cpu_write(0x5200, 0x80 | 0x08);
        cart.cpu_write(0x5201, 40);
        {
            let mut aux = aux.borrow_mut();
            aux.exram[5 * 32] = 0x22;
            assert_eq!(aux.split_tile(0, 0), Some(0x22));
            assert_eq!(aux.split_pattern(0, 0, 0, false), Some(0x2000));
        }
    }

    #[test]
    fn mmc5_pcm_read_mode_irq() {
        let rom = synth_banked(5, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");

        // $5010 = read mode, IRQ enabled; bit 0 reads back as MMC5A revision 1.
        cart.cpu_write(0x5010, 0x81);
        // PRG mode 3: $5114 selects the 8 KiB bank at $8000-$9FFF. The synth
        // image fills each bank with its index.
        cart.cpu_write(0x5100, 3);
        cart.cpu_write(0x5114, 0x81); // bit 7 = ROM, bank 1
        assert_eq!(cart.cpu_read_io(0x8000), 1);
        assert!(!cart.irq_line(), "nonzero read must not trip");

        // Bank 0 is all zeros: a read trips the PCM IRQ.
        cart.cpu_write(0x5114, 0x80); // bit 7 = ROM, bank 0 (all zeros)
        assert_eq!(cart.cpu_read_io(0x8000), 0);
        assert!(cart.irq_line(), "zero read should trip the PCM IRQ");
        let status = cart.cpu_read_io(0x5010);
        assert_eq!(status & 0x80, 0x80, "IRQ flag latched");
        assert_eq!(status & 0x01, 0x01, "MMC5A revision bit");
        assert!(!cart.irq_line(), "reading $5010 acknowledges the IRQ");

        // Write mode: $5011 feeds the DAC and a zero write trips the IRQ.
        cart.cpu_write(0x5010, 0x80);
        cart.cpu_write(0x5011, 0x40);
        assert!(!cart.irq_line());
        cart.cpu_write(0x5011, 0x00);
        assert!(cart.irq_line(), "zero DAC write should trip the IRQ");
        assert_eq!(cart.cpu_read_io(0x5010) & 0x80, 0x80);
        assert!(!cart.irq_line());

        // With the IRQ disabled the zero update still trips, but the line
        // stays released.
        cart.cpu_write(0x5010, 0x00);
        cart.cpu_read_io(0x8000);
        assert!(!cart.irq_line());
    }

    #[test]
    fn mmc5_nametable_mapping_selects_per_slot_cirarn_exram_and_fill() {
        let mut aux = Mmc5Aux::new(vec![0; 0x1000], false);

        // $5105 = $50: vertical arrangement, NT0/NT1 use CIRAM page 0 and
        // NT2/NT3 use page 1 — each slot picks its page independently.
        aux.set_registers(0, [0, 0, 1, 1], 0, 0, 0);
        assert_eq!(aux.nametable_index(0x2000), Some(0x000));
        assert_eq!(aux.nametable_index(0x2400), Some(0x000));
        assert_eq!(aux.nametable_index(0x2800), Some(0x400));
        assert_eq!(aux.nametable_index(0x2C00), Some(0x400));

        // $5105 = $44: horizontal arrangement, NT0/NT2 page 0, NT1/NT3 page 1.
        aux.set_registers(0, [0, 1, 0, 1], 0, 0, 0);
        assert_eq!(aux.nametable_index(0x2000), Some(0x000));
        assert_eq!(aux.nametable_index(0x2400), Some(0x400));
        assert_eq!(aux.nametable_index(0x2800), Some(0x000));
        assert_eq!(aux.nametable_index(0x2C00), Some(0x400));

        // Single-screen CIRAM page 1 ($55) maps all four slots to page 1.
        aux.set_registers(0, [1, 1, 1, 1], 0, 0, 0);
        for address in [0x2000, 0x2400, 0x2800, 0x2C00] {
            assert_eq!(aux.nametable_index(address), Some(0x400));
        }

        // ExRAM is a nametable in modes 0/1 and reads as zeros in modes 2/3.
        aux.set_registers(0, [2, 2, 2, 2], 0, 0, 0);
        aux.exram[0] = 0x77;
        assert_eq!(aux.nametable_override(0x2000), Some(0x77));
        assert!(aux.nametable_write(0x2000, 0x11));
        assert_eq!(aux.exram[0], 0x11);
        aux.set_registers(2, [2, 2, 2, 2], 0, 0, 0);
        assert_eq!(aux.nametable_override(0x2000), Some(0x00));
        aux.nametable_write(0x2000, 0x22);
        assert_eq!(aux.exram[0], 0x11, "mode 2 ignores PPU ExRAM writes");

        // Fill mode supplies the tile and attribute bytes.
        aux.set_registers(0, [3, 3, 3, 3], 0x42, 0x05, 0);
        assert_eq!(aux.nametable_override(0x2000), Some(0x42));
        assert_eq!(aux.nametable_override(0x23C0), Some(0x05));
    }

    #[test]
    fn mmc5_multiplier_ram_protect_exram_and_mirroring() {
        let rom = synth_banked(5, 512, 256);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 5);

        // Unsigned 8x8 multiplier.
        cart.cpu_write(0x5205, 7);
        cart.cpu_write(0x5206, 9);
        assert_eq!(cart.cpu_read(0x5205), 63);
        assert_eq!(cart.cpu_read(0x5206), 0);
        cart.cpu_write(0x5205, 0x10);
        cart.cpu_write(0x5206, 0x10);
        assert_eq!(cart.cpu_read(0x5205), 0);
        assert_eq!(cart.cpu_read(0x5206), 1);

        // PRG-RAM writes need $5102=2 and $5103=1.
        cart.cpu_write(0x6000, 0x11);
        assert_eq!(cart.cpu_read(0x6000), 0);
        cart.cpu_write(0x5102, 0x02);
        cart.cpu_write(0x5103, 0x01);
        cart.cpu_write(0x6000, 0x5A);
        assert_eq!(cart.cpu_read(0x6000), 0x5A);
        cart.cpu_write(0x5113, 1);
        cart.cpu_write(0x6000, 0x33);
        cart.cpu_write(0x5113, 0);
        assert_eq!(cart.cpu_read(0x6000), 0x5A);

        // ExRAM: mode 2 is read/write, mode 3 is read-only.
        cart.cpu_write(0x5104, 2);
        cart.cpu_write(0x5C00, 0xA5);
        assert_eq!(cart.cpu_read(0x5C00), 0xA5);
        cart.cpu_write(0x5104, 3);
        cart.cpu_write(0x5C00, 0x11);
        assert_eq!(cart.cpu_read(0x5C00), 0xA5);

        // Nametable assignment (approximated as vertical/horizontal until the
        // PPU supports per-nametable pages).
        cart.cpu_write(0x5105, 0x00);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0x5105, 0x44);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn sachen172_reverses_data_and_switches_mirroring() {
        let rom = synth_banked(172, 32, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 172);

        // Reversed input $01 -> internal $20; output selects CHR bank 2.
        cart.cpu_write(0x4102, 0x01);
        cart.cpu_write(0x4100, 0);
        assert_eq!(cart.cpu_read(0x4100), 0x01);
        cart.cpu_write(0x8000, 0);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        assert_eq!(cart.ppu_read(0x0000), 0x90);

        // Invert (CPU bit 5) turns the latch write into vertical mirroring.
        cart.cpu_write(0x4101, 0x20);
        cart.cpu_write(0x8000, 0);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn ntd_tc112_banks_prg_chr_and_mirroring() {
        let rom = synth_banked(193, 128, 256);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 193);
        assert_eq!(cart.ppu_read(0x0000), 0x80);

        // 4 KiB CHR bank 2 at $0000.
        cart.cpu_write(0x6000, (2 << 2) | 3);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
        assert_eq!(cart.ppu_read(0x0C00), 0x8B);
        // 2 KiB banks 3 and 5.
        cart.cpu_write(0x6001, (3 << 1) | 1);
        assert_eq!(cart.ppu_read(0x1000), 0x86);
        cart.cpu_write(0x6002, 5 << 1);
        assert_eq!(cart.ppu_read(0x1800), 0x8A);

        // 16 KiB PRG bank 3 at $8000, last three banks fixed above it.
        cart.cpu_write(0x6003, 3);
        assert_eq!(cart.cpu_read(0x8000), 3);
        assert_eq!(cart.cpu_read(0xA000), 13);
        assert_eq!(cart.cpu_read(0xC000), 14);
        assert_eq!(cart.cpu_read(0xE000), 15);

        cart.cpu_write(0x6004, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0x6004, 0);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
    }

    #[test]
    fn mmc3_197_shifted_chr_wiring() {
        let rom = synth_banked(197, 64, 512);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 197);

        // Reg 0 = $0C -> 4 KiB ROM bank 24 at PPU $0000-$0FFF.
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 0x0C);
        assert_eq!(cart.ppu_read(0x0000), 0x98);
        assert_eq!(cart.ppu_read(0x0400), 0x99);
        assert_eq!(cart.ppu_read(0x0C00), 0x9B);

        // Reg 2 = 5 -> 2 KiB bank 10 at $1000; reg 3 = 7 -> bank 14 at $1800.
        cart.cpu_write(0x8000, 2);
        cart.cpu_write(0x8001, 5);
        assert_eq!(cart.ppu_read(0x1000), 0x8A);
        assert_eq!(cart.ppu_read(0x1400), 0x8B);
        cart.cpu_write(0x8000, 3);
        cart.cpu_write(0x8001, 7);
        assert_eq!(cart.ppu_read(0x1800), 0x8E);
        assert_eq!(cart.ppu_read(0x1C00), 0x8F);
    }

    #[test]
    fn sachen136_latches_output_and_inverts() {
        let rom = synth_banked(136, 64, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 136);
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 2);

        // Input $13 -> register -> output: CHR bank 3, PRG bank 1.
        cart.cpu_write(0x4102, 0x13);
        cart.cpu_write(0x4100, 0);
        cart.cpu_write(0x8000, 0);
        assert_eq!(cart.cpu_read(0x4100), 0x13);
        assert_eq!(cart.ppu_read(0x0000), 0x98);
        assert_eq!(cart.cpu_read(0x8000), 4);

        // Invert flips the read-side high bits and the write-side low nibble.
        cart.cpu_write(0x4101, 1);
        assert_eq!(cart.cpu_read(0x4100), 0x23);
        cart.cpu_write(0x4100, 0);
        assert_eq!(cart.cpu_read(0x4100), 0x2C);
        cart.cpu_write(0x4101, 0);
        cart.cpu_write(0x4102, 0x13);
        cart.cpu_write(0x4100, 0);

        // Mode 1 increments bits 0-3 only.
        cart.cpu_write(0x4103, 1);
        cart.cpu_write(0x4100, 0);
        assert_eq!(cart.cpu_read(0x4100), 0x14);
    }

    #[test]
    fn sunsoft4_banks_prg_chr_and_mirroring() {
        // 128 KiB PRG (16 x 8 KiB), 64 KiB CHR (64 x 1 KiB).
        let rom = synth_banked(68, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 68);

        // $8000 is the switchable 16 KiB bank; $C000 is the fixed last bank.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 14);
        cart.cpu_write(0xF000, 3);
        assert_eq!(cart.cpu_read(0x8000), 6);
        assert_eq!(cart.cpu_read(0xA000), 7);
        assert_eq!(cart.cpu_read(0xC000), 14);

        // CHR: four independent 2 KiB banks.
        assert_eq!(cart.ppu_read(0x0000), 0x80);
        assert_eq!(cart.ppu_read(0x0800), 0x82);
        assert_eq!(cart.ppu_read(0x1000), 0x84);
        assert_eq!(cart.ppu_read(0x1800), 0x86);
        cart.cpu_write(0x8000, 4);
        assert_eq!(cart.ppu_read(0x0000), 0x88);

        // Mirroring comes from $E000 bits 1:0.
        cart.cpu_write(0xE000, 0);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0xE000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
        cart.cpu_write(0xE000, 2);
        assert_eq!(cart.mirroring(), Mirroring::SingleLower);
        cart.cpu_write(0xE000, 3);
        assert_eq!(cart.mirroring(), Mirroring::SingleUpper);

        // ROM nametables: $E000.4 selects CHR banks from $C000/$D000. D7 of
        // each register is unconnected and reads as 1.
        assert!(cart.has_rom_nametables());
        assert_eq!(cart.rom_nametable_banks(), None);
        cart.cpu_write(0xE000, 0x10);
        assert_eq!(cart.rom_nametable_banks(), Some([0x80, 0x81]));
        cart.cpu_write(0xC000, 3);
        cart.cpu_write(0xD000, 5);
        assert_eq!(cart.rom_nametable_banks(), Some([0x83, 0x85]));

        // PRG-RAM is only visible while $F000.4 is set.
        cart.cpu_write(0x6000, 0x5A);
        assert_eq!(cart.cpu_read(0x6000), 0);
        cart.cpu_write(0xF000, 0x10);
        cart.cpu_write(0x6000, 0x5A);
        assert_eq!(cart.cpu_read(0x6000), 0x5A);
        assert_eq!(cart.prg_ram()[0], 0x5A);
    }

    #[test]
    fn uxrom_switches_low_bank() {
        let mut rom = synth_banked(2, 64, 8);
        for bank in 0..4usize {
            rom[16 + bank * 0x4000] = bank as u8;
        }
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 3);
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xC000), 3);
    }

    #[test]
    fn cnrom_switches_chr_bank() {
        let rom = synth_banked(3, 32, 24);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.ppu_read(0x0000), 0x80);
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x90);
    }

    #[test]
    fn axrom_switches_32k_bank_and_mirroring() {
        let mut rom = synth_banked(7, 64, 8);
        // AxROM has bus conflicts; make one write address readable as $FF so the
        // value passes through.
        rom[16 + 0x100] = 0xFF;
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x8000), 0);
        cart.cpu_write(0x8100, 0x11);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.mirroring(), Mirroring::SingleUpper);
    }

    #[test]
    fn mmc3_maps_prg_and_chr() {
        let rom = synth_banked(4, 64, 8);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Default bank data: slot 0/1 use R6/R7 (0), last two fixed.
        assert_eq!(cart.cpu_read(0xE000), 7);
        // Select R6 = 2, then switch PRG mode so R6 appears at $8000.
        cart.cpu_write(0x8000, 6);
        cart.cpu_write(0x8001, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        // CHR 2 KiB mode: select R0 = 2 (even) -> banks 2/3.
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
        assert_eq!(cart.ppu_read(0x0400), 0x83);
    }

    #[test]
    fn color_dreams_banks_prg_and_chr() {
        let mut rom = synth_banked(11, 128, 32);
        rom[16 + 0x100] = 0xFF; // readable for bus conflicts
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.ppu_read(0x0000), 0x80);
        cart.cpu_write(0x8100, 0x21);
        assert_eq!(cart.cpu_read(0x8000), 8);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
    }

    #[test]
    fn cprom_banks_upper_chr_ram() {
        let rom = synth_banked(13, 32, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert!(cart.chr_is_ram());
        cart.cpu_write(0x8000, 0x20);
        cart.ppu_write(0x1000, 0x5A);
        assert_eq!(cart.ppu_read(0x1000), 0x5A);
        cart.ppu_write(0x0000, 0x11);
        assert_eq!(cart.ppu_read(0x0000), 0x11);
        assert_ne!(cart.ppu_read(0x1000), 0x11);
    }

    #[test]
    fn gxrom_banks_prg_and_chr() {
        let rom = synth_banked(66, 64, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 0x11);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
    }

    #[test]
    fn camerica_banks_16k() {
        let rom = synth_banked(71, 64, 8);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0xC000), 6);
        cart.cpu_write(0x8000, 0x12);
        assert_eq!(cart.cpu_read(0x8000), 4);
        assert_eq!(cart.mirroring(), Mirroring::SingleUpper);
    }

    #[test]
    fn nanjing163_switches_32k_bank() {
        let rom = synth_banked(163, 1024, 0);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Reset: mode 0 forces A15/A16 high -> 32 KiB bank 3.
        assert_eq!(cart.cpu_read(0x8000), 12);
        // `$5300 = $04` enables register banking.
        cart.cpu_write(0x5300, 0x04);
        cart.cpu_write(0x5000, 0x02);
        assert_eq!(cart.cpu_read(0x8000), 8);
        cart.cpu_write(0x5200, 0x01);
        assert_eq!(cart.cpu_read(0x8000), 72);
    }

    #[test]
    fn sachen150_indexed_registers() {
        let rom = synth_banked(150, 128, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // R5 = PRG bank 2.
        cart.cpu_write(0x4100, 5);
        cart.cpu_write(0x4101, 2);
        assert_eq!(cart.cpu_read(0x8000), 8);
        // R4 = CHR A15, R6 = CHR A14..A13 -> 8 KiB bank 5.
        cart.cpu_write(0x4100, 4);
        cart.cpu_write(0x4101, 1);
        cart.cpu_write(0x4100, 6);
        cart.cpu_write(0x4101, 1);
        assert_eq!(cart.ppu_read(0x0000), 0xA8);
        // Registers are readable.
        cart.cpu_write(0x4100, 5);
        assert_eq!(cart.cpu_read(0x4101), 2);
    }

    #[test]
    fn fme7_banks_prg_and_chr() {
        let rom = synth_banked(69, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        // Command 9: PRG bank at $8000 (8 KiB bank 2 -> chunk 2).
        cart.cpu_write(0x8000, 9);
        cart.cpu_write(0xA000, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        // $E000 is fixed to the last 8 KiB bank (bank 15 -> chunk 15).
        assert_eq!(cart.cpu_read(0xE000), 15);
        // Command 0: CHR 1 KiB bank 0.
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0xA000, 0);
        assert_eq!(cart.ppu_read(0x0000), 0x80);
    }

    #[test]
    fn nina148_latches_at_8000() {
        let mut rom = synth_banked(148, 128, 32);
        rom[16 + 0x100] = 0xFF; // visible for bus conflicts
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8100, 0x21);
        assert_eq!(cart.cpu_read(0x8000), 8);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
    }

    #[test]
    fn ss88006_banks_prg_and_chr() {
        let rom = synth_banked(18, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        cart.cpu_write(0xA000, 3);
        assert_eq!(cart.ppu_read(0x0000), 0x83);
        assert_eq!(cart.cpu_read(0xE000), 15);
    }

    #[test]
    fn unrom512_switches_16k_bank() {
        let mut rom = synth_banked(30, 256, 0);
        rom[16 + 0x100] = 0xFF; // readable for bus conflicts
        rom[6] |= 0x02; // battery: disable conflicts for this synthetic image
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 30);
        cart.cpu_write(0x8100, 3);
        assert_eq!(cart.cpu_read(0x8000), 6);
        assert_eq!(cart.cpu_read(0xC000), 30);
    }

    #[test]
    fn bnrom_switches_32k_bank() {
        let mut rom = synth_banked(34, 128, 0);
        // Make $8001 readable as $FF so the bus-conflict AND passes through.
        rom[17] = 0xFF;
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.cpu_read(0x8000), 0);
        cart.cpu_write(0x8001, 2);
        assert_eq!(cart.cpu_read(0x8000), 8);
    }

    #[test]
    fn nina001_switches_chr_banks() {
        // CHR > 8 KiB selects the NINA-001 register set.
        let rom = synth_banked(34, 64, 16);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x7FFD, 1);
        assert_eq!(cart.cpu_read(0x8000), 4);
        cart.cpu_write(0x7FFE, 1);
        cart.cpu_write(0x7FFF, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x84);
        assert_eq!(cart.ppu_read(0x1000), 0x88);
    }

    #[test]
    fn nina79_banks_prg_and_chr() {
        let rom = synth_banked(79, 128, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x4100, 0x21); // PRG 2, CHR 1
        assert_eq!(cart.cpu_read(0x8000), 8);
        assert_eq!(cart.ppu_read(0x0000), 0x88);
    }

    #[test]
    fn dxrom_banks_prg_and_chr() {
        let rom = synth_banked(206, 64, 32);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x8000, 6);
        cart.cpu_write(0x8001, 2);
        assert_eq!(cart.cpu_read(0x8000), 2);
        cart.cpu_write(0x8000, 0);
        cart.cpu_write(0x8001, 2);
        assert_eq!(cart.ppu_read(0x0000), 0x82);
        assert_eq!(cart.ppu_read(0x0400), 0x83);
        // $C000/$E000 are always the last two banks.
        assert_eq!(cart.cpu_read(0xE000), 7);
    }

    #[test]
    fn mmc3_irq_counts_scanlines() {
        let rom = synth_banked(4, 64, 8);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0xC000, 3); // latch
        cart.cpu_write(0xC001, 0); // reload
        cart.cpu_write(0xE001, 0); // enable
        for _ in 0..3 {
            cart.clock_scanline();
        }
        assert!(!cart.irq_line(), "IRQ too early");
        cart.clock_scanline();
        assert!(cart.irq_line(), "IRQ did not fire");
        cart.cpu_write(0xE000, 0);
        assert!(!cart.irq_line(), "IRQ did not clear");
    }

    #[test]
    fn mmc5_maps_prg_chr_and_irq() {
        let rom = synth_banked(5, 64, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0x5100, 3); // 8 KiB PRG mode
        cart.cpu_write(0x5114, 0x82); // bit 7 = ROM, bank 2
        assert_eq!(cart.cpu_read(0x8000), 2);
        cart.cpu_write(0x5114, 0x02); // bit 7 clear = PRG-RAM
        assert_eq!(cart.cpu_read(0x8000), 0);
        cart.cpu_write(0x5101, 3); // 1 KiB CHR mode
        cart.cpu_write(0x5120, 3);
        assert_eq!(cart.ppu_read(0x0000), 0x83);
        // IRQ: latch 2, enable, then 3 scanlines.
        cart.cpu_write(0x5203, 2);
        cart.cpu_write(0x5204, 0x80);
        cart.clock_scanline();
        assert!(!cart.irq_line());
        cart.clock_scanline();
        assert!(cart.irq_line());
        cart.cpu_write(0x5204, 0x00);
        assert!(!cart.irq_line());
    }

    #[test]
    fn vrc_maps_prg_and_chr() {
        // Mapper 23 defaults to VRC4e: sub-registers at $x000/$x004/$x008/$x00C.
        let rom = synth_banked(23, 64, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 23);
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xA000), 1);
        assert_eq!(cart.cpu_read(0xC000), 6); // fixed second-to-last.
        assert_eq!(cart.cpu_read(0xE000), 7); // fixed last.

        cart.cpu_write(0x8000, 3);
        assert_eq!(cart.cpu_read(0x8000), 3);
        assert_eq!(cart.cpu_read(0xC000), 6);
        cart.cpu_write(0xA000, 2);
        assert_eq!(cart.cpu_read(0xA000), 2);

        // CHR bank 0 is written as a low/high nibble pair.
        cart.cpu_write(0xB000, 0x0F);
        cart.cpu_write(0xB004, 0x01);
        assert_eq!(cart.ppu_read(0x0000), 0x9F);

        cart.cpu_write(0x9000, 1);
        assert_eq!(cart.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn vrc_irq_counts_cpu_cycles() {
        let rom = synth_banked(23, 64, 64);
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        cart.cpu_write(0xF000, 2); // latch low nibble.
        cart.cpu_write(0xF004, 0); // latch high nibble.
        cart.cpu_write(0xF008, 2); // enable.
        cart.clock_cpu();
        assert!(cart.irq_line());
        cart.acknowledge_irq();
        assert!(!cart.irq_line());
    }

    #[test]
    fn mmc1_switches_prg_bank() {
        // 4 x 16 KiB PRG banks: fill banks with 0,1,2,3 while building.
        let mut rom = synthetic(1, 4, 2);
        for bank in 0..4usize {
            let start = 16 + bank * 0x4000;
            for byte in &mut rom[start..start + 0x4000] {
                *byte = bank as u8;
            }
        }
        rom[6] = 0x10; // mapper 1 low nibble
        rom[7] = 0x00;
        let mut cart = Cartridge::from_ines(&rom).expect("cart");
        assert_eq!(cart.mapper(), 1);
        // Default control $0C: bank 0 at $8000, last bank at $C000.
        assert_eq!(cart.cpu_read(0x8000), 0);
        assert_eq!(cart.cpu_read(0xC000), 3);
        // Serial write: control = 0x1D (fix last bank at $C000, switch $8000).
        for bit in [1, 0, 1, 1, 1] {
            cart.cpu_write(0x8000, bit);
        }
        // Select bank 2 at $8000.
        for bit in [0, 1, 0, 0, 0] {
            cart.cpu_write(0xE000, bit);
        }
        assert_eq!(cart.cpu_read(0x8000), 2);
        assert_eq!(cart.cpu_read(0xC000), 3);
    }
}
