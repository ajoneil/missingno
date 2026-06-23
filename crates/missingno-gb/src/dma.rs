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

/// NAVO terminal-count decode: bits 0,1,2,3,4,7 — fires at `dma_a == 0x9F`
/// (159, the 160th byte).
const NAVO_DECODE: u8 = 0b1001_1111;

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

    /// LYXE: DMA-requested S-R latch — set by the `lavy` $FF46 write
    /// strobe, reset by `loko`. Drives `lupa → luvy`.
    lyxe: bool,
    /// LUVY: DFF on `dma_phi`, `d = lupa` (= `lyxe` at every `dma_phi↑`).
    luvy: bool,
    /// LENE: DFF on `dma_phi_n`, `d = luvy`. Arms the run latch and the
    /// counter reset.
    lene: bool,
    /// MYTE: DFF on `dma_phi_n`, `d = nolo` (terminal count), reset by
    /// `lapa`. Drops the run latch at byte 159.
    myte: bool,
    /// LARA/LOKY cross-coupled NAND run latch: set when `lene_n=0`,
    /// reset when `myte_n=0`.
    loky: bool,
    /// MATU: `dma_run`, `loky` re-sampled on `dma_phi↑`.
    dma_run: bool,
    /// NAKY..MUGU 8-bit ripple counter — OAM offset / source low byte.
    dma_a: u8,

    /// Master edge at which `dma_run` engaged — the byte clock's phase origin
    /// (1.5 M-cycles after FF46), used to align against a concurrent VRAM-DMA bus.
    start_edge: u64,

    /// Previous `data_phase` for `dma_phi`/`dma_phi_n` edge detection.
    prev_data_phase: bool,
}

impl Dma {
    pub fn new() -> Self {
        Self {
            source_register: 0xFF,
            source: 0,
            source_bus: Bus::External,
            lyxe: false,
            luvy: false,
            lene: false,
            myte: false,
            loky: false,
            dma_run: false,
            dma_a: 0,
            start_edge: 0,
            prev_data_phase: false,
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

    /// $FF46 write: `lavy` strobe sets the LYXE request latch and latches
    /// the source page. The arm then propagates through `luvy`/`lene` to
    /// `dma_run` over the next 1.5 M-cycles.
    pub fn begin_transfer(&mut self, source: u8) {
        self.source_register = source;
        self.source = (source as u16) * 0x100;
        self.source_bus = Bus::of(self.source).unwrap_or(Bus::External);
        self.lyxe = true;
    }

    /// Master edge at which `dma_run` engaged — the byte clock's phase origin.
    pub fn start_edge(&self) -> u64 {
        self.start_edge
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
            self.lene = self.luvy;
            // MYTE: d = nolo, async-reset by lapa (= counter reset).
            self.myte = !self.counter_held_reset() && self.nolo();
            self.settle_latches();
        }

        if dma_phi_rising {
            // META = AND2(dma_phi, loky): reset dominates, else advance.
            if self.counter_held_reset() {
                self.dma_a = 0;
            } else if self.loky {
                self.dma_a = self.dma_a.wrapping_add(1);
            }
            self.luvy = self.lyxe;
            // `dma_run` engaging marks the byte clock's phase origin.
            if !self.dma_run && self.loky {
                self.start_edge = master_edge;
            }
            self.dma_run = self.loky;
            self.settle_latches();
        }
    }

    /// `lapa = 0` (counter + MYTE held in reset) while `lene = 1` — the
    /// arm window, where `loko = lene` forces the reset.
    fn counter_held_reset(&self) -> bool {
        self.lene
    }

    /// NAVO terminal detect: `dma_a` has bits 0,1,2,3,4,7 set (0x9F=159).
    fn nolo(&self) -> bool {
        self.dma_a & NAVO_DECODE == NAVO_DECODE
    }

    /// Settle the level S-R latches after a DFF edge: LYXE reset by
    /// `loko = lene`; LOKY set by `lene_n=0`, reset by `myte_n=0`.
    fn settle_latches(&mut self) {
        if self.lene {
            self.lyxe = false;
            self.loky = true;
        } else if self.myte {
            self.loky = false;
        }
    }

    #[cfg(feature = "gbtrace")]
    pub fn dma_run(&self) -> bool {
        self.dma_run
    }

    #[cfg(feature = "gbtrace")]
    pub fn byte_index(&self) -> u8 {
        self.dma_a
    }

    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::DmaSnapshot) -> Dma {
        let mut dma = Dma::new();
        if snap.active {
            dma.source_register = (snap.source >> 8) as u8;
            dma.source = snap.source;
            dma.source_bus = Bus::of(snap.source).unwrap_or(Bus::External);
            dma.dma_run = true;
            dma.dma_a = snap.byte_index;
            dma.loky = true;
        }
        dma
    }
}
