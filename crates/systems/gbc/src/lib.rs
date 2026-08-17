//! Game Boy Color emulation.
//!
//! The CGB reuses the shared SM83-based hardware modules from
//! `missingno-gb` through the generic [`Console`](missingno_gb::Console)
//! core; this crate supplies only the CGB-specific [`Model`] seams.
//! CGB behaviour (color palette memory, VRAM/WRAM banking, double-speed,
//! HDMA, object priority) attaches there.
//!
//! No SGB co-processor and no DMG OAM corruption bug — both are
//! DMG-only hardware quirks.
//!
//! ## Target SoC revision
//!
//! The CGB went through several CPU-SoC revisions (CPU-CGB-A through
//! CPU-CGB-E). Behaviour differs subtly between them — STOP/double-speed
//! wakeup timing, PPU mode-boundary alignment, STAT IRQ edges, APU
//! envelope retrigger, and so on. This crate targets **CPU-CGB-C**:
//! the most commonly-targeted revision across emulators (Gambatte's
//! `cgb04c`), the best-documented in test ROMs, and behaviourally
//! representative of the mainstream CGB hardware run.
//!
//! Test suites filter their ROM selection accordingly — CGB-E-only or
//! CGB-B-only ROMs are excluded from the CGB-C-passing set.

pub mod debug;
pub mod launch;
pub mod screen;
pub mod state_schema;
pub mod timing;

mod apu;
mod boundary;
mod bus;
mod compat_palette;
mod console_state;
mod cram;
mod dmg_palette_data;
mod obj_fifo;
mod post_boot;
mod ppu_model;
mod read_latch;
mod registers;
mod speed_switch;
mod vram;
mod vram_dma;

pub use apu::CgbApu;
pub use compat_palette::{DMG_COMPAT_BG, DMG_COMPAT_OBJ, dmg_compat_shade};
pub use console_state::CgbConsoleState;
pub use cram::{ColorRam, ColorRegister};
pub use debug::{CgbSnapshot, CgbView, cram_palettes};
pub use obj_fifo::CgbObjShifter;
pub use ppu_model::{CgbPpu, SyncedStatCells, TileSelResetGlitch};
pub use vram::{BgAttribute, CgbVram};

use missingno_gb::ppu::Ppu;
use missingno_gb::ppu::rendering::Mode;
use missingno_gb::{
    Chassis, Console, Model, audio::Audio, cartridge::Cartridge, cpu::Cpu, dma::Dma,
    joypad::Joypad, timers::Timers,
};

use crate::screen::Screen;
use crate::vram_dma::VramDma;

pub use crate::vram_dma::VramDmaStatus;

/// The Game Boy Color [`Model`]. Remaining CGB features (the color pixel
/// pipeline) attach here as they land.
pub struct Cgb {
    /// 8 × 4 KiB work-RAM banks. C000-CFFF is fixed bank 0; D000-DFFF is the
    /// SVBK-selected bank.
    wram: Box<[u8; 0x8000]>,
    /// SVBK ($FF70) bits 0-2 as written; the effective D000 bank is `max(svbk, 1)`.
    svbk: u8,
    /// KEY1 ($FF4D) bit 0 — speed-switch arm.
    key1_armed: bool,
    /// KEY1 ($FF4D) bit 7 — current speed (false = normal, true = double).
    /// The switch toggles it; the 2× clock cadence itself lands later.
    pub(crate) double_speed: bool,
    /// A DMG cartridge is running in compatibility mode (KEY0 bit 2). Read back
    /// from KEY0 ($FF4C) as $04.
    dmg_compat: bool,
    /// VRAM DMA ($FF51-55).
    vram_dma: VramDma,
    /// Remaining master edges of the double-speed switch blackout. The CPU
    /// clock is held (the dot clock / divider keep running off the master)
    /// until this drains, then the SM83 re-engages at the new speed and the
    /// dot-clock phase the count expired on. 0 = not switching.
    pub(crate) speed_switch_blackout: u32,
    /// HALT-wake intake countdown: a timer overflowing during the post-STOP
    /// HALT spends one WakeIntake M-cycle (the divider ticking through it)
    /// before its dispatch, like any HALT wake. `None` = no wake in flight;
    /// armed to `Some(1)` on the IF-set edge, re-engaging once it reaches 0.
    speed_switch_wake_latency: Option<u8>,
    /// Pre-ALET-rise XYMU (mode-3) state, sampled before this dot's ALET edge
    /// (where VOGA captures) — the pre-transition view a double-speed FF41 read's
    /// `data_phase_n↑` latch saw; `resolve_read_latch` resolves the read's STAT
    /// mode to it.
    pre_ppu_clock_rendering: bool,
    /// Pre-ALET-rise lock for a pending lockable (OAM/VRAM) read — the lock
    /// analogue of `pre_ppu_clock_rendering`. A double-speed read's `data_phase_n↑`
    /// latch saw this pre-transition lock; `resolve_read_latch` ORs it with the
    /// latch-edge lock so a mode-3→0 release between the two still floats.
    pre_ppu_clock_lock: Option<bool>,
    /// A pending OAM read's lock at the drive enable (tobe↑) — the
    /// single-speed decisive grant sample, taken before that fall's lock
    /// onset (`resolve_read_latch` consumes it).
    read_drive_oam_lock: Option<bool>,
    /// The clock mux lands displaced after a p3-entered 1×→2× relock; a
    /// displaced 2×→1× completes a dot early, and the next 1×→2× re-syncs.
    switch_relock_debit: bool,
    /// LY_old stashed at a double-speed mid-M LY tick with an FF44 read in
    /// flight; the read's latch ANDs it with the settled LY (mux ripple).
    ff44_ripple_old: Option<u8>,
    /// Undocumented CGB scratch registers: $FF72/$FF73 full bytes, $FF74
    /// (CGB mode only; open bus in compat), $FF75 bits 6-4 (the rest read 1).
    ff72: u8,
    ff73: u8,
    ff74: u8,
    ff75: u8,
    /// CGB ≤C extra OAM rows: 24 RAM bytes behind a decoder that ignores
    /// address bits 3-4 (three 8-byte rows at $FEA0/$FEC0/$FEE0, each
    /// aliased 4x in its block).
    extra_oam: [u8; 24],
    /// Console-level arbitration state (speed-switch blackout anchor, HDMA
    /// bus-park, VRAM-source OAM-zero conflict).
    console_state: CgbConsoleState,
}

impl Default for Cgb {
    fn default() -> Self {
        // WRAM powers on with the SRAM stripe pattern, not zeroed.
        let mut wram = Box::new([0u8; 0x8000]);
        for bank in wram.chunks_mut(0x2000) {
            missingno_gb::dmg_sram::fill(bank);
        }
        Self {
            wram,
            svbk: 1,
            key1_armed: false,
            double_speed: false,
            dmg_compat: false,
            vram_dma: VramDma::default(),
            speed_switch_blackout: 0,
            speed_switch_wake_latency: None,
            pre_ppu_clock_rendering: false,
            pre_ppu_clock_lock: None,
            read_drive_oam_lock: None,
            switch_relock_debit: false,
            ff44_ripple_old: None,
            ff72: 0,
            ff73: 0,
            ff74: 0,
            ff75: 0,
            extra_oam: [0; 24],
            console_state: CgbConsoleState::default(),
        }
    }
}

impl Cgb {
    /// KEY1 speed bit: the CPU is running at double speed.
    pub fn double_speed(&self) -> bool {
        self.double_speed
    }

    /// Effective D000-DFFF work-RAM bank (SVBK, floored to 1).
    pub fn wram_bank(&self) -> u8 {
        self.svbk.max(1)
    }

    /// SVBK ($FF70) bits 0-2 as written — the raw register value, distinct from
    /// the floored effective bank. The save-state capture keys on this so a
    /// written `0` round-trips (its FF70 read differs from `1`).
    pub fn svbk_register(&self) -> u8 {
        self.svbk
    }

    /// A read-only snapshot of the VRAM-DMA engine for the debugger.
    pub fn vram_dma_status(&self) -> VramDmaStatus {
        self.vram_dma.status()
    }

    /// The KEY1 arm bit and the undocumented $FF72-$FF75 scratch registers, as
    /// stored — for save-state capture.
    pub(crate) fn scratch_registers(&self) -> (bool, u8, u8, u8, u8) {
        (self.key1_armed, self.ff72, self.ff73, self.ff74, self.ff75)
    }

    /// The 24 bytes of CGB extra OAM ($FEA0-$FEFF), for save-state capture.
    pub(crate) fn extra_oam_bytes(&self) -> &[u8; 24] {
        &self.extra_oam
    }
}

impl Model for Cgb {
    type Ppu = CgbPpu;
    type Screen = Screen;
    const TRACE_MODEL_NAME: &'static str = "CGB-C";
    const HAS_PCM_REGISTERS: bool = true;
    const VRAM_BANKS: u8 = 2;
    const LCD_PANEL: missingno_core::LcdPanel = missingno_core::LcdPanel::ActiveTft;

    type ConsoleState = CgbConsoleState;
    type Apu = CgbApu;

    fn console_state(&self) -> &CgbConsoleState {
        &self.console_state
    }
    fn console_state_mut(&mut self) -> &mut CgbConsoleState {
        &mut self.console_state
    }

    /// The eight 4 KB WRAM banks, in bank order — the bank-complete image the
    /// debugger walks above the bus.
    fn wram_image(&self) -> Option<&[u8]> {
        Some(&self.wram[..])
    }

    fn selected_wram_bank(&self) -> Option<u8> {
        Some(self.wram_bank())
    }

    fn oam_dma_bus_conflict(&self, cpu_addr: u16, dma_source: u16) -> bool {
        bus::oam_dma_bus_conflict(cpu_addr, dma_source)
    }

    fn oam_dma_wram_remap(&self, cpu_addr: u16, dma_source: u16) -> Option<u16> {
        bus::oam_dma_wram_remap(cpu_addr, dma_source)
    }

    fn oam_dma_write_conflict_byte(&self, src_byte: u8, cpu_value: u8, dma_source: u16) -> u8 {
        bus::oam_dma_write_conflict_byte(src_byte, cpu_value, dma_source)
    }

    fn oam_dma_conflict_zeroes_oam(&self, cpu_addr: u16, dma_source: u16) -> bool {
        bus::oam_dma_conflict_zeroes_oam(cpu_addr, dma_source)
    }

    fn oam_dma_source_bank_write(&self, address: u16, dma_source: u16) -> bool {
        bus::oam_dma_source_bank_write(address, dma_source)
    }

    fn dma_source_open_bus(&self, address: u16) -> Option<u8> {
        bus::dma_source_open_bus(address)
    }

    fn cpu_post_boot(checksum: u8) -> Cpu {
        post_boot::cpu(checksum)
    }

    fn has_serial_fast_clock(&self) -> bool {
        !self.dmg_compat
    }

    // The T2-rise presample holds at both speeds — double speed shifts the
    // dot↔T-cycle ratio, not where in the M-cycle the comparator samples.
    const HALT_WAKE_SAMPLES_EARLY: bool = true;

    // Silicon property of the ack reset-hold, independent of DMG-compat.
    const IRQ_ACK_HOLDS_THROUGH_BOUNDARY_SET: bool = true;

    fn timers_post_boot(cgb_cart: bool) -> Timers {
        post_boot::timers(cgb_cart)
    }

    fn audio_post_boot(internal_counter: u16, cgb_cart: bool) -> Audio<CgbApu> {
        post_boot::audio(internal_counter, cgb_cart)
    }

    fn ppu_post_boot(cgb_cart: bool) -> Ppu<CgbPpu> {
        post_boot::ppu(cgb_cart)
    }

    fn joypad_post_boot() -> Joypad {
        post_boot::joypad()
    }

    fn dma_post_boot() -> Dma {
        post_boot::dma()
    }

    fn resolve_stop(&mut self, chassis: &mut Chassis<Self>) -> bool {
        self.attempt_speed_switch(chassis)
    }

    fn speed_switch_wake_ready(&mut self, mcycle_boundary: bool) -> bool {
        self.wake_intake_ready(mcycle_boundary)
    }

    fn speed_switch_in_progress(&self) -> bool {
        self.blackout_active()
    }

    fn drain_speed_switch_blackout(&mut self, elapsed: u32) -> bool {
        self.drain_blackout(elapsed)
    }

    fn speed_switch_divider_active(&self) -> bool {
        self.blackout_divider_active()
    }

    fn cpu_steps_per_dot(&self) -> u8 {
        self.steps_per_dot()
    }

    fn note_pre_ppu_clock_rendering(&mut self, rendering: bool) {
        self.set_pre_ppu_clock_rendering(rendering);
    }

    fn note_pre_ppu_clock_lock(&mut self, lock: Option<bool>) {
        self.set_pre_ppu_clock_lock(lock);
    }

    fn note_read_drive_phase(&mut self, oam_lock: Option<bool>) {
        self.set_read_drive_lock(oam_lock);
    }

    fn note_ff44_ripple_old(&mut self, ly: Option<u8>) {
        self.set_ff44_ripple_old(ly);
    }

    fn take_ff44_ripple_old(&mut self) -> Option<u8> {
        self.take_ff44_ripple()
    }

    fn resolve_read_latch(&self, address: u16, value: u8, latch_lock: Option<bool>) -> u8 {
        self.resolve_latched_read(address, value, latch_lock)
    }

    fn on_reset(&mut self, cartridge: &Cartridge, has_boot_rom: bool) {
        self.reset_for_cartridge(cartridge, has_boot_rom);
    }

    fn restore_work_ram(
        &mut self,
        _external: &mut missingno_gb::memory::ExternalBus,
        bytes: &[u8],
    ) {
        self.restore_wram_banks(bytes);
    }

    fn validate_boundary(
        &self,
        record: &missingno_core::state::StateRecord,
    ) -> Result<(), missingno_core::system::StateError> {
        boundary::check_double_speed(record)
    }

    fn restore_boundary_delta(
        &mut self,
        chassis: &mut Chassis<Self>,
        record: &missingno_core::state::StateRecord,
        memory: &[(String, Vec<u8>)],
    ) -> Result<(), missingno_core::system::StateError> {
        self.restore_delta(chassis, record, memory)
    }

    fn map_read(&self, address: u16, ppu: &Ppu<CgbPpu>, vram: &CgbVram) -> Option<u8> {
        self.map_read_byte(address, ppu, vram)
    }

    fn map_write(
        &mut self,
        address: u16,
        value: u8,
        ppu: &mut Ppu<CgbPpu>,
        vram: &mut CgbVram,
    ) -> bool {
        self.map_write_byte(address, value, ppu, vram)
    }

    fn vram_dma_edge(&mut self, chassis: &mut Chassis<Self>, mode: Mode) {
        self.vram_dma_fall_edge(chassis, mode);
    }

    fn vram_dma_drain_escape(&mut self) -> Option<(u16, u16)> {
        self.vram_dma.drain_escape()
    }

    fn vram_dma_park_waits_for_fetch(&self) -> bool {
        self.vram_dma.park_waits_for_fetch()
    }

    fn vram_dma_instruction_retired(&mut self) {
        self.vram_dma.instruction_retired();
    }

    fn vram_dma_request_standing(&self) -> bool {
        self.vram_dma.request_standing()
    }

    fn vram_dma_holds_cpu(&self) -> bool {
        self.vram_dma.holds_cpu()
    }

    fn vram_dma_lcd_disabled(&mut self) {
        self.vram_dma_on_lcd_disabled();
    }

    fn vram_dma_seizes_bus(&self) -> bool {
        self.vram_dma.seizes_bus()
    }

    fn vram_dma_conflict_source(&self, address: u16) -> Option<u16> {
        self.vram_dma_write_conflict_source(address)
    }

    fn vram_dma_arbitrate_oam(&mut self, chassis: &mut Chassis<Self>) -> bool {
        self.vram_dma_arbitrate_oam_bus(chassis)
    }

    fn vram_dma_boundary(&mut self, chassis: &mut Chassis<Self>) {
        self.vram_dma_commit_bytes(chassis);
    }
}

/// The Game Boy Color.
pub type GameBoyColor = Console<Cgb>;
