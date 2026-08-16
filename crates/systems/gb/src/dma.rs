use super::memory::Bus;

/// Which side drives the OAM SRAM address bus this T-cycle. During DMA
/// the CPU/Parse/Render OAM drivers all tri-state via `boge = !dma_run`,
/// so DMA owns the bus uncontested.
pub enum OamBusOwner {
    /// A PPU-side driver owns the bus; the OAM data latch captures the
    /// addressed byte-pair as normal.
    Ppu,
    /// DMA drives the bus at this OAM byte offset (0..=159). The OAM
    /// data latch is gated off (mode2 forced low via `boge`), so the
    /// (Y, X) latches hold their prior values throughout the overlap.
    Dma(u8),
}

/// NAVO byte-159 decode: bits 0,1,2,3,4,7 — fires at `dma_a == 0x9F`
/// (159, the 160th byte).
const BYTE_159_DECODE: u8 = 0b1001_1111;

/// OAM DMA controller, modelled as the DMG control-gate pipeline. The
/// arm/run/terminate gates are clocked by `dma_phi = !data_phase`
/// (run/counter side) and `dma_phi_n` (arm side) so the FF46-write →
/// `dma_run` engage latency (1.5 M-cycles) and the 160-byte transfer
/// emerge from the gate timing rather than a fixed delay.
pub struct Dma {
    /// Last value written to the DMA register (0xFF46) — source page.
    source_register: u8,
    /// Base source address (page * 0x100).
    source: u16,
    /// Which bus the DMA source resides on.
    source_bus: Bus,

    /// FF46 store latch (LYXE) — set by the LAVY store strobe, reset by
    /// LOKO. Feeds the start synchroniser through LUPA.
    store_latched: bool,
    /// Start synchroniser stage 1 (LUVY): DFF on `dma_phi`, `d = LUPA`
    /// (the store latch once the strobe ends).
    start_sync_1: bool,
    /// Start synchroniser stage 2 (LENE): DFF on `dma_phi_n`. Arms the
    /// run-request latch and the counter reset.
    start_sync_2: bool,
    /// Last-byte flip-flop (MYTE): DFF on `dma_phi_n`, `d = NOLO`, reset
    /// by LAPA. Drops the run-request latch at byte 159.
    last_byte: bool,
    /// Run-request latch (LOKY/LARA cross-coupled NANDs): set when
    /// `lene_n=0`, reset when `myte_n=0`.
    run_request: bool,
    /// MATU: `dma_run`, the run request re-sampled on `dma_phi↑`.
    dma_run: bool,
    /// NAKY..MUGU 8-bit ripple counter — OAM offset / source low byte.
    dma_a: u8,

    /// Master edge at which `dma_run` engaged — the byte clock's phase origin
    /// (1.5 M-cycles after FF46), used to align against a concurrent VRAM-DMA bus.
    start_edge: u64,

    /// Previous `data_phase` for `dma_phi`/`dma_phi_n` edge detection.
    prev_data_phase: bool,

    /// A switch-cancel escape byte's bus tenure gates one counter advance:
    /// the stalled slot re-drives on the next M-cycle instead of skipping.
    advance_stalled: bool,
}

impl Default for Dma {
    fn default() -> Self {
        Self::new()
    }
}

impl Dma {
    pub fn new() -> Self {
        Self {
            source_register: 0xFF,
            source: 0,
            source_bus: Bus::External,
            store_latched: false,
            start_sync_1: false,
            start_sync_2: false,
            last_byte: false,
            run_request: false,
            dma_run: false,
            dma_a: 0,
            start_edge: 0,
            prev_data_phase: false,
            advance_stalled: false,
        }
    }

    /// Idle DMA with a model-specific FF46 reset value.
    pub fn with_source_register(source_register: u8) -> Self {
        Self {
            source_register,
            ..Self::new()
        }
    }

    /// Last value written to the DMA register (0xFF46).
    pub fn source_register(&self) -> u8 {
        self.source_register
    }

    /// Reseat the DMA register (FF46) readback from a save state.
    pub fn set_source_register(&mut self, value: u8) {
        self.source_register = value;
    }

    /// Base source address of the transfer (`source_register << 8`).
    pub fn source(&self) -> u16 {
        self.source
    }

    /// Bus that DMA is actively driving (`dma_run` asserted). `None`
    /// when idle or still arming.
    pub fn is_active_on_bus(&self) -> Option<Bus> {
        self.dma_run.then_some(self.source_bus)
    }

    /// Which side drives the OAM SRAM address bus — DMA while `dma_run`,
    /// otherwise the PPU.
    pub fn oam_bus_owner(&self) -> OamBusOwner {
        if self.dma_run {
            OamBusOwner::Dma(self.dma_a.min(159))
        } else {
            OamBusOwner::Ppu
        }
    }

    /// `(source address, destination offset)` of the byte DMA is driving
    /// this M-cycle, without mutating state. `None` when not transferring.
    pub fn peek_transfer(&self) -> Option<(u16, u8)> {
        (self.dma_run && self.dma_a < 160).then_some((self.source + self.dma_a as u16, self.dma_a))
    }

    /// $FF46 write: the LAVY strobe sets the store latch and latches the
    /// source page. The arm then propagates through the start
    /// synchroniser to `dma_run` over the next 1.5 M-cycles.
    pub fn begin_transfer(&mut self, source: u8) {
        self.source_register = source;
        self.source = (source as u16) * 0x100;
        self.source_bus = Bus::of(self.source).unwrap_or(Bus::External);
        self.store_latched = true;
    }

    /// Master edge at which `dma_run` engaged — the byte clock's phase origin.
    pub fn start_edge(&self) -> u64 {
        self.start_edge
    }

    /// An escape byte's bus tenure stalls this slot: gate the counter's next
    /// advance so the slot re-drives instead of skipping.
    pub fn stall_advance(&mut self) {
        self.advance_stalled = true;
    }

    /// Advance the control gates one master-clock edge. `data_phase` is
    /// the CPU data-phase net; `dma_phi = !data_phase` clocks the
    /// run/counter DFFs (MATU/LUVY/counter) on its rising edge,
    /// `dma_phi_n` the arm DFFs (LENE/MYTE). The byte itself is committed
    /// separately at the M-cycle data phase via `peek_transfer`. During
    /// HALT `data_phase` is held low, so `dma_phi` never rises and the
    /// engine freezes.
    pub fn tick(&mut self, data_phase: bool, master_edge: u64) {
        let dma_phi_rising = self.prev_data_phase && !data_phase;
        let dma_phi_n_rising = !self.prev_data_phase && data_phase;
        self.prev_data_phase = data_phase;

        if dma_phi_n_rising {
            self.start_sync_2 = self.start_sync_1;
            // MYTE: d = NOLO, async-reset by LAPA (= counter reset).
            self.last_byte = !self.counter_held_reset() && self.last_byte_term();
            self.settle_latches();
        }

        if dma_phi_rising {
            // META = AND2(dma_phi, LOKY): reset dominates, else advance —
            // unless an escape byte's bus tenure gated this advance (the
            // stalled slot re-drives next M-cycle).
            if self.counter_held_reset() {
                self.dma_a = 0;
            } else if self.run_request {
                if self.advance_stalled {
                    self.advance_stalled = false;
                } else {
                    self.dma_a = self.dma_a.wrapping_add(1);
                }
            }
            self.start_sync_1 = self.store_latched;
            // `dma_run` engaging marks the byte clock's phase origin.
            if !self.dma_run && self.run_request {
                self.start_edge = master_edge;
            }
            self.dma_run = self.run_request;
            self.settle_latches();
        }
    }

    /// `LAPA = 0` (counter + MYTE held in reset) while `LENE = 1` — the
    /// arm window, where `LOKO = LENE` forces the reset.
    fn counter_held_reset(&self) -> bool {
        self.start_sync_2
    }

    /// Last-byte term (NOLO = NOT(NAVO)): `dma_a` has bits 0,1,2,3,4,7
    /// set (0x9F = 159).
    fn last_byte_term(&self) -> bool {
        self.dma_a & BYTE_159_DECODE == BYTE_159_DECODE
    }

    /// Settle the level S-R latches after a DFF edge: LYXE reset by
    /// `LOKO = LENE`; LOKY set by `lene_n=0`, reset by `myte_n=0`.
    fn settle_latches(&mut self) {
        if self.start_sync_2 {
            self.store_latched = false;
            self.run_request = true;
        } else if self.last_byte {
            self.run_request = false;
        }
    }

    pub fn dma_run(&self) -> bool {
        self.dma_run
    }

    pub fn byte_index(&self) -> u8 {
        self.dma_a
    }

    pub fn from_snapshot(snap: &crate::snapshot::DmaSnapshot) -> Dma {
        let mut dma = Dma::new();
        if snap.active {
            dma.source_register = (snap.source >> 8) as u8;
            dma.source = snap.source;
            dma.source_bus = Bus::of(snap.source).unwrap_or(Bus::External);
            dma.dma_run = true;
            dma.dma_a = snap.byte_index;
            dma.run_request = true;
        }
        dma
    }
}
