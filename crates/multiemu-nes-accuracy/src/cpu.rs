//! Cycle-stepped 6502 / Ricoh 2A03.
//!
//! Every CPU cycle is a real bus access or an explicit internal cycle, so the
//! wrapper (PPU/APU/DMA) can advance in lockstep. Instruction semantics follow
//! the public 6502 documentation; the 2A03 decimal flag is ignored for `ADC`
//! and `SBC`, as on the NES.

use crate::bus::CpuBus;

/// Processor status register bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Status(u8);

impl Status {
    /// Carry.
    pub const CARRY: u8 = 0x01;
    /// Zero.
    pub const ZERO: u8 = 0x02;
    /// IRQ disable.
    pub const INTERRUPT: u8 = 0x04;
    /// Decimal (ignored by the 2A03; kept for completeness).
    pub const DECIMAL: u8 = 0x08;
    /// Break (only meaningful when pushed).
    pub const BREAK: u8 = 0x10;
    /// Unused, reads as 1.
    pub const UNUSED: u8 = 0x20;
    /// Overflow.
    pub const OVERFLOW: u8 = 0x40;
    /// Negative.
    pub const NEGATIVE: u8 = 0x80;

    /// Raw bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Builds from raw bits, forcing the unused bit high.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits | Self::UNUSED)
    }

    /// Whether a bit is set.
    #[must_use]
    pub const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    /// Sets or clears a bit.
    pub fn set(&mut self, flag: u8, value: bool) {
        if value {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
        self.0 |= Self::UNUSED;
    }

    /// Sets or clears a bit and returns the previous value.
    pub fn replace(&mut self, flag: u8, value: bool) -> bool {
        let previous = self.contains(flag);
        self.set(flag, value);
        previous
    }

    /// Updates Z and N from a value.
    pub fn set_zn(&mut self, value: u8) {
        self.set(Self::ZERO, value == 0);
        self.set(Self::NEGATIVE, value & 0x80 != 0);
    }
}

/// Visible 6502 registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registers {
    /// Accumulator.
    pub a: u8,
    /// X index.
    pub x: u8,
    /// Y index.
    pub y: u8,
    /// Stack pointer (low byte of `$0100`).
    pub sp: u8,
    /// Program counter.
    pub pc: u16,
    /// Status.
    pub p: Status,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            p: Status::from_bits(Status::INTERRUPT),
        }
    }
}

/// The CPU state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cpu {
    /// Registers.
    pub registers: Registers,
    /// Cycles executed since power-on.
    pub cycles: u64,
    /// Set by a KIL/JAM opcode.
    pub halted: bool,
    /// High byte of the literal (pre-index) address of the last indexed access.
    base_high: u8,
    /// Last observed NMI line level.
    last_nmi: bool,
    /// Last observed IRQ line level.
    last_irq: bool,
    /// NMI edge latched by the CPU.
    nmi_latched: bool,
    /// Latch state during the penultimate cycle of the current instruction.
    prev_latched: bool,
    /// IRQ line state during the penultimate cycle.
    prev_irq: bool,
    /// Service an NMI at the next instruction boundary.
    service_nmi: bool,
    /// Service an IRQ at the next instruction boundary.
    service_irq: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    ZpX,
    ZpY,
    Abs,
    AbsX,
    AbsY,
    IndX,
    IndY,
    Rel,
    Ind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Penalty {
    Read,
    Store,
    Rmw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Adc,
    And,
    Asl,
    Bcc,
    Bcs,
    Beq,
    Bit,
    Bmi,
    Bne,
    Bpl,
    Brk,
    Bvc,
    Bvs,
    Clc,
    Cld,
    Cli,
    Clv,
    Cmp,
    Cpx,
    Cpy,
    Dec,
    Dex,
    Dey,
    Eor,
    Inc,
    Inx,
    Iny,
    Jmp,
    Jsr,
    Lda,
    Ldx,
    Ldy,
    Lsr,
    Nop,
    Ora,
    Pha,
    Php,
    Pla,
    Plp,
    Rol,
    Ror,
    Rti,
    Rts,
    Sbc,
    Sec,
    Sed,
    Sei,
    Sta,
    Stx,
    Sty,
    Tax,
    Tay,
    Tsx,
    Txa,
    Txs,
    Tya,
    Slo,
    Rla,
    Sre,
    Rra,
    Sax,
    Lax,
    Dcp,
    Isc,
    Anc,
    Alr,
    Arr,
    Xaa,
    Axs,
    Ahx,
    Shy,
    Shx,
    Tas,
    Las,
    Kil,
}

impl Cpu {
    /// Power-on state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Performs the 7-cycle reset sequence.
    pub fn reset(&mut self, bus: &mut impl CpuBus) {
        self.registers.p = Status::from_bits(Status::INTERRUPT);
        self.registers.pc = 0;

        self.read(bus, 0);
        self.read(bus, 0);
        for _ in 0..3 {
            let address = 0x0100 | u16::from(self.registers.sp);
            self.read(bus, address);
            self.registers.sp = self.registers.sp.wrapping_sub(1);
        }
        let low = self.read(bus, 0xFFFC);
        let high = self.read(bus, 0xFFFD);
        self.registers.pc = u16::from(low) | (u16::from(high) << 8);
        self.halted = false;
        self.last_nmi = false;
        self.last_irq = false;
        self.nmi_latched = false;
        self.prev_latched = false;
        self.prev_irq = false;
        self.service_nmi = false;
        self.service_irq = false;
    }

    /// Executes one instruction (or interrupt) and returns its cycle count.
    pub fn step(&mut self, bus: &mut impl CpuBus) -> u32 {
        if self.halted {
            return 0;
        }
        let start = self.cycles;
        bus.set_cpu_pc(self.registers.pc);

        if self.service_nmi {
            self.service_nmi = false;
            self.nmi_latched = false;
            self.interrupt(bus, 0xFFFA, false);
        } else if self.service_irq {
            self.service_irq = false;
            self.interrupt(bus, 0xFFFE, false);
        } else {
            let opcode = self.fetch(bus);
            let i_before = self.registers.p.contains(Status::INTERRUPT);
            self.execute(bus, opcode);

            // The 6502 polls interrupt lines during the penultimate cycle.
            if self.prev_latched && !bus.nmi_suppressed() {
                self.service_nmi = true;
            }
            if bus.nmi_suppressed() {
                self.nmi_latched = false;
            }
            // CLI, SEI and PLP update the I flag on their final cycle, after
            // the interrupt poll has already sampled the old value.
            let poll_i = if matches!(opcode, 0x58 | 0x78 | 0x28) {
                i_before
            } else {
                self.registers.p.contains(Status::INTERRUPT)
            };
            if self.prev_irq && !poll_i {
                self.service_irq = true;
            }
        }

        self.cycles += u64::from(bus.take_stall());

        (self.cycles - start) as u32
    }

    fn interrupt(&mut self, bus: &mut impl CpuBus, vector: u16, brk: bool) {
        self.idle(bus);
        self.idle(bus);
        self.push(bus, (self.registers.pc >> 8) as u8);
        self.push(bus, self.registers.pc as u8);
        let mut p = self.registers.p;
        p.set(Status::BREAK, brk);
        self.push(bus, p.bits());
        self.registers.p.set(Status::INTERRUPT, true);
        let low = self.read(bus, vector);
        let high = self.read(bus, vector + 1);
        self.registers.pc = u16::from(low) | (u16::from(high) << 8);
    }

    fn dummy_read_pc(&mut self, bus: &mut impl CpuBus) {
        let pc = self.registers.pc;
        self.read(bus, pc);
    }

    fn dummy_read_stack(&mut self, bus: &mut impl CpuBus) {
        let address = 0x0100 | u16::from(self.registers.sp);
        self.read(bus, address);
    }

    fn execute(&mut self, bus: &mut impl CpuBus, opcode: u8) {
        let (kind, mode) = decode(opcode);
        match kind {
            Kind::Adc => {
                let value = self.read_value(bus, mode);
                self.adc(bus, value);
            }
            Kind::Sbc => {
                let value = self.read_value(bus, mode);
                self.sbc(bus, value);
            }
            Kind::And => {
                let value = self.read_value(bus, mode);
                self.registers.a &= value;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
            }
            Kind::Ora => {
                let value = self.read_value(bus, mode);
                self.registers.a |= value;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
            }
            Kind::Eor => {
                let value = self.read_value(bus, mode);
                self.registers.a ^= value;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
            }
            Kind::Lda => {
                let value = self.read_value(bus, mode);
                self.registers.a = value;
                self.registers.p.set_zn(value);
            }
            Kind::Ldx => {
                let value = self.read_value(bus, mode);
                self.registers.x = value;
                self.registers.p.set_zn(value);
            }
            Kind::Ldy => {
                let value = self.read_value(bus, mode);
                self.registers.y = value;
                self.registers.p.set_zn(value);
            }
            Kind::Lax => {
                let value = self.read_value(bus, mode);
                self.registers.a = value;
                self.registers.x = value;
                self.registers.p.set_zn(value);
            }
            Kind::Bit => {
                let value = self.read_value(bus, mode);
                self.registers
                    .p
                    .set(Status::ZERO, self.registers.a & value == 0);
                self.registers.p.set(Status::NEGATIVE, value & 0x80 != 0);
                self.registers.p.set(Status::OVERFLOW, value & 0x40 != 0);
            }
            Kind::Cmp => {
                let value = self.read_value(bus, mode);
                self.compare(self.registers.a, value);
            }
            Kind::Cpx => {
                let value = self.read_value(bus, mode);
                self.compare(self.registers.x, value);
            }
            Kind::Cpy => {
                let value = self.read_value(bus, mode);
                self.compare(self.registers.y, value);
            }
            Kind::Sta => {
                let address = self.address(bus, mode, Penalty::Store);
                self.write(bus, address, self.registers.a);
            }
            Kind::Stx => {
                let address = self.address(bus, mode, Penalty::Store);
                self.write(bus, address, self.registers.x);
            }
            Kind::Sty => {
                let address = self.address(bus, mode, Penalty::Store);
                self.write(bus, address, self.registers.y);
            }
            Kind::Sax => {
                let address = self.address(bus, mode, Penalty::Store);
                self.write(bus, address, self.registers.a & self.registers.x);
            }
            Kind::Asl => self.rmw(bus, mode, |cpu, value| {
                cpu.registers.p.set(Status::CARRY, value & 0x80 != 0);
                let value = value << 1;
                cpu.registers.p.set_zn(value);
                value
            }),
            Kind::Lsr => self.rmw(bus, mode, |cpu, value| {
                cpu.registers.p.set(Status::CARRY, value & 0x01 != 0);
                let value = value >> 1;
                cpu.registers.p.set_zn(value);
                value
            }),
            Kind::Rol => self.rmw(bus, mode, |cpu, value| {
                let carry = u8::from(cpu.registers.p.contains(Status::CARRY));
                cpu.registers.p.set(Status::CARRY, value & 0x80 != 0);
                let value = (value << 1) | carry;
                cpu.registers.p.set_zn(value);
                value
            }),
            Kind::Ror => self.rmw(bus, mode, |cpu, value| {
                let carry = if cpu.registers.p.contains(Status::CARRY) {
                    0x80
                } else {
                    0
                };
                cpu.registers.p.set(Status::CARRY, value & 0x01 != 0);
                let value = (value >> 1) | carry;
                cpu.registers.p.set_zn(value);
                value
            }),
            Kind::Inc => self.rmw(bus, mode, |cpu, value| {
                let value = value.wrapping_add(1);
                cpu.registers.p.set_zn(value);
                value
            }),
            Kind::Dec => self.rmw(bus, mode, |cpu, value| {
                let value = value.wrapping_sub(1);
                cpu.registers.p.set_zn(value);
                value
            }),
            Kind::Slo => self.rmw(bus, mode, |cpu, value| {
                cpu.registers.p.set(Status::CARRY, value & 0x80 != 0);
                let shifted = value << 1;
                cpu.registers.a |= shifted;
                cpu.registers.p.set_zn(cpu.registers.a);
                shifted
            }),
            Kind::Rla => self.rmw(bus, mode, |cpu, value| {
                let carry = u8::from(cpu.registers.p.contains(Status::CARRY));
                cpu.registers.p.set(Status::CARRY, value & 0x80 != 0);
                let rotated = (value << 1) | carry;
                cpu.registers.a &= rotated;
                cpu.registers.p.set_zn(cpu.registers.a);
                rotated
            }),
            Kind::Sre => self.rmw(bus, mode, |cpu, value| {
                cpu.registers.p.set(Status::CARRY, value & 0x01 != 0);
                let shifted = value >> 1;
                cpu.registers.a ^= shifted;
                cpu.registers.p.set_zn(cpu.registers.a);
                shifted
            }),
            Kind::Rra => self.rmw(bus, mode, |cpu, value| {
                let carry = if cpu.registers.p.contains(Status::CARRY) {
                    0x80
                } else {
                    0
                };
                cpu.registers.p.set(Status::CARRY, value & 0x01 != 0);
                let rotated = (value >> 1) | carry;
                let value = rotated;
                cpu.adc_value(value);
                rotated
            }),
            Kind::Dcp => self.rmw(bus, mode, |cpu, value| {
                let value = value.wrapping_sub(1);
                cpu.compare(cpu.registers.a, value);
                value
            }),
            Kind::Isc => self.rmw(bus, mode, |cpu, value| {
                let value = value.wrapping_add(1);
                cpu.sbc_value(value);
                value
            }),
            Kind::Anc => {
                let value = self.fetch(bus);
                self.registers.a &= value;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
                self.registers.p.set(Status::CARRY, a & 0x80 != 0);
            }
            Kind::Alr => {
                let value = self.fetch(bus);
                self.registers.a &= value;
                self.registers
                    .p
                    .set(Status::CARRY, self.registers.a & 0x01 != 0);
                self.registers.a >>= 1;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
            }
            Kind::Arr => {
                let value = self.fetch(bus);
                self.registers.a &= value;
                let carry = if self.registers.p.contains(Status::CARRY) {
                    0x80
                } else {
                    0
                };
                self.registers.a = (self.registers.a >> 1) | carry;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
                self.registers.p.set(Status::CARRY, a & 0x40 != 0);
                self.registers
                    .p
                    .set(Status::OVERFLOW, (a ^ (a << 1)) & 0x40 != 0);
            }
            Kind::Xaa => {
                let value = self.fetch(bus);
                self.registers.a = self.registers.x & value;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
            }
            Kind::Axs => {
                let value = self.fetch(bus);
                let base = self.registers.a & self.registers.x;
                let result = base.wrapping_sub(value);
                self.registers.x = result;
                self.registers.p.set(Status::CARRY, base >= value);
                self.registers.p.set_zn(result);
            }
            Kind::Ahx => {
                let address = self.address(bus, mode, Penalty::Store);
                let value = if bus.dma_event() {
                    // A DMA drove the data bus instead of the high-byte fetch,
                    // so the "H+1" AND term is not part of the value.
                    self.registers.a & self.registers.x
                } else {
                    self.registers.a & self.registers.x & self.unstable_high(address)
                };
                self.unstable_store(bus, address, value);
            }
            Kind::Shy => {
                let address = self.address(bus, mode, Penalty::Store);
                let value = if bus.dma_event() {
                    self.registers.y
                } else {
                    self.registers.y & self.unstable_high(address)
                };
                self.unstable_store(bus, address, value);
            }
            Kind::Shx => {
                let address = self.address(bus, mode, Penalty::Store);
                let value = if bus.dma_event() {
                    self.registers.x
                } else {
                    self.registers.x & self.unstable_high(address)
                };
                self.unstable_store(bus, address, value);
            }
            Kind::Tas => {
                self.registers.sp = self.registers.a & self.registers.x;
                let address = self.address(bus, mode, Penalty::Store);
                let value = if bus.dma_event() {
                    self.registers.sp
                } else {
                    self.registers.sp & self.unstable_high(address)
                };
                self.unstable_store(bus, address, value);
            }
            Kind::Las => {
                let address = self.address(bus, mode, Penalty::Read);
                let value = self.read(bus, address) & self.registers.sp;
                self.registers.a = value;
                self.registers.x = value;
                self.registers.sp = value;
                self.registers.p.set_zn(value);
            }
            Kind::Jmp => match mode {
                Mode::Abs => {
                    let low = self.fetch(bus);
                    let high = self.fetch(bus);
                    self.registers.pc = u16::from(low) | (u16::from(high) << 8);
                }
                Mode::Ind => {
                    let pointer = self.address(bus, Mode::Ind, Penalty::Read);
                    let low = self.read(bus, pointer);
                    let high = self.read(
                        bus,
                        (pointer & 0xFF00) | (u16::from(pointer as u8).wrapping_add(1) & 0x00FF),
                    );
                    self.registers.pc = u16::from(low) | (u16::from(high) << 8);
                }
                _ => unreachable!("JMP mode"),
            },
            Kind::Jsr => {
                let low = self.fetch(bus);
                self.idle(bus);
                let pc = self.registers.pc;
                self.push(bus, (pc >> 8) as u8);
                self.push(bus, pc as u8);
                let high = self.fetch(bus);
                self.registers.pc = u16::from(low) | (u16::from(high) << 8);
            }
            Kind::Rts => {
                self.idle(bus);
                self.idle(bus);
                let low = self.pull(bus);
                let high = self.pull(bus);
                self.idle(bus);
                self.registers.pc = (u16::from(low) | (u16::from(high) << 8)).wrapping_add(1);
            }
            Kind::Rti => {
                self.idle(bus);
                let p = self.pull(bus);
                self.registers.p = Status::from_bits(p & !Status::BREAK);
                let low = self.pull(bus);
                let high = self.pull(bus);
                self.idle(bus);
                self.registers.pc = u16::from(low) | (u16::from(high) << 8);
            }
            Kind::Brk => {
                self.fetch(bus);
                self.push(bus, (self.registers.pc >> 8) as u8);
                self.push(bus, self.registers.pc as u8);
                let mut p = self.registers.p;
                p.set(Status::BREAK, true);
                self.push(bus, p.bits());
                self.registers.p.set(Status::INTERRUPT, true);
                let low = self.read(bus, 0xFFFE);
                let high = self.read(bus, 0xFFFF);
                self.registers.pc = u16::from(low) | (u16::from(high) << 8);
            }
            Kind::Pha => {
                self.dummy_read_pc(bus);
                self.push(bus, self.registers.a);
            }
            Kind::Php => {
                self.dummy_read_pc(bus);
                let mut p = self.registers.p;
                p.set(Status::BREAK, true);
                self.push(bus, p.bits());
            }
            Kind::Pla => {
                self.dummy_read_pc(bus);
                self.dummy_read_stack(bus);
                let value = self.pull(bus);
                self.registers.a = value;
                self.registers.p.set_zn(value);
            }
            Kind::Plp => {
                self.dummy_read_pc(bus);
                self.dummy_read_stack(bus);
                let p = self.pull(bus);
                self.registers.p = Status::from_bits(p & !Status::BREAK);
            }
            Kind::Tax => {
                self.dummy_read_pc(bus);
                self.registers.x = self.registers.a;
                let x = self.registers.x;
                self.registers.p.set_zn(x);
            }
            Kind::Tay => {
                self.dummy_read_pc(bus);
                self.registers.y = self.registers.a;
                let y = self.registers.y;
                self.registers.p.set_zn(y);
            }
            Kind::Txa => {
                self.dummy_read_pc(bus);
                self.registers.a = self.registers.x;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
            }
            Kind::Tya => {
                self.dummy_read_pc(bus);
                self.registers.a = self.registers.y;
                let a = self.registers.a;
                self.registers.p.set_zn(a);
            }
            Kind::Tsx => {
                self.dummy_read_pc(bus);
                self.registers.x = self.registers.sp;
                let x = self.registers.x;
                self.registers.p.set_zn(x);
            }
            Kind::Txs => {
                self.dummy_read_pc(bus);
                self.registers.sp = self.registers.x;
            }
            Kind::Inx => {
                self.dummy_read_pc(bus);
                self.registers.x = self.registers.x.wrapping_add(1);
                let x = self.registers.x;
                self.registers.p.set_zn(x);
            }
            Kind::Iny => {
                self.dummy_read_pc(bus);
                self.registers.y = self.registers.y.wrapping_add(1);
                let y = self.registers.y;
                self.registers.p.set_zn(y);
            }
            Kind::Dex => {
                self.dummy_read_pc(bus);
                self.registers.x = self.registers.x.wrapping_sub(1);
                let x = self.registers.x;
                self.registers.p.set_zn(x);
            }
            Kind::Dey => {
                self.dummy_read_pc(bus);
                self.registers.y = self.registers.y.wrapping_sub(1);
                let y = self.registers.y;
                self.registers.p.set_zn(y);
            }
            Kind::Clc => {
                self.dummy_read_pc(bus);
                self.registers.p.set(Status::CARRY, false);
            }
            Kind::Sec => {
                self.dummy_read_pc(bus);
                self.registers.p.set(Status::CARRY, true);
            }
            Kind::Cli => {
                self.dummy_read_pc(bus);
                self.registers.p.set(Status::INTERRUPT, false);
            }
            Kind::Sei => {
                self.dummy_read_pc(bus);
                self.registers.p.set(Status::INTERRUPT, true);
            }
            Kind::Clv => {
                self.dummy_read_pc(bus);
                self.registers.p.set(Status::OVERFLOW, false);
            }
            Kind::Cld => {
                self.dummy_read_pc(bus);
                self.registers.p.set(Status::DECIMAL, false);
            }
            Kind::Sed => {
                self.dummy_read_pc(bus);
                self.registers.p.set(Status::DECIMAL, true);
            }
            Kind::Nop => match mode {
                Mode::Imp => self.dummy_read_pc(bus),
                Mode::Imm => {
                    self.fetch(bus);
                }
                _ => {
                    let address = self.address(bus, mode, Penalty::Read);
                    self.read(bus, address);
                }
            },
            Kind::Bpl => self.branch(bus, !self.registers.p.contains(Status::NEGATIVE)),
            Kind::Bmi => self.branch(bus, self.registers.p.contains(Status::NEGATIVE)),
            Kind::Bvc => self.branch(bus, !self.registers.p.contains(Status::OVERFLOW)),
            Kind::Bvs => self.branch(bus, self.registers.p.contains(Status::OVERFLOW)),
            Kind::Bcc => self.branch(bus, !self.registers.p.contains(Status::CARRY)),
            Kind::Bcs => self.branch(bus, self.registers.p.contains(Status::CARRY)),
            Kind::Bne => self.branch(bus, !self.registers.p.contains(Status::ZERO)),
            Kind::Beq => self.branch(bus, self.registers.p.contains(Status::ZERO)),
            Kind::Kil => self.halted = true,
        }
    }

    fn adc(&mut self, _bus: &mut impl CpuBus, value: u8) {
        self.adc_value(value);
    }

    fn adc_value(&mut self, value: u8) {
        let a = self.registers.a;
        let carry = u16::from(self.registers.p.contains(Status::CARRY));
        let sum = u16::from(a) + u16::from(value) + carry;
        self.registers.p.set(Status::CARRY, sum > 0xFF);
        self.registers.p.set(
            Status::OVERFLOW,
            (!(a ^ value) & (a ^ (sum as u8)) & 0x80) != 0,
        );
        self.registers.a = sum as u8;
        let result = self.registers.a;
        self.registers.p.set_zn(result);
    }

    fn sbc(&mut self, _bus: &mut impl CpuBus, value: u8) {
        self.sbc_value(value);
    }

    fn sbc_value(&mut self, value: u8) {
        self.adc_value(!value);
    }

    fn compare(&mut self, register: u8, value: u8) {
        let result = register.wrapping_sub(value);
        self.registers.p.set(Status::CARRY, register >= value);
        self.registers.p.set_zn(result);
    }

    fn branch(&mut self, bus: &mut impl CpuBus, condition: bool) {
        let offset = self.fetch(bus) as i8;
        if condition {
            // Cycle 3: dummy read of the next opcode address.
            self.read(bus, self.registers.pc);
            let old = self.registers.pc;
            let target = old.wrapping_add(offset as u16);
            if (old & 0xFF00) != (target & 0xFF00) {
                // Cycle 4: dummy read at the old high byte with the new low byte.
                self.read(bus, (old & 0xFF00) | (target & 0x00FF));
            }
            self.registers.pc = target;
        }
    }

    fn read_value(&mut self, bus: &mut impl CpuBus, mode: Mode) -> u8 {
        match mode {
            Mode::Imm => self.fetch(bus),
            Mode::Acc => self.registers.a,
            _ => {
                let address = self.address(bus, mode, Penalty::Read);
                self.read(bus, address)
            }
        }
    }

    fn rmw(
        &mut self,
        bus: &mut impl CpuBus,
        mode: Mode,
        operation: impl FnOnce(&mut Self, u8) -> u8,
    ) {
        if mode == Mode::Acc {
            self.dummy_read_pc(bus);
            let value = self.registers.a;
            self.registers.a = operation(self, value);
            let a = self.registers.a;
            self.registers.p.set_zn(a);
            return;
        }
        let address = self.address(bus, mode, Penalty::Rmw);
        let value = self.read(bus, address);
        bus.set_rmw_dummy(true);
        self.write(bus, address, value);
        bus.set_rmw_dummy(false);
        let result = operation(self, value);
        self.write(bus, address, result);
        let _ = result;
    }

    fn address(&mut self, bus: &mut impl CpuBus, mode: Mode, penalty: Penalty) -> u16 {
        match mode {
            Mode::Imp | Mode::Acc | Mode::Imm => 0,
            Mode::Zp => u16::from(self.fetch(bus)),
            Mode::ZpX => {
                let base = self.fetch(bus);
                self.read(bus, u16::from(base));
                u16::from(base.wrapping_add(self.registers.x))
            }
            Mode::ZpY => {
                let base = self.fetch(bus);
                self.read(bus, u16::from(base));
                u16::from(base.wrapping_add(self.registers.y))
            }
            Mode::Abs => {
                let low = self.fetch(bus);
                let high = self.fetch(bus);
                u16::from(low) | (u16::from(high) << 8)
            }
            Mode::AbsX => {
                let low = self.fetch(bus);
                let high = self.fetch(bus);
                let base = u16::from(low) | (u16::from(high) << 8);
                self.base_high = (base >> 8) as u8;
                let address = base.wrapping_add(u16::from(self.registers.x));
                let crossed = (base & 0xFF00) != (address & 0xFF00);
                if crossed || penalty != Penalty::Read {
                    self.read(bus, (base & 0xFF00) | (address & 0x00FF));
                }
                address
            }
            Mode::AbsY => {
                let low = self.fetch(bus);
                let high = self.fetch(bus);
                let base = u16::from(low) | (u16::from(high) << 8);
                self.base_high = (base >> 8) as u8;
                let address = base.wrapping_add(u16::from(self.registers.y));
                let crossed = (base & 0xFF00) != (address & 0xFF00);
                if crossed || penalty != Penalty::Read {
                    self.read(bus, (base & 0xFF00) | (address & 0x00FF));
                }
                address
            }
            Mode::IndX => {
                let base = self.fetch(bus);
                self.read(bus, u16::from(base));
                let pointer = base.wrapping_add(self.registers.x);
                let low = self.read(bus, u16::from(pointer));
                let high = self.read(bus, u16::from(pointer.wrapping_add(1)));
                u16::from(low) | (u16::from(high) << 8)
            }
            Mode::IndY => {
                let pointer = self.fetch(bus);
                let low = self.read(bus, u16::from(pointer));
                let high = self.read(bus, u16::from(pointer.wrapping_add(1)));
                let base = u16::from(low) | (u16::from(high) << 8);
                self.base_high = (base >> 8) as u8;
                let address = base.wrapping_add(u16::from(self.registers.y));
                let crossed = (base & 0xFF00) != (address & 0xFF00);
                if crossed || penalty != Penalty::Read {
                    self.read(bus, (base & 0xFF00) | (address & 0x00FF));
                }
                address
            }
            Mode::Rel => {
                self.fetch(bus);
                0
            }
            Mode::Ind => {
                let low = self.fetch(bus);
                let high = self.fetch(bus);
                u16::from(low) | (u16::from(high) << 8)
            }
        }
    }

    /// Constant used by SHA/SHX/SHY/SHS. Without a page crossing it is the
    /// literal high byte + 1; with one it is the computed high byte.
    fn unstable_high(&self, address: u16) -> u8 {
        let computed = (address >> 8) as u8;
        if computed != self.base_high {
            computed
        } else {
            self.base_high.wrapping_add(1)
        }
    }

    /// SH*/TAS stores corrupt the address high byte on a page crossing: the
    /// high byte becomes the stored value (the same AND term used for data).
    fn unstable_store(&mut self, bus: &mut impl CpuBus, address: u16, value: u8) {
        let computed = (address >> 8) as u8;
        let store_address = if computed != self.base_high {
            (u16::from(value) << 8) | (address & 0x00FF)
        } else {
            address
        };
        self.write(bus, store_address, value);
    }

    fn fetch(&mut self, bus: &mut impl CpuBus) -> u8 {
        let value = self.read(bus, self.registers.pc);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        value
    }

    fn read(&mut self, bus: &mut impl CpuBus, address: u16) -> u8 {
        self.wait_for_dma(bus, Some(address), None);
        self.cycles += 1;
        bus.begin_cycle(Some(address), None);
        let value = bus.read(address);
        bus.end_cycle();
        self.sample_cycle(bus);
        value
    }

    fn write(&mut self, bus: &mut impl CpuBus, address: u16, value: u8) {
        self.wait_for_dma(bus, Some(address), Some(value));
        self.cycles += 1;
        bus.begin_cycle(Some(address), Some(value));
        bus.write(address, value);
        bus.end_cycle();
        self.sample_cycle(bus);
    }

    fn idle(&mut self, bus: &mut impl CpuBus) {
        self.wait_for_dma(bus, None, None);
        self.cycles += 1;
        bus.begin_cycle(None, None);
        bus.tick();
        bus.end_cycle();
        self.sample_cycle(bus);
    }

    fn wait_for_dma(&mut self, bus: &mut impl CpuBus, address: Option<u16>, write: Option<u8>) {
        while bus.dma_active() {
            self.cycles += 1;
            bus.begin_cycle(address, write);
            bus.end_cycle();
        }
    }

    fn sample_cycle(&mut self, bus: &impl CpuBus) {
        self.prev_latched = self.nmi_latched;
        let nmi = bus.nmi_line();
        if nmi && !self.last_nmi {
            self.nmi_latched = true;
        }
        self.last_nmi = nmi;

        self.prev_irq = self.last_irq;
        self.last_irq = bus.irq_line();
    }

    fn push(&mut self, bus: &mut impl CpuBus, value: u8) {
        let address = 0x0100 | u16::from(self.registers.sp);
        self.write(bus, address, value);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
    }

    fn pull(&mut self, bus: &mut impl CpuBus) -> u8 {
        self.registers.sp = self.registers.sp.wrapping_add(1);
        self.read(bus, 0x0100 | u16::from(self.registers.sp))
    }
}

fn decode(opcode: u8) -> (Kind, Mode) {
    use Kind::*;
    use Mode::*;
    match opcode {
        0x00 => (Brk, Imp),
        0x01 => (Ora, IndX),
        0x05 => (Ora, Zp),
        0x06 => (Asl, Zp),
        0x08 => (Php, Imp),
        0x09 => (Ora, Imm),
        0x0A => (Asl, Acc),
        0x0D => (Ora, Abs),
        0x0E => (Asl, Abs),
        0x10 => (Bpl, Rel),
        0x11 => (Ora, IndY),
        0x15 => (Ora, ZpX),
        0x16 => (Asl, ZpX),
        0x18 => (Clc, Imp),
        0x19 => (Ora, AbsY),
        0x1D => (Ora, AbsX),
        0x1E => (Asl, AbsX),
        0x20 => (Jsr, Abs),
        0x21 => (And, IndX),
        0x24 => (Bit, Zp),
        0x25 => (And, Zp),
        0x26 => (Rol, Zp),
        0x28 => (Plp, Imp),
        0x29 => (And, Imm),
        0x2A => (Rol, Acc),
        0x2C => (Bit, Abs),
        0x2D => (And, Abs),
        0x2E => (Rol, Abs),
        0x30 => (Bmi, Rel),
        0x31 => (And, IndY),
        0x35 => (And, ZpX),
        0x36 => (Rol, ZpX),
        0x38 => (Sec, Imp),
        0x39 => (And, AbsY),
        0x3D => (And, AbsX),
        0x3E => (Rol, AbsX),
        0x40 => (Rti, Imp),
        0x41 => (Eor, IndX),
        0x45 => (Eor, Zp),
        0x46 => (Lsr, Zp),
        0x48 => (Pha, Imp),
        0x49 => (Eor, Imm),
        0x4A => (Lsr, Acc),
        0x4C => (Jmp, Abs),
        0x4D => (Eor, Abs),
        0x4E => (Lsr, Abs),
        0x50 => (Bvc, Rel),
        0x51 => (Eor, IndY),
        0x55 => (Eor, ZpX),
        0x56 => (Lsr, ZpX),
        0x58 => (Cli, Imp),
        0x59 => (Eor, AbsY),
        0x5D => (Eor, AbsX),
        0x5E => (Lsr, AbsX),
        0x60 => (Rts, Imp),
        0x61 => (Adc, IndX),
        0x65 => (Adc, Zp),
        0x66 => (Ror, Zp),
        0x68 => (Pla, Imp),
        0x69 => (Adc, Imm),
        0x6A => (Ror, Acc),
        0x6C => (Jmp, Ind),
        0x6D => (Adc, Abs),
        0x6E => (Ror, Abs),
        0x70 => (Bvs, Rel),
        0x71 => (Adc, IndY),
        0x75 => (Adc, ZpX),
        0x76 => (Ror, ZpX),
        0x78 => (Sei, Imp),
        0x79 => (Adc, AbsY),
        0x7D => (Adc, AbsX),
        0x7E => (Ror, AbsX),
        0x81 => (Sta, IndX),
        0x84 => (Sty, Zp),
        0x85 => (Sta, Zp),
        0x86 => (Stx, Zp),
        0x88 => (Dey, Imp),
        0x8A => (Txa, Imp),
        0x8C => (Sty, Abs),
        0x8D => (Sta, Abs),
        0x8E => (Stx, Abs),
        0x90 => (Bcc, Rel),
        0x91 => (Sta, IndY),
        0x94 => (Sty, ZpX),
        0x95 => (Sta, ZpX),
        0x96 => (Stx, ZpY),
        0x98 => (Tya, Imp),
        0x99 => (Sta, AbsY),
        0x9A => (Txs, Imp),
        0x9D => (Sta, AbsX),
        0xA0 => (Ldy, Imm),
        0xA1 => (Lda, IndX),
        0xA2 => (Ldx, Imm),
        0xA4 => (Ldy, Zp),
        0xA5 => (Lda, Zp),
        0xA6 => (Ldx, Zp),
        0xA8 => (Tay, Imp),
        0xA9 => (Lda, Imm),
        0xAA => (Tax, Imp),
        0xAC => (Ldy, Abs),
        0xAD => (Lda, Abs),
        0xAE => (Ldx, Abs),
        0xB0 => (Bcs, Rel),
        0xB1 => (Lda, IndY),
        0xB4 => (Ldy, ZpX),
        0xB5 => (Lda, ZpX),
        0xB6 => (Ldx, ZpY),
        0xB8 => (Clv, Imp),
        0xB9 => (Lda, AbsY),
        0xBA => (Tsx, Imp),
        0xBC => (Ldy, AbsX),
        0xBD => (Lda, AbsX),
        0xBE => (Ldx, AbsY),
        0xC0 => (Cpy, Imm),
        0xC1 => (Cmp, IndX),
        0xC4 => (Cpy, Zp),
        0xC5 => (Cmp, Zp),
        0xC6 => (Dec, Zp),
        0xC8 => (Iny, Imp),
        0xC9 => (Cmp, Imm),
        0xCA => (Dex, Imp),
        0xCC => (Cpy, Abs),
        0xCD => (Cmp, Abs),
        0xCE => (Dec, Abs),
        0xD0 => (Bne, Rel),
        0xD1 => (Cmp, IndY),
        0xD5 => (Cmp, ZpX),
        0xD6 => (Dec, ZpX),
        0xD8 => (Cld, Imp),
        0xD9 => (Cmp, AbsY),
        0xDD => (Cmp, AbsX),
        0xDE => (Dec, AbsX),
        0xE0 => (Cpx, Imm),
        0xE1 => (Sbc, IndX),
        0xE4 => (Cpx, Zp),
        0xE5 => (Sbc, Zp),
        0xE6 => (Inc, Zp),
        0xE8 => (Inx, Imp),
        0xE9 | 0xEB => (Sbc, Imm),
        0xEA => (Nop, Imp),
        0xEC => (Cpx, Abs),
        0xED => (Sbc, Abs),
        0xEE => (Inc, Abs),
        0xF0 => (Beq, Rel),
        0xF1 => (Sbc, IndY),
        0xF5 => (Sbc, ZpX),
        0xF6 => (Inc, ZpX),
        0xF8 => (Sed, Imp),
        0xF9 => (Sbc, AbsY),
        0xFD => (Sbc, AbsX),
        0xFE => (Inc, AbsX),
        // Illegal opcodes.
        0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
            (Kil, Imp)
        }
        0x03 => (Slo, IndX),
        0x07 => (Slo, Zp),
        0x0F => (Slo, Abs),
        0x13 => (Slo, IndY),
        0x17 => (Slo, ZpX),
        0x1B => (Slo, AbsY),
        0x1F => (Slo, AbsX),
        0x23 => (Rla, IndX),
        0x27 => (Rla, Zp),
        0x2F => (Rla, Abs),
        0x33 => (Rla, IndY),
        0x37 => (Rla, ZpX),
        0x3B => (Rla, AbsY),
        0x3F => (Rla, AbsX),
        0x43 => (Sre, IndX),
        0x47 => (Sre, Zp),
        0x4F => (Sre, Abs),
        0x53 => (Sre, IndY),
        0x57 => (Sre, ZpX),
        0x5B => (Sre, AbsY),
        0x5F => (Sre, AbsX),
        0x63 => (Rra, IndX),
        0x67 => (Rra, Zp),
        0x6F => (Rra, Abs),
        0x73 => (Rra, IndY),
        0x77 => (Rra, ZpX),
        0x7B => (Rra, AbsY),
        0x7F => (Rra, AbsX),
        0x83 => (Sax, IndX),
        0x87 => (Sax, Zp),
        0x8F => (Sax, Abs),
        0x97 => (Sax, ZpY),
        0xA3 => (Lax, IndX),
        0xA7 => (Lax, Zp),
        0xAF => (Lax, Abs),
        0xB3 => (Lax, IndY),
        0xB7 => (Lax, ZpY),
        0xBF => (Lax, AbsY),
        0xAB => (Lax, Imm),
        0xC3 => (Dcp, IndX),
        0xC7 => (Dcp, Zp),
        0xCF => (Dcp, Abs),
        0xD3 => (Dcp, IndY),
        0xD7 => (Dcp, ZpX),
        0xDB => (Dcp, AbsY),
        0xDF => (Dcp, AbsX),
        0xE3 => (Isc, IndX),
        0xE7 => (Isc, Zp),
        0xEF => (Isc, Abs),
        0xF3 => (Isc, IndY),
        0xF7 => (Isc, ZpX),
        0xFB => (Isc, AbsY),
        0xFF => (Isc, AbsX),
        0x0B | 0x2B => (Anc, Imm),
        0x4B => (Alr, Imm),
        0x6B => (Arr, Imm),
        0x8B => (Xaa, Imm),
        0xCB => (Axs, Imm),
        0x93 => (Ahx, IndY),
        0x9F => (Ahx, AbsY),
        0x9C => (Shy, AbsX),
        0x9E => (Shx, AbsY),
        0x9B => (Tas, AbsY),
        0xBB => (Las, AbsY),
        0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => (Nop, Imp),
        0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => (Nop, Imm),
        0x04 | 0x44 | 0x64 => (Nop, Zp),
        0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => (Nop, ZpX),
        0x0C => (Nop, Abs),
        0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => (Nop, AbsX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::RamBus;

    #[test]
    fn status_bits() {
        let mut status = Status::default();
        status.set(Status::CARRY, true);
        assert!(status.contains(Status::CARRY));
        assert!(status.replace(Status::CARRY, false));
        assert!(!status.contains(Status::CARRY));
        assert!(status.contains(Status::UNUSED));
    }

    #[test]
    fn set_zn() {
        let mut status = Status::default();
        status.set_zn(0);
        assert!(status.contains(Status::ZERO));
        assert!(!status.contains(Status::NEGATIVE));
        status.set_zn(0x80);
        assert!(!status.contains(Status::ZERO));
        assert!(status.contains(Status::NEGATIVE));
    }

    #[test]
    fn reset_vectors_and_cycles() {
        let mut bus = RamBus::new();
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        assert_eq!(cpu.registers.pc, 0x8000);
        assert_eq!(cpu.registers.sp, 0xFA);
        assert!(cpu.registers.p.contains(Status::INTERRUPT));
        assert_eq!(cpu.cycles, 7);
        assert_eq!(bus.cycles, 7);
    }

    #[test]
    fn executes_lda_imm_and_sta_abs() {
        let mut bus = RamBus::new();
        let mut cpu = Cpu::new();
        bus.memory[0x8000] = 0xA9; // LDA #$42
        bus.memory[0x8001] = 0x42;
        bus.memory[0x8002] = 0x8D; // STA $1234
        bus.memory[0x8003] = 0x34;
        bus.memory[0x8004] = 0x12;
        cpu.registers.pc = 0x8000;
        assert_eq!(cpu.step(&mut bus), 2);
        assert_eq!(cpu.registers.a, 0x42);
        assert_eq!(cpu.step(&mut bus), 4);
        assert_eq!(bus.memory[0x1234], 0x42);
        assert_eq!(cpu.cycles, 6);
    }

    #[test]
    fn branches_take_extra_cycles() {
        let mut bus = RamBus::new();
        let mut cpu = Cpu::new();
        cpu.registers.pc = 0x8000;
        cpu.registers.p.set(Status::ZERO, true);
        bus.memory[0x8000] = 0xF0; // BEQ +$02
        bus.memory[0x8001] = 0x02;
        assert_eq!(cpu.step(&mut bus), 3);
        assert_eq!(cpu.registers.pc, 0x8004);
    }
}
