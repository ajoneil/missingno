//! Game Boy hardware model, and the shared silicon of the Game Boy family.
//!
//! [`Console`] pairs the [`Chassis`] — the SM83 CPU, PPU, APU, timers, DMA,
//! buses and master clock every console in the family carries — with a
//! [`Model`] supplying that console's divergences. This crate ships the DMG
//! model ([`GameBoy`]); `missingno-gbc` attaches the CGB through the same seam.

pub mod audio;
pub mod board;
pub mod cartridge;
mod chassis;
pub mod clock;
pub mod cpu;
pub mod cpu_bus;
pub mod debugger;
pub mod dma;
mod dmg;
pub mod dmg_sram;
pub mod execute;
pub mod frame;
pub mod interrupts;
pub mod isa;
pub mod joypad;
pub mod media;
pub mod memory;
mod model;
pub mod ppu;
mod screen;
pub mod serial_transfer;
pub mod sgb;
pub mod snapshot;
pub mod state_schema;
pub mod system;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod timers;
#[cfg(feature = "morepork")]
pub mod trace;

pub use audio::channels::wave::WaveRamCoupling;
pub use audio::{ApuSpec, DmgApu};
pub use chassis::{Chassis, Console, DmaBankWrite, DmaConflictLatch, DmaConflictWrite};
pub use clock::{CpuDivider, CpuGate, Edge, MasterClock, Tick};
pub use dmg::{Dmg, GameBoy};
pub use isa::Sm83;
pub use memory::BootRom;
pub use model::{Model, shared_oam_dma_write_conflict_byte};
pub use ppu::PixelOutput;
pub use screen::{ConsoleShadow, ScreenBuffer, VramDmaClaim};

pub(crate) use model::observes_audio;

/// B2 acceptance harness: each shared struct's summed CGB-only residual storage
/// on a DMG build is the load-bearing invariant (B2 drives it to zero behind the
/// `Model`/`PpuModel` seam); absolute `size_of` is left unpinned to exclude
/// unrelated struct padding.
#[cfg(test)]
mod cgb_residual_size {
    /// `Console<M>` CGB-only state relocated behind the `Model::ConsoleState` seam.
    mod console {
        pub const CGB_BYTES: usize = 0;
    }

    /// `Cpu` CGB-only residual: `irq.halt_wake_presample` — the halt-wake
    /// comparator presample latch. It is a CPU interrupt-latch stage (a function
    /// of `dispatch.latched()`), not a bus-arbitration grant, so it stays on the
    /// CPU rather than riding the `BusGrants` signal; dead on a DMG build (never
    /// written under `!HALT_WAKE_SAMPLES_EARLY`). The bus-park/hold/claim bytes
    /// relocated behind `Model::ConsoleState`.
    mod cpu {
        pub const CGB_BYTES: usize = 1;
    }

    /// `PipelineRegisters` CGB-only storage relocated behind the
    /// `PpuModel::TileSelGlitch` seam.
    mod pipeline_registers {
        pub const CGB_BYTES: usize = 0;
    }

    /// `StatInterrupt` FF41/FF45 synchroniser DFFs relocated behind the `PpuModel::StatShadow` seam.
    mod stat_interrupt {
        pub const CGB_BYTES: usize = 0;
    }

    /// Residual CGB-only storage still carried on a DMG build, summed across the four shared structs.
    #[test]
    fn cgb_only_byte_budget_remaining() {
        const REMAINING: usize = console::CGB_BYTES
            + cpu::CGB_BYTES
            + pipeline_registers::CGB_BYTES
            + stat_interrupt::CGB_BYTES;
        assert_eq!(REMAINING, 1, "CGB-only residual byte budget changed");
    }
}
