//! CPU bus contract.
//!
//! The bus owns timing: every `read`/`write` is one CPU cycle, and the CPU calls
//! [`CpuBus::tick`] for internal cycles. The PPU/APU advance from `tick`, so a
//! bus that does nothing is still enough to run CPU-only test ROMs.

/// What the 6502 can ask of the machine.
pub trait CpuBus {
    /// Reads one byte. The implementation must advance exactly one CPU cycle
    /// (PPU/APU catch-up) before or after the value is produced, consistently.
    fn read(&mut self, address: u16) -> u8;

    /// Writes one byte and advances exactly one CPU cycle.
    fn write(&mut self, address: u16, value: u8);

    /// Advances one internal CPU cycle with no bus access.
    fn tick(&mut self);

    /// Current NMI line state.
    fn nmi_line(&self) -> bool {
        false
    }

    /// Current IRQ line state (level, active low on hardware).
    fn irq_line(&self) -> bool {
        false
    }

    /// Takes a latched NMI edge, if the machine provides one.
    ///
    /// Buses without an edge latch can leave the default and rely on
    /// [`CpuBus::nmi_line`].
    fn take_nmi(&mut self) -> bool {
        false
    }

    /// Whether NMI edges are suppressed for the current frame.
    fn nmi_suppressed(&self) -> bool {
        false
    }

    /// Extra CPU cycles consumed by DMA or other stalls since the last call.
    fn take_stall(&mut self) -> u32 {
        0
    }

    /// Records the CPU program counter for peripherals that observe the
    /// 6502 address bus during DMA.
    fn set_cpu_pc(&mut self, _pc: u16) {}

    /// Called once at the start of every CPU cycle, before the bus access.
    ///
    /// Implementations advance the rest of the machine and may start or step a
    /// DMA here. Buses without timing can leave the default.
    fn begin_cycle(&mut self, _address: Option<u16>, _write: Option<u8>) {}

    /// Completes the PPU portion of the current CPU cycle.
    fn end_cycle(&mut self) {}

    /// Whether a DMA engine used the bus on the bus cycle that just ran.
    fn dma_event(&self) -> bool {
        false
    }

    /// Marks the dummy write cycle of a read-modify-write instruction.
    fn set_rmw_dummy(&mut self, _dummy: bool) {}

    /// Whether a DMA engine currently owns the bus, so the CPU must wait.
    fn dma_active(&self) -> bool {
        false
    }
}

/// A flat 64 KiB RAM bus for CPU unit tests.
///
/// This is not a NES memory map; it exists so the CPU can be tested in
/// isolation before the system bus lands.
pub struct RamBus {
    /// Flat address space.
    pub memory: Box<[u8; 0x10000]>,
    /// CPU cycles elapsed.
    pub cycles: u64,
    /// NMI line.
    pub nmi: bool,
    /// IRQ line.
    pub irq: bool,
}

impl RamBus {
    /// Empty bus with a reset vector at `0x8000`.
    #[must_use]
    pub fn new() -> Self {
        let mut memory = Box::new([0u8; 0x10000]);
        memory[0xFFFC] = 0x00;
        memory[0xFFFD] = 0x80;
        Self {
            memory,
            cycles: 0,
            nmi: false,
            irq: false,
        }
    }
}

impl Default for RamBus {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuBus for RamBus {
    fn read(&mut self, address: u16) -> u8 {
        self.cycles += 1;
        self.memory[address as usize]
    }

    fn write(&mut self, address: u16, value: u8) {
        self.cycles += 1;
        self.memory[address as usize] = value;
    }

    fn tick(&mut self) {
        self.cycles += 1;
    }

    fn nmi_line(&self) -> bool {
        self.nmi
    }

    fn irq_line(&self) -> bool {
        self.irq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_bus_counts_cycles() {
        let mut bus = RamBus::new();
        bus.write(0x1234, 0xAB);
        assert_eq!(bus.read(0x1234), 0xAB);
        bus.tick();
        assert_eq!(bus.cycles, 3);
    }
}
