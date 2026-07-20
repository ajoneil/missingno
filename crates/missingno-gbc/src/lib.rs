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
pub mod render;
pub mod screen;
pub mod state_schema;
pub mod timing;

mod apu;
mod bus;
mod compat_palette;
mod console_state;
mod cram;
mod dmg_palette_data;
mod obj_fifo;
mod ppu_model;
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
use missingno_gb::ppu::memory::Vram;
use missingno_gb::ppu::rendering::Mode;
use missingno_gb::{
    Chassis, Console, ConsoleShadow, Model, VramDmaClaim,
    audio::Audio,
    cartridge::Cartridge,
    clock::CpuDivider,
    cpu::Cpu,
    cpu::flags::Flags,
    dma::Dma,
    joypad::{Buttons, Joypad},
    shared_oam_dma_write_conflict_byte,
    timers::Timers,
};

use crate::bus::{CgbBus, cgb_bus, cgb_dma_source_bus};
use crate::screen::Screen;
use crate::vram_dma::{TransferMode, VramDma};

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
    pre_alet_rendering: bool,
    /// Pre-ALET-rise lock for a pending lockable (OAM/VRAM) read — the lock
    /// analogue of `pre_alet_rendering`. A double-speed read's `data_phase_n↑`
    /// latch saw this pre-transition lock; `resolve_read_latch` ORs it with the
    /// latch-edge lock so a mode-3→0 release between the two still floats.
    pre_alet_lock: Option<bool>,
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
            pre_alet_rendering: false,
            pre_alet_lock: None,
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
    /// Index into `extra_oam` for a $FEA0-$FEFF address: row from address
    /// bits 6-5, offset from bits 2-0 (bits 3-4 ignored by the decoder).
    fn extra_oam_index(address: u16) -> usize {
        let row = ((address >> 5) & 0x7) as usize - 5;
        row * 8 + (address & 0x7) as usize
    }

    /// Index into `wram` for a work-RAM or echo-RAM address, else `None`.
    fn wram_index(&self, address: u16) -> Option<usize> {
        let bank = if self.svbk == 0 { 1 } else { self.svbk } as usize;
        let banked = |within: u16| bank * 0x1000 + within as usize;
        match address {
            0xC000..=0xCFFF => Some((address - 0xC000) as usize),
            0xD000..=0xDFFF => Some(banked(address - 0xD000)),
            0xE000..=0xEFFF => Some((address - 0xE000) as usize),
            0xF000..=0xFDFF => Some(banked(address - 0xF000)),
            _ => None,
        }
    }

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
        cgb_bus(cpu_addr) == Some(cgb_dma_source_bus(dma_source))
    }

    /// A WRAM-bus access taken while the DMA sources from the cart bus has its
    /// `$C000`/`$D000` half-selector (A12) driven by the DMA source page; the low
    /// 12 bits stay the CPU's. A VRAM or WRAM source leaves the access untouched.
    fn oam_dma_wram_remap(&self, cpu_addr: u16, dma_source: u16) -> Option<u16> {
        (cgb_bus(cpu_addr) == Some(CgbBus::WorkRam)
            && cgb_dma_source_bus(dma_source) == CgbBus::Cartridge)
            .then_some((dma_source & 0x1000) | (cpu_addr & 0x0FFF) | 0xC000)
    }

    /// On the WRAM bus the colliding CPU write sits on a different bus from the
    /// DMA source, so it never reaches the OAM write phase — the DMA deposits the
    /// raw byte it fetched. Other source buses follow the shared model.
    fn oam_dma_write_conflict_byte(&self, src_byte: u8, cpu_value: u8, dma_source: u16) -> u8 {
        if cgb_dma_source_bus(dma_source) == CgbBus::WorkRam {
            src_byte
        } else {
            shared_oam_dma_write_conflict_byte(src_byte, cpu_value, dma_source)
        }
    }

    fn oam_dma_conflict_zeroes_oam(&self, cpu_addr: u16, dma_source: u16) -> bool {
        cgb_dma_source_bus(dma_source) == CgbBus::Video && cgb_bus(cpu_addr) == Some(CgbBus::Video)
    }

    /// VBK re-banks a VRAM-source DMA, SVBK a WRAM-source DMA; the matching write
    /// latches one byte late so the coincident DMA byte reads the prior bank.
    fn oam_dma_source_bank_write(&self, address: u16, dma_source: u16) -> bool {
        match address {
            0xFF4F => cgb_dma_source_bus(dma_source) == CgbBus::Video,
            0xFF70 => cgb_dma_source_bus(dma_source) == CgbBus::WorkRam,
            _ => false,
        }
    }

    fn dma_source_open_bus(&self, address: u16) -> Option<u8> {
        (address >= 0xE000).then_some(0xFF)
    }

    fn cpu_post_boot(_checksum: u8) -> Cpu {
        // CPU-CGB-C post-boot register file. A=$11 signals CGB hardware to the
        // cartridge; unlike DMG, the flags don't depend on the header checksum.
        Cpu::post_boot_with(0x11, 0x00, 0x00, 0x00, 0x08, 0x00, 0x7c, Flags::ZERO)
    }

    fn has_serial_fast_clock(&self) -> bool {
        !self.dmg_compat
    }

    const DOUBLE_SPEED: bool = true;

    // The T2-rise presample holds at both speeds — double speed shifts the
    // dot↔T-cycle ratio, not where in the M-cycle the comparator samples.
    const HALT_WAKE_SAMPLES_EARLY: bool = true;

    // Silicon property of the ack reset-hold, independent of DMG-compat.
    const IRQ_ACK_HOLDS_THROUGH_BOUNDARY_SET: bool = true;

    /// CGB boot-ROM handoff divider phase. The boot ROM runs longer for a
    /// DMG cartridge (compat-palette setup): FF04 reads $1E / $26.
    fn timers_post_boot(cgb_cart: bool) -> Timers {
        Timers::post_boot_with_counter(if cgb_cart { 0x47A8 } else { 0x099F })
    }

    /// The CGB boot ROM hands the APU off one frame-sequencer step earlier than
    /// the DMG boot ROM (measured at PC=$0100). DMG-compat carts run a different
    /// boot sequence whose phase is unmeasured, so they keep the DMG handoff.
    fn audio_post_boot(internal_counter: u16, cgb_cart: bool) -> Audio<CgbApu> {
        if cgb_cart {
            let mut audio = Audio::post_boot_with_fs_step(internal_counter, 1);
            // The CGB boot chime leaves CH1 at this duty/divider phase, distinct
            // from the DMG handoff the `Default` channel state encodes.
            audio.set_ch1_post_boot_phase(6, 0x7DA);
            audio
        } else {
            Audio::post_boot(internal_counter)
        }
    }

    /// CGB boot-ROM handoff is mid-VBlank; the line depends on the boot
    /// duration (CGB cart: line 144, dot ~164; DMG cart: line 148, dot ~356).
    /// The boot ROM also zeroes OBP0/OBP1 (DMG leaves them at $FF).
    fn ppu_post_boot(cgb_cart: bool) -> Ppu<CgbPpu> {
        let mut ppu = if cgb_cart {
            Ppu::post_boot_vblank_handoff(144, 41)
        } else {
            Ppu::post_boot_vblank_handoff(148, 88)
        };
        ppu.set_post_boot_object_palettes(0x00);
        ppu
    }

    /// The CGB boot ROM hands off with both key-matrix lines deselected
    /// (P1 reads $FF).
    fn joypad_post_boot() -> Joypad {
        Joypad {
            read_buttons: false,
            read_dpad: false,
            pressed: Buttons::empty(),
        }
    }

    /// The CGB boot ROM leaves FF46 reading $00.
    fn dma_post_boot() -> Dma {
        Dma::with_source_register(0x00)
    }

    fn resolve_stop(&mut self, chassis: &mut Chassis<Self>) -> bool {
        // The settle is bus-coupled: a bus master holding the CPU defers it.
        if self.console_state().dma_cpu_hold() {
            return false;
        }
        // Mid-blackout: the held-edge stepping owns the countdown and re-engage.
        if self.speed_switch_in_progress() {
            return false;
        }
        if !self.key1_armed {
            return false;
        }
        let entry_dot_phase = chassis.ppu.dot_in_mcycle_phase();

        // The clock-mux settle is bus-coupled, and only the upward swap
        // disturbs the mux and resets the trigger's request/commit
        // chain (the CPU-written arming/length registers persist).
        if self.double_speed {
            // Downward: the chain survives, so a granted burst keeps
            // the bus and the settle waits for its release.
            if self.vram_dma.block.remaining > 0 && !self.vram_dma.block.ready_in.active() {
                return false;
            }
        } else {
            // Upward: the reset grades the committed block, which is
            // cancel-immune and ignores the arming flag. Not yet
            // bus-eligible: discarded whole. Bus-eligible: the dropped
            // grant latches the stop condition — the in-flight byte
            // completes outside the latched length.
            if self.vram_dma.block.remaining > 0 {
                // Either grading leg revokes the committed block's FF55
                // count (the dropped grant re-arms the status).
                self.vram_dma.arb.granted_ahead = self.vram_dma.arb.granted_ahead.saturating_sub(1);
                self.vram_dma.arb.grant_counted = false;
                if !self.vram_dma.block.ready_in.active() {
                    self.vram_dma.cursor.mode = TransferMode::Idle;
                    self.vram_dma.block.remaining = 1;
                    self.vram_dma.cursor.escape_byte = true;
                } else {
                    self.vram_dma.block.remaining = 0;
                    self.vram_dma.block.ready_in.clear();
                    self.vram_dma.block.setup_cells.clear();
                }
            }
            self.vram_dma.arb.pend = false;
        }
        self.double_speed = !self.double_speed;
        self.key1_armed = false;
        self.speed_switch_blackout = self.speed_switch_blackout_master_edges();
        if self.double_speed {
            self.vram_dma
                .arb
                .wake_pend_blind
                .arm(VramDma::WAKE_PEND_BLIND_TICKS);
        }
        // A 1×→2× relock entered at dot-in-M phase p3 lands the mux
        // displaced (cost-free); a displaced 2×→1× completes a dot early
        // and stays displaced; the following 1×→2× spends the dot back
        // re-syncing — and a re-sync suppresses a fresh displacement.
        if self.double_speed {
            if self.switch_relock_debit {
                self.speed_switch_blackout += 2;
                self.switch_relock_debit = false;
            } else if entry_dot_phase == Some(3) {
                self.switch_relock_debit = true;
            }
        } else if self.switch_relock_debit {
            self.speed_switch_blackout -= 2;
        }

        // Hardware resets DIV across the switch (the speed bit is now toggled).
        // The CPU clock is held while the dot clock runs the blackout out; the
        // held-edge stepping advances the master clock every edge and re-engages
        // at the phase the count expires on.
        let old_counter = chassis.timers.internal_counter();
        let to_double = self.double_speed;
        chassis.timers.reset_for_speed_switch();
        chassis
            .audio
            .on_div_write(old_counter.wrapping_sub(1), !to_double);
        chassis.audio.on_speed_switch(to_double);
        if let Some(interrupt) = chassis
            .serial
            .on_div_write(old_counter, self.has_serial_fast_clock())
        {
            chassis.interrupts.request(interrupt);
        }
        // KEY1 has flipped the speed bit; align the clock's ÷1/÷2 cell to the
        // new ratio so the clock stays the sole ratio owner.
        chassis.clock.set_divider(if to_double {
            CpuDivider::Two
        } else {
            CpuDivider::One
        });
        // An interrupt pending with IME set at the STOP preempts the post-STOP
        // HALT: the switch happens but the CPU services the interrupt at once
        // (DIV ≈ 0), not after the long wait.
        if chassis.cpu.interrupts_enabled() && chassis.interrupts.triggered().is_some() {
            self.preempt_speed_switch_halt();
        }
        // Anchor the held-edge count at the current master edge; the blackout's
        // elapsed count is `master_edge - blackout_anchor`.
        let anchor = chassis.clock.master_edge();
        self.console_state_mut().set_blackout_anchor(anchor);
        true
    }

    fn speed_switch_wake_ready(&mut self, mcycle_boundary: bool) -> bool {
        let latency = match self.speed_switch_wake_latency {
            None => 1, // First IF-set edge: arm the WakeIntake M-cycle.
            Some(n) if mcycle_boundary && n > 0 => n - 1,
            Some(n) => n,
        };
        self.speed_switch_wake_latency = Some(latency);
        latency == 0
    }

    fn speed_switch_in_progress(&self) -> bool {
        self.speed_switch_blackout > 0
    }

    fn drain_speed_switch_blackout(&mut self, elapsed: u32) -> bool {
        self.speed_switch_blackout = self.speed_switch_blackout.saturating_sub(elapsed);
        if self.speed_switch_blackout == 0 {
            self.speed_switch_wake_latency = None;
        }
        self.speed_switch_blackout == 0
    }

    fn speed_switch_divider_active(&self) -> bool {
        // The divider runs through the hold but freezes during the relock tail:
        // the CPU clock is still settling there, so it gains no edges (this keeps
        // the re-phase from disturbing DIV). The tail is the final `relock`
        // master edges of the count. Placing the quiet edges at the tail vs the
        // head is observationally identical (no test in the corpus latches a
        // divider-driven event in that window), so this picks the resume-side
        // offset that SameBoy/gambatte also model.
        self.speed_switch_blackout > self.relock_edges()
    }

    fn cpu_steps_per_dot(&self) -> u8 {
        if self.double_speed { 2 } else { 1 }
    }

    fn note_pre_alet_rendering(&mut self, rendering: bool) {
        if self.double_speed {
            self.pre_alet_rendering = rendering;
        }
    }

    fn note_pre_alet_lock(&mut self, lock: Option<bool>) {
        if self.double_speed {
            self.pre_alet_lock = lock;
        }
    }

    fn note_read_drive_phase(&mut self, oam_lock: Option<bool>) {
        self.read_drive_oam_lock = oam_lock;
    }

    fn note_ff44_ripple_old(&mut self, ly: Option<u8>) {
        self.ff44_ripple_old = ly;
    }

    fn take_ff44_ripple_old(&mut self) -> Option<u8> {
        self.ff44_ripple_old.take()
    }

    fn resolve_read_latch(&self, address: u16, value: u8, latch_lock: Option<bool>) -> u8 {
        match address {
            // Double-speed STAT mode bits: the read's data_phase_n↑ latches
            // before this dot's ALET edge, where VOGA clears XYMU (mode 3→0).
            // So a read taken while the PPU was rendering just before that edge
            // reads mode 3 even though the post-edge live mode has already
            // fallen to 0. This is the CGB CPU↔ALET half-dot phase — distinct
            // from the DMG, whose lockstep timing lands the latch after the edge.
            0xFF41 if self.double_speed => {
                if self.pre_alet_rendering {
                    value | 0b11
                } else {
                    value
                }
            }
            // Single speed: OR-of-accessibility over the drive-enable grant
            // sample and the latch-edge lock — the bus keeps the byte OAM
            // drove while addressed and unlocked. (The earlier address-phase
            // grant is double-speed-only; a single-speed onset between the
            // address phase and tobe↑ still floats the read.)
            0xFE00..=0xFEFF if !self.double_speed => match (self.read_drive_oam_lock, latch_lock) {
                (Some(false), _) => value,
                (_, Some(true)) => 0xFF,
                _ => value,
            },
            // Double-speed VRAM/OAM lock: data_phase_n↑ latches before this dot's
            // ALET edge — the same CGB CPU↔ALET half-dot phase as the STAT mode bits.
            // The read floats if it was locked at the pre-ALET view OR at the latch
            // edge, so a mode-3→0 release landing between them still floats. OR like
            // the single-speed OAM grant/latch arm — never removes a lock the latch sees.
            0x8000..=0x9FFF | 0xFE00..=0xFEFF if self.double_speed => {
                if self.pre_alet_lock == Some(true) || latch_lock == Some(true) {
                    0xFF
                } else {
                    value
                }
            }
            // An HDMA idle claim (a wake-tenure-consumed entry whose block is
            // owed but unserviced) holds the VRAM select without driving
            // data: an unlocked CPU VRAM read captures the undriven bus.
            0x8000..=0x9FFF if latch_lock != Some(true) && self.vram_dma.arb.idle_claim => 0x00,
            // A seized block tenure owns the VRAM select against the PPU: a
            // CPU VRAM read during a wake drain (the only tenure outside
            // mode 0) sees the actual byte, not the mode-3 float.
            0x8000..=0x9FFF
                if latch_lock == Some(true)
                    && self.vram_dma.block.remaining > 0
                    && self.vram_dma.cursor.remaining > 0 =>
            {
                value
            }
            _ if latch_lock == Some(true) => 0xFF,
            _ => value,
        }
    }

    fn on_reset(&mut self, cartridge: &Cartridge, has_boot_rom: bool) {
        *self = Self::default();
        // A DMG cartridge boots the CGB into compatibility mode (KEY0 bit 2).
        // With a real boot ROM that decision is the boot ROM's (via KEY0);
        // only HLE it on the skip-boot path.
        if !has_boot_rom {
            self.dmg_compat = !cartridge.is_cgb();
        }
    }

    fn restore_work_ram(
        &mut self,
        _external: &mut missingno_gb::memory::ExternalBus,
        bytes: &[u8],
    ) {
        // CGB work RAM lives in the model's eight banks, not the shared bus.
        let len = bytes.len().min(self.wram.len());
        self.wram[..len].copy_from_slice(&bytes[..len]);
    }

    fn validate_boundary(
        &self,
        record: &missingno_core::state::StateRecord,
    ) -> Result<(), missingno_core::system::StateError> {
        // A double-speed save carries no boundary-observable dot-phase alignment
        // (the free-running dot clock's parity a speed switch left is Tier-2b
        // state); reconstructing it would be a guess, so refuse the restore.
        if let Some(missingno_core::state::StateValue::Bool(true)) = record.get("double_speed") {
            return Err(missingno_core::system::StateError::DoubleSpeedBoundary);
        }
        Ok(())
    }

    fn restore_boundary_delta(
        &mut self,
        chassis: &mut Chassis<Self>,
        record: &missingno_core::state::StateRecord,
        memory: &[(String, Vec<u8>)],
    ) -> Result<(), missingno_core::system::StateError> {
        use missingno_core::state::StateValue;
        use missingno_core::system::StateError;

        let int = |name: &str| -> Result<u32, StateError> {
            match record.get(name) {
                Some(StateValue::Int(v)) => Ok(*v),
                _ => Err(StateError::Corrupt),
            }
        };
        let flag = |name: &str| -> Result<bool, StateError> {
            match record.get(name) {
                Some(StateValue::Bool(b)) => Ok(*b),
                _ => Err(StateError::Corrupt),
            }
        };
        let region = |name: &str| -> [u8; 64] {
            let mut out = [0u8; 64];
            if let Some((_, data)) = memory.iter().find(|(n, _)| n == name) {
                let len = data.len().min(64);
                out[..len].copy_from_slice(&data[..len]);
            }
            out
        };

        // Parse every field the delta needs BEFORE mutating any state — a bad or
        // missing field then leaves the console untouched rather than
        // half-restored.
        let svbk = (int("svbk")? as u8) & 0x07;
        let vbk = int("vbk")? as u8;
        let bcps = int("bcps")? as u8;
        let ocps = int("ocps")? as u8;
        let opri = int("opri")? as u8;
        let key1_armed = flag("key1_armed")?;
        let ff72 = int("ff72")? as u8;
        let ff73 = int("ff73")? as u8;
        let ff74 = int("ff74")? as u8;
        let ff75 = int("ff75")? as u8;
        let hdma_active = flag("hdma_active")?;
        let hdma_hblank = flag("hdma_hblank")?;
        let (hdma_source, hdma_dest, hdma_remaining) = if hdma_active && hdma_hblank {
            (
                int("hdma_source")? as u16,
                int("hdma_dest")? as u16,
                (int("hdma_remaining")? as u16) * 16,
            )
        } else {
            (0, 0, 0)
        };
        let bg = region("cram_bg");
        let obj = region("cram_obj");
        let extra_oam = {
            let mut out = [0u8; 24];
            if let Some((_, data)) = memory.iter().find(|(n, _)| n == "extra_oam") {
                let len = data.len().min(24);
                out[..len].copy_from_slice(&data[..len]);
            }
            out
        };

        // Everything parsed: now mutate. Speed is single-speed only (double speed
        // refused in validate_boundary); reset the KEY1/blackout transients to
        // their idle values, but honour the captured arm bit.
        self.double_speed = false;
        self.key1_armed = key1_armed;
        self.speed_switch_blackout = 0;
        self.speed_switch_wake_latency = None;
        self.switch_relock_debit = false;
        self.pre_alet_rendering = false;
        self.pre_alet_lock = None;
        self.read_drive_oam_lock = None;
        self.ff44_ripple_old = None;
        self.console_state = CgbConsoleState::default();
        chassis.clock.set_divider(CpuDivider::One);

        // Undocumented scratch registers and the extra OAM RAM.
        self.ff72 = ff72;
        self.ff73 = ff73;
        self.ff74 = ff74;
        self.ff75 = ff75;
        self.extra_oam = extra_oam;

        // Work-RAM bank select (SVBK) and the VRAM bank select (VBK).
        self.svbk = svbk;
        chassis.vram_bus.vram.write_bank_select(vbk);

        // Palette RAM and its index/priority registers.
        let dmg_compat = self.dmg_compat;
        chassis
            .ppu
            .model_mut()
            .restore_boundary(bg, obj, bcps, ocps, opri, dmg_compat);

        // VRAM-DMA engine: rebuild the cursor for an armed HBlank transfer (a
        // GDMA holds the CPU for its whole run, so it cannot straddle a
        // boundary). The arbitration transients reset to idle — at a boundary no
        // block is in flight.
        self.vram_dma = VramDma::default();
        if hdma_active && hdma_hblank {
            self.vram_dma.cursor.mode = TransferMode::HBlank;
            self.vram_dma.cursor.source = hdma_source;
            self.vram_dma.cursor.dest = hdma_dest;
            self.vram_dma.cursor.remaining = hdma_remaining;
        }

        Ok(())
    }

    fn map_read(&self, address: u16, ppu: &Ppu<CgbPpu>, vram: &CgbVram) -> Option<u8> {
        if let Some(i) = self.wram_index(address) {
            return Some(self.wram[i]);
        }
        match address {
            0xFEA0..=0xFEFF => Some(self.extra_oam[Self::extra_oam_index(address)]),
            // DMG-compat locks out the speed/banking/priority registers and
            // the $FF74 scratch byte — open bus for the rest of the session.
            0xFF4C | 0xFF4D | 0xFF6C | 0xFF70 | 0xFF74 if self.dmg_compat => Some(0xFF),
            // KEY0: boot-locked; reads the latched mode ($00 = CGB).
            0xFF4C => Some(0x00),
            0xFF4D => Some(0x7E | ((self.double_speed as u8) << 7) | self.key1_armed as u8), // KEY1
            0xFF4F => Some(vram.read_bank_select()),                                         // VBK
            // HDMA1-4 are write-only.
            0xFF51..=0xFF54 => Some(0xFF),
            // HDMA5 status: bit 7 = 0 while an HDMA is active, blocks-left-minus-1
            // in bits 6-0. Idle/done/stopped reads bit 7 = 1 (done = $FF). A GDMA
            // is never observable here — it holds the CPU for its whole duration.
            0xFF55 => {
                let visible = (self.vram_dma.cursor.remaining / 16)
                    .saturating_sub(self.vram_dma.arb.granted_ahead as u16);
                let active = self.vram_dma.cursor.mode == TransferMode::HBlank && visible > 0;
                Some(((!active as u8) << 7) | (visible.wrapping_sub(1) & 0x7F) as u8)
            }
            0xFF68 => Some(
                ppu.model()
                    .read_color_register(ColorRegister::BackgroundIndex),
            ), // BCPS
            0xFF69 => Some(
                ppu.model()
                    .read_color_register(ColorRegister::BackgroundData),
            ), // BCPD
            0xFF6A => Some(ppu.model().read_color_register(ColorRegister::ObjectIndex)), // OCPS
            0xFF6B => Some(ppu.model().read_color_register(ColorRegister::ObjectData)),  // OCPD
            0xFF6C => Some(ppu.read_object_priority()),                                  // OPRI
            0xFF70 => Some(self.svbk | 0xF8), // SVBK: bits 0-2
            0xFF72 => Some(self.ff72),
            0xFF73 => Some(self.ff73),
            0xFF74 => Some(self.ff74),
            0xFF75 => Some(0x8F | self.ff75),
            _ => None,
        }
    }

    fn map_write(
        &mut self,
        address: u16,
        value: u8,
        ppu: &mut Ppu<CgbPpu>,
        vram: &mut CgbVram,
    ) -> bool {
        if let Some(i) = self.wram_index(address) {
            self.wram[i] = value;
            return true;
        }
        match address {
            0xFEA0..=0xFEFF => {
                self.extra_oam[Self::extra_oam_index(address)] = value;
                true
            }
            // DMG-compat locks out the speed/banking/priority/VRAM-DMA
            // registers and the $FF74 scratch byte.
            0xFF4D | 0xFF51..=0xFF55 | 0xFF6C | 0xFF70 | 0xFF74 if self.dmg_compat => true,
            0xFF4C => true, // KEY0: boot-locked, ignore
            0xFF4D => {
                self.key1_armed = value & 0x01 != 0;
                true
            }
            0xFF4F => {
                vram.write_bank_select(value); // VBK
                true
            }
            0xFF51 => {
                self.vram_dma.cursor.source =
                    (self.vram_dma.cursor.source & 0x00FF) | ((value as u16) << 8);
                true
            }
            0xFF52 => {
                self.vram_dma.cursor.source =
                    (self.vram_dma.cursor.source & 0xFF00) | (value & 0xF0) as u16;
                true
            }
            0xFF53 => {
                self.vram_dma.cursor.dest =
                    ((value as u16) << 8) | (self.vram_dma.cursor.dest & 0x00FF);
                true
            }
            0xFF54 => {
                self.vram_dma.cursor.dest =
                    (self.vram_dma.cursor.dest & 0xFF00) | (value & 0xF0) as u16;
                true
            }
            0xFF55 => {
                let length = ((value & 0x7F) as u16 + 1) * 16;
                self.vram_dma.arb.granted_ahead = 0;
                self.vram_dma.arb.grant_counted = false;
                self.vram_dma.arb.pend_granted = false;
                if value & 0x80 != 0 {
                    // Arm HDMA: one 16-byte block per HBlank. A block already
                    // latched by the trigger is immune and keeps flowing; an
                    // arm landing during mode 0 pends at this fall's trigger
                    // evaluation. With the LCD off no HBlank will come — the
                    // arm strobe services one block immediately.
                    self.vram_dma.cursor.mode = TransferMode::HBlank;
                    self.vram_dma.cursor.remaining = length;
                    self.vram_dma.arb.armed_this_fall = true;
                    if !ppu.control().video_enabled() {
                        self.vram_dma.block.remaining = 16;
                        self.vram_dma.arb.pend_from_arm = true;
                        self.vram_dma.block.setup_cells.clear();
                        self.vram_dma.block.ready_in.arm(2);
                    }
                } else if self.vram_dma.cursor.mode == TransferMode::HBlank {
                    // bit 7 = 0 while an HDMA runs clears the arming only (no
                    // GDMA starts); a latched block completes. Bits 6-0 are
                    // the length register and store on every write — the
                    // status read reflects them.
                    self.vram_dma.cursor.mode = TransferMode::Idle;
                    self.vram_dma.cursor.remaining = length;
                } else {
                    // GDMA: copy the whole length while holding the CPU.
                    self.vram_dma.cursor.mode = TransferMode::General;
                    self.vram_dma.cursor.remaining = length;
                }
                true
            }
            0xFF68 => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::BackgroundIndex, value); // BCPS
                true
            }
            0xFF69 => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::BackgroundData, value); // BCPD
                true
            }
            0xFF6A => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::ObjectIndex, value); // OCPS
                true
            }
            0xFF6B => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::ObjectData, value); // OCPD
                true
            }
            0xFF6C => {
                ppu.write_object_priority(value); // OPRI
                true
            }
            0xFF70 => {
                self.svbk = value & 0x07;
                true
            }
            0xFF72 => {
                self.ff72 = value;
                true
            }
            0xFF73 => {
                self.ff73 = value;
                true
            }
            0xFF74 => {
                self.ff74 = value;
                true
            }
            0xFF75 => {
                self.ff75 = value & 0x70;
                true
            }
            _ => false,
        }
    }

    fn vram_dma_edge(&mut self, chassis: &mut Chassis<Self>, mode: Mode) {
        let cpu_halted = chassis.cpu.is_halted();
        // The engine thaws at the IF rise, ahead of the CPU's halt-exit latency
        // (a wake-coincident block is decided before the first fetch and the
        // dispatch pick); the taken-clear waits for the CPU's own resume.
        let engine_gated = (cpu_halted && !chassis.cpu.irq_latched()) || chassis.cpu.is_stopped();
        let master_edge = chassis.clock.master_edge();
        let in_hblank = mode == Mode::HorizontalBlank;
        let entry_edge = in_hblank && !self.vram_dma.arb.prev_view_hblank;
        self.vram_dma.arb.prev_view_hblank = in_hblank;
        if cpu_halted {
            self.vram_dma.arb.halted_falls = self.vram_dma.arb.halted_falls.saturating_add(1);
        } else {
            self.vram_dma.arb.halted_falls = 0;
        }
        // The taken-clear stays live through the halt-latch M-cycle, then
        // freezes until the CPU's own resume (halt only; STOP freezes it
        // outright via the engine gate). One M-cycle is 4 PPU dots single
        // speed, 2 in double speed.
        let taken_clear_window = if self.double_speed { 2 } else { 4 };
        if !in_hblank && (!cpu_halted || self.vram_dma.arb.halted_falls <= taken_clear_window) {
            self.vram_dma.block.taken = false;
        }
        // A halt wake blinds entry edges on the wake fall and just after it
        // (the halt analogue of the STOP re-engage window, without the
        // first-tick exemption — the wake fall's own entry is blinded too).
        // The halt line is sampled every tick, gated or not — a gated halt
        // (no latched IRQ) still arms the blind at its wake; the countdown
        // runs on engine ticks only.
        if self.vram_dma.arb.prev_cpu_halted && !cpu_halted {
            self.vram_dma
                .arb
                .halt_wake_blind
                .arm(VramDma::HALT_WAKE_BLIND_TICKS);
        }
        // An arm-strobe launch must ready with a fall of mode-0 margin: the
        // readiness latch's confirmation sample on the following fall reads
        // the mode-0 level, and a launch confirmed outside mode 0 reverts
        // (FF55 count restored) to wait for the next entry.
        if self.vram_dma.block.arm_ready_probation {
            self.vram_dma.block.arm_ready_probation = false;
            if !in_hblank && self.vram_dma.block.from_arm && self.vram_dma.block.remaining == 16 {
                self.vram_dma.block.remaining = 0;
                self.vram_dma.block.from_arm = false;
                if self.vram_dma.arb.granted_ahead > 0 {
                    self.vram_dma.arb.granted_ahead -= 1;
                }
            }
        }
        // A block whose readiness the HALT latch lands inside joins the halt:
        // its bytes defer to the wake as a standing granted claim (FF55
        // already counted it), where the halt-release handover applies. A
        // latch after readiness leaves the block to drain in-halt.
        if cpu_halted
            && !self.vram_dma.arb.prev_cpu_halted
            && self.vram_dma.block.remaining == 16
            && self.vram_dma.block.ready_in.active()
        {
            self.vram_dma.block.remaining = 0;
            self.vram_dma.block.ready_in.clear();
            self.vram_dma.block.setup_cells.clear();
            self.vram_dma.arb.pend = true;
            self.vram_dma.arb.pend_granted = true;
            self.vram_dma.arb.grant_counted = true;
            self.vram_dma.arb.pend_age = 0;
        }
        // The halted view entering this fall — a wake flipping on the commit
        // fall itself still counts as a halted-CPU commit for the grant mode.
        let halted_entering = cpu_halted || self.vram_dma.arb.prev_cpu_halted;
        self.vram_dma.arb.prev_cpu_halted = cpu_halted;
        // The engine gate freezes commit/grant; the mode-0 entry detector
        // keeps running — an entry registers a pend-request (consulting the
        // taken flag) that persists through the freeze and commits at the
        // thaw. A latched block keeps draining.
        if engine_gated {
            if self.vram_dma.arb.pend {
                self.vram_dma.arb.pend_age = self.vram_dma.arb.pend_age.saturating_add(1);
            }
            if cpu_halted
                && entry_edge
                && self.vram_dma.cursor.mode == TransferMode::HBlank
                && !self.vram_dma.block.taken
                && self.vram_dma.cursor.remaining > 0
            {
                self.vram_dma.arb.pend = true;
                self.vram_dma.arb.pend_from_arm = false;
                self.vram_dma.arb.pend_age = 0;
            }
            // An in-halt mode-0 within the halt-latch window grants the block's
            // FF55 status (the halted CPU does not contend); the seizure transfer
            // still waits for the resume.
            if cpu_halted
                && entry_edge
                && self.vram_dma.cursor.mode == TransferMode::HBlank
                && self.vram_dma.arb.halted_falls <= 2
                && self.vram_dma.cursor.remaining / 16 > self.vram_dma.arb.granted_ahead as u16
            {
                self.vram_dma.arb.granted_ahead += 1;
                self.vram_dma.arb.grant_counted = true;
                self.vram_dma.arb.pend_granted = true;
            }
            self.vram_dma.cursor.quota = if self.vram_dma.moving() {
                if self.double_speed { 1 } else { 2 }
            } else {
                0
            };
            return;
        }

        if self.vram_dma.arb.halt_wake_blind.tick() && entry_edge {
            self.vram_dma.block.taken = true;
        }

        // The post-switch re-engage window: a fresh mode-0 entry edge inside
        // it passes unserviced (no pend, no retry). The first running tick
        // after the blackout carries the frozen pre-switch view — a mode-0
        // level there is blackout-carried, not an entry, and stays eligible.
        let wake_pend_first_tick =
            self.vram_dma.arb.wake_pend_blind.remaining() == VramDma::WAKE_PEND_BLIND_TICKS;
        if self.vram_dma.arb.wake_pend_blind.tick() && entry_edge && !wake_pend_first_tick {
            self.vram_dma.block.taken = true;
        }

        // Two-stage trigger, evaluated each fall on the post-rise mode view
        // with this fall's FF55 commit visible: a pend commits to a
        // cancel-immune block one fall later; an FF55 write at either fall
        // kills the pend (armed is consulted at both stages).
        let armed = self.vram_dma.cursor.mode == TransferMode::HBlank;
        let committing = self.vram_dma.arb.pend
            && armed
            // An arm-strobe pend latched while in HBlank commits even if HBlank
            // ended in the one-fall pend->commit gap; an in-halt-granted pend
            // commits at the thaw whatever the live mode.
            && (in_hblank || self.vram_dma.arb.pend_from_arm || self.vram_dma.arb.pend_granted)
            // A granted pend commits at the engine thaw even while the CPU is
            // still halted (an interrupt-dispatch wake) — the grant is the halt's.
            && (!cpu_halted || self.vram_dma.arb.pend_age <= 2 || self.vram_dma.arb.pend_granted);
        if committing {
            self.vram_dma.block.remaining = 16;
            self.vram_dma.block.start_edge = master_edge;
            self.vram_dma.block.taken = true;
            self.vram_dma.arb.idle_claim = false;
            self.vram_dma.block.from_arm = self.vram_dma.arb.pend_from_arm;
            // A granted-DRIVEN commit (only reachable through the grant: the
            // thaw lies outside HBlank) pre-charged its setup during the halt;
            // a granted pend that commits inside a live HBlank is an ordinary
            // commit and charges setup normally.
            let granted = self.vram_dma.arb.pend_granted && !in_hblank;
            // A dispatch wake (the CPU still halted at the thaw) spends the
            // halt-exit window dispatching, so its setup runs after instead of
            // arriving pre-charged.
            let precharged = granted && !cpu_halted;
            self.vram_dma.arb.park_waits_for_fetch =
                !(self.vram_dma.arb.pend_from_arm || halted_entering);
            self.vram_dma
                .block
                .ready_in
                .arm(if precharged { 0 } else { 2 });
            self.vram_dma
                .block
                .setup_cells
                .arm(if self.vram_dma.arb.pend_from_arm || precharged {
                    0
                } else {
                    1
                });
            // FF55 counts the block out at commit, not at drain end.
            if !self.vram_dma.arb.grant_counted {
                self.vram_dma.arb.granted_ahead += 1;
            }
            self.vram_dma.arb.grant_counted = false;
            if granted {
                self.vram_dma
                    .arb
                    .wake_tenure
                    .arm(VramDma::WAKE_TENURE_TICKS);
            }
            self.vram_dma.arb.pend_granted = false;
        }
        // A granted pend whose thaw tick passes without committing reverts to
        // an ordinary pend (its next-HBlank commit charges setup normally).
        if !committing && !cpu_halted {
            self.vram_dma.arb.pend_granted = false;
        }
        // The wake drain's bus tenure consumes an HBlank entry landing inside
        // it — the entry neither pends nor retries, but its claim stands: the
        // engine holds the VRAM select, undriven, until the owed block is
        // really serviced.
        if self.vram_dma.arb.wake_tenure.tick() && entry_edge {
            self.vram_dma.block.taken = true;
            if self.vram_dma.cursor.remaining > 0 {
                self.vram_dma.arb.idle_claim = true;
            }
        }
        self.vram_dma.arb.pend = !committing
            && armed
            && in_hblank
            && !self.vram_dma.block.taken
            && self.vram_dma.cursor.remaining > 0
            && self.vram_dma.block.remaining == 0;
        if self.vram_dma.arb.pend {
            self.vram_dma.arb.pend_from_arm = self.vram_dma.arb.armed_this_fall;
            self.vram_dma.arb.pend_age = 0;
        }
        self.vram_dma.arb.armed_this_fall = false;
        // An arm-strobe launch must ready inside mode 0: readiness completing
        // after the mode-0 exit reverts the block (FF55 count restored) to wait
        // for the next entry. Single speed only: the double-speed
        // readiness-vs-exit margin is unmeasured, and the DS late_enable rows
        // expect the arm serviced.
        if self.vram_dma.block.ready_in.expired()
            && self.vram_dma.block.from_arm
            && self.vram_dma.block.remaining == 16
            && !self.double_speed
        {
            self.vram_dma.block.arm_ready_probation = true;
        }

        // Refill this M-cycle's byte budget while the transfer is moving bytes:
        // 2/M-cycle single speed, 1 in double speed.
        self.vram_dma.cursor.quota = if self.vram_dma.moving() {
            if self.double_speed { 1 } else { 2 }
        } else {
            0
        };
        if self.vram_dma_seizes_bus() {
            self.vram_dma.block.seize_falls = self.vram_dma.block.seize_falls.saturating_add(1);
        } else {
            self.vram_dma.block.seize_falls = 0;
        }
        let claim = VramDmaClaim {
            committed: committing,
            // A claim is standing once it has aged through one full M-cycle
            // of the freeze — the synchronizer stage that carries it into
            // the CPU's M-cycle clock domain; a younger claim hasn't crossed when
            // the halt-release fetch starts.
            standing: committing && self.vram_dma.arb.pend_age >= 4,
        };
        if claim.committed {
            // An active OAM DMA already owns a bus, blocking the handover that
            // would take the halt-release fetch's tail.
            let bus_free = chassis.dma.is_active_on_bus().is_none();
            self.console_state.set_vram_dma_claim(VramDmaClaim {
                committed: true,
                standing: claim.standing && bus_free,
            });
        }
    }

    fn vram_dma_drain_escape(&mut self) -> Option<(u16, u16)> {
        if self.vram_dma.cursor.escape_byte && self.vram_dma.moving() {
            self.vram_dma.cursor.quota = 1;
            self.vram_dma_next_byte()
        } else {
            None
        }
    }

    fn vram_dma_park_waits_for_fetch(&self) -> bool {
        self.vram_dma.arb.park_waits_for_fetch
    }

    fn vram_dma_instruction_retired(&mut self) {
        self.vram_dma.arb.park_waits_for_fetch = false;
    }

    fn vram_dma_request_standing(&self) -> bool {
        self.vram_dma.arb.pend
            || (self.vram_dma.block.remaining > 0 && self.vram_dma.cursor.remaining > 0)
    }

    fn vram_dma_holds_cpu(&self) -> bool {
        self.vram_dma.cursor.mode == TransferMode::General && self.vram_dma.cursor.remaining > 0
    }

    fn vram_dma_lcd_disabled(&mut self) {
        // VID_RST re-anchors the dot unit: the mux displacement is void.
        self.switch_relock_debit = false;
        self.vram_dma.arb.idle_claim = false;
        if self.vram_dma.cursor.mode == TransferMode::HBlank
            && self.vram_dma.cursor.remaining > 0
            && !self.vram_dma.block.taken
            && self.vram_dma.block.remaining == 0
        {
            self.vram_dma.block.remaining = 16;
            self.vram_dma.arb.pend_from_arm = true;
            self.vram_dma.block.setup_cells.clear();
            self.vram_dma.block.ready_in.arm(2);
        }
    }

    fn vram_dma_seizes_bus(&self) -> bool {
        !self.vram_dma.block.ready_in.active()
            && (self.vram_dma.block.setup_cells.active()
                || (self.vram_dma.block.remaining > 0 && self.vram_dma.cursor.remaining > 0))
    }

    fn vram_dma_conflict_source(&self, address: u16) -> Option<u16> {
        // Double speed only: at single speed the CPU read-latch lands after the
        // block on its own (the byte-identical single-speed reads pass already);
        // at 2× the read collides with the byte the block is writing — but only
        // once the bus seizure has settled one full prior fall (the half-dot from
        // seizure to byte-readable; the count includes this fall, so `>= 2`).
        let writing = self.double_speed
            && self.vram_dma.block.seize_falls >= 2
            && self.vram_dma.block.remaining > 0
            && self.vram_dma.cursor.remaining > 0;
        (writing && address == self.vram_dma.write_address()).then_some(self.vram_dma.cursor.source)
    }

    fn vram_dma_arbitrate_oam(&mut self, chassis: &mut Chassis<Self>) -> bool {
        let oam = chassis.dma.peek_transfer();
        let hdma_active = self.console_state.dma_cpu_hold() || self.console_state.bus_suspended();
        // The OAM-DMA's final byte still shares the bus on the M-cycle it
        // completes; edge-detect its active→done boundary so that M-cycle still
        // contends with a concurrent VRAM-DMA.
        let oam_transferring = oam.is_some();
        let oam_just_completed = self.vram_dma.oam.was_transferring && !oam_transferring;
        self.vram_dma.oam.was_transferring = oam_transferring;
        // The two engines share one bus: when an OAM-DMA and a VRAM-DMA block
        // both move a byte this M-cycle, the OAM-DMA latches the VRAM-DMA byte
        // that coincides with its write rather than its own source.
        let contended = !self.double_speed
            && (oam_transferring || oam_just_completed)
            && hdma_active
            && self.vram_dma_will_move();
        // Double speed: a switch-cancel escape byte's bus tenure stalls the
        // concurrent OAM-DMA byte one M-cycle (the engine resumes it next M).
        let escape_stall =
            self.double_speed && oam_transferring && hdma_active && self.vram_dma_escape_pending();
        if escape_stall {
            chassis.dma.stall_advance();
        }
        self.vram_dma.oam.contended = contended;
        contended || escape_stall
    }

    fn vram_dma_boundary(&mut self, chassis: &mut Chassis<Self>) {
        let hdma_active = self.console_state.dma_cpu_hold() || self.console_state.bus_suspended();
        if !hdma_active {
            return;
        }
        let contended = self.vram_dma.oam.contended;
        // Commit the bytes the VRAM DMA moves while it actually holds the bus —
        // the hold keeps the transfer from overlapping the arming instruction.
        // (The trigger/quota tick ran before this edge's write commit.)
        let mut hdma_bytes: [Option<(u16, u8)>; 2] = [None, None];
        if !self.vram_dma_take_setup_cell() {
            while let Some((src, dst)) = self.vram_dma_next_byte() {
                let byte = self.read_hdma_source(chassis, src);
                chassis.dma_commit(src, dst, byte);
                if contended {
                    if hdma_bytes[0].is_none() {
                        hdma_bytes[0] = Some((src, byte));
                    } else if hdma_bytes[1].is_none() {
                        hdma_bytes[1] = Some((src, byte));
                    }
                }
            }
        }

        // The OAM-DMA's deposit this M-cycle is the coinciding VRAM-DMA byte,
        // landing at OAM[source_low]. Which of the M-cycle's two VRAM-DMA bytes
        // coincides is the phase between the two byte clocks (the OAM-DMA's start
        // edge vs the block's), 2nd byte at residue {0,3}. The coinciding OAM
        // byte commits one T-cycle after start_edge marks dma_run engaging, so
        // align the phase to that commit before taking the residue.
        if contended {
            const OAM_BYTE_COMMIT_LAG_EDGES: u64 = 2;
            let phase = self
                .vram_dma
                .block
                .start_edge
                .wrapping_sub(chassis.dma.start_edge())
                .wrapping_sub(OAM_BYTE_COMMIT_LAG_EDGES)
                / 2
                % 4;
            let coinciding = if matches!(phase, 0 | 3) {
                hdma_bytes[1].or(hdma_bytes[0])
            } else {
                hdma_bytes[0]
            };
            if let Some((hsrc, hdata)) = coinciding {
                chassis.dma_commit(hsrc, 0xfe00 | (hsrc & 0xFF), hdata);
            }
        }
    }
}

impl Cgb {
    /// Whether the VRAM DMA will move at least one byte this M-cycle (block
    /// active, quota available, no setup cell pending) — for the OAM-DMA
    /// bus-contention check.
    fn vram_dma_will_move(&self) -> bool {
        !self.vram_dma.block.setup_cells.active()
            && self.vram_dma.cursor.quota > 0
            && self.vram_dma.moving()
    }

    /// The byte the VRAM DMA is about to move is a switch-cancel escape byte;
    /// its bus tenure stalls a concurrent OAM-DMA byte at double speed.
    fn vram_dma_escape_pending(&self) -> bool {
        self.vram_dma.cursor.escape_byte && self.vram_dma_will_move()
    }

    /// An entry-triggered block spends one leading no-data cell — the engine
    /// loading its working pointers from the HDMA1-4 holding registers (the FF55
    /// arm strobe performs that load itself). Consumed once per block.
    fn vram_dma_take_setup_cell(&mut self) -> bool {
        self.vram_dma.block.setup_cells.tick()
    }

    /// The next byte the VRAM DMA moves this M-cycle — `(source, destination)`
    /// resolved addresses — advancing its cursor. `None` once this M-cycle's
    /// quota is spent.
    fn vram_dma_next_byte(&mut self) -> Option<(u16, u16)> {
        if self.vram_dma.cursor.quota == 0 || !self.vram_dma.moving() {
            return None;
        }
        let pair = (self.vram_dma.cursor.source, self.vram_dma.write_address());
        // Pointers advance per byte and persist for any follow-on transfer. A
        // switch-cancel escape byte does not count against the latched length.
        self.vram_dma.cursor.source = self.vram_dma.cursor.source.wrapping_add(1);
        let (next_dest, carried) = self.vram_dma.cursor.dest.overflowing_add(1);
        self.vram_dma.cursor.dest = next_dest;
        if self.vram_dma.cursor.escape_byte {
            self.vram_dma.cursor.escape_byte = false;
        } else {
            self.vram_dma.cursor.remaining -= 1;
        }
        self.vram_dma.cursor.quota -= 1;
        if self.vram_dma.block.remaining > 0 {
            self.vram_dma.block.remaining -= 1;
            // A block granted ahead in-halt rejoins the FF55 count as its bytes
            // finally drain on the post-resume path.
            if self.vram_dma.block.remaining == 0 {
                if self.vram_dma.arb.granted_ahead > 0 {
                    self.vram_dma.arb.granted_ahead -= 1;
                }
                self.vram_dma.arb.park_waits_for_fetch = false;
                self.vram_dma.block.from_arm = false;
            }
        }
        if carried {
            // The 16-bit dest register carried out of $FFFF — the transfer ends
            // here rather than wrapping back into VRAM.
            self.vram_dma.cursor.remaining = 0;
        }
        if self.vram_dma.cursor.remaining == 0 {
            self.vram_dma.cursor.mode = TransferMode::Idle;
            self.vram_dma.arb.idle_claim = false;
        }
        Some(pair)
    }

    /// Open-bus value a VRAM-DMA source read returns, or None for a normal read.
    /// A VRAM-DMA source must be ROM/cart-RAM; VRAM ($8000-$9FFF) is off that
    /// source bus and floats to `$FF`.
    fn vram_dma_source_open_bus(&self, source: u16) -> Option<u8> {
        (0x8000..=0x9FFF).contains(&source).then_some(0xFF)
    }

    /// Read one VRAM-DMA source byte: the VRAM float, then the cart-bus float,
    /// the CGB register/banked-WRAM map, and finally chassis storage — the
    /// VRAM-DMA counterpart of `Console::read_dma_source`.
    fn read_hdma_source(&self, chassis: &Chassis<Self>, source: u16) -> u8 {
        if let Some(open) = self.vram_dma_source_open_bus(source) {
            return open;
        }
        if let Some(value) = self.dma_source_open_bus(source) {
            return value;
        }
        if let Some(value) = self.map_read(source, &chassis.ppu, &chassis.vram_bus.vram) {
            return value;
        }
        chassis.read_dma_storage(source)
    }
}

/// The Game Boy Color.
pub type GameBoyColor = Console<Cgb>;
