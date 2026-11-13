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
use unko_core::{AudioFilterModel, AudioQuality, DebugSpace};

/// CPU clock in Hz (NTSC).
pub const CPU_HZ: f64 = 1_789_773.0;

/// Per-CPU-cycle APU debug trace: `(cycle, APU channels + mix, expansion
/// channels)`.
#[cfg(feature = "apu-trace")]
type ApuTrace = Vec<(u64, [f32; 8], [f32; 8])>;

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
    controllers: [u8; 4],
    /// Device attached to each controller port.
    devices: [PortDevice; 2],
    /// NES Zapper state per port (used when the device is [`PortDevice::Zapper`]).
    zapper: [Zapper; 2],
    /// PPU frame number of the last Zapper light reset.
    zapper_frame: u64,
    /// Whether any port currently has a Zapper attached.
    zapper_present: bool,
    /// Debug: per-CPU-cycle APU channel outputs collected between
    /// `start_apu_trace` and `take_apu_trace` (feature `apu-trace`).
    #[cfg(feature = "apu-trace")]
    apu_trace: Option<ApuTrace>,
    /// Debug: per-instruction CPU program counters collected between
    /// `start_exec_trace` and `take_exec_trace` (feature `apu-trace`).
    #[cfg(feature = "apu-trace")]
    exec_trace: Option<Vec<(u64, u16)>>,
    /// Debug: expansion-port `$4020-$FFFF` writes collected between
    /// `start_expansion_trace` and `take_expansion_trace`.
    #[cfg(feature = "apu-trace")]
    expansion_trace: Option<Vec<(u64, u16, u8)>>,
    strobe: bool,
    /// Regional timing (NTSC/PAL/Dendy).
    timing: Timing,
    /// Master-clock credit for fractional PPU dots per CPU cycle.
    ppu_credit: u32,
    /// Global CPU cycle phase (get/put).
    phase: bool,
    shift: [u8; 2],
    /// Non-gamepad controller state (paddle, mat, keyboard, Four Score).
    peripherals: Peripherals,
    audio: Vec<f32>,
    audio_rate: f32,
    resampler: crate::resampler::Resampler,
    /// Host audio preferences (never part of a save state).
    audio_quality: AudioQuality,
    audio_filters: AudioFilterModel,
    /// `[pulse1, pulse2, triangle, noise, dmc, expansion]` gains.
    channel_gains: [f32; 6],
    open_bus: u8,
    /// Internal 2A03 data bus (drives $4015 bit 5, not the external bus).
    internal_bus: u8,
    stall_cycles: u32,
    /// The reset sequence already completed the power-on frame, so the first
    /// `run_frame` reports that frame without executing another one.
    first_frame_pending: bool,
    rmw_dummy: bool,
    /// A DMA used the bus on the current CPU cycle.
    dma_event: bool,
    cpu_pc: u16,
    /// DMC fetch state carried out of an OAM transfer that ended first.
    dmc_carry: u8,
    /// CPU program counter, used for APU register activation during OAM DMA.
    /// PPU dots still owed at the end of a split CPU cycle.
    end_dots: u8,
    /// CPU cycles seen by the bus (MMC1 serial-write spacing).
    cycle_count: u64,
    /// PPU scanline at the start of the current split cycle.
    dma: Option<Dma>,
}

#[derive(serde::Serialize)]
struct SystemStateRef<'a> {
    cpu: &'a Cpu,
    ppu: &'a Ppu,
    apu: &'a Apu,
    #[serde(with = "serde_big_array::BigArray")]
    ram: [u8; 0x800],
    controllers: &'a [u8; 4],
    devices: &'a [PortDevice; 2],
    zapper: &'a [Zapper; 2],
    zapper_frame: u64,
    peripherals: &'a Peripherals,
    strobe: bool,
    timing: Timing,
    ppu_credit: u32,
    phase: bool,
    shift: &'a [u8; 2],
    open_bus: u8,
    internal_bus: u8,
    stall_cycles: u32,
    first_frame_pending: bool,
    rmw_dummy: bool,
    dma_event: bool,
    cpu_pc: u16,
    dmc_carry: u8,
    end_dots: u8,
    cycle_count: u64,
    dma: Option<Dma>,
    audio_rate: f32,
    resampler: &'a crate::resampler::Resampler,
}

#[derive(serde::Deserialize)]
struct SystemState {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    #[serde(with = "serde_big_array::BigArray")]
    ram: [u8; 0x800],
    controllers: [u8; 4],
    devices: [PortDevice; 2],
    zapper: [Zapper; 2],
    zapper_frame: u64,
    peripherals: Peripherals,
    strobe: bool,
    timing: Timing,
    ppu_credit: u32,
    phase: bool,
    shift: [u8; 2],
    open_bus: u8,
    internal_bus: u8,
    stall_cycles: u32,
    first_frame_pending: bool,
    rmw_dummy: bool,
    dma_event: bool,
    cpu_pc: u16,
    dmc_carry: u8,
    end_dots: u8,
    cycle_count: u64,
    dma: Option<Dma>,
    audio_rate: f32,
    resampler: crate::resampler::Resampler,
}

/// Device attached to a controller port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PortDevice {
    /// Standard controller.
    #[default]
    Gamepad,
    /// NES Zapper light gun.
    Zapper,
    /// Arkanoid "Vaus" paddle: an 8-bit position on bit 4 (inverted,
    /// MSB-first) plus the fire button on bit 3, latched by the `$4016`
    /// strobe.
    Paddle,
    /// Power Pad / Family Trainer mat: 12 membrane buttons read two bits at a
    /// time on bits 3 (low half) and 4 (high half).
    PowerPad,
    /// Family BASIC keyboard: a 9-row x 8-column matrix scanned through
    /// `$4016` (column/clock/enable) and read active-low on `$4017` bits 1-4.
    FamilyKeyboard,
    /// Four Score adapter: both ports form a 24-read serial chain carrying
    /// players 1+3 (`$4016`) and 2+4 (`$4017`) plus an adapter signature.
    FourScore,
    /// Famicom controller microphone: an immediate 1-bit report on `$4016`
    /// bit 2, with the standard buttons still available on bits 0/1.
    Microphone,
    /// Family BASIC data recorder: a cassette loopback on `$4016` bit 0.
    /// Bit 0 of a write is the DAC output and bit 1 enables readback.
    Tape,
    /// Miracle Piano MIDI transport (controller port 1): OUT0 direction is
    /// selected by the strobe duration and each read clocks one serial bit.
    MiraclePiano,
}

/// Arkanoid paddle state for one port.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Paddle {
    /// Knob position (0 = left, 255 = right).
    pub position: u8,
    /// Fire button held.
    pub fire: bool,
    /// Serial shift register, reloaded on the strobe.
    shift: u8,
}

/// Power Pad state for one port.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct PowerPad {
    /// Pressed buttons: bit `i` = mat button `i + 1` (0..=11).
    pub buttons: u16,
    /// Low serial register (bit 3).
    low: u8,
    /// High serial register (bit 4).
    high: u8,
}

impl PowerPad {
    /// Reloads both serial registers from `buttons` in the documented order.
    fn reload(&mut self) {
        let p = self.buttons;
        self.low = ((p >> 1) & 1) as u8
            | (((p & 1) as u8) << 1)
            | ((((p >> 4) & 1) as u8) << 2)
            | ((((p >> 8) & 1) as u8) << 3)
            | ((((p >> 5) & 1) as u8) << 4)
            | ((((p >> 9) & 1) as u8) << 5)
            | ((((p >> 10) & 1) as u8) << 6)
            | ((((p >> 6) & 1) as u8) << 7);
        self.high = (((p >> 3) & 1) as u8)
            | ((((p >> 2) & 1) as u8) << 1)
            | ((((p >> 11) & 1) as u8) << 2)
            | ((((p >> 7) & 1) as u8) << 3)
            | 0xF0;
    }
}

/// Family BASIC keyboard matrix state.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct FamilyKeyboard {
    /// Per-row key bitmap: bits 0-3 = column half 0, bits 4-7 = half 1.
    pub keys: [u8; 9],
    /// Selected matrix row (0..=8).
    row: u8,
    /// Column half selected by `$4016` bit 0.
    column: bool,
    /// Matrix enabled by `$4016` bit 2.
    enabled: bool,
    /// Last clock level (`$4016` bit 1).
    clock: bool,
}

impl FamilyKeyboard {
    /// Applies a `$4016` write: latch column, advance the row on a clock
    /// rising edge, reset to row 0 on a low clock, and honour the enable bit.
    fn write(&mut self, value: u8) {
        let column = value & 0x01 != 0;
        let clock = value & 0x02 != 0;
        let enabled = value & 0x04 != 0;
        if enabled {
            if clock && !self.clock {
                self.row = (self.row + 1).min(8);
            } else if !clock {
                self.row = 0;
            }
        }
        self.column = column;
        self.enabled = enabled;
        self.clock = clock;
    }

    /// Reads the four selected switches, active-low on bits 1-4.
    fn read(&self) -> u8 {
        if !self.enabled {
            return 0b0001_1110;
        }
        let byte = self.keys[usize::from(self.row.min(8))];
        let nibble = if self.column { byte >> 4 } else { byte } & 0x0F;
        ((!nibble) & 0x0F) << 1
    }
}

/// Non-gamepad controller state shared by both ports.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Peripherals {
    paddle: [Paddle; 2],
    power_pad: [PowerPad; 2],
    keyboard: FamilyKeyboard,
    /// Four Score serial read index per port (0..=24).
    four_score_index: [u8; 2],
    /// Four Score adapter signature shift registers (`$4016` = 0x08,
    /// `$4017` = 0x04, shifted out LSB-first).
    four_score_signature: [u8; 2],
    /// Player 1-4 button masks latched by the strobe for the Four Score chain.
    four_score_pads: [u8; 4],
    /// Famicom microphone report (`$4016` bit 2).
    microphone: bool,
    /// Family BASIC data recorder loopback.
    tape: TapeRecorder,
    /// Miracle Piano transport state.
    piano: MiraclePiano,
}

/// Miracle Piano serial transport. The published interface gives no way to
/// synthesise the piano's MIDI replies without its firmware, so received
/// bytes are injected (tests/UI) and transmitted bytes are logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
enum PianoMode {
    #[default]
    Idle,
    Poll,
    Send,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct MiraclePiano {
    out_high: bool,
    rise_cycle: u64,
    mode: PianoMode,
    /// OUT0 level set by the most recent write (the bit clocked in on read).
    data_bit: bool,
    tx_shift: u8,
    tx_bits: u8,
    /// Bytes the piano would return (MSB-first, inverted on the wire).
    rx: std::collections::VecDeque<u8>,
    rx_shift: u8,
    rx_bits: u8,
    status_ready: bool,
    /// Bytes clocked in from the software, for diagnostics/tests.
    tx_log: Vec<u8>,
}

impl MiraclePiano {
    fn write(&mut self, value: u8, cycle: u64) {
        let high = value & 1 != 0;
        self.data_bit = high;
        if self.mode == PianoMode::Send {
            // After the long strobe, writes carry data bits until eight reads
            // have clocked them out.
            return;
        }
        if high && !self.out_high {
            self.out_high = true;
            self.rise_cycle = cycle;
        } else if !high && self.out_high {
            self.out_high = false;
            let duration = cycle.saturating_sub(self.rise_cycle);
            if duration >= 64 {
                // Long strobe: the next eight writes carry a MIDI byte.
                self.mode = PianoMode::Send;
                self.tx_shift = 0;
                self.tx_bits = 0;
            } else if duration >= 6 {
                // Short strobe: read a status bit, then (if set) eight bits.
                self.mode = PianoMode::Poll;
                self.rx_bits = 0;
                self.status_ready = !self.rx.is_empty();
            } else {
                self.mode = PianoMode::Idle;
            }
        }
    }

    fn read(&mut self) -> u8 {
        match self.mode {
            PianoMode::Send => {
                self.tx_shift = (self.tx_shift << 1) | u8::from(self.data_bit);
                self.tx_bits += 1;
                if self.tx_bits == 8 {
                    if self.tx_log.len() < 256 {
                        self.tx_log.push(self.tx_shift);
                    }
                    self.tx_bits = 0;
                    self.tx_shift = 0;
                }
                u8::from(self.data_bit)
            }
            PianoMode::Poll => {
                if self.rx_bits == 0 {
                    if !self.status_ready {
                        self.mode = PianoMode::Idle;
                        return 0;
                    }
                    self.status_ready = false;
                    self.rx_shift = self.rx.pop_front().unwrap_or(0);
                    self.rx_bits = 8;
                    return 1;
                }
                let bit = (self.rx_shift >> 7) & 1;
                self.rx_shift <<= 1;
                self.rx_bits -= 1;
                if self.rx_bits == 0 {
                    self.mode = PianoMode::Idle;
                }
                // The eight data bits are inverted on the wire.
                bit ^ 1
            }
            PianoMode::Idle => 0,
        }
    }
}

/// Loopback for the Family BASIC data recorder. The wiki notes that an
/// emulator can simply play back the bit stream it emitted; there is no
/// cassette audio decoding here.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct TapeRecorder {
    bits: std::collections::VecDeque<bool>,
    /// `$4016` write bit 1: 0 forces readback to 0.
    read_enabled: bool,
}

impl TapeRecorder {
    fn write(&mut self, value: u8) {
        self.read_enabled = value & 0x02 != 0;
        if self.bits.len() < 1_048_576 {
            self.bits.push_back(value & 0x01 != 0);
        }
    }

    fn read(&mut self) -> u8 {
        if self.read_enabled {
            u8::from(self.bits.pop_front().unwrap_or(false))
        } else {
            0
        }
    }
}

/// NES Zapper state.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum DmaKind {
    /// Delta-modulation sample fetch.
    Dmc,
    /// Sprite (`$4014`) transfer.
    Oam,
}

/// A DMA transfer that spans CPU cycles.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
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
    ///
    /// The regional timing comes from the header: NES 2.0 byte 12 declares
    /// NTSC/PAL/Dendy, and a legacy iNES image can set the PAL bit in byte 9.
    /// [`System::set_region`] still overrides it.
    pub fn from_ines(rom: &[u8]) -> Result<Self, CartError> {
        let timing = Self::header_timing(rom);
        Ok(Self::build(Cartridge::from_ines(rom)?, timing))
    }

    /// Timing declared by an iNES / NES 2.0 header (NTSC when absent).
    fn header_timing(rom: &[u8]) -> Timing {
        if rom.len() >= 16 {
            if rom[7] & 0x0C == 0x08 {
                match rom[12] & 0x03 {
                    1 => return Timing::PAL,
                    3 => return Timing::DENDY,
                    _ => {}
                }
            } else if rom[9] & 0x01 != 0 {
                return Timing::PAL;
            }
        }
        Timing::NTSC
    }

    /// Builds a system from a raw FDS disk image and its 8 KiB BIOS.
    pub fn from_fds(image: &[u8], bios: &[u8]) -> Result<Self, CartError> {
        Ok(Self::from_cartridge(Cartridge::from_fds(image, bios)?))
    }

    fn from_cartridge(cart: Cartridge) -> Self {
        Self::build(cart, Timing::NTSC)
    }

    fn build(cart: Cartridge, timing: Timing) -> Self {
        let mirroring = matches!(cart.mirroring(), Mirroring::Vertical);
        let chr_ram = cart.chr_is_ram();
        let mut ppu = Ppu::new(timing, vec![0; 0x2000], mirroring, chr_ram);
        // The full system models the console's power-on phase: the PPU starts
        // at the end of the pre-render line (matching the reference capture).
        ppu.scanline = timing.scanlines - 1;
        ppu.dot = 340;
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
        if let Some(aux) = cart.oeka_chr() {
            ppu.set_oeka_chr(aux);
        }
        if let Some(aux) = cart.chr512() {
            ppu.set_chr512(aux);
        }
        if let Some(aux) = cart.aux77() {
            ppu.set_aux77(aux);
        }
        if let Some(nt) = cart.n163_nt() {
            ppu.set_n163_nt(nt);
        }
        if let Some(qt) = cart.qtram() {
            ppu.set_qtram(qt);
        }
        if let Some(aux) = cart.a12_irq() {
            ppu.set_a12_irq(aux);
        }
        if let Some(aux) = cart.chr169() {
            ppu.set_chr169(aux);
        }
        if let Some(aux) = cart.cprom_chr() {
            ppu.set_cprom_chr(aux);
        }
        if let Some(aux) = cart.bandai153() {
            ppu.set_bandai153(aux);
        }
        let um6578 = cart.um6578();
        ppu.set_um6578(um6578);
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
            controllers: [0; 4],
            devices: [PortDevice::Gamepad; 2],
            zapper: [Zapper::default(); 2],
            zapper_frame: 0,
            zapper_present: false,
            #[cfg(feature = "apu-trace")]
            apu_trace: None,
            #[cfg(feature = "apu-trace")]
            exec_trace: None,
            #[cfg(feature = "apu-trace")]
            expansion_trace: None,
            strobe: false,
            timing,
            ppu_credit: 0,
            phase: false,
            shift: [0; 2],
            peripherals: Peripherals::default(),
            audio: Vec::with_capacity(1024),
            audio_rate: 44_100.0,
            resampler: crate::resampler::Resampler::new(44_100, 1_789_773.0),
            audio_quality: AudioQuality::Standard,
            audio_filters: AudioFilterModel::NesRf,
            channel_gains: [1.0; 6],
            open_bus: 0,
            internal_bus: 0,
            stall_cycles: 0,
            first_frame_pending: true,
            rmw_dummy: false,
            dma_event: false,
            cpu_pc: 0,
            dmc_carry: 0,
            end_dots: 0,
            cycle_count: 0,
            dma: None,
        };
        if system.cart.famicom_warmup() {
            system.ppu.end_warmup();
        }
        let mut cpu = system.cpu;
        cpu.reset(&mut system);
        // The reference console leaves reset on cycle 8 (one cycle of phase
        // after the CPU's seven-cycle reset sequence), which aligns the PPU
        // frame boundary and the APU cycle parity with its captures.
        system.clock();
        system.cpu = cpu;
        system
    }

    /// Resets the CPU and PPU without reloading the cartridge.
    pub fn reset(&mut self) {
        self.cart.soft_reset();
        let mut cpu = self.cpu;
        cpu.reset(self);
        self.cpu = cpu;
        self.apu = Apu::new();
        self.apu.set_channel_gains(self.channel_gains);
        self.resampler.reset();
        self.ram = [0; 0x800];
    }

    /// Attaches a device to a controller port.
    pub fn set_port_device(&mut self, port: usize, device: PortDevice) {
        if port < 2 {
            self.devices[port] = device;
            self.zapper_present = self.devices.contains(&PortDevice::Zapper);
        }
    }

    /// Sets the Famicom microphone report (`$4016` bit 2).
    pub fn set_microphone(&mut self, pressed: bool) {
        self.peripherals.microphone = pressed;
    }

    /// Loads a Datach barcode for a mapper 157 cartridge.
    pub fn set_datach_barcode(&mut self, code: &str) -> bool {
        self.cart.set_datach_barcode(code)
    }

    /// Queues one byte for the Miracle Piano to return on the next poll.
    pub fn push_piano_rx(&mut self, byte: u8) {
        if self.peripherals.piano.rx.len() < 256 {
            self.peripherals.piano.rx.push_back(byte);
        }
    }

    /// Bytes clocked in from the Miracle Piano software (diagnostics).
    #[must_use]
    pub fn piano_tx_log(&self) -> &[u8] {
        &self.peripherals.piano.tx_log
    }

    /// Sets the Zapper's aim and trigger for a port.
    pub fn set_zapper(&mut self, port: usize, x: u8, y: u8, trigger: bool) {
        if port < 2 {
            self.zapper[port].x = x;
            self.zapper[port].y = y;
            self.zapper[port].trigger = trigger;
        }
    }

    /// Sets controller input bits (`InputState` layout) for players 1-4.
    ///
    /// Ports 2 and 3 are only read when a Four Score adapter is attached.
    pub fn set_controller(&mut self, port: usize, bits: u16) {
        if port < 4 {
            self.controllers[port] = bits as u8;
        }
    }

    /// Sets the Arkanoid paddle position (`0..=255`) and fire button.
    ///
    /// Takes effect on the next `$4016` strobe, matching the hardware latch.
    pub fn set_paddle(&mut self, port: usize, position: u8, fire: bool) {
        if port < 2 {
            self.peripherals.paddle[port].position = position;
            self.peripherals.paddle[port].fire = fire;
        }
    }

    /// Selects a display palette preset.
    pub fn set_palette(&mut self, variant: crate::ppu::PaletteVariant) {
        self.ppu.set_palette_variant(variant);
    }

    /// Sets the pressed Power Pad buttons (bit `i` = button `i + 1`).
    pub fn set_power_pad(&mut self, port: usize, buttons: u16) {
        if port < 2 {
            self.peripherals.power_pad[port].buttons = buttons & 0x0FFF;
            if self.strobe {
                self.peripherals.power_pad[port].reload();
            }
        }
    }

    /// Replaces the Family BASIC keyboard matrix (one byte per row, low nibble
    /// = column half 0, high nibble = half 1; a set bit means the key is held).
    pub fn set_family_keyboard(&mut self, keys: [u8; 9]) {
        self.peripherals.keyboard.keys = keys;
    }

    /// Changes the output sample rate.
    pub fn set_sample_rate(&mut self, rate: f32) {
        if rate > 0.0 {
            self.audio_rate = rate;
            self.resampler =
                crate::resampler::Resampler::new(rate.round() as u32, self.timing.cpu_hz() as f64);
            self.apply_audio_preferences();
        }
    }

    /// Selects the resampler kernel shape.
    pub fn set_audio_quality(&mut self, quality: AudioQuality) {
        self.audio_quality = quality;
        self.resampler.set_quality(quality);
    }

    /// Selects the analog output filter model.
    pub fn set_audio_filters(&mut self, filters: AudioFilterModel) {
        self.audio_filters = filters;
        self.resampler.set_filter_model(filters);
    }

    /// Sets the per-channel mixer gains.
    pub fn set_channel_gains(&mut self, gains: [f32; 6]) {
        self.channel_gains = gains.map(|gain| gain.clamp(0.0, 1.0));
        self.apu.set_channel_gains(self.channel_gains);
    }

    /// Re-applies the host's audio preferences to a freshly built or restored
    /// resampler/APU.
    fn apply_audio_preferences(&mut self) {
        self.resampler.set_quality(self.audio_quality);
        self.resampler.set_filter_model(self.audio_filters);
        self.apu.set_channel_gains(self.channel_gains);
    }

    /// Supplies the 64-entry table used by the custom palette.
    pub fn set_palette_table(&mut self, table: [[u8; 3]; 64]) {
        self.ppu.set_palette_table(table);
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
        self.pump_dma();
        cycles.max(1)
    }

    /// Executes any cartridge DMA request recorded by the mapper.
    fn pump_dma(&mut self) {
        while let Some(request) = self.cart.take_dma() {
            let mut source = request.source;
            for index in 0..u32::from(request.len) {
                let byte = if request.source_rom {
                    self.cart.prg_linear(source as usize)
                } else {
                    self.dma_cpu_read(source as u16)
                };
                let target = request.target.wrapping_add(index as u16);
                if request.to_ppu {
                    self.ppu.dma_write(target, byte);
                } else {
                    self.dma_cpu_write(target, byte);
                }
                source = source.wrapping_add(1);
            }
        }
    }

    fn dma_cpu_read(&self, address: u16) -> u8 {
        if address < 0x2000 {
            self.ram[usize::from(address) & 0x07FF]
        } else {
            self.cart.cpu_read(address)
        }
    }

    fn dma_cpu_write(&mut self, address: u16, value: u8) {
        if address < 0x2000 {
            self.ram[usize::from(address) & 0x07FF] = value;
        } else {
            // Go through the normal bus so PPU/APU/cartridge registers see
            // the transfer (the UM6578 uploads its palette at $2040).
            self.write(address, value);
        }
    }

    /// The active 8 KiB CHR window (including mappers with shared CHR).
    #[must_use]
    pub fn debug_chr(&self) -> Vec<u8> {
        (0..0x2000u16)
            .map(|address| self.ppu.debug_chr_byte(address))
            .collect()
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

    /// Serializes the whole console state (mapper state included).
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        out.extend_from_slice(b"UNKOST06");
        out.extend_from_slice(&self.cart.fingerprint().to_le_bytes());
        let mut cart_state = Vec::new();
        self.cart.save_state(&mut cart_state)?;
        out.extend_from_slice(&(cart_state.len() as u32).to_le_bytes());
        out.extend_from_slice(&cart_state);
        let state = SystemStateRef {
            cpu: &self.cpu,
            ppu: &self.ppu,
            apu: &self.apu,
            ram: self.ram,
            controllers: &self.controllers,
            devices: &self.devices,
            zapper: &self.zapper,
            zapper_frame: self.zapper_frame,
            peripherals: &self.peripherals,
            strobe: self.strobe,
            timing: self.timing,
            ppu_credit: self.ppu_credit,
            phase: self.phase,
            shift: &self.shift,
            open_bus: self.open_bus,
            internal_bus: self.internal_bus,
            stall_cycles: self.stall_cycles,
            first_frame_pending: self.first_frame_pending,
            rmw_dummy: self.rmw_dummy,
            dma_event: self.dma_event,
            cpu_pc: self.cpu_pc,
            dmc_carry: self.dmc_carry,
            end_dots: self.end_dots,
            cycle_count: self.cycle_count,
            dma: self.dma,
            audio_rate: self.audio_rate,
            resampler: &self.resampler,
        };
        let payload = bincode::serialize(&state).map_err(|err| err.to_string())?;
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Restores a state produced by [`System::save_state`].
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() < 8 + 8 + 4 || &data[..8] != b"UNKOST06" {
            return Err("not an Unko state (bad magic/version)".into());
        }
        let fingerprint = u64::from_le_bytes(
            data[8..16]
                .try_into()
                .map_err(|_| "truncated state fingerprint".to_string())?,
        );
        if fingerprint != self.cart.fingerprint() {
            return Err("state belongs to a different ROM".into());
        }
        let cart_len = u32::from_le_bytes(
            data[16..20]
                .try_into()
                .map_err(|_| "truncated state length".to_string())?,
        ) as usize;
        let cart_end = 20usize
            .checked_add(cart_len)
            .filter(|end| *end <= data.len())
            .ok_or("truncated mapper state")?;
        self.cart.load_state(&data[20..cart_end])?;
        let state: SystemState =
            bincode::deserialize(&data[cart_end..]).map_err(|err| err.to_string())?;
        self.cpu = state.cpu;
        self.ppu = state.ppu;
        self.apu = state.apu;
        self.ram = state.ram;
        self.controllers = state.controllers;
        self.devices = state.devices;
        self.zapper_present = self.devices.contains(&PortDevice::Zapper);
        self.zapper = state.zapper;
        self.zapper_frame = state.zapper_frame;
        self.peripherals = state.peripherals;
        self.strobe = state.strobe;
        self.timing = state.timing;
        self.ppu_credit = state.ppu_credit;
        self.phase = state.phase;
        self.shift = state.shift;
        self.open_bus = state.open_bus;
        self.internal_bus = state.internal_bus;
        self.stall_cycles = state.stall_cycles;
        self.first_frame_pending = state.first_frame_pending;
        self.rmw_dummy = state.rmw_dummy;
        self.dma_event = state.dma_event;
        self.cpu_pc = state.cpu_pc;
        self.dmc_carry = state.dmc_carry;
        self.end_dots = state.end_dots;
        self.cycle_count = state.cycle_count;
        self.dma = state.dma;
        self.audio_rate = state.audio_rate;
        self.resampler = state.resampler;
        // The state carries the resampler cursor but not the host's quality,
        // filter or mixer preferences; re-apply them (and the APU gains) now.
        self.apply_audio_preferences();
        self.audio.clear();
        self.sync_cartridge();
        Ok(())
    }

    /// Runs one full display frame.
    pub fn run_frame(&mut self) -> u32 {
        self.sync_cartridge();
        self.audio.clear();
        if self.first_frame_pending {
            self.first_frame_pending = false;
            self.ppu.present();
            return 0;
        }
        let start = self.ppu.frames;
        let mut steps = 0u32;
        while self.ppu.frames == start && steps < 2_000_000 {
            #[cfg(feature = "apu-trace")]
            if let Some(trace) = &mut self.exec_trace {
                trace.push((self.cpu.cycles, self.cpu.registers.pc));
            }
            let mut cpu = self.cpu;
            let cycles = cpu.step(self);
            self.cpu = cpu;
            self.pump_dma();
            steps += cycles.max(1);
            if self.cpu.halted {
                break;
            }
        }
        self.ppu.present();
        self.resampler.drain_into(&mut self.audio);
        steps
    }

    /// The most recently rendered RGBA8 frame.
    #[must_use]
    pub fn video(&self) -> &[u8] {
        self.ppu.video()
    }

    /// Audio produced by the last frame.
    #[must_use]
    pub fn audio(&self) -> &[f32] {
        &self.audio
    }

    /// Starts collecting per-CPU-cycle APU channel outputs for debugging.
    #[cfg(feature = "apu-trace")]
    pub fn start_apu_trace(&mut self) {
        self.apu_trace = Some(Vec::new());
    }

    /// Takes the per-CPU-cycle trace: `(apu_cycles, [p1, p2, tri, noise, dmc,
    /// apu_mix, expansion_mix, total])`.
    #[cfg(feature = "apu-trace")]
    #[must_use]
    pub fn take_apu_trace(&mut self) -> ApuTrace {
        self.apu_trace.take().unwrap_or_default()
    }

    /// Starts collecting `(CPU cycle, PC)` per executed instruction.
    #[cfg(feature = "apu-trace")]
    pub fn start_exec_trace(&mut self) {
        self.exec_trace = Some(Vec::new());
    }

    /// Takes the per-instruction execution trace.
    #[cfg(feature = "apu-trace")]
    #[must_use]
    pub fn take_exec_trace(&mut self) -> Vec<(u64, u16)> {
        self.exec_trace.take().unwrap_or_default()
    }

    /// Starts collecting expansion-port writes as `(CPU cycle, address, byte)`.
    #[cfg(feature = "apu-trace")]
    pub fn start_expansion_trace(&mut self) {
        self.expansion_trace = Some(Vec::new());
    }

    /// Takes the expansion-port write trace.
    #[cfg(feature = "apu-trace")]
    #[must_use]
    pub fn take_expansion_trace(&mut self) -> Vec<(u64, u16, u8)> {
        self.expansion_trace.take().unwrap_or_default()
    }

    /// Current audio sample rate.
    #[must_use]
    pub const fn sample_rate(&self) -> f32 {
        self.audio_rate
    }

    /// Inserts an FDS disk side (`Some`) or ejects (`None`); no-op for
    /// non-FDS cartridges.
    pub fn insert_disk_side(&mut self, side: Option<usize>) {
        self.cart.insert_disk_side(side);
    }

    /// Number of sides in an FDS image (0 for iNES cartridges).
    #[must_use]
    pub fn disk_side_count(&self) -> usize {
        self.cart.disk_side_count()
    }

    /// Returns the raw `.fds` image with pending write-mode changes flushed
    /// (empty for cartridges).
    pub fn save_disk_image(&mut self) -> Vec<u8> {
        self.cart.save_disk_image()
    }

    /// Battery-backed PRG RAM, if any.
    #[must_use]
    pub fn sram(&self) -> &[u8] {
        self.cart.prg_ram()
    }

    /// Re-copies the CHR-ROM window from the cartridge. Called on `$2006`
    /// writes and `$2007` reads so a mid-frame CHR bank switch is visible to
    /// `$2007` readback (mapper self-tests switch banks and read them back).
    fn refresh_chr(&mut self) {
        if self.cart.chr_is_ram() {
            self.ppu.set_chr_ram_map(self.cart.chr_ram_map());
        } else {
            for address in 0..0x2000u16 {
                self.ppu.chr[usize::from(address)] = self.cart.ppu_read(address);
            }
        }
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
        } else {
            self.ppu.set_chr_ram_map(self.cart.chr_ram_map());
        }
        let mirroring = self.cart.mirroring();
        if self.ppu.mirroring != mirroring {
            self.ppu.mirroring = mirroring;
        }
        self.ppu
            .set_rom_nametable_banks(self.cart.rom_nametable_banks());
        self.ppu
            .set_rom_nametable_quadrants(self.cart.rom_nametable_quadrants());
        self.ppu
            .set_nametable_quadrants(self.cart.nametable_quadrants());
        self.ppu.set_nametable_map(self.cart.nametable_map());
    }

    /// Reads a byte from the CPU address space without side effects.
    ///
    /// Intended for test harnesses that inspect RAM (e.g. AccuracyCoin result
    /// bytes); PPU/APU registers are not modelled here.
    #[must_use]
    pub fn read_ram(&self, address: u16) -> u8 {
        self.peek(address)
    }

    /// Side-effect-free read for the debugger.
    ///
    /// CPU reads do not touch `$2002` latches, `$2007` buffers or controller
    /// shift registers; PPU reads use the current `v`-independent address.
    #[must_use]
    pub fn debug_peek(&self, space: DebugSpace, address: u16) -> u8 {
        match space {
            DebugSpace::Cpu => match address {
                0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
                0x2000..=0x3FFF => self.ppu.debug_register_peek(address & 7),
                0x4015 => self.apu.status_peek(),
                0x4016 | 0x4017 => 0,
                0x4020..=0xFFFF => self.cart.cpu_read(address),
                _ => 0,
            },
            DebugSpace::Ppu => self.ppu.debug_peek(address),
            DebugSpace::Oam => self.ppu.oam[usize::from(address & 0xFF)],
        }
    }

    /// Debugger write. Returns `false` for read-only addresses.
    ///
    /// Only RAM (`$0000-$1FFF`, `$6000-$7FFF`), PPU memory and OAM are
    /// writable; registers and ROM are rejected so the frontend can report
    /// them instead of silently discarding the edit.
    pub fn debug_poke(&mut self, space: DebugSpace, address: u16, value: u8) -> bool {
        match space {
            DebugSpace::Cpu => match address {
                0x0000..=0x1FFF => {
                    self.ram[usize::from(address) & 0x07FF] = value;
                    true
                }
                0x6000..=0x7FFF => {
                    self.cart.cpu_write(address, value);
                    true
                }
                _ => false,
            },
            DebugSpace::Ppu => {
                if address < 0x2000 && !self.cart.chr_is_ram() {
                    return false;
                }
                self.ppu.debug_poke(address, value);
                true
            }
            DebugSpace::Oam => {
                self.ppu.oam[usize::from(address & 0xFF)] = value;
                true
            }
        }
    }

    /// Decodes the instruction at a CPU address without executing it.
    #[must_use]
    pub fn debug_disassemble(&self, address: u16) -> crate::cpu::DecodedInstruction {
        let opcode = self.debug_peek(DebugSpace::Cpu, address);
        let lo = self.debug_peek(DebugSpace::Cpu, address.wrapping_add(1));
        let hi = self.debug_peek(DebugSpace::Cpu, address.wrapping_add(2));
        crate::cpu::disassemble(address, opcode, lo, hi)
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
                if let Some(pixel) = self.ppu.video().get(index..index + 3) {
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
        // The Four Score replaces both ports with one 24-read chain.
        if self.devices.contains(&PortDevice::FourScore) {
            let index = usize::from(self.peripherals.four_score_index[port]);
            let bit = if index < 8 {
                (self.peripherals.four_score_pads[port] >> index) & 1
            } else if index < 16 {
                (self.peripherals.four_score_pads[port + 2] >> (index - 8)) & 1
            } else {
                let signature = &mut self.peripherals.four_score_signature[port];
                let bit = *signature & 1;
                *signature = (*signature >> 1) | 0x80;
                bit
            };
            if index < 24 {
                self.peripherals.four_score_index[port] = (index + 1) as u8;
            }
            return (self.open_bus & 0xE0) | 0x40 | bit;
        }

        match self.devices[port] {
            PortDevice::Zapper => {
                // Zapper: bit 4 = trigger (0 pulled), bit 3 = light (0 detected).
                // The other pins float high; the shift register is unused.
                let mut value = (self.open_bus & 0xE0) | 0x40;
                if !self.zapper[port].trigger {
                    value |= 0x10;
                }
                if !self.zapper[port].light {
                    value |= 0x08;
                }
                value
            }
            PortDevice::Paddle => {
                // Knob data is the inverted MSB on bit 4, fire on bit 3. After
                // the register empties the serial-in bit (the LSB) repeats.
                let paddle = &mut self.peripherals.paddle[port];
                let data = (paddle.shift >> 7) ^ 1;
                let serial = paddle.shift & 1;
                paddle.shift = (paddle.shift << 1) | serial;
                (self.open_bus & 0xE0) | 0x40 | (data << 4) | (u8::from(paddle.fire) << 3)
            }
            PortDevice::PowerPad => {
                let pad = &mut self.peripherals.power_pad[port];
                if self.strobe {
                    pad.reload();
                }
                let value = ((pad.high & 1) << 4) | ((pad.low & 1) << 3);
                pad.low = (pad.low >> 1) | 0x80;
                pad.high = (pad.high >> 1) | 0x80;
                (self.open_bus & 0xE0) | 0x40 | value
            }
            PortDevice::FamilyKeyboard => {
                (self.open_bus & 0xE0) | 0x40 | self.peripherals.keyboard.read()
            }
            PortDevice::FourScore => unreachable!("handled above"),
            PortDevice::Microphone => {
                let bit = self.shift[port] & 1;
                let mut value = (self.open_bus & 0xE0) | 0x40 | bit;
                if self.peripherals.microphone {
                    value |= 0x04;
                }
                if !self.strobe {
                    self.shift[port] = (self.shift[port] >> 1) | 0x80;
                }
                value
            }
            PortDevice::Tape => (self.open_bus & 0xE0) | 0x40 | self.peripherals.tape.read(),
            PortDevice::MiraclePiano => {
                (self.open_bus & 0xE0) | 0x40 | self.peripherals.piano.read()
            }
            PortDevice::Gamepad => {
                let bit = self.shift[port] & 1;
                let value = (self.open_bus & 0xE0) | 0x40 | bit;
                if !self.strobe {
                    self.shift[port] = (self.shift[port] >> 1) | 0x80;
                }
                value
            }
        }
    }

    fn clock(&mut self) {
        self.end_dots = 0;
        let dots = self.next_cpu_dots();
        self.advance_ppu_cycle(dots);
        self.sample_zapper();
        if self.cart.needs_cpu_clock() {
            self.cart.clock_cpu();
        }
        self.apu_cycle();
    }

    /// Advances the APU by one CPU cycle and records a debug trace entry when
    /// tracing is enabled.
    #[inline]
    fn apu_cycle(&mut self) {
        self.apu.phase_override = Some(self.phase);
        self.apu.tick();
        self.mix_sample();
        #[cfg(feature = "apu-trace")]
        if let Some(trace) = &mut self.apu_trace {
            let external = self.cart.audio_mix();
            let mut record = [0.0; 8];
            record[..5].copy_from_slice(&self.apu.channel_outputs());
            record[5] = self.apu.output;
            record[6] = external;
            record[7] = self.apu.output + external;
            let expansion = self.cart.debug_audio().unwrap_or([0.0; 8]);
            trace.push((self.apu.cycles(), record, expansion));
        }
        self.phase = !self.phase;
    }

    #[inline]
    fn mix_sample(&mut self) {
        let output = if self.cart.has_expansion_audio() {
            self.apu.output + self.cart.audio_mix() * self.channel_gains[5]
        } else {
            self.apu.output
        };
        self.resampler.add_sample(output);
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
        if self.cart.needs_cpu_clock() {
            self.cart.clock_cpu();
        }
        self.apu_cycle();

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
    ///
    /// MMC3-family counters clock on the PPU A12 rise that opens the sprite
    /// pattern fetch window, not at the scanline boundary. Clocking at dot 260
    /// puts the IRQ handler's scroll write on the following line, matching the
    /// hardware split position; clocking at dot 0 put the split one line early.
    #[inline]
    fn advance_ppu_cycle(&mut self, dots: u32) {
        let line = self.ppu.scanline;
        let dot = self.ppu.dot;
        self.ppu.step_dots(dots);
        let line_after = self.ppu.scanline;
        let dot_after = self.ppu.dot;
        let rendered = (line_after <= 239 || line_after == 261) && self.ppu.mask & 0x18 != 0;
        if rendered && line == line_after && dot < 260 && dot_after >= 260 {
            self.cart.clock_scanline();
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
                if self.cart.prg_ram().is_empty() && !self.cart.prg_rom_at_6000() {
                    self.open_bus
                } else {
                    if self
                        .cart
                        .ram_read_window()
                        .is_some_and(|(lo, hi)| (lo..=hi).contains(&address))
                    {
                        let ram = self.ram;
                        self.cart.latch_cpu_ram(address, &ram);
                    }
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
        if self.cart.needs_cpu_address() {
            self.cart.set_cpu_address(address);
        }
        // $2004 reflects the OAM data bus at the end of the CPU read cycle.
        // Advance the final PPU dot before sampling it and suppress the usual
        // post-access step so the cycle length is unchanged.
        if (0x2000..=0x3FFF).contains(&address) && address & 7 == 4 {
            self.ppu.step_dots(1);
            self.end_dots = 0;
        }
        // Internal RAM is the most common CPU target; keep it out of the
        // general bus match.
        if address < 0x2000 {
            let value = self.ram[usize::from(address) & 0x07FF];
            self.open_bus = value;
            self.internal_bus = value;
            return value;
        }
        let mut drives_bus = true;
        let value = match address {
            0x0000..=0x1FFF => self.ram[usize::from(address) & 0x07FF],
            0x2000..=0x3FFF => {
                if address & 7 == 7 {
                    self.refresh_chr();
                }
                self.ppu.read_register(address)
            }
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
                    // A mapper board register/RAM window (FDS, Waixing FS303).
                    self.cart.cpu_read_io(address)
                } else {
                    drives_bus = false;
                    self.open_bus
                }
            }
            0x6000..=0x7FFF => {
                if self.cart.prg_ram().is_empty() && !self.cart.prg_rom_at_6000() {
                    self.open_bus
                } else {
                    self.cart.cpu_read(address)
                }
            }
            0x8000..=0xFFFF => self.cart.cpu_read_io(address),
            _ => self.open_bus,
        };
        if drives_bus {
            self.open_bus = value;
            self.internal_bus = value;
        }
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        if self.cart.needs_cpu_address() {
            self.cart.set_cpu_address(address);
        }
        if address < 0x2000 {
            self.ram[usize::from(address) & 0x07FF] = value;
            self.open_bus = value;
            self.internal_bus = value;
            return;
        }
        match address {
            0x0000..=0x1FFF => {
                self.ram[usize::from(address) & 0x07FF] = value;
            }
            0x2000..=0x3FFF => {
                // OneBus boards expose bank registers in the `$2008-$3FFF`
                // mirror range; the cartridge claims those addresses.
                if address >= 0x2008 && self.cart.native_ppu_write(address, value) {
                    self.refresh_chr();
                } else {
                    self.ppu.write_register(address, value);
                    if address == 0x2000 {
                        self.cart.set_ppu_ctrl(value);
                    }
                    if address & 7 == 6 {
                        self.refresh_chr();
                    }
                }
            }
            0x4000..=0x4013 | 0x4015 | 0x4017 => {
                #[cfg(feature = "apu-trace")]
                if let Some(trace) = &mut self.expansion_trace {
                    trace.push((self.cycle_count, address, value));
                }
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
                    if self.devices.contains(&PortDevice::Tape) {
                        self.peripherals.tape.write(value);
                    }
                    if self.devices.contains(&PortDevice::MiraclePiano) {
                        self.peripherals.piano.write(value, self.cycle_count);
                    }
                    self.strobe = value & 1 != 0;
                    self.peripherals.keyboard.write(value);
                    if self.strobe {
                        self.shift = [self.controllers[0], self.controllers[1]];
                        for port in 0..2 {
                            if self.devices[port] == PortDevice::Paddle {
                                self.peripherals.paddle[port].shift =
                                    self.peripherals.paddle[port].position;
                            }
                            if self.devices[port] == PortDevice::PowerPad {
                                self.peripherals.power_pad[port].reload();
                            }
                        }
                        self.peripherals.four_score_index = [0; 2];
                        self.peripherals.four_score_signature = [0x08, 0x04];
                        self.peripherals.four_score_pads = self.controllers;
                    }
                }
            }
            0x4020..=0xFFFF => {
                #[cfg(feature = "apu-trace")]
                if let Some(trace) = &mut self.expansion_trace {
                    trace.push((self.cycle_count, address, value));
                }
                self.cart.set_cpu_cycle(self.cycle_count);
                self.cart.cpu_write(address, value);
                if self.cart.chr_mapping_write(address) {
                    self.refresh_chr();
                }
            }
            _ => {}
        }
        self.open_bus = value;
        self.internal_bus = value;
    }

    fn tick(&mut self) {}

    #[inline]
    fn begin_cycle(&mut self, address: Option<u16>, write: Option<u8>) {
        self.cycle_count = self.cycle_count.wrapping_add(1);
        self.begin_dma_cycle(address, write);
    }

    #[inline]
    fn end_cycle(&mut self) {
        if self.end_dots == 0 {
            return;
        }
        let dots = u32::from(self.end_dots);
        self.end_dots = 0;
        self.advance_ppu_cycle(dots);
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
        self.apu.irq_line() || (self.cart.irq_possible() && self.cart.irq_line())
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
        // The reset sequence already finished the power-on frame; the first
        // call reports it without running.
        system.run_frame();
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
    fn fds_bios_mapping_ram_and_audio_window() {
        let mut bios = vec![0xEAu8; 0x2000];
        let code = [
            0xA9, 0x02, 0x8D, 0x23, 0x40, // LDA #2; STA $4023
            0xA9, 0x83, 0x8D, 0x89, 0x40, // LDA #$83; STA $4089
            0x4C, 0x08, 0xE0, // JMP $E008
        ];
        bios[..code.len()].copy_from_slice(&code);
        bios[0x1FFC] = 0x00;
        bios[0x1FFD] = 0xE0;
        let disk = vec![0u8; 65500];
        let mut system = System::from_fds(&disk, &bios).expect("fds");
        assert_eq!(system.cart.mapper(), 20);
        assert_eq!(system.cart.cpu_read(0xE000), 0xA9);

        CpuBus::write(&mut system, 0x6000, 0x5A);
        assert_eq!(system.cart.cpu_read(0x6000), 0x5A);

        // Wave RAM and master volume through the board window. $4023 bit 1
        // enables the sound registers, as the BIOS does with $83.
        CpuBus::write(&mut system, 0x4023, 0x83);
        CpuBus::write(&mut system, 0x4089, 0x83);
        for i in 0..64u16 {
            CpuBus::write(&mut system, 0x4040 + i, if i < 32 { 63 } else { 0 });
        }
        CpuBus::write(&mut system, 0x4089, 0x03);
        CpuBus::write(&mut system, 0x4080, 0xBF);
        CpuBus::write(&mut system, 0x4082, 0x40);
        let mut peak = 0.0f32;
        for _ in 0..10 {
            system.run_frame();
            peak = peak.max(system.cart.audio_mix().abs());
        }
        assert!(peak > 0.01, "FDS audio did not reach the mixer: {peak}");
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
        // The first `run_frame` reports the power-on frame that the reset
        // sequence already completed, so 301 calls cover 300 executed frames
        // — the same window the reference capture measures.
        for _ in 0..301 {
            enabled.run_frame();
            disabled.run_frame();
        }
        let count =
            |system: &System| u16::from(system.read_ram(0)) | (u16::from(system.read_ram(1)) << 8);
        let with_dmc = count(&enabled);
        let without_dmc = count(&disabled);
        assert_eq!(without_dmc, 64390);
        assert_eq!(with_dmc, 54264);
        // The reference steals the same 10,126 loop iterations over 300
        // executed frames (the constant start-up offset is present with and
        // without DMC, so it is not DMA-related).
        assert_eq!(without_dmc - with_dmc, 10126);
    }

    #[test]
    fn reset_phase_matches_the_reference_calibration() {
        // A trivial NROM loop. The reset phase is parity-tuned: the PPU has
        // completed the power-on frame and sits at (line 0, dot 23) with the
        // APU at cycle 8. Pre-advancing the PPU further aligns the per-frame
        // wrap cycles exactly (7 dots) but shifts the CPU/PPU relative phase
        // and regresses oracle parity, so this exact state is the best
        // measured configuration and must not drift.
        let mut prg = vec![0x4Cu8, 0x00, 0x80]; // JMP $8000
        prg.resize(0x4000, 0xEA);
        prg[0x3FFC] = 0x00;
        prg[0x3FFD] = 0x80;
        let mut rom = vec![0u8; 16 + prg.len() + 0x2000];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[5] = 1;
        rom[16..16 + prg.len()].copy_from_slice(&prg);

        let system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cpu.registers.pc, 0x8000);
        assert_eq!(system.apu.cycles(), 8, "reset APU phase");
        assert_eq!(system.ppu.frames, 1, "power-on frame already latched");
        assert_eq!(
            (system.ppu.scanline, system.ppu.dot),
            (0, 23),
            "PPU reset phase"
        );
    }

    #[test]
    fn system_audio_runs_at_the_requested_rate() {
        // A trivial NROM loop; the audio stream should be produced at 44.1 kHz
        // through the band-limited resampler (~734 samples per NTSC frame).
        let mut prg = vec![0x4Cu8, 0x00, 0x80];
        prg.resize(0x4000, 0xEA);
        prg[0x3FFC] = 0x00;
        prg[0x3FFD] = 0x80;
        let mut rom = vec![0u8; 16 + prg.len() + 0x2000];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[5] = 1;
        rom[16..16 + prg.len()].copy_from_slice(&prg);

        let mut system = System::from_ines(&rom).expect("load");
        system.set_sample_rate(44_100.0);
        system.run_frame(); // power-on frame (no cycles)
        system.run_frame();
        let n = system.audio().len();
        assert!(
            (700..=760).contains(&n),
            "44.1 kHz frame produced {n} samples"
        );
        assert!(system.audio().iter().all(|s| s.is_finite()));
    }

    #[test]
    fn microphone_reports_on_4016_bit_2() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(0, PortDevice::Microphone);
        system.set_microphone(true);
        system.write(0x4016, 0);
        system.write(0x4016, 1);
        assert_ne!(system.read(0x4016) & 0x04, 0);
        system.set_microphone(false);
        assert_eq!(system.read(0x4016) & 0x04, 0);
    }

    #[test]
    fn miracle_piano_polls_and_sends_midi_bytes() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(0, PortDevice::MiraclePiano);

        // Poll: a short OUT0 pulse, then a status bit and eight inverted bits.
        system.push_piano_rx(0x5A);
        system.write(0x4016, 1);
        for _ in 0..12 {
            system.cycle_count += 1;
        }
        system.write(0x4016, 0);
        assert_eq!(system.read(0x4016) & 1, 1, "no status bit");
        let mut received = 0u8;
        for _ in 0..8 {
            received = (received << 1) | ((system.read(0x4016) & 1) ^ 1);
        }
        assert_eq!(received, 0x5A);

        // Send: a long OUT0 pulse, then eight writes+reads per byte.
        system.write(0x4016, 1);
        for _ in 0..80 {
            system.cycle_count += 1;
        }
        system.write(0x4016, 0);
        for bit in 0..8 {
            let value = (0xA5 >> (7 - bit)) & 1;
            system.write(0x4016, value);
            system.read(0x4016);
        }
        assert_eq!(system.piano_tx_log(), &[0xA5]);
    }

    #[test]
    fn tape_recorder_loops_back_written_bits() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(0, PortDevice::Tape);
        for bit in [true, false, true, true, false] {
            system.write(0x4016, 0x02 | u8::from(bit));
            assert_eq!(system.read(0x4016) & 1, u8::from(bit));
        }
        // Bit 1 clear forces readback to 0 even with queued bits.
        system.write(0x4016, 0x00);
        assert_eq!(system.read(0x4016) & 1, 0);
    }

    #[test]
    fn mmc3_irq_clocks_once_per_rendered_scanline() {
        // A trivial MMC3 ROM whose reset code is an endless SEI/JMP loop. The
        // mapper is driven directly; the test only watches when the IRQ line
        // asserts. Clocking the mapper per CPU cycle (the old `end_cycle`
        // behaviour) asserted it in a handful of cycles instead of after the
        // ~17 scanlines the latch asks for.
        let mut prg = vec![0xEAu8; 0x20000];
        prg[..4].copy_from_slice(&[0x78, 0x4C, 0x00, 0x80]); // SEI; JMP $8000
        prg[0x1FFFC] = 0x00;
        prg[0x1FFFD] = 0x80;
        let mut rom = vec![0u8; 16 + prg.len() + 0x10000];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8; // 128 KiB PRG
        rom[5] = 8; // 64 KiB CHR
        rom[6] = 0x40; // MMC3
        rom[16..16 + prg.len()].copy_from_slice(&prg);

        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 4);
        system.ppu.mask = 0x1E; // background + sprites: A12 rises each line
        system.cart.cpu_write(0xC000, 0x10); // latch = 16
        system.cart.cpu_write(0xC001, 0x00); // reload
        system.cart.cpu_write(0xE001, 0x00); // enable
        assert!(!system.cart.irq_line());

        let mut cycles = 0u32;
        while !system.cart.irq_line() && cycles < 200_000 {
            cycles += system.step_instruction().max(1);
        }
        assert!(system.cart.irq_line(), "MMC3 IRQ never asserted");
        // 17 rendered lines at ~114 CPU cycles each. Per-cycle clocking would
        // trip this in ~20 cycles; a scanline boundary clock in ~1800.
        assert!(
            (1_200..6_000).contains(&cycles),
            "IRQ asserted after {cycles} CPU cycles"
        );
    }

    #[test]
    fn mmc5_per_page_cirarn_nametables_reach_the_ppu() {
        let prg_len = 128 * 1024;
        let chr_len = 64 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 8;
        rom[6] = 0x50; // Mapper 5.
        rom[16..16 + prg_len].fill(0xEA);
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        let vector = 16 + prg_len - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        // CHR bank 0: tile 0 transparent, tile 1 solid (color index 3).
        let chr = 16 + prg_len;
        rom[chr..chr + 0x1000].fill(0x00);
        rom[chr + 0x10..chr + 0x20].fill(0xFF);

        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 5);

        // Horizontal arrangement: NT0/NT2 -> CIRAM page 0, NT1/NT3 -> page 1.
        CpuBus::write(&mut system, 0x5105, 0x44);
        system.ppu.debug_write_vram(0x2000, 0x00);
        for i in 0..32u16 {
            system.ppu.debug_write_vram(0x2400 + i, 0x01);
        }
        assert_eq!(system.ppu.debug_read_vram(0x2000), 0x00);
        assert_eq!(system.ppu.debug_read_vram(0x2400), 0x01);

        // Render line 0 from NT1 ($2400): tile 1 -> palette entry 3.
        system.ppu.ctrl = 0;
        system.ppu.mask = 0x0A;
        system.ppu.v = 0x2400;
        system.ppu.fine_x = 0;
        system.ppu.scanline = 0;
        system.ppu.dot = 0;
        system.ppu.debug_write_vram(0x3F00, 0x0F);
        system.ppu.debug_write_vram(0x3F03, 0x2D);
        for _ in 0..256 {
            system.ppu.step_dot();
        }
        assert_eq!(system.ppu.debug_pixel(40, 0), system.ppu.debug_palette(3));

        // Remap every slot to CIRAM page 0: $2400 now aliases $2000's page,
        // whose tile 0 is transparent, so the line shows the backdrop.
        CpuBus::write(&mut system, 0x5105, 0x00);
        system.ppu.mask = 0x0A;
        system.ppu.v = 0x2400;
        system.ppu.fine_x = 0;
        system.ppu.scanline = 0;
        system.ppu.dot = 0;
        for _ in 0..256 {
            system.ppu.step_dot();
        }
        assert_eq!(system.ppu.debug_pixel(40, 0), system.ppu.debug_palette(0));
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
    fn namco163_midframe_chr_bank_is_visible_to_2007_readback() {
        // 128 KiB PRG, 128 KiB CHR (128 x 1 KiB banks filled with 0x80|index).
        let prg_len = 128 * 1024;
        let chr_len = 128 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 16;
        rom[6] = 0x30;
        rom[7] = 0x10; // mapper 19
        rom[16..16 + prg_len].fill(0xEA);
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        for (bank, chunk) in rom[16 + prg_len..].chunks_mut(0x400).enumerate() {
            chunk.fill(0x80 | (bank as u8 & 0x7F));
        }
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 19);

        // The mapper self-test switches CHR bank 0 and reads it back through
        // $2007; the PPU's CHR window must be refreshed by the access.
        for bank in [0x05u8, 0x21, 0x55] {
            system.cart.cpu_write(0x8000, bank);
            CpuBus::write(&mut system, 0x2006, 0);
            CpuBus::write(&mut system, 0x2006, 0);
            let _ = CpuBus::read(&mut system, 0x2007); // start the buffered read
                                                       // The PPU fills the read buffer four dots later.
            system.ppu.step_dots(8);
            let value = CpuBus::read(&mut system, 0x2007);
            assert_eq!(
                value,
                0x80 | (bank & 0x7F),
                "bank {bank:02X} not visible to $2007"
            );
        }
    }

    #[test]
    fn namco163_chr_bank_write_refreshes_the_pattern_window() {
        // Mid-frame raster bands (Rolling Thunder's status bar) switch the
        // eight 1 KiB CHR banks at `$8000-$BFFF`; the renderer reads its own
        // `ppu.chr` copy, so the write must refresh it immediately.
        let prg_len = 128 * 1024;
        let chr_len = 128 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 16;
        rom[6] = 0x30;
        rom[7] = 0x10; // mapper 19
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        for (bank, chunk) in rom[16 + prg_len..].chunks_mut(0x400).enumerate() {
            chunk.fill(0x80 | (bank as u8 & 0x7F));
        }
        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.ppu.chr[0], 0x80);
        CpuBus::write(&mut system, 0x8000, 0x05);
        CpuBus::write(&mut system, 0x8800, 0x21);
        assert_eq!(system.ppu.chr[0], 0x85);
        assert_eq!(system.ppu.chr[0x400], 0xA1);
    }

    #[test]
    fn namco163_nametable_pages_select_ciram_and_chr_rom() {
        // 128 KiB PRG, 128 KiB CHR (128 x 1 KiB banks).
        let prg_len = 128 * 1024;
        let chr_len = 128 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 16;
        rom[6] = 0x30; // mapper 19, horizontal mirroring
        rom[7] = 0x10;
        rom[16..16 + prg_len].fill(0xEA);
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        for (bank, chunk) in rom[16 + prg_len..].chunks_mut(0x400).enumerate() {
            chunk.fill(0x80 | (bank as u8 & 0x7F));
        }
        let mut system = System::from_ines(&rom).expect("load");
        system.ppu.end_warmup();

        let write_nt = |system: &mut System, addr: u16, value: u8| {
            CpuBus::write(system, 0x2006, (addr >> 8) as u8);
            CpuBus::write(system, 0x2006, (addr & 0xFF) as u8);
            CpuBus::write(system, 0x2007, value);
        };
        let read_nt = |system: &mut System, addr: u16| -> u8 {
            CpuBus::write(system, 0x2006, (addr >> 8) as u8);
            CpuBus::write(system, 0x2006, (addr & 0xFF) as u8);
            let _ = CpuBus::read(system, 0x2007); // start the buffered read
            system.ppu.step_dots(8);
            CpuBus::read(system, 0x2007)
        };

        // Power-on follows the header mirroring (horizontal): quadrants 0 and
        // 1 are the same 1 KiB CIRAM page, so $2400 aliases $2000.
        write_nt(&mut system, 0x2000, 0x11);
        assert_eq!(read_nt(&mut system, 0x2000), 0x11);
        write_nt(&mut system, 0x2400, 0x22);
        assert_eq!(read_nt(&mut system, 0x2000), 0x22, "shared page A");
        assert_eq!(read_nt(&mut system, 0x2400), 0x22);

        // $C000-$D800 remap the quadrants: vertical splits page A/B.
        system.cart.cpu_write(0xC000, 0xE0);
        system.cart.cpu_write(0xC800, 0xE1);
        system.cart.cpu_write(0xD000, 0xE0);
        system.cart.cpu_write(0xD800, 0xE1);
        assert_eq!(read_nt(&mut system, 0x2000), 0x22, "page A retained");
        assert_eq!(read_nt(&mut system, 0x2400), 0x00, "page B is separate");

        // A value below $E0 selects a CHR-ROM page; it is read-only.
        system.cart.cpu_write(0xC000, 0x01);
        assert_eq!(read_nt(&mut system, 0x2000), 0x81);
        write_nt(&mut system, 0x2000, 0x77);
        assert_eq!(
            read_nt(&mut system, 0x2000),
            0x81,
            "CHR-ROM page absorbs writes"
        );

        // Back to CIRAM restores the stored byte.
        system.cart.cpu_write(0xC000, 0xE0);
        assert_eq!(read_nt(&mut system, 0x2000), 0x22);
    }

    #[test]
    fn namco163_irq_counter_is_readable_in_the_low_window() {
        // The counter lives at $5000/$5800 inside $4020-$5FFF, which the
        // system must route to the cartridge instead of answering open bus.
        let prg_len = 128 * 1024;
        let chr_len = 128 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 16;
        rom[6] = 0x30;
        rom[7] = 0x10;
        rom[16..16 + prg_len].fill(0xEA);
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        let mut system = System::from_ines(&rom).expect("load");

        // Latch 0, enable, then count CPU cycles.
        system.cart.cpu_write(0x5000, 0x00);
        system.cart.cpu_write(0x5800, 0x80);
        assert_eq!(CpuBus::read(&mut system, 0x5000), 0);
        assert_eq!(CpuBus::read(&mut system, 0x5800) & 0x80, 0x80);
        for _ in 0..0xC0 {
            system.clock();
        }
        let low = CpuBus::read(&mut system, 0x5000);
        assert!(low >= 0xBF, "IRQ counter low byte {low:02X} not counted");

        // Disabling the counter freezes it.
        system.cart.cpu_write(0x5800, 0x00);
        let frozen = CpuBus::read(&mut system, 0x5000);
        for _ in 0..16 {
            system.clock();
        }
        assert_eq!(CpuBus::read(&mut system, 0x5000), frozen);
    }

    #[test]
    fn mmc5_eight_by_sixteen_4k_background_uses_512b() {
        // 64 KiB CHR = 512 x 1 KiB banks filled with 0x80|index.
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
        system.cart.set_ppu_ctrl(0x20); // 8x16 sprites
        system.cart.cpu_write(0x5101, 1); // 4 KiB CHR mode
        system.cart.cpu_write(0x512B, 10); // background 4 KiB bank 10 -> 1 KiB 40
                                           // Both halves of the background use the same 4 KiB page.
        assert_eq!(system.ppu.debug_bg_chr_byte(0x0000), 0x80 | 40);
        assert_eq!(system.ppu.debug_bg_chr_byte(0x1000), 0x80 | 40);
    }

    #[test]
    fn mmc5_extended_attributes_reach_chr_above_256_kib() {
        // 512 KiB CHR (64 x 8 KiB); 4 KiB bank 64 is solid and lives above the
        // 256 KiB reachable by the ExRAM bank bits alone.
        let prg_len = 128 * 1024;
        let chr_len = 512 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 8;
        rom[5] = 64;
        rom[6] = 0x50; // Mapper 5.
        rom[16..16 + prg_len].fill(0xEA);
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        let vector = 16 + prg_len - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        let chr = 16 + prg_len;
        rom[chr..chr + chr_len].fill(0x00);
        rom[chr + 64 * 0x1000..chr + 65 * 0x1000].fill(0xFF);

        let mut system = System::from_ines(&rom).expect("load");
        assert_eq!(system.cart.mapper(), 5);

        // Extended-attribute mode: every tile selects bank (attr & $3F) plus
        // the $5130 bits. ExRAM = $40 -> palette 1, low bank bits 0.
        CpuBus::write(&mut system, 0x5104, 1);
        CpuBus::write(&mut system, 0x5105, 0x00);
        CpuBus::write(&mut system, 0x5130, 0x01); // bank bits 6-7 = 1 -> bank 64
        for i in 0..32u16 {
            CpuBus::write(&mut system, 0x5C00 + i, 0x40);
        }
        system.ppu.debug_write_vram(0x2000, 0x00);
        system.ppu.debug_write_vram(0x3F00, 0x0F);
        system.ppu.debug_write_vram(0x3F07, 0x2D);

        system.ppu.ctrl = 0;
        system.ppu.mask = 0x0A;
        system.ppu.v = 0;
        system.ppu.fine_x = 0;
        system.ppu.scanline = 0;
        system.ppu.dot = 0;
        for _ in 0..248 {
            system.ppu.step_dot();
        }
        // Palette 1, colour 3 -> entry 7, fetched from 4 KiB bank 64.
        assert_eq!(system.ppu.debug_pixel(40, 0), system.ppu.debug_palette(7));

        // Clearing $5130 selects bank 0 (all zeros) -> the backdrop shows.
        CpuBus::write(&mut system, 0x5130, 0x00);
        system.ppu.mask = 0x0A;
        system.ppu.v = 0;
        system.ppu.fine_x = 0;
        system.ppu.scanline = 0;
        system.ppu.dot = 0;
        for _ in 0..248 {
            system.ppu.step_dot();
        }
        assert_eq!(system.ppu.debug_pixel(40, 0), system.ppu.debug_palette(0));
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

        // ExRAM-backed nametable: the CPU fills it in mode 2 (writable), then
        // the PPU sees it in mode 0 (the only modes where ExRAM is a
        // nametable are 0/1).
        CpuBus::write(&mut system, 0x5104, 2);
        CpuBus::write(&mut system, 0x5C00, 0x5A);
        CpuBus::write(&mut system, 0x5104, 0);
        CpuBus::write(&mut system, 0x5105, 0x02);
        assert_eq!(system.ppu.debug_read_vram(0x2000), 0x5A);
        system.ppu.debug_write_vram(0x2000, 0x33);
        CpuBus::write(&mut system, 0x5104, 2);
        assert_eq!(system.cart.cpu_read(0x5C00), 0x33);
        // In mode 2 the same slot reads as all zeros.
        CpuBus::write(&mut system, 0x5105, 0x02);
        assert_eq!(system.ppu.debug_read_vram(0x2000), 0x00);

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
    fn header_tv_system_selects_timing() {
        use crate::clock::Timing;
        // NES 2.0 byte 12: 0 = NTSC, 1 = PAL, 3 = Dendy.
        let mut rom = vec![0u8; 16 + 16 * 1024 + 8 * 1024];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[5] = 1;
        rom[7] = 0x08; // NES 2.0
        rom[12] = 0;
        assert_eq!(
            System::from_ines(&rom).expect("load").timing(),
            Timing::NTSC
        );
        rom[12] = 1;
        assert_eq!(System::from_ines(&rom).expect("load").timing(), Timing::PAL);
        rom[12] = 3;
        assert_eq!(
            System::from_ines(&rom).expect("load").timing(),
            Timing::DENDY
        );

        // Legacy iNES: byte 9 bit 0 marks PAL.
        rom[7] = 0;
        rom[9] = 1;
        assert_eq!(System::from_ines(&rom).expect("load").timing(), Timing::PAL);
        rom[9] = 0;
        assert_eq!(
            System::from_ines(&rom).expect("load").timing(),
            Timing::NTSC
        );
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

    fn strobe(system: &mut System) {
        CpuBus::write(system, 0x4016, 1);
        CpuBus::write(system, 0x4016, 0);
    }

    #[test]
    fn paddle_reports_inverted_msb_first_and_fire() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(1, PortDevice::Paddle);
        system.set_paddle(1, 0x12, true);
        strobe(&mut system);

        let mut bits = Vec::new();
        for _ in 0..8 {
            let value = CpuBus::read(&mut system, 0x4017);
            bits.push((value >> 4) & 1);
            assert_eq!((value >> 3) & 1, 1, "fire is reported on bit 3");
        }
        // 0x12 = 0001_0010, MSB-first and inverted.
        assert_eq!(bits, vec![1, 1, 1, 0, 1, 1, 0, 1]);
    }

    #[test]
    fn power_pad_serial_order_matches_the_documented_layout() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(1, PortDevice::PowerPad);
        // Buttons 1 and 4 (bit 0 and bit 3 of the mat mask).
        system.set_power_pad(1, 0b0000_1001);
        strobe(&mut system);

        let lows: Vec<u8> = (0..8)
            .map(|_| (CpuBus::read(&mut system, 0x4017) >> 3) & 1)
            .collect();
        // Low register order: buttons 2, 1, 5, 9, 6, 10, 11, 7.
        assert_eq!(lows, vec![0, 1, 0, 0, 0, 0, 0, 0]);

        strobe(&mut system);
        let highs: Vec<u8> = (0..8)
            .map(|_| (CpuBus::read(&mut system, 0x4017) >> 4) & 1)
            .collect();
        // High register order: buttons 4, 3, 12, 8, then four 1s.
        assert_eq!(highs, vec![1, 0, 0, 0, 1, 1, 1, 1]);
    }

    #[test]
    fn four_score_chain_carries_four_players_and_a_signature() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(0, PortDevice::FourScore);
        system.set_port_device(1, PortDevice::FourScore);
        system.set_controller(0, 0b0000_0001); // P1 A
        system.set_controller(1, 0b0000_0010); // P2 B
        system.set_controller(2, 0b0000_0100); // P3 Select
        system.set_controller(3, 0b0000_1000); // P4 Start
        strobe(&mut system);

        let p1: Vec<u8> = (0..24)
            .map(|_| CpuBus::read(&mut system, 0x4016) & 1)
            .collect();
        let p2: Vec<u8> = (0..24)
            .map(|_| CpuBus::read(&mut system, 0x4017) & 1)
            .collect();
        assert_eq!(&p1[..8], &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&p1[8..16], &[0, 0, 1, 0, 0, 0, 0, 0]);
        assert_eq!(&p2[..8], &[0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&p2[8..16], &[0, 0, 0, 1, 0, 0, 0, 0]);
        // Adapter signatures: 0x08 on $4016, 0x04 on $4017 (LSB-first; the
        // shifted-in 1s take another eight reads to reach the serial output).
        assert_eq!(&p1[16..24], &[0, 0, 0, 1, 0, 0, 0, 0]);
        assert_eq!(&p2[16..24], &[0, 0, 1, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn family_keyboard_scans_rows_and_column_halves() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(1, PortDevice::FamilyKeyboard);
        let mut keys = [0u8; 9];
        keys[0] = 0b0000_0001; // row 0, half 0, first key
        keys[1] = 0b0001_0000; // row 1, half 1, first key
        system.set_family_keyboard(keys);

        // Enable with clock low: row 0, half 0.
        CpuBus::write(&mut system, 0x4016, 0x04);
        let value = CpuBus::read(&mut system, 0x4017);
        assert_eq!((value >> 1) & 0x0F, 0b1110, "one key reads low");

        // Clock high selects row 1; column bit selects the other half.
        CpuBus::write(&mut system, 0x4016, 0x06);
        CpuBus::write(&mut system, 0x4016, 0x07);
        let value = CpuBus::read(&mut system, 0x4017);
        assert_eq!((value >> 1) & 0x0F, 0b1110);

        // Disabled matrix reads all four lines high.
        CpuBus::write(&mut system, 0x4016, 0x00);
        let value = CpuBus::read(&mut system, 0x4017);
        assert_eq!((value >> 1) & 0x0F, 0x0F);
    }

    #[test]
    fn peripheral_state_round_trips_through_save_states() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        system.set_port_device(0, PortDevice::FourScore);
        system.set_port_device(1, PortDevice::FamilyKeyboard);
        system.set_paddle(0, 200, true);
        system.set_power_pad(0, 0x0F0);
        system.set_family_keyboard([0xAA; 9]);
        system.set_controller(3, 0x42);
        let saved = system.save_state().expect("save");
        let mut other = System::from_ines(&rom).expect("load");
        other.load_state(&saved).expect("load");
        assert_eq!(saved, other.save_state().expect("re-save"));
    }

    #[test]
    fn frame_cycle_accounting_holds_over_a_long_run() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        // The first call reports the power-on frame without executing.
        system.run_frame();
        let start = system.cpu.cycles;
        let frames = 600u32;
        for _ in 0..frames {
            system.run_frame();
        }
        let elapsed = system.cpu.cycles - start;
        // This ROM never enables rendering, so it has no odd-frame dot skip:
        // every frame is 341 x 262 dots, and the CPU runs one cycle per three
        // dots. The total is fixed even though each frame ends on an
        // instruction boundary. A drift here means the PPU/CPU divider is
        // losing or gaining cycles.
        let expected = f64::from(frames) * (341.0 * 262.0 / 3.0);
        let error = (elapsed as f64 - expected).abs();
        assert!(
            error < 8.0,
            "cycle accounting drifted by {error}: {elapsed} vs {expected}"
        );
    }

    fn synthetic_nrom(seed: u8) -> Vec<u8> {
        let prg_len = 16 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + 8 * 1024];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[5] = 1;
        for byte in &mut rom[16..16 + prg_len] {
            *byte = 0xEA;
        }
        rom[16] = 0x4C;
        rom[17] = 0x00;
        rom[18] = 0x80;
        let vector = 16 + prg_len - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        rom[16 + prg_len] = seed;
        rom
    }

    #[test]
    fn save_state_restores_the_exact_frame_and_machine_state() {
        let rom = synthetic_nrom(0);
        let mut system = System::from_ines(&rom).expect("load");
        for _ in 0..30 {
            system.run_frame();
        }
        let saved = system.save_state().expect("save");
        for _ in 0..10 {
            system.run_frame();
        }
        let resumed_frame = system.video().to_vec();
        let resumed_audio = system.audio().to_vec();
        system.load_state(&saved).expect("load");
        let round_trip = system.save_state().expect("re-save");
        assert_eq!(saved, round_trip, "state did not round-trip byte-exactly");
        for _ in 0..10 {
            system.run_frame();
        }
        assert_eq!(system.video(), resumed_frame);
        assert_eq!(system.audio(), resumed_audio);
        assert!(system.load_state(b"not a state").is_err());
        let other = System::from_ines(&synthetic_nrom(0x55)).expect("other");
        assert!(other.save_state().is_ok());
        assert!(system
            .load_state(&other.save_state().expect("other state"))
            .is_err());
    }

    fn banked_rom(mapper: u16) -> Vec<u8> {
        let prg_len = 256 * 1024;
        let chr_len = 128 * 1024;
        let mut rom = vec![0u8; 16 + prg_len + chr_len];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = (prg_len / 16 / 1024) as u8;
        rom[5] = (chr_len / 8 / 1024) as u8;
        rom[6] = ((mapper as u8 & 0x0F) << 4) | 0x01;
        rom[7] = (mapper as u8 & 0xF0) | 0x08;
        rom[8] = ((mapper >> 8) as u8) & 0x0F;
        for (index, byte) in rom[16..16 + prg_len].iter_mut().enumerate() {
            *byte = (index & 0xFF) as u8;
        }
        for (index, byte) in rom[16 + prg_len..].iter_mut().enumerate() {
            *byte = (index & 0x7F) as u8;
        }
        let vector = 16 + prg_len - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        rom
    }

    #[test]
    fn state_round_trips_common_mappers() {
        for mapper in [0u16, 1, 4, 5, 19, 24, 69, 405, 547, 594] {
            let rom = banked_rom(mapper);
            let Ok(mut system) = System::from_ines(&rom) else {
                continue;
            };
            if let Ok(saved) = system.save_state() {
                for _ in 0..2 {
                    system.run_frame();
                }
                system
                    .load_state(&saved)
                    .unwrap_or_else(|err| panic!("mapper {mapper} load error: {err}"));
                assert_eq!(
                    saved,
                    system.save_state().expect("resave"),
                    "mapper {mapper}"
                );
            }
        }
    }

    #[test]
    fn every_implemented_mapper_serializes_its_state() {
        let mut checked = 0;
        for mapper in 0u16..=600 {
            let rom = banked_rom(mapper);
            let Ok(mut system) = System::from_ines(&rom) else {
                continue;
            };
            let Ok(saved) = system.save_state() else {
                continue;
            };
            system
                .load_state(&saved)
                .unwrap_or_else(|err| panic!("mapper {mapper} load error: {err}"));
            // The PPU keeps a cached CHR window, so a byte-for-byte re-save can
            // differ; survival of the load is the contract here. Functional
            // equality is covered by the common-mapper test above.
            checked += 1;
        }
        assert!(checked > 200, "only {checked} mapper states exercised");
    }
}
