//! Whole-console wiring (A4): cartridge + CPU + PPU + APU.
//!
//! [`System`] implements [`CpuBus`] directly, so the CPU, PPU and APU advance
//! from the same cycle stream. It covers the NES memory map, OAM DMA, DMC
//! sample fetches, controller shift registers and frame/audio capture.

use crate::apu::Apu;
use crate::bus::CpuBus;
use crate::cart::{CartError, Cartridge, Mirroring};
use crate::clock::Timing;
use crate::cpu::Cpu;
use crate::ppu::Ppu;

/// CPU clock in Hz (NTSC).
pub const CPU_HZ: f64 = 1_789_773.0;

/// A whole NES console.
pub struct System {
    /// Cartridge and mapper state.
    pub cart: Cartridge,
    /// CPU.
    pub cpu: Cpu,
    /// PPU.
    pub ppu: Ppu,
    /// APU.
    pub apu: Apu,
    ram: [u8; 0x800],
    controllers: [u8; 2],
    strobe: bool,
    /// Global CPU cycle phase (get/put).
    phase: bool,
    shift: [u8; 2],
    frame: Vec<u8>,
    audio: Vec<f32>,
    audio_accum: f32,
    audio_count: u32,
    audio_rate: f32,
    sample_cycle: u32,
    open_bus: u8,
    /// Internal 2A03 data bus (drives $4015 bit 5, not the external bus).
    internal_bus: u8,
    stall_cycles: u32,
    rmw_dummy: bool,
    cpu_pc: u16,
    /// CPU program counter, used for APU register activation during OAM DMA.
    /// PPU dots still owed at the end of a split CPU cycle.
    end_dots: u8,
    /// PPU scanline at the start of the current split cycle.
    cycle_before_scanline: u16,
    dma: Option<Dma>,
}

/// Which DMA engine owns the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmaKind {
    /// Delta-modulation sample fetch.
    Dmc,
    /// Sprite (`$4014`) transfer.
    Oam,
}

/// A DMA transfer that spans CPU cycles.
#[derive(Debug, Clone, Copy)]
struct Dma {
    kind: DmaKind,
    /// Total cycles the transfer needs.
    total: u16,
    /// Cycles still to run.
    remaining: u16,
    /// Address of the CPU access the DMA is stealing from (for dummy reads).
    cpu_address: Option<u16>,
    /// OAM source page, high byte.
    page: u16,
    /// OAMADDR latched at the start of an OAM transfer.
    oam_start: u8,
    /// OAM transfer progress.
    oam_counter: u8,
    oam_buffer: u8,
    oam_write_ready: bool,
    oam_halt_done: bool,
}

impl System {
    /// Builds a system from an iNES image.
    pub fn from_ines(rom: &[u8]) -> Result<Self, CartError> {
        let cart = Cartridge::from_ines(rom)?;
        let mirroring = matches!(cart.mirroring(), Mirroring::Vertical);
        let chr_ram = cart.chr_is_ram();
        let mut ppu = Ppu::new(Timing::NTSC, vec![0; 0x2000], mirroring, chr_ram);
        for address in 0..0x2000u16 {
            ppu.chr[usize::from(address)] = cart.ppu_read(address);
        }
        let mut system = Self {
            cart,
            cpu: Cpu::new(),
            ppu,
            apu: Apu::new(),
            ram: [0; 0x800],
            controllers: [0; 2],
            strobe: false,
            phase: false,
            shift: [0; 2],
            frame: vec![0; 256 * 240 * 4],
            audio: Vec::with_capacity(1024),
            audio_accum: 0.0,
            audio_count: 0,
            audio_rate: 48_000.0,
            sample_cycle: 0,
            open_bus: 0,
            internal_bus: 0,
            stall_cycles: 0,
            rmw_dummy: false,
            cpu_pc: 0,
            end_dots: 0,
            cycle_before_scanline: 0,
            dma: None,
        };
        let mut cpu = system.cpu;
        cpu.reset(&mut system);
        system.cpu = cpu;
        Ok(system)
    }

    /// Resets the CPU and PPU without reloading the cartridge.
    pub fn reset(&mut self) {
        let mut cpu = self.cpu;
        cpu.reset(self);
        self.cpu = cpu;
        self.apu = Apu::new();
        self.ram = [0; 0x800];
    }

    /// Sets controller input bits (`InputState` layout).
    pub fn set_controller(&mut self, port: usize, bits: u16) {
        if port < 2 {
            self.controllers[port] = bits as u8;
        }
    }

    /// Changes the output sample rate.
    pub fn set_sample_rate(&mut self, rate: f32) {
        if rate > 0.0 {
            self.audio_rate = rate;
            self.sample_cycle = (CPU_HZ / f64::from(rate)).round() as u32;
        }
    }

    /// Runs one full display frame.
    pub fn run_frame(&mut self) -> u32 {
        self.sync_cartridge();
        self.audio.clear();
        let start = self.ppu.frames;
        let mut steps = 0u32;
        while self.ppu.frames == start && steps < 2_000_000 {
            let mut cpu = self.cpu;
            let cycles = cpu.step(self);
            self.cpu = cpu;
            steps += cycles.max(1);
            if self.cpu.halted {
                break;
            }
        }
        self.frame = self.ppu.render_frame();
        steps
    }

    /// The most recently rendered RGBA8 frame.
    #[must_use]
    pub fn video(&self) -> &[u8] {
        &self.frame
    }

    /// Audio produced by the last frame.
    #[must_use]
    pub fn audio(&self) -> &[f32] {
        &self.audio
    }

    /// Current audio sample rate.
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.audio_rate
    }

    /// Battery-backed PRG RAM, if any.
    #[must_use]
    pub fn sram(&self) -> &[u8] {
        self.cart.prg_ram()
    }

    fn sync_cartridge(&mut self) {
        // CHR-ROM is copied once per frame; mid-frame bank switches are an A4
        // follow-up. CHR-RAM is owned by the PPU because games upload tiles to
        // it through $2007, so it must not be overwritten.
        if !self.cart.chr_is_ram() {
            for address in 0..0x2000u16 {
                self.ppu.chr[usize::from(address)] = self.cart.ppu_read(address);
            }
        }
        let mirroring = matches!(self.cart.mirroring(), Mirroring::Vertical);
        if self.ppu.vertical_mirroring != mirroring {
            self.ppu.vertical_mirroring = mirroring;
        }
    }

    /// Reads a byte from the CPU address space without side effects.
    ///
    /// Intended for test harnesses that inspect RAM (e.g. AccuracyCoin result
    /// bytes); PPU/APU registers are not modelled here.
    #[must_use]
    pub fn read_ram(&self, address: u16) -> u8 {
        self.peek(address)
    }

    fn peek(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x2000..=0x3FFF => 0,
            0x4020..=0xFFFF => self.cart.cpu_read(address),
            _ => 0,
        }
    }

    fn read_controller(&mut self, port: usize) -> u8 {
        let bit = if self.shift[port] & 1 != 0 { 0x01 } else { 0 };
        let value = (self.open_bus & 0xE0) | 0x40 | bit;
        if !self.strobe {
            self.shift[port] = (self.shift[port] >> 1) | 0x80;
        }
        value
    }

    fn clock(&mut self) {
        self.end_dots = 0;
        let before = self.ppu.scanline;
        self.ppu.step_dots(3);
        let after = self.ppu.scanline;
        if before != after {
            // MMC3-style mappers clock once per rendered scanline on the PPU
            // A12 rise. Visible lines and the pre-render line both fetch
            // patterns; vblank does not.
            let rendered = (after <= 239 || after == 261) && self.ppu.mask & 0x18 != 0;
            if rendered {
                self.cart.clock_scanline();
            }
        }
        self.cart.clock_cpu();
        self.apu.phase_override = Some(self.phase);
        self.apu.tick();
        self.mix_sample();
        self.phase = !self.phase;
    }

    fn mix_sample(&mut self) {
        self.audio_accum += self.apu.output;
        self.audio_count += 1;
        if self.audio_count >= self.sample_cycle.max(1) {
            let sample = self.audio_accum / self.audio_count as f32;
            // Normalise the mixer's 0..~95 range to -1..1 around the midpoint.
            self.audio.push((sample / 50.0) - 1.0);
            self.audio_accum = 0.0;
            self.audio_count = 0;
        }
    }

    /// Starts or steps the DMA engine for one CPU cycle.
    ///
    /// `address`/`write` describe the CPU access that would have happened this
    /// cycle; the DMA uses it for the halt/alignment dummy reads and to detect
    /// a `$4015` disable that aborts an in-flight DMC fetch.
    fn begin_dma_cycle(&mut self, address: Option<u16>, write: Option<u8>) {
        if let Some(mut dma) = self.dma.take() {
            // A `$4015` write with the DMC enable bit clear aborts a DMC fetch.
            let disables_dmc = dma.kind == DmaKind::Dmc
                && address.is_some_and(|value| value & 0x1F == 0x15)
                && write.is_some_and(|value| value & 0x10 == 0);
            if disables_dmc {
                // The halt cycle already ran; the disable lands on the next
                // cycle, so the transfer costs one cycle and then aborts.
                self.clock();
                return;
            }
            self.step_dma(&mut dma);
            if dma.remaining > 0 {
                self.dma = Some(dma);
            }
            return;
        }

        // Advance the APU for this CPU cycle before sampling the DMC request,
        // matching the reference where the APU runs before the CPU bus access.
        self.cart.clock_cpu();
        self.apu.phase_override = Some(self.phase);
        self.apu.tick();
        self.mix_sample();
        self.phase = !self.phase;

        if write.is_none() && self.apu.dmc_needs_sample() {
            let short_fetch = self.apu.take_dmc_held();
            // Halt cycle: the CPU read is stolen and the current address is
            // used for the dummy read, exactly as the reference DMA routine.
            let skip = address.is_some_and(|a| matches!(a & 0x1F, 0x16 | 0x17));
            if let Some(addr) = address {
                // The halt read always occurs; only the alignment read skips
                // the controller ports (the "double read" quirk).
                let value = self.dma_read(addr);
                if addr != 0x4015 {
                    self.open_bus = value;
                }
            }
            self.advance_ppu_cycle(3);
            let mut dummy = false;
            let mut extra = 0u32;
            loop {
                if self.phase {
                    // put cycle: DMC alignment / dummy read
                    dummy = true;
                    if let Some(addr) = address {
                        if !skip {
                            let value = self.dma_read(addr);
                            if addr != 0x4015 {
                                self.open_bus = value;
                            }
                        }
                    }
                } else if dummy {
                    // get cycle: real sample read
                    let addr = self.apu.dmc.address;
                    let byte = self.dma_read(addr);
                    if addr != 0x4015 {
                        self.open_bus = byte;
                    }
                    self.apu.dmc_sample(byte);
                    self.clock();
                    extra += 1;
                    break;
                } else {
                    dummy = true;
                    if let Some(addr) = address {
                        if !skip {
                            let value = self.dma_read(addr);
                            if addr != 0x4015 {
                                self.open_bus = value;
                            }
                        }
                    }
                }
                self.clock();
                extra += 1;
            }
            if short_fetch {
                // The held request was already attempted one cycle earlier;
                // keep the shorter [halt][put][get] form.
                self.apu.dmc_catch_up();
            } else {
                self.clock();
                extra += 1;
            }
            self.stall_cycles += extra;
            return;
        }

        // Normal CPU cycle: two PPU dots before the bus access, one after.
        self.advance_ppu_cycle(2);
        self.end_dots = 1;
    }

    /// Advances the PPU by `dots` and clocks scanline-based mappers.
    fn advance_ppu_cycle(&mut self, dots: u32) {
        let before = self.ppu.scanline;
        self.ppu.step_dots(dots);
        let after = self.ppu.scanline;
        if before != after {
            let rendered = (after <= 239 || after == 261) && self.ppu.mask & 0x18 != 0;
            if rendered {
                self.cart.clock_scanline();
            }
        }
    }

    /// Runs one cycle of an active transfer.
    fn step_dma(&mut self, dma: &mut Dma) {
        let index = if dma.kind == DmaKind::Dmc {
            let index = dma.total - dma.remaining;
            dma.remaining -= 1;
            index
        } else {
            0
        };
        let phase_now = self.phase;
        self.clock();
        match dma.kind {
            DmaKind::Dmc => {
                if index + 2 < dma.total {
                    // Halt and alignment cycles dummy-read the CPU address.
                    // The NES controller port only sees one of the two reads
                    // (the "double read" quirk), so the alignment cycle clocks
                    // without shifting it again.
                    if let Some(address) = dma.cpu_address {
                        if index == 0 || !matches!(address & 0x1F, 0x16 | 0x17) {
                            let value = self.dma_read(address);
                            if address != 0x4015 {
                                self.open_bus = value;
                            }
                        }
                    }
                } else if index + 2 == dma.total {
                    let address = self.apu.dmc.address;
                    let byte = self.dma_read(address);
                    if address != 0x4015 {
                        self.open_bus = byte;
                    }
                    self.apu.dmc_sample(byte);
                }
            }
            DmaKind::Oam => {
                // Phase-driven: reads happen on get cycles and writes on put
                // cycles. Starting on a put cycle inserts one alignment cycle.
                let phase = phase_now;
                if !dma.oam_halt_done {
                    dma.oam_halt_done = true;
                } else if phase {
                    if dma.oam_write_ready {
                        let address = dma.oam_start.wrapping_add(dma.oam_counter);
                        let mut value = dma.oam_buffer;
                        if address & 3 == 2 {
                            value &= 0xE3;
                        }
                        self.ppu.oam[usize::from(address)] = value;
                        dma.oam_write_ready = false;
                        dma.oam_counter = dma.oam_counter.wrapping_add(1);
                        if dma.oam_counter == 0 {
                            dma.remaining = 0;
                        }
                    } else if let Some(address) = dma.cpu_address {
                        let value = self.dma_read(address);
                        self.open_bus = value;
                    }
                } else {
                    let address = dma.page + u16::from(dma.oam_counter);
                    let byte = if self.cpu_pc >> 8 == 0x40 {
                        // With the 6502 address bus in $4000-$40FF the APU
                        // registers and their $20-byte mirrors are active for
                        // every OAM DMA read.
                        self.dma_read(0x4000 | (address & 0x001F))
                    } else if (0x4000..=0x401F).contains(&address) {
                        self.open_bus
                    } else {
                        self.dma_read(address)
                    };
                    self.open_bus = byte;
                    dma.oam_buffer = byte;
                    dma.oam_write_ready = true;
                }
            }
        }
    }

    /// A DMA bus read: same side effects as a CPU read, but without advancing
    /// this CPU cycle (the DMA engine advances its own).
    fn dma_read(&mut self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x2000..=0x3FFF => self.ppu.read_register(address),
            0x4015 => self.apu.read_status(),
            0x4016 => self.read_controller(0),
            0x4017 => self.read_controller(1),
            0x4018..=0x401F => self.open_bus,
            0x4020..=0x5FFF => {
                if address == 0x5204 {
                    let value = self.cart.cpu_read(address);
                    self.cart.acknowledge_irq();
                    value
                } else {
                    self.open_bus
                }
            }
            0x6000..=0x7FFF => {
                if self.cart.prg_ram().is_empty() {
                    self.open_bus
                } else {
                    self.cart.cpu_read(address)
                }
            }
            0x8000..=0xFFFF => {
                let rom = self.cart.cpu_read(address);
                // The APU registers are mirrored every $20 bytes; a DMA read
                // whose low five bits select a controller port conflicts with
                // it: bits 5-7 come from ROM, bits 0-4 from the controller.
                match address & 0x1F {
                    // $4015 does not drive the data bus, but the read still
                    // acknowledges/clears the frame IRQ flag.
                    0x15 => {
                        // $4015 acknowledges the frame IRQ, but the sample
                        // ROM value still drives the external data bus.
                        self.apu.read_status();
                        rom
                    }
                    0x16 => {
                        (rom & 0xA0) | (self.open_bus & 0x40) | (self.read_controller(0) & 0x1F)
                    }
                    0x17 => {
                        (rom & 0xA0) | (self.open_bus & 0x40) | (self.read_controller(1) & 0x1F)
                    }
                    _ => rom,
                }
            }
            _ => self.open_bus,
        }
    }
}

impl CpuBus for System {
    fn read(&mut self, address: u16) -> u8 {
        // $2004 reflects the OAM data bus at the end of the CPU read cycle.
        // Advance the final PPU dot before sampling it and suppress the usual
        // post-access step so the cycle length is unchanged.
        if (0x2000..=0x3FFF).contains(&address) && address & 7 == 4 {
            self.ppu.step_dots(1);
            self.end_dots = 0;
        }
        let mut drives_bus = true;
        let value = match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x2000..=0x3FFF => self.ppu.read_register(address),
            0x4000..=0x4013 => self.open_bus,
            0x4015 => {
                // $4015 is internal to the 2A03 and does not drive the CPU
                // data bus; open bus keeps its previous value.
                drives_bus = false;
                let status = self.apu.read_status();
                (status & 0xDF) | (self.internal_bus & 0x20)
            }
            0x4016 => self.read_controller(0),
            0x4017 => self.read_controller(1),
            0x4018..=0x401F => self.open_bus,
            0x4020..=0x5FFF => {
                if address == 0x5204 {
                    let value = self.cart.cpu_read(address);
                    self.cart.acknowledge_irq();
                    value
                } else {
                    drives_bus = false;
                    self.open_bus
                }
            }
            0x6000..=0x7FFF => {
                if self.cart.prg_ram().is_empty() {
                    self.open_bus
                } else {
                    self.cart.cpu_read(address)
                }
            }
            0x8000..=0xFFFF => self.cart.cpu_read(address),
            _ => self.open_bus,
        };
        if drives_bus {
            self.open_bus = value;
            self.internal_bus = value;
        }
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1FFF => {
                if std::env::var("CTRL").is_ok() && (0x50..=0x53).contains(&address) {
                    eprintln!(
                        "W {:02X}={:02X} cyc={}",
                        address & 0x7F,
                        value,
                        self.cpu.cycles
                    );
                }
                self.ram[usize::from(address) & 0x07FF] = value;
            }
            0x2000..=0x3FFF => {
                self.ppu.write_register(address, value);
            }
            0x4000..=0x4013 | 0x4015 | 0x4017 => self.apu.write(address, value),
            0x4014 if self.rmw_dummy => {
                // The dummy write of an RMW instruction does not start a
                // second transfer; the real write supplies the page.
            }
            0x4014 => {
                // One halt cycle plus 256 get/put pairs = 513 cycles, during
                // which the CPU is stalled.
                self.dma = Some(Dma {
                    kind: DmaKind::Oam,
                    total: 0,
                    remaining: 1,
                    cpu_address: None,
                    page: u16::from(value) << 8,
                    oam_start: self.ppu.oam_addr,
                    oam_counter: 0,
                    oam_buffer: 0,
                    oam_write_ready: false,
                    oam_halt_done: false,
                });
            }
            0x4016 => {
                // A normal write always strobes. An RMW dummy write only
                // reaches the controller when it lands on a put cycle.
                if !self.rmw_dummy || self.phase {
                    self.strobe = value & 1 != 0;
                    if self.strobe {
                        self.shift = self.controllers;
                    }
                }
            }
            0x4020..=0xFFFF => self.cart.cpu_write(address, value),
            _ => {}
        }
        self.open_bus = value;
        self.internal_bus = value;
    }

    fn tick(&mut self) {}

    fn begin_cycle(&mut self, address: Option<u16>, write: Option<u8>) {
        self.begin_dma_cycle(address, write);
    }

    fn end_cycle(&mut self) {
        if self.end_dots == 0 {
            return;
        }
        self.ppu.step_dots(u32::from(self.end_dots));
        self.end_dots = 0;
        let before = self.cycle_before_scanline;
        let after = self.ppu.scanline;
        if before != after {
            let rendered = (after <= 239 || after == 261) && self.ppu.mask & 0x18 != 0;
            if rendered {
                self.cart.clock_scanline();
            }
        }
    }

    fn set_rmw_dummy(&mut self, dummy: bool) {
        self.rmw_dummy = dummy;
    }

    fn set_cpu_pc(&mut self, pc: u16) {
        self.cpu_pc = pc;
    }

    fn dma_active(&self) -> bool {
        self.dma.is_some()
    }

    fn nmi_line(&self) -> bool {
        self.ppu.nmi_line
    }

    fn irq_line(&self) -> bool {
        self.apu.irq_line() || self.cart.irq_line()
    }

    fn take_nmi(&mut self) -> bool {
        if self.ppu.nmi_pending {
            self.ppu.nmi_pending = false;
            true
        } else {
            false
        }
    }

    fn nmi_suppressed(&self) -> bool {
        self.ppu.nmi_suppressed
    }

    fn take_stall(&mut self) -> u32 {
        let stall = self.stall_cycles;
        self.stall_cycles = 0;
        stall
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn boots_an_nrom_and_renders_a_frame() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
        let path = root.join("nestest.nes");
        if !path.exists() {
            eprintln!("skipping: test-roms/nestest.nes missing");
            return;
        }
        let rom = fs::read(path).expect("rom");
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 0);
        let start = system.ppu.frames;
        let steps = system.run_frame();
        assert!(steps > 0, "CPU stalled");
        assert!(system.ppu.frames > start, "no frame completed");
        assert_eq!(system.video().len(), 256 * 240 * 4);
        assert!(!system.audio().is_empty(), "no audio produced");
    }
}
