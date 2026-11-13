//! 2A03 APU: pulse, triangle, noise, DMC, frame counter and mixer.
//!
//! Timing base is the CPU cycle. A3 begins with the register/IRQ/length
//! behaviour needed by the public APU suites; audio is mixed to a float sample.

/// Length counter load table.
pub const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

const DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

const TRIANGLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

const NOISE_PERIODS: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

const DMC_PERIODS: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

/// Mixer output for each possible pulse sum (0..=30).
const PULSE_MIX: [f32; 31] = {
    let mut table = [0.0f32; 31];
    let mut i = 0;
    while i < 31 {
        let pulse = i as f32;
        table[i] = if pulse > 0.0 {
            95.52 / (8128.0 / pulse + 100.0)
        } else {
            0.0
        };
        i += 1;
    }
    table
};

/// Mixer output for each possible `3*triangle + 2*noise + DMC` sum (0..=202).
const TND_MIX: [f32; 203] = {
    let mut table = [0.0f32; 203];
    let mut i = 0;
    while i < 203 {
        let tnd = i as f32;
        table[i] = if tnd > 0.0 {
            163.67 / (24329.0 / tnd + 100.0)
        } else {
            0.0
        };
        i += 1;
    }
    table
};

/// One pulse channel.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Pulse {
    /// Channel enabled from `$4015`.
    pub enabled: bool,
    /// Length counter.
    pub length: u8,
    /// Deferred length reload value.
    length_reload: u8,
    /// Counter value observed when the reload was written.
    length_prev: u8,
    /// Halt/length-enable flag.
    pub halt: bool,
    /// Constant-volume flag.
    pub constant: bool,
    /// Envelope volume / divider period.
    pub volume: u8,
    /// Envelope divider.
    envelope_divider: u8,
    /// Envelope decay level.
    envelope_level: u8,
    /// Envelope start flag.
    envelope_start: bool,
    /// Duty index.
    pub duty: u8,
    /// Duty step.
    duty_step: u8,
    /// 11-bit timer reload.
    pub timer: u16,
    /// Timer counter.
    timer_counter: u16,
    /// Sweep enabled.
    pub sweep_enabled: bool,
    /// Sweep period.
    pub sweep_period: u8,
    /// Sweep negate.
    pub sweep_negate: bool,
    /// Sweep shift.
    pub sweep_shift: u8,
    /// Sweep divider.
    sweep_divider: u8,
    /// Sweep reload flag.
    sweep_reload: bool,
    /// Channel muted by sweep.
    pub muted: bool,
    /// Pulse 1 uses ones' complement for negate.
    ones_complement: bool,
}

impl Pulse {
    fn write(&mut self, index: u16, value: u8) {
        match index {
            0 => {
                self.duty = value >> 6;
                self.halt = value & 0x20 != 0;
                self.constant = value & 0x10 != 0;
                self.volume = value & 0x0F;
            }
            1 => {
                self.sweep_enabled = value & 0x80 != 0;
                self.sweep_period = (value >> 4) & 0x07;
                self.sweep_negate = value & 0x08 != 0;
                self.sweep_shift = value & 0x07;
                self.sweep_reload = true;
            }
            2 => {
                self.timer = (self.timer & 0x0700) | u16::from(value);
            }
            3 => {
                self.timer = (self.timer & 0x00FF) | (u16::from(value & 0x07) << 8);
                if self.enabled {
                    self.length_reload = LENGTH_TABLE[usize::from(value >> 3)];
                    self.length_prev = self.length;
                }
                self.duty_step = 0;
                self.envelope_start = true;
            }
            _ => {}
        }
    }

    fn reload_length(&mut self) {
        if self.length_reload > 0 {
            if self.length == self.length_prev {
                self.length = self.length_reload;
            }
            self.length_reload = 0;
        }
    }

    fn clock_timer(&mut self) {
        if self.timer_counter == 0 {
            self.timer_counter = self.timer;
            self.duty_step = (self.duty_step + 1) & 7;
        } else {
            self.timer_counter -= 1;
        }
    }

    fn clock_quarter(&mut self) {
        if self.envelope_start {
            self.envelope_start = false;
            self.envelope_level = 15;
            self.envelope_divider = self.volume;
        } else if self.envelope_divider == 0 {
            self.envelope_divider = self.volume;
            if self.envelope_level > 0 {
                self.envelope_level -= 1;
            } else if self.halt {
                self.envelope_level = 15;
            }
        } else {
            self.envelope_divider -= 1;
        }
    }

    fn clock_half(&mut self) {
        if !self.halt && self.length > 0 {
            self.length -= 1;
        }
        if self.sweep_enabled {
            if self.sweep_divider == 0 {
                if self.sweep_shift > 0 {
                    let delta = self.timer >> self.sweep_shift;
                    let target = if self.sweep_negate {
                        if self.ones_complement {
                            self.timer.wrapping_sub(delta).wrapping_sub(1)
                        } else {
                            self.timer.wrapping_sub(delta)
                        }
                    } else {
                        self.timer.wrapping_add(delta)
                    };
                    self.muted = self.timer < 8 || target > 0x7FF;
                }
                self.sweep_divider = self.sweep_period;
            } else {
                self.sweep_divider -= 1;
            }
            if self.sweep_reload {
                self.sweep_reload = false;
                self.sweep_divider = self.sweep_period;
            }
        }
    }

    fn output(&self) -> f32 {
        if !self.enabled || self.length == 0 || self.muted || self.timer < 8 {
            return 0.0;
        }
        if DUTY[usize::from(self.duty & 3)][usize::from(self.duty_step)] == 0 {
            return 0.0;
        }
        if self.constant {
            f32::from(self.volume)
        } else {
            f32::from(self.envelope_level)
        }
    }
}

/// Triangle channel.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Triangle {
    /// Enabled.
    pub enabled: bool,
    /// Length counter.
    pub length: u8,
    /// Deferred length reload value.
    length_reload: u8,
    /// Counter value observed when the reload was written.
    length_prev: u8,
    /// Control (halt).
    pub control: bool,
    /// Linear counter reload value.
    pub linear_reload: u8,
    /// Linear counter.
    pub linear: u8,
    /// Linear reload flag.
    linear_reload_flag: bool,
    /// Timer reload.
    pub timer: u16,
    timer_counter: u16,
    /// Sequence step.
    step: u8,
}

impl Triangle {
    fn write(&mut self, index: u16, value: u8) {
        match index {
            0 => {
                self.control = value & 0x80 != 0;
                self.linear_reload = value & 0x7F;
            }
            2 => self.timer = (self.timer & 0x0700) | u16::from(value),
            3 => {
                self.timer = (self.timer & 0x00FF) | (u16::from(value & 0x07) << 8);
                if self.enabled {
                    self.length_reload = LENGTH_TABLE[usize::from(value >> 3)];
                    self.length_prev = self.length;
                }
                self.linear_reload_flag = true;
            }
            _ => {}
        }
    }

    fn reload_length(&mut self) {
        if self.length_reload > 0 {
            if self.length == self.length_prev {
                self.length = self.length_reload;
            }
            self.length_reload = 0;
        }
    }

    fn clock_timer(&mut self) {
        // Periods below 2 freeze the sequencer (ultrasonic silence).
        if self.timer < 2 {
            return;
        }
        if self.timer_counter == 0 {
            self.timer_counter = self.timer;
            if self.length > 0 && self.linear > 0 {
                self.step = (self.step + 1) & 31;
            }
        } else {
            self.timer_counter -= 1;
        }
    }

    fn clock_quarter(&mut self) {
        if self.linear_reload_flag {
            self.linear = self.linear_reload;
        } else if self.linear > 0 {
            self.linear -= 1;
        }
        if !self.control {
            self.linear_reload_flag = false;
        }
    }

    fn clock_half(&mut self) {
        if !self.control && self.length > 0 {
            self.length -= 1;
        }
    }

    fn output(&self) -> f32 {
        // The DAC always outputs the current sequencer step; the length and
        // linear counters only gate whether the sequencer advances.
        f32::from(TRIANGLE[usize::from(self.step)])
    }
}

/// Noise channel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Noise {
    /// Enabled.
    pub enabled: bool,
    /// Length counter.
    pub length: u8,
    /// Deferred length reload value.
    length_reload: u8,
    /// Counter value observed when the reload was written.
    length_prev: u8,
    /// Halt flag.
    pub halt: bool,
    /// Constant-volume flag.
    pub constant: bool,
    /// Envelope period / volume.
    pub volume: u8,
    envelope_divider: u8,
    envelope_level: u8,
    envelope_start: bool,
    /// Mode flag (short LFSR).
    pub mode: bool,
    /// Period index.
    pub period_index: u8,
    timer: u16,
    lfsr: u16,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            enabled: false,
            length: 0,
            length_reload: 0,
            length_prev: 0,
            halt: false,
            constant: false,
            volume: 0,
            envelope_divider: 0,
            envelope_level: 0,
            envelope_start: false,
            mode: false,
            period_index: 0,
            timer: NOISE_PERIODS[0],
            lfsr: 1,
        }
    }
}

impl Noise {
    fn write(&mut self, index: u16, value: u8) {
        match index {
            0 => {
                self.halt = value & 0x20 != 0;
                self.constant = value & 0x10 != 0;
                self.volume = value & 0x0F;
            }
            2 => {
                self.mode = value & 0x80 != 0;
                self.period_index = value & 0x0F;
            }
            3 => {
                if self.enabled {
                    self.length_reload = LENGTH_TABLE[usize::from(value >> 3)];
                    self.length_prev = self.length;
                }
                self.envelope_start = true;
            }
            _ => {}
        }
    }

    fn reload_length(&mut self) {
        if self.length_reload > 0 {
            if self.length == self.length_prev {
                self.length = self.length_reload;
            }
            self.length_reload = 0;
        }
    }

    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = NOISE_PERIODS[usize::from(self.period_index & 0x0F)];
            let bit = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.mode {
                self.lfsr = (self.lfsr & !0x40) | (bit << 6);
            }
        } else {
            self.timer -= 1;
        }
    }

    fn clock_quarter(&mut self) {
        if self.envelope_start {
            self.envelope_start = false;
            self.envelope_level = 15;
            self.envelope_divider = self.volume;
        } else if self.envelope_divider == 0 {
            self.envelope_divider = self.volume;
            if self.envelope_level > 0 {
                self.envelope_level -= 1;
            } else if self.halt {
                self.envelope_level = 15;
            }
        } else {
            self.envelope_divider -= 1;
        }
    }

    fn clock_half(&mut self) {
        if !self.halt && self.length > 0 {
            self.length -= 1;
        }
    }

    fn output(&self) -> f32 {
        if !self.enabled || self.length == 0 || (self.lfsr & 1) != 0 {
            0.0
        } else if self.constant {
            f32::from(self.volume)
        } else {
            f32::from(self.envelope_level)
        }
    }
}

/// Delta modulation channel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Dmc {
    /// Enabled.
    pub enabled: bool,
    /// IRQ enabled.
    pub irq_enabled: bool,
    /// Loop flag.
    pub looped: bool,
    /// Timer period index.
    pub period_index: u8,
    timer: u16,
    /// Current output level.
    pub level: u8,
    /// Sample buffer.
    pub sample_buffer: u8,
    /// Whether the sample buffer is empty.
    pub buffer_empty: bool,
    /// Bits remaining in the current sample byte.
    bits_remaining: u8,
    /// Shift register.
    shift: u8,
    /// Output muted (empty buffer).
    silence: bool,
    /// Cycles to wait before the first fetch after a start.
    init: u8,
    /// Cycles until a re-enable write to `$4015` takes effect.
    enable_delay: u8,
    /// A fetch request was held because the enable latch was still running.
    held_request: bool,
    /// CPU cycles until a `$4015` D4=0 write takes effect (2 or 3).
    disable_delay: u8,
    /// The delayed stop took effect on this CPU cycle.
    disable_event: bool,
    /// A sample byte fetch is pending.
    pub dma_pending: bool,
    /// Reload address from `$4012`.
    pub sample_address: u16,
    /// Reload length from `$4013`.
    pub length: u16,
    /// Current address.
    pub address: u16,
    /// Bytes remaining.
    pub bytes_remaining: u16,
    /// IRQ flag.
    pub irq: bool,
}

impl Default for Dmc {
    fn default() -> Self {
        Self {
            enabled: false,
            irq_enabled: false,
            looped: false,
            period_index: 0,
            timer: DMC_PERIODS[0] - 1,
            level: 0,
            sample_buffer: 0,
            buffer_empty: true,
            bits_remaining: 8,
            shift: 0,
            silence: true,
            init: 0,
            enable_delay: 0,
            held_request: false,
            disable_delay: 0,
            disable_event: false,
            dma_pending: false,
            sample_address: 0xC000,
            length: 1,
            address: 0xC000,
            bytes_remaining: 0,
            irq: false,
        }
    }
}

impl Dmc {
    fn write(&mut self, index: u16, value: u8) {
        match index {
            0 => {
                self.irq_enabled = value & 0x80 != 0;
                if !self.irq_enabled {
                    self.irq = false;
                }
                self.looped = value & 0x40 != 0;
                self.period_index = value & 0x0F;
            }
            1 => self.level = value & 0x7F,
            2 => self.sample_address = 0xC000 | (u16::from(value) << 6),
            3 => {
                self.length = (u16::from(value) << 4) | 1;
            }
            _ => {}
        }
    }

    fn init_sample(&mut self, _cycle: u64, write_get: Option<bool>) {
        self.address = self.sample_address;
        self.bytes_remaining = self.length;
        // The start delay is selected by the DMC timer parity and only applies
        // while the sample buffer is empty. `timer` here is one less than the
        // reference period counter, so its parity is complemented.
        self.init = if !self.buffer_empty {
            0
        } else {
            match write_get {
                // A write that lands on a get pushes the load DMA out to the
                // get cycle of the second following APU cycle. The one
                // exception is the implicit-abort window, where the fetched
                // byte is being handed to the shift register as the timer
                // rolls over.
                Some(true) if !(self.bits_remaining == 1 && self.timer < 2) => 4,
                // The abort window, put writes and the standalone harnesses
                // keep the timer-parity calibration.
                _ => {
                    if self.timer & 1 == 0 {
                        2
                    } else {
                        3
                    }
                }
            }
        };
    }

    /// Counts the `$4015` enable delay.
    fn clock_enable_delay(&mut self) {
        self.enable_delay = self.enable_delay.saturating_sub(1);
    }

    /// Counts the `$4015` disable delay.
    ///
    /// The write stops playback, but the byte counter is only cleared two or
    /// three CPU cycles later, so a fetch that starts inside that window is
    /// cancelled or aborted instead of completing.
    fn clock_disable_delay(&mut self) {
        if self.disable_delay > 0 {
            self.disable_delay -= 1;
            if self.disable_delay == 0 {
                self.bytes_remaining = 0;
                self.enable_delay = 0;
                self.held_request = false;
                self.disable_event = true;
            }
        }
    }

    /// Counts the start delay and requests the first byte when ready.
    fn clock_init(&mut self) {
        if self.init > 0 {
            self.init -= 1;
            if self.init == 0 && self.buffer_empty && self.bytes_remaining > 0 {
                self.dma_pending = true;
            }
        }
    }

    /// Stores a fetched sample byte, returning whether the sample ended.
    fn load_buffer(&mut self, byte: u8) {
        self.dma_pending = false;
        if self.bytes_remaining > 0 {
            self.sample_buffer = byte;
            self.buffer_empty = false;
            if self.address == 0xFFFF {
                self.address = 0x8000;
            } else {
                self.address = self.address.wrapping_add(1);
            }
            self.bytes_remaining -= 1;
            if self.bytes_remaining == 0 {
                if self.looped {
                    // Loop restart reloads the address/length immediately; the
                    // $4015 enable delay only applies to the first fetch.
                    self.address = self.sample_address;
                    self.bytes_remaining = self.length;
                } else if self.irq_enabled {
                    self.irq = true;
                }
            }
        }
    }

    fn clock_timer(&mut self) {
        if self.timer == 0 {
            let period = DMC_PERIODS[usize::from(self.period_index & 0x0F)];
            self.timer = period - 1;
            self.step_output();
        } else {
            self.timer -= 1;
        }
    }

    fn step_output(&mut self) {
        if !self.silence {
            if self.shift & 1 != 0 {
                if self.level <= 125 {
                    self.level += 2;
                }
            } else if self.level >= 2 {
                self.level -= 2;
            }
            self.shift >>= 1;
        }
        if self.bits_remaining > 0 {
            self.bits_remaining -= 1;
        }
        if self.bits_remaining == 0 {
            self.bits_remaining = 8;
            self.silence = self.buffer_empty;
            if !self.buffer_empty {
                self.shift = self.sample_buffer;
                self.buffer_empty = true;
                if self.bytes_remaining > 0 {
                    self.dma_pending = true;
                }
            }
        }
    }

    fn output(&self) -> f32 {
        // The 7-bit DAC output reaches the mixer regardless of the `$4015`
        // enable bit; only the automatic playback unit is gated. An empty
        // sample buffer stops the counter from changing, it does not mute it.
        // Games silence the channel by writing `$4011` instead.
        f32::from(self.level)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum FrameMode {
    Four,
    Five,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FrameCounter {
    counter: u16,
    step: usize,
    mode: FrameMode,
    inhibit: bool,
    irq_pending: bool,
    odd: bool,
    delay_irq: bool,
    delay_counter: [bool; 2],
}

const PERIOD4: [u16; 6] = [7457, 7456, 7458, 7457, 1, 1];
const PERIOD5: [u16; 6] = [7457, 7456, 7458, 7458, 7452, 1];

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct FrameEvents {
    quarter: bool,
    half: bool,
}

impl Default for FrameCounter {
    fn default() -> Self {
        Self {
            counter: PERIOD4[0],
            step: 0,
            mode: FrameMode::Four,
            inhibit: false,
            irq_pending: false,
            odd: true,
            delay_irq: false,
            delay_counter: [false; 2],
        }
    }
}

impl FrameCounter {
    fn period(&self) -> u16 {
        match self.mode {
            FrameMode::Four => PERIOD4[self.step],
            FrameMode::Five => PERIOD5[self.step],
        }
    }

    fn write(&mut self, value: u8) {
        self.mode = if value & 0x80 != 0 {
            FrameMode::Five
        } else {
            FrameMode::Four
        };
        self.delay_counter[1] = true;
        self.inhibit = value & 0x40 != 0;
        if self.inhibit {
            self.irq_pending = false;
        }
    }

    fn tick(&mut self, phase: Option<bool>) -> FrameEvents {
        let mut events = FrameEvents::default();
        self.counter = self.counter.wrapping_sub(1);
        self.odd = phase.unwrap_or(!self.odd);
        if !self.odd {
            // The IRQ flag is cleared on the GET cycle after a $4015 read.
            if self.delay_irq {
                self.delay_irq = false;
                self.irq_pending = false;
            }
            if self.delay_counter[0] {
                self.step = 0;
                self.counter = self.period();
                if self.mode == FrameMode::Five {
                    events.half = true;
                }
            }
            self.delay_counter[0] = self.delay_counter[1];
            self.delay_counter[1] = false;
        }
        if self.counter == 0 {
            match self.step {
                0 => {
                    self.step = 1;
                    events.quarter = true;
                }
                1 => {
                    self.step = 2;
                    events.half = true;
                }
                2 => {
                    self.step = 3;
                    events.quarter = true;
                }
                3 => {
                    self.step = 4;
                    if self.mode == FrameMode::Four {
                        self.irq_pending = true;
                    }
                }
                4 => {
                    self.step = 5;
                    events.half = true;
                    if self.mode == FrameMode::Four {
                        self.irq_pending = true;
                    }
                }
                _ => {
                    self.step = 0;
                    if self.mode == FrameMode::Four {
                        self.irq_pending = !self.inhibit;
                    }
                }
            }
            self.counter = self.period();
        }
        events
    }
}

/// The full APU.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Apu {
    /// CPU get/put phase of the current `$4015` write, when known.
    ///
    /// `None` keeps the timer-parity start delay used by the standalone APU
    /// harnesses; the system bus sets the real phase.
    pub write_get: Option<bool>,
    /// Pulse 1.
    pub pulse1: Pulse,
    /// Pulse 2.
    pub pulse2: Pulse,
    /// Triangle.
    pub triangle: Triangle,
    /// Noise.
    pub noise: Noise,
    /// DMC.
    pub dmc: Dmc,
    frame: FrameCounter,
    /// Optional CPU get/put phase supplied by the system each cycle.
    pub phase_override: Option<bool>,
    /// Frame-counter IRQ flag.
    pub frame_irq: bool,
    cycle: u64,
    /// Mixed output from the last cycle.
    pub output: f32,
    /// Last written `$4015` value.
    status: u8,
    /// Halt-bit changes waiting for the next APU cycle (applied after the
    /// length-counter clock, matching hardware write latency).
    pending_halt: [Option<bool>; 4],
}

impl Apu {
    /// Creates an APU in its power-on state.
    #[must_use]
    pub fn new() -> Self {
        let mut apu = Self::default();
        apu.pulse1.ones_complement = true;
        apu
    }

    /// Advances one CPU cycle.
    pub fn tick(&mut self) {
        self.cycle += 1;
        self.dmc.clock_enable_delay();
        // The pulse sequencers run at the APU rate (every other CPU cycle);
        // the triangle runs at the CPU rate and the noise/DMC tables are
        // already expressed in CPU cycles.
        if self.cycle & 1 == 1 {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
        }
        self.triangle.clock_timer();
        self.noise.clock_timer();
        self.dmc.clock_timer();
        self.dmc.clock_init();
        self.dmc.clock_disable_delay();
        self.clock_frame();
        self.pulse1.reload_length();
        self.pulse2.reload_length();
        self.triangle.reload_length();
        self.noise.reload_length();
        for index in 0..4 {
            if let Some(halt) = self.pending_halt[index].take() {
                self.set_halt(index, halt);
            }
        }
        self.output = self.mix();
    }

    fn clock_frame(&mut self) {
        let phase = self.phase_override.take();
        let events = self.frame.tick(phase);
        if events.quarter {
            self.pulse1.clock_quarter();
            self.pulse2.clock_quarter();
            self.triangle.clock_quarter();
            self.noise.clock_quarter();
        }
        if events.half {
            self.pulse1.clock_half();
            self.pulse2.clock_half();
            self.triangle.clock_half();
            self.noise.clock_half();
        }
        self.frame_irq = self.frame.irq_pending;
    }

    /// Writes a register in `$4000..=$4017`.
    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x4000..=0x4003 => {
                let old = self.pulse1.halt;
                self.pulse1.write(address - 0x4000, value);
                self.delay_halt(0, old, self.pulse1.halt);
            }
            0x4004..=0x4007 => {
                let old = self.pulse2.halt;
                self.pulse2.write(address - 0x4004, value);
                self.delay_halt(1, old, self.pulse2.halt);
            }
            0x4008..=0x400B => {
                let old = self.triangle.control;
                self.triangle.write(address - 0x4008, value);
                self.delay_halt(2, old, self.triangle.control);
            }
            0x400C..=0x400F => {
                let old = self.noise.halt;
                self.noise.write(address - 0x400C, value);
                self.delay_halt(3, old, self.noise.halt);
            }
            0x4010..=0x4013 => self.dmc.write(address - 0x4010, value),
            0x4015 => {
                self.status = value;
                self.dmc.irq = false;
                self.pulse1.enabled = value & 0x01 != 0;
                self.pulse2.enabled = value & 0x02 != 0;
                self.triangle.enabled = value & 0x04 != 0;
                self.noise.enabled = value & 0x08 != 0;
                let was_enabled = self.dmc.enabled;
                self.dmc.enabled = value & 0x10 != 0;
                if value & 0x01 == 0 {
                    self.pulse1.length = 0;
                }
                if value & 0x02 == 0 {
                    self.pulse2.length = 0;
                }
                if value & 0x04 == 0 {
                    self.triangle.length = 0;
                }
                if value & 0x08 == 0 {
                    self.noise.length = 0;
                }
                if value & 0x10 == 0 {
                    // The stop takes effect two or three CPU cycles later and
                    // is re-armed by a second write only once it has expired.
                    if self.dmc.disable_delay == 0 {
                        self.dmc.disable_delay = if self.cycle & 1 == 0 { 2 } else { 3 };
                    }
                } else {
                    let in_flight = !self.dmc.buffer_empty || self.dmc.bits_remaining < 8;
                    if self.dmc.bytes_remaining == 0 {
                        self.dmc.init_sample(self.cycle, self.write_get);
                    }
                    if !was_enabled && in_flight {
                        // A channel re-enabled while a byte is still shifting
                        // takes three CPU cycles to become effective.
                        self.dmc.enable_delay = 3;
                    }
                }
            }
            0x4017 => {
                self.frame.write(value);
                if value & 0x40 != 0 {
                    self.frame_irq = false;
                }
            }
            _ => {}
        }
    }

    fn delay_halt(&mut self, index: usize, old: bool, new: bool) {
        if new != old {
            self.pending_halt[index] = Some(new);
            self.set_halt(index, old);
        }
    }

    fn set_halt(&mut self, index: usize, halt: bool) {
        match index {
            0 => self.pulse1.halt = halt,
            1 => self.pulse2.halt = halt,
            2 => self.triangle.control = halt,
            _ => self.noise.halt = halt,
        }
    }

    /// Reads `$4015` and clears the frame IRQ flag.
    pub fn read_status(&mut self) -> u8 {
        let value = self.status_bits();
        // The flag is actually cleared on the next GET cycle; reading $4015
        // only requests the clear.
        self.frame.delay_irq = true;
        value
    }

    /// `$4015` value without the read side effect, for debuggers.
    #[must_use]
    pub fn status_peek(&self) -> u8 {
        self.status_bits()
    }

    fn status_bits(&self) -> u8 {
        let mut value = 0;
        if self.pulse1.length > 0 {
            value |= 0x01;
        }
        if self.pulse2.length > 0 {
            value |= 0x02;
        }
        if self.triangle.length > 0 {
            value |= 0x04;
        }
        if self.noise.length > 0 {
            value |= 0x08;
        }
        if self.dmc.bytes_remaining > 0 {
            value |= 0x10;
        }
        if self.frame.irq_pending {
            value |= 0x40;
        }
        if self.dmc.irq {
            value |= 0x80;
        }
        value
    }

    /// IRQ line state.
    #[must_use]
    pub fn irq_line(&self) -> bool {
        (self.frame.irq_pending && !self.frame.inhibit) || self.dmc.irq
    }

    /// Whether the DMC needs a sample byte from CPU memory.
    #[must_use]
    pub fn dmc_needs_sample(&mut self) -> bool {
        if self.dmc.dma_pending && self.dmc.enable_delay > 0 {
            // The request is already queued; it is only held off until the
            // enable latch expires, which makes the eventual transfer shorter.
            self.dmc.held_request = true;
            return false;
        }
        self.dmc.dma_pending
    }

    /// Whether a sample byte fetch is waiting for its halt cycle.
    #[must_use]
    pub fn dmc_request_pending(&self) -> bool {
        self.dmc.dma_pending
    }

    /// Drops a queued sample byte fetch.
    pub fn take_dmc_request(&mut self) {
        self.dmc.dma_pending = false;
    }

    /// Whether the delayed `$4015` stop took effect on this cycle.
    #[must_use]
    pub fn dmc_disable_event(&self) -> bool {
        self.dmc.disable_event
    }

    /// Takes the "the delayed `$4015` stop just took effect" marker.
    pub fn take_dmc_disable_event(&mut self) -> bool {
        std::mem::take(&mut self.dmc.disable_event)
    }

    /// Takes the "this fetch was held by the enable latch" marker.
    pub fn take_dmc_held(&mut self) -> bool {
        std::mem::take(&mut self.dmc.held_request)
    }

    /// Advances the DMC timer by one extra cycle for a held request.
    pub fn dmc_catch_up(&mut self) {
        self.dmc.clock_timer();
    }

    /// Supplies a byte fetched for the DMC.
    pub fn dmc_sample(&mut self, byte: u8) {
        self.dmc.load_buffer(byte);
    }

    /// Debug view of the five channel DAC outputs in the order
    /// `[pulse1, pulse2, triangle, noise, dmc]`, before mixing.
    #[must_use]
    pub fn channel_outputs(&self) -> [f32; 5] {
        [
            self.pulse1.output(),
            self.pulse2.output(),
            self.triangle.output(),
            self.noise.output(),
            self.dmc.output(),
        ]
    }

    /// Mixed output in the range `0..~100`.
    #[must_use]
    pub fn mix(&self) -> f32 {
        // Exact lookup of the two non-linear mixer curves; the sums are
        // integer-valued channel outputs so the tables reproduce the formula
        // bit-for-bit without the per-cycle divisions.
        let pulse = (self.pulse1.output() + self.pulse2.output()) as usize;
        let tnd = 3 * self.triangle.output() as usize
            + 2 * self.noise.output() as usize
            + self.dmc.output() as usize;
        PULSE_MIX[pulse.min(PULSE_MIX.len() - 1)] + TND_MIX[tnd.min(TND_MIX.len() - 1)]
    }

    /// Current frame-counter position `(step, cycle)`.
    #[must_use]
    pub const fn frame_position(&self) -> (usize, u32) {
        (self.frame.step, self.frame.counter as u32)
    }

    /// Cycles elapsed.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_timer_period_matches_the_reference_probe() {
        // Mirrors the differential probe ROM: enable pulse 1, 50% duty at
        // constant volume 15, timer $40, length index 1. A per-cycle trace of
        // the reference (RustyNES) shows every sequencer step exactly
        // 2 * (timer + 1) CPU cycles apart, so consecutive output edges of a
        // 50% duty pattern are eight timer periods apart.
        let mut apu = Apu::new();
        apu.write(0x4015, 0x01);
        apu.write(0x4000, 0xBF);
        apu.write(0x4002, 0x40);
        apu.write(0x4003, 0x08);
        let mut edges = Vec::new();
        let mut last = 0.0;
        for cycle in 0..2000u32 {
            apu.tick();
            let output = apu.channel_outputs()[0];
            if output != last {
                edges.push(cycle);
                last = output;
            }
        }
        assert!(
            edges.len() >= 4,
            "expected several duty edges, got {edges:?}"
        );
        // 50% duty means four silent and four audible sequencer steps, so
        // consecutive output edges are four timer periods apart.
        let edge_period = 8 * (0x40 + 1);
        for pair in edges.windows(2).skip(1) {
            assert_eq!(
                pair[1] - pair[0],
                edge_period,
                "duty edges must be four timer periods apart"
            );
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn mixer_uses_the_nonlinear_curves() {
        let pulse_15 = 95.52 / (8128.0 / 15.0 + 100.0);
        assert!((PULSE_MIX[15] - pulse_15).abs() < 1e-6);
        let tnd_82 = 163.67 / (24329.0 / 82.0 + 100.0);
        assert!((TND_MIX[82] - tnd_82).abs() < 1e-6);
        // The curve is compressive: doubling the sum does not double the output.
        assert!(PULSE_MIX[20] < 2.0 * PULSE_MIX[10]);
        assert!(TND_MIX[100] < 2.0 * TND_MIX[50]);
    }

    #[test]
    fn pulse_sequencer_clocks_at_the_apu_rate() {
        // The pulse duty sequencer runs at half the CPU rate: with timer = 4,
        // one step is 2 * (4 + 1) = 10 CPU cycles. Clocking it every CPU
        // cycle (the old behaviour) produced steps every 5 cycles and doubled
        // the pitch.
        let mut apu = Apu::new();
        apu.write(0x4015, 0x01);
        apu.write(0x4000, 0xBF); // duty 2, halt, constant volume 15
        apu.write(0x4002, 0x04);
        apu.write(0x4003, 0x08);
        let mut last = apu.pulse1.duty_step;
        let mut changes = Vec::new();
        for _ in 0..120 {
            apu.tick();
            let step = apu.pulse1.duty_step;
            if step != last {
                changes.push(apu.cycle);
                last = step;
            }
        }
        assert!(changes.len() >= 3, "sequencer did not advance: {changes:?}");
        let interval = changes[2] - changes[1];
        assert_eq!(interval, 10, "duty step interval was {interval} cycles");
    }

    #[test]
    fn dmc_dac_always_feeds_the_mixer() {
        // NESdev APU DMC: the 7-bit counter output "is sent to the mixer
        // regardless the channel is enabled or not", and a $4011 write
        // updates it even with automatic playback deactivated.
        let mut apu = Apu::new();
        apu.write(0x4011, 0x40);
        assert_eq!(apu.channel_outputs()[4], 64.0);
        assert!(apu.mix() > 0.0, "the DAC bypassed the mixer");
        apu.write(0x4015, 0x00);
        apu.tick();
        assert_eq!(apu.channel_outputs()[4], 64.0);
        assert!(apu.mix() > 0.0, "the disabled channel was muted");
        apu.write(0x4011, 0x7F);
        assert_eq!(apu.channel_outputs()[4], 127.0);
    }

    #[test]
    fn length_table_loads_on_write() {
        let mut apu = Apu::new();
        apu.write(0x4015, 0x01);
        apu.write(0x4003, 0xF8);
        // The length reload is applied on the next APU update.
        apu.tick();
        assert!(apu.pulse1.length > 0);
        assert_eq!(apu.read_status() & 1, 1);
    }

    #[test]
    fn frame_irq_sets_and_clears() {
        let mut apu = Apu::new();
        for _ in 0..30_000 {
            apu.tick();
        }
        assert!(apu.frame_irq);
        let status = apu.read_status();
        assert_eq!(status & 0x40, 0x40);
        // The flag clears on the next GET cycle, not immediately.
        apu.tick();
        assert!(!apu.frame_irq);
    }
}
