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
    /// Device attached to each controller port.
    devices: [PortDevice; 2],
    /// NES Zapper state per port (used when the device is [`PortDevice::Zapper`]).
    zapper: [Zapper; 2],
    /// PPU frame number of the last Zapper light reset.
    zapper_frame: u64,
    strobe: bool,
    /// Regional timing (NTSC/PAL/Dendy).
    timing: Timing,
    /// Master-clock credit for fractional PPU dots per CPU cycle.
    ppu_credit: u32,
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
    /// A DMA used the bus on the current CPU cycle.
    dma_event: bool,
    cpu_pc: u16,
    /// DMC fetch state carried out of an OAM transfer that ended first.
    dmc_carry: u8,
    /// CPU program counter, used for APU register activation during OAM DMA.
    /// PPU dots still owed at the end of a split CPU cycle.
    end_dots: u8,
    /// PPU scanline at the start of the current split cycle.
    cycle_before_scanline: u16,
    dma: Option<Dma>,
}

/// Device attached to a controller port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortDevice {
    /// Standard controller.
    #[default]
    Gamepad,
    /// NES Zapper light gun.
    Zapper,
}

/// NES Zapper state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Zapper {
    /// Sensor X position (0..=255), top-left of the 8x8 detection area.
    pub x: u8,
    /// Sensor Y position (0..=239).
    pub y: u8,
    /// Trigger pulled (full pull).
    pub trigger: bool,
    /// The beam swept a bright pixel under the sensor this frame.
    light: bool,
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
    /// Interleaved DMC fetch state: 0 idle, 1 halted, 2 ready to read.
    dmc_state: u8,
}

impl System {
    /// Builds a system from an iNES image.
    pub fn from_ines(rom: &[u8]) -> Result<Self, CartError> {
        let cart = Cartridge::from_ines(rom)?;
        let mirroring = matches!(cart.mirroring(), Mirroring::Vertical);
        let chr_ram = cart.chr_is_ram();
        let mut ppu = Ppu::new(Timing::NTSC, vec![0; 0x2000], mirroring, chr_ram);
        ppu.mirroring = cart.mirroring();
        for address in 0..0x2000u16 {
            ppu.chr[usize::from(address)] = cart.ppu_read(address);
        }
        // MMC2/MMC4 swap 4 KiB pattern banks during rendering, so the PPU
        // keeps the full CHR ROM and drives the latches itself.
        if let (Some(rom), Some(banks)) = (cart.chr_rom(), cart.chr_banks()) {
            ppu.set_mmc2_chr(rom.to_vec(), banks);
        }
        if let Some(hybrid) = cart.hybrid_chr() {
            ppu.set_hybrid_chr(hybrid);
        }
        if let Some(aux) = cart.mmc5_aux() {
            ppu.set_mmc5_aux(aux);
        }
        if let Some(aux) = cart.vrc7_chr() {
            ppu.set_vrc7_chr(aux);
        }
        if cart.has_rom_nametables() {
            if let Some(rom) = cart.chr_rom() {
                ppu.set_rom_nametable_rom(rom.to_vec());
            }
        }
        let mut system = Self {
            cart,
            cpu: Cpu::new(),
            ppu,
            apu: Apu::new(),
            ram: [0; 0x800],
            controllers: [0; 2],
            devices: [PortDevice::Gamepad; 2],
            zapper: [Zapper::default(); 2],
            zapper_frame: 0,
            strobe: false,
            timing: Timing::NTSC,
            ppu_credit: 0,
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
            dma_event: false,
            cpu_pc: 0,
            dmc_carry: 0,
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

    /// Attaches a device to a controller port.
    pub fn set_port_device(&mut self, port: usize, device: PortDevice) {
        if port < 2 {
            self.devices[port] = device;
        }
    }

    /// Sets the Zapper's aim and trigger for a port.
    pub fn set_zapper(&mut self, port: usize, x: u8, y: u8, trigger: bool) {
        if port < 2 {
            self.zapper[port].x = x;
            self.zapper[port].y = y;
            self.zapper[port].trigger = trigger;
        }
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
            self.sample_cycle = (self.timing.cpu_hz() as f64 / f64::from(rate)).round() as u32;
        }
    }

    /// Selects the regional timing model (NTSC, PAL or Dendy).
    ///
    /// The PPU scanline count and the CPU/PPU clock ratio follow the region.
    /// PAL/Dendy run without the NTSC odd-frame dot skip.
    pub fn set_region(&mut self, region: crate::clock::Region) {
        self.timing = Timing::for_region(region);
        self.ppu.timing = self.timing;
        self.ppu_credit = 0;
        self.set_sample_rate(self.audio_rate);
    }

    /// Regional timing in use.
    #[must_use]
    pub const fn timing(&self) -> Timing {
        self.timing
    }

    /// PPU dots for the current CPU cycle, spreading the fractional remainder.
    fn next_cpu_dots(&mut self) -> u32 {
        self.ppu_credit += self.timing.cpu_divider;
        let mut dots = 0u32;
        while self.ppu_credit >= self.timing.ppu_divider {
            self.ppu_credit -= self.timing.ppu_divider;
            dots += 1;
        }
        dots
    }

    /// Executes one CPU instruction (including any DMA stalls it triggers)
    /// and returns the cycles consumed.
    ///
    /// Intended for debuggers and single-step tracing; ordinary use should
    /// call [`System::run_frame`].
    pub fn step_instruction(&mut self) -> u32 {
        let mut cpu = self.cpu;
        let cycles = cpu.step(self);
        self.cpu = cpu;
        cycles.max(1)
    }

    /// A one-line CPU/PPU/APU state summary for debugger output.
    #[must_use]
    pub fn debug_state(&self) -> String {
        let r = &self.cpu.registers;
        format!(
            "frame={} pc={:04X} a={:02X} x={:02X} y={:02X} sp={:02X} p={:02X} | ppu sl={} dot={} ctrl={:02X} mask={:02X} v={:04X} t={:04X} | apu cyc={}",
            self.ppu.frames,
            r.pc,
            r.a,
            r.x,
            r.y,
            r.sp,
            r.p.bits(),
            self.ppu.scanline,
            self.ppu.dot,
            self.ppu.ctrl,
            self.ppu.mask,
            self.ppu.v,
            self.ppu.t,
            self.apu.cycles(),
        )
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
        if let Some(banks) = self.cart.chr_banks() {
            self.ppu.set_mmc2_banks(banks);
        } else if !self.cart.chr_is_ram() {
            for address in 0..0x2000u16 {
                self.ppu.chr[usize::from(address)] = self.cart.ppu_read(address);
            }
        }
        let mirroring = self.cart.mirroring();
        if self.ppu.mirroring != mirroring {
            self.ppu.mirroring = mirroring;
        }
        self.ppu
            .set_rom_nametable_banks(self.cart.rom_nametable_banks());
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

    /// Samples the Zapper light sensor as the PPU beam sweeps its 8x8 area.
    fn sample_zapper(&mut self) {
        if self.ppu.frames != self.zapper_frame {
            self.zapper_frame = self.ppu.frames;
            for zapper in &mut self.zapper {
                zapper.light = false;
            }
        }
        let scanline = self.ppu.scanline;
        let dot = self.ppu.dot;
        for port in 0..2 {
            let zapper = self.zapper[port];
            if self.devices[port] != PortDevice::Zapper || zapper.light {
                continue;
            }
            if scanline >= u16::from(zapper.y)
                && scanline < u16::from(zapper.y) + 8
                && dot >= u16::from(zapper.x)
                && dot < u16::from(zapper.x) + 8
            {
                let index = (usize::from(scanline) * 256 + usize::from(dot)) * 4;
                if let Some(pixel) = self.frame.get(index..index + 3) {
                    let luma =
                        (u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2])) / 3;
                    if luma > 0x60 {
                        self.zapper[port].light = true;
                    }
                }
            }
        }
    }

    fn read_controller(&mut self, port: usize) -> u8 {
        if self.devices[port] == PortDevice::Zapper {
            // Zapper: bit 4 = trigger (0 pulled), bit 3 = light (0 detected).
            // The other pins float high; the shift register is unused.
            let mut value = (self.open_bus & 0xE0) | 0x40;
            if !self.zapper[port].trigger {
                value |= 0x10;
            }
            if !self.zapper[port].light {
                value |= 0x08;
            }
            return value;
        }
        let bit = if self.shift[port] & 1 != 0 { 0x01 } else { 0 };
        let value = (self.open_bus & 0xE0) | 0x40 | bit;
        if !self.strobe {
            self.shift[port] = (self.shift[port] >> 1) | 0x80;
        }
        value
    }

    fn clock(&mut self) {
        self.end_dots = 0;
        let dots = self.next_cpu_dots();
        let before = self.ppu.scanline;
        self.ppu.step_dots(dots);
        self.sample_zapper();
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
        self.audio_accum += self.apu.output + self.cart.audio_mix();
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
        self.dma_event = false;
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

        if self.dmc_carry != 0 {
            // The OAM transfer ended while a DMC fetch was in flight, so the
            // halt (and possibly the dummy) already ran against OAM cycles.
            // The fetch continues on this CPU cycle, which the top of this
            // function already advanced; only additional cycles cost ticks.
            let mut phase = !self.phase;
            let mut dummy = self.dmc_carry == 2;
            let mut extra = 0u32;
            loop {
                if phase {
                    // Put cycle: alignment/dummy read of the CPU address.
                    dummy = true;
                    if let Some(addr) = address {
                        let value = self.dma_read(addr);
                        if addr != 0x4015 {
                            self.open_bus = value;
                        }
                    }
                } else if dummy {
                    // Get cycle: the real sample read.
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
                    // Get cycle before the dummy: consume it as the dummy.
                    dummy = true;
                    if let Some(addr) = address {
                        let value = self.dma_read(addr);
                        if addr != 0x4015 {
                            self.open_bus = value;
                        }
                    }
                }
                self.clock();
                extra += 1;
                phase = !phase;
            }
            self.dmc_carry = 0;
            self.stall_cycles += extra;
            self.dma_event = true;
            return;
        }

        if self.apu.take_dmc_disable_event() && self.apu.dmc_request_pending() {
            // A stop landed in the abort window. A fetch that has not reached
            // its halt cycle yet is dropped without any cycle cost on a write;
            // on a read cycle it still halts once, then gives up.
            self.apu.take_dmc_request();
            if write.is_none() {
                if let Some(addr) = address {
                    let value = self.dma_read(addr);
                    if addr != 0x4015 {
                        self.open_bus = value;
                    }
                }
                // The halt cycle is the whole transfer: one stolen cycle,
                // after which the fetch is abandoned.
                self.clock();
                self.stall_cycles += 1;
                self.dma_event = true;
                return;
            }
        }

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
            let halt_dots = self.next_cpu_dots();
            self.advance_ppu_cycle(halt_dots);
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
            self.dma_event = true;
            self.stall_cycles += extra;
            return;
        }

        // Normal CPU cycle: two PPU dots before the bus access, the remainder
        // after it (one NTSC, one or two PAL/Dendy).
        let dots = self.next_cpu_dots();
        let before = dots.min(2);
        self.advance_ppu_cycle(before);
        self.end_dots = (dots - before) as u8;
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
                // A DMC fetch may run at the same time. Its halt and dummy
                // cycles are no-operation cycles, so the OAM transfer keeps
                // going through them; only the DMC get displaces an OAM read.
                let phase = phase_now;
                let mut dmc = dma.dmc_state;
                // A new request performs its halt cycle during this OAM cycle;
                // the dummy cycle is the next one. Both are no-ops for the
                // sprite transfer, and only the get displaces an OAM read.
                let halt_now = dmc == 0 && self.apu.dmc_needs_sample();

                let mut ended = false;
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
                            ended = true;
                        }
                    } else if let Some(address) = dma.cpu_address {
                        let value = self.dma_read(address);
                        self.open_bus = value;
                    }
                } else if dmc == 2 {
                    // The DMC read takes priority; the OAM transfer stays
                    // paused and picks up again after an alignment cycle.
                    let address = self.apu.dmc.address;
                    let byte = self.dma_read(address);
                    if address != 0x4015 {
                        self.open_bus = byte;
                    }
                    self.apu.dmc_sample(byte);
                    dmc = 0;
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

                if halt_now {
                    // The halt cycle has now run; the dummy is next.
                    dmc = 1;
                } else if dmc == 1 {
                    // The dummy cycle has now run; the read is next.
                    dmc = 2;
                }
                dma.dmc_state = dmc;
                if ended && dmc != 0 {
                    // The OAM transfer finished first; the fetch continues
                    // from the CPU side with its halt/dummy work already done.
                    self.dmc_carry = dmc;
                    dma.dmc_state = 0;
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
                } else if self
                    .cart
                    .low_window()
                    .is_some_and(|(lo, hi)| (lo..=hi).contains(&address))
                {
                    // A mapper PRG-RAM board (e.g. Waixing FS303 at $5000).
                    self.cart.cpu_read(address)
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
                self.ram[usize::from(address) & 0x07FF] = value;
            }
            0x2000..=0x3FFF => {
                self.ppu.write_register(address, value);
                if address == 0x2000 {
                    self.cart.set_ppu_ctrl(value);
                }
            }
            0x4000..=0x4013 | 0x4015 | 0x4017 => {
                // `self.phase` has already toggled to the next cycle, so the
                // write cycle is a get when it is set.
                self.apu.write_get = Some(self.phase);
                self.apu.write(address, value);
            }
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
                    dmc_state: 0,
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

    fn dma_event(&self) -> bool {
        self.dma_event
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

    #[test]
    fn mmc5_split_screen_renders_exram_region() {
        let prg_len = 128 * 1024;
        let chr_len = 64 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 8;
        rom[6] = 0x50; // Mapper 5.
                       // PRG: NOPs with a JMP $8000 loop at the reset vector.
        for byte in &mut rom[16..16 + prg_len] {
            *byte = 0xEA;
        }
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        let vector = 16 + prg_len - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        // CHR: blank except 4 KiB bank 2, which is solid so the split bank is
        // visible. Bank 2 covers $5202=2 for the split region.
        rom[16 + prg_len..].fill(0x00);
        let split_bank = 16 + prg_len + 0x2000;
        rom[split_bank..split_bank + 0x1000].fill(0xFF);
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 5);

        // Left split, threshold 16 tiles, ExRAM nametable rows of tile $10.
        CpuBus::write(&mut system, 0x5104, 0);
        CpuBus::write(&mut system, 0x5105, 0);
        for i in 0..32u16 {
            CpuBus::write(&mut system, 0x5C00 + i, 0x10);
        }
        CpuBus::write(&mut system, 0x5200, 0x80 | 0x10);
        CpuBus::write(&mut system, 0x5201, 0);
        CpuBus::write(&mut system, 0x5202, 2);

        // Rendering on, background only, no scrolling.
        system.ppu.ctrl = 0;
        system.ppu.mask = 0x0A;
        system.ppu.v = 0;
        system.ppu.fine_x = 0;
        system.ppu.scanline = 0;
        system.ppu.dot = 0;
        system.ppu.debug_write_vram(0x3F00, 0x0F);
        system.ppu.debug_write_vram(0x3F03, 0x30);
        for _ in 0..256 {
            system.ppu.step_dot();
        }
        // x=40 is inside the left split, x=200 is the normal nametable. The
        // split tile fetches CHR bank 2 (solid), so it selects palette entry 1
        // while the normal tile is the backdrop.
        let split = system.ppu.debug_pixel(40, 0);
        let normal = system.ppu.debug_pixel(200, 0);
        assert_eq!(split, system.ppu.debug_palette(3), "split region wrong");
        assert_eq!(normal, system.ppu.debug_palette(0), "normal region wrong");
    }

    #[test]
    fn vrc7_chr_ram_banks_are_shared_with_the_ppu() {
        let prg_len = 128 * 1024;
        let mut rom = vec![0u8; 16 + prg_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 0; // CHR RAM.
        rom[6] = 0x50;
        rom[7] = 0x58;
        rom[8] = 0x20; // NES 2.0, submapper 2, mapper high bits 0.
        for byte in &mut rom[16..] {
            *byte = 0xEA;
        }
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 85);
        assert!(system.cart.chr_is_ram());

        // Slot 0 bank 2, upload a tile, then switch to bank 0 and back.
        system.cart.cpu_write(0xA000, 2);
        system.ppu.debug_write_vram(0x0000, 0xAB);
        assert_eq!(system.cart.ppu_read(0x0000), 0xAB);
        system.cart.cpu_write(0xA000, 0);
        system.ppu.debug_write_vram(0x0000, 0xCD);
        assert_eq!(system.cart.ppu_read(0x0000), 0xCD);
        system.cart.cpu_write(0xA000, 2);
        assert_eq!(system.cart.ppu_read(0x0000), 0xAB);
    }

    #[test]
    fn dmc_dma_stealing_matches_the_reference_benchmark() {
        // A synthetic NROM that loops incrementing a 16-bit counter while the
        // DMC plays a looping one-byte sample at the fastest rate. The loop
        // count difference against the DMC-disabled variant is the number of
        // CPU iterations eaten by DMC cycle stealing.
        fn rom(enable: u8) -> Vec<u8> {
            let mut prg = vec![0xEAu8; 0x4000];
            let code = [
                0x78, 0xD8, 0xA2, 0xFF, 0x9A, 0xA9, 0x00, 0x85, 0x00, 0x85, 0x01, 0xA9, 0x4F, 0x8D,
                0x10, 0x40, 0xA9, 0x00, 0x8D, 0x11, 0x40, 0x8D, 0x12, 0x40, 0x8D, 0x13, 0x40, 0xA9,
                enable, 0x8D, 0x15, 0x40, 0xE6, 0x00, 0xD0, 0xFC, 0xE6, 0x01, 0x4C, 0x20, 0x80,
            ];
            prg[..code.len()].copy_from_slice(&code);
            prg[0x3FFA] = 0x00;
            prg[0x3FFB] = 0x80;
            prg[0x3FFC] = 0x00;
            prg[0x3FFD] = 0x80;
            prg[0x3FFE] = 0x00;
            prg[0x3FFF] = 0x80;
            let mut rom = vec![0u8; 16 + prg.len() + 0x2000];
            rom[0..4].copy_from_slice(b"NES\x1a");
            rom[4] = 1;
            rom[5] = 1;
            rom[16..16 + prg.len()].copy_from_slice(&prg);
            rom
        }

        let mut enabled = System::from_ines(&rom(0x10)).expect("load");
        let mut disabled = System::from_ines(&rom(0x00)).expect("load");
        for _ in 0..300 {
            enabled.run_frame();
            disabled.run_frame();
        }
        let count =
            |system: &System| u16::from(system.read_ram(0)) | (u16::from(system.read_ram(1)) << 8);
        let with_dmc = count(&enabled);
        let without_dmc = count(&disabled);
        assert_eq!(without_dmc, 64388);
        assert_eq!(with_dmc, 54262);
        // The reference steals the same 10,126 loop iterations over 300
        // frames (the constant two-iteration start-up offset is present with
        // and without DMC, so it is not DMA-related).
        assert_eq!(without_dmc - with_dmc, 10126);
    }

    #[test]
    fn mmc5_eight_by_sixteen_background_banks_are_separate() {
        let prg_len = 128 * 1024;
        let chr_len = 64 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 8;
        rom[6] = 0x50;
        for (bank, chunk) in rom[16 + prg_len..].chunks_mut(0x400).enumerate() {
            chunk.fill(0x80 | bank as u8);
        }
        let mut system = System::from_ines(&rom).expect("load");
        system.cart.cpu_write(0x5120, 1); // sprite slot 0
        system.cart.cpu_write(0x5128, 2); // background slot 0 in 8x16 mode
        system.cart.set_ppu_ctrl(0x20);
        assert_eq!(system.ppu.debug_chr_byte(0x0000), 0x81);
        assert_eq!(system.ppu.debug_bg_chr_byte(0x0000), 0x82);
    }

    #[test]
    fn mmc5_extended_attributes_select_palette_and_chr_bank() {
        let prg_len = 128 * 1024;
        let chr_len = 64 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 8;
        rom[6] = 0x50; // Mapper 5.
        for byte in &mut rom[16..16 + prg_len] {
            *byte = 0xEA;
        }
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        let vector = 16 + prg_len - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        // CHR: 4 KiB bank 0 blank, bank 1 solid so the ExRAM bank is visible.
        rom[16 + prg_len..].fill(0x00);
        let bank1 = 16 + prg_len + 0x1000;
        rom[bank1..bank1 + 0x1000].fill(0xFF);

        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 5);

        // Extended-attribute mode; the whole first row selects bank 1 and
        // palette 1.
        CpuBus::write(&mut system, 0x5104, 1);
        CpuBus::write(&mut system, 0x5105, 0);
        for i in 0..32u16 {
            CpuBus::write(&mut system, 0x5C00 + i, 0x41);
        }
        system.ppu.debug_write_vram(0x2000, 0x10);

        system.ppu.ctrl = 0;
        system.ppu.mask = 0x0A;
        system.ppu.v = 0;
        system.ppu.fine_x = 0;
        system.ppu.scanline = 0;
        system.ppu.dot = 0;
        system.ppu.debug_write_vram(0x3F00, 0x0F);
        system.ppu.debug_write_vram(0x3F03, 0x2D);
        system.ppu.debug_write_vram(0x3F07, 0x30);
        for _ in 0..256 {
            system.ppu.step_dot();
        }
        let pixel = system.ppu.debug_pixel(40, 0);
        assert_eq!(pixel, system.ppu.debug_palette(7));
        assert_ne!(pixel, system.ppu.debug_palette(3));
    }

    #[test]
    fn mmc5_exram_nametables_fill_and_chr_map_reach_the_ppu() {
        let prg_len = 128 * 1024;
        let chr_len = 64 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 8;
        rom[6] = 0x50; // Mapper 5.
        for (bank, chunk) in rom[16 + prg_len..].chunks_mut(0x400).enumerate() {
            chunk.fill(0x80 | bank as u8);
        }
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 5);

        // ExRAM-backed nametable in mode 2, mapped to slot 0.
        CpuBus::write(&mut system, 0x5104, 2);
        CpuBus::write(&mut system, 0x5105, 0x02);
        CpuBus::write(&mut system, 0x5C00, 0x5A);
        assert_eq!(system.ppu.debug_read_vram(0x2000), 0x5A);
        system.ppu.debug_write_vram(0x2000, 0x33);
        assert_eq!(system.cart.cpu_read(0x5C00), 0x33);

        // Fill mode: tile bytes return $5106, attributes $5107.
        CpuBus::write(&mut system, 0x5105, 0x03);
        CpuBus::write(&mut system, 0x5106, 0x42);
        CpuBus::write(&mut system, 0x5107, 0x05);
        assert_eq!(system.ppu.debug_read_vram(0x2000), 0x42);
        assert_eq!(system.ppu.debug_read_vram(0x23C0), 0x05);

        // 1 KiB CHR map, updated mid-frame by a register write.
        CpuBus::write(&mut system, 0x5101, 3);
        CpuBus::write(&mut system, 0x5120, 5);
        assert_eq!(system.ppu.debug_chr_byte(0x0000), 0x85);
        CpuBus::write(&mut system, 0x5120, 7);
        assert_eq!(system.ppu.debug_chr_byte(0x0000), 0x87);
    }

    #[test]
    fn waixing195_low_ram_is_cpu_visible() {
        let prg_len = 128 * 1024;
        let chr_len = 64 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 8;
        rom[6] = 0x30; // Mapper 195 low nibble 3.
        rom[7] = 0xC0; // Mapper 195 high nibble 12.
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 195);
        assert_eq!(system.cart.low_window(), Some((0x5000, 0x5FFF)));
        CpuBus::write(&mut system, 0x5B00, 0xEA);
        assert_eq!(CpuBus::read(&mut system, 0x5B00), 0xEA);
    }

    #[test]
    fn sunsoft4_rom_nametables_reach_the_ppu() {
        let prg_len = 128 * 1024;
        let chr_len = 64 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8; // 128 KiB PRG.
        rom[5] = 8; // 64 KiB CHR.
        rom[6] = 0x40; // Mapper 68 low nibble.
        rom[7] = 0x40; // Mapper 68 high nibble.
        for (bank, chunk) in rom[16 + prg_len..].chunks_mut(0x400).enumerate() {
            chunk.fill(bank as u8);
        }
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 68);
        assert!(system.cart.has_rom_nametables());

        system.cart.cpu_write(0xC000, 3);
        system.cart.cpu_write(0xD000, 5);
        system.cart.cpu_write(0xE000, 0x10); // ROM nametables, vertical.
        system.sync_cartridge();
        assert_eq!(system.ppu.debug_read_vram(0x2000), 3);
        assert_eq!(system.ppu.debug_read_vram(0x2400), 5);

        system.cart.cpu_write(0xE000, 0x11); // Horizontal.
        system.sync_cartridge();
        assert_eq!(system.ppu.debug_read_vram(0x2400), 3);
        assert_eq!(system.ppu.debug_read_vram(0x2800), 5);
    }

    #[test]
    fn mmc2_chr_banks_reach_the_ppu() {
        let prg_len = 8 * 16 * 1024;
        let chr_len = 8 * 8 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8; // 128 KiB PRG.
        rom[5] = 8; // 64 KiB CHR.
        rom[6] = 9 << 4; // Mapper 9.
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 9);
        assert!(system.ppu.has_mmc2_chr());
        assert_eq!(system.ppu.mmc2_banks(), Some([0, 0, 0, 0]));

        system.cart.cpu_write(0xB000, 3);
        system.cart.cpu_write(0xD000, 9);
        system.sync_cartridge();
        assert_eq!(system.ppu.mmc2_banks(), Some([3, 0, 9, 0]));
    }

    #[test]
    fn region_selects_timing_geometry() {
        use crate::clock::{Region, Timing};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
        let path = root.join("nestest.nes");
        if !path.exists() {
            eprintln!("skipping: test-roms/nestest.nes missing");
            return;
        }
        let rom = fs::read(path).expect("rom");
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.timing(), Timing::NTSC);
        assert_eq!(system.ppu.timing.scanlines, 262);

        system.set_region(Region::Pal);
        assert_eq!(system.timing(), Timing::PAL);
        assert_eq!(system.ppu.timing.scanlines, 312);
        assert_eq!(system.ppu.timing.dots_per_line, 341);
        assert!((system.timing().frame_rate() - 50.007).abs() < 0.01);

        system.set_region(Region::Dendy);
        assert_eq!(system.timing(), Timing::DENDY);
        assert_eq!(system.ppu.timing.scanlines, 312);
        assert_eq!(system.timing().cpu_hz(), 1_773_447);

        system.set_region(Region::Ntsc);
        assert_eq!(system.ppu.timing.scanlines, 262);
    }

    #[test]
    fn zapper_register_reports_trigger_and_light() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
        let path = root.join("nestest.nes");
        if !path.exists() {
            eprintln!("skipping: test-roms/nestest.nes missing");
            return;
        }
        let rom = fs::read(path).expect("rom");
        let mut system = System::from_ines(&rom).expect("load");

        system.set_port_device(1, PortDevice::Zapper);
        system.set_zapper(1, 100, 100, false);
        let released = CpuBus::read(&mut system, 0x4017);
        assert_eq!(
            released & 0x18,
            0x18,
            "released trigger and no light leave both bits high"
        );

        system.set_zapper(1, 100, 100, true);
        let pulled = CpuBus::read(&mut system, 0x4017);
        assert_eq!(pulled & 0x10, 0x00, "pulled trigger pulls bit 4 low");
        assert_eq!(pulled & 0x08, 0x08, "no light sensed leaves bit 3 high");

        // Switching the port back to a gamepad restores the shift register.
        system.set_port_device(1, PortDevice::Gamepad);
        system.set_controller(1, 0x01);
        CpuBus::write(&mut system, 0x4016, 1);
        CpuBus::write(&mut system, 0x4016, 0);
        let pad = CpuBus::read(&mut system, 0x4017);
        assert_eq!(pad & 0x01, 0x01, "gamepad serial bit is read from the port");
    }
}
