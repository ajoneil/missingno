//! The console: 6507 + TIA + RIOT + cartridge on one colour-clock master.
//!
//! One CPU cycle = exactly three colour clocks. A CPU cycle's bus access
//! lands first, then its three TIA clocks — so a register write at CPU
//! cycle N shapes the beam from colour clock 3N, the coupling "racing the
//! beam" kernels depend on. WSYNC parks the CPU through the 6502 module's
//! RDY pin; the TIA raises it again as the beam wraps.

use crate::cartridge::{Cartridge, CartridgeError};
use crate::cpu::{Bus, Cpu};
use crate::riot::Riot;
use crate::tia::{Scanline, Tia, VISIBLE_CLOCKS};

/// One VSYNC-delimited frame. Height is whatever the kernel produced —
/// there is no hardware frame, only the software's sync pattern.
pub struct Frame {
    pub lines: Vec<[u8; VISIBLE_CLOCKS]>,
}

pub struct Vcs {
    pub cpu: Cpu,
    pub tia: Tia,
    pub riot: Riot,
    cartridge: Cartridge,
    clock_phase: u8,
    pending_tia_write: Option<TiaWrite>,
    building: Vec<Scanline>,
    in_vsync: bool,
    finished_frame: Option<Frame>,
}

/// The 6507's view of the board: A12 selects the cartridge; below it, A7
/// splits TIA from RIOT and A9 splits RIOT RAM from its I/O registers.
///
/// TIA writes are deferred: the data bus is valid at φ2, two colour
/// clocks into the CPU cycle, so the write registers at clock 3N+2 —
/// pinned by the RESP landing corpus (hblank x=3 AND mid-line strobe+5).
struct BoardBus<'a> {
    tia: &'a mut Tia,
    riot: &'a mut Riot,
    cartridge: &'a Cartridge,
    pending_tia_write: &'a mut Option<TiaWrite>,
}

pub(crate) struct TiaWrite {
    address: u16,
    data: u8,
    clocks_until_effective: u8,
}

impl Bus for BoardBus<'_> {
    fn read(&mut self, address: u16) -> u8 {
        if address & 0x1000 != 0 {
            self.cartridge.read(address)
        } else if address & 0x0080 == 0 {
            self.tia.read(address)
        } else if address & 0x0200 == 0 {
            self.riot.ram[(address & 0x7F) as usize]
        } else {
            self.riot.read(address)
        }
    }

    fn write(&mut self, address: u16, data: u8) {
        if address & 0x1000 != 0 {
        } else if address & 0x0080 == 0 {
            *self.pending_tia_write = Some(TiaWrite {
                address,
                data,
                clocks_until_effective: 3,
            });
        } else if address & 0x0200 == 0 {
            self.riot.ram[(address & 0x7F) as usize] = data;
        } else {
            self.riot.write(address, data);
        }
    }
}

impl Vcs {
    pub fn new(rom: &[u8]) -> Result<Vcs, CartridgeError> {
        let cartridge = Cartridge::load(rom)?;
        let mut cpu = Cpu::new();
        cpu.reset();
        Ok(Vcs {
            cpu,
            tia: Tia::new(),
            riot: Riot::new(),
            cartridge,
            clock_phase: 0,
            pending_tia_write: None,
            building: Vec::new(),
            in_vsync: false,
            finished_frame: None,
        })
    }

    /// Advance one colour clock. Every third clock carries the CPU (and
    /// RIOT) cycle before the TIA sees its clock.
    pub fn step_clock(&mut self) {
        if self.clock_phase == 0 {
            self.cpu.rdy = self.tia.cpu_ready;
            let mut bus = BoardBus {
                tia: &mut self.tia,
                riot: &mut self.riot,
                cartridge: &self.cartridge,
                pending_tia_write: &mut self.pending_tia_write,
            };
            self.cpu.step_cycle(&mut bus);
            self.riot.tick();
        }
        self.clock_phase = (self.clock_phase + 1) % 3;

        if let Some(write) = &mut self.pending_tia_write {
            write.clocks_until_effective -= 1;
            if write.clocks_until_effective == 0 {
                let write = self.pending_tia_write.take().unwrap();
                self.tia.write(write.address, write.data);
            }
        }
        self.tia.step_clock();
        if let Some(line) = self.tia.take_line() {
            self.collect_line(line);
        }
    }

    fn collect_line(&mut self, line: Scanline) {
        if line.vsync && !self.in_vsync {
            let lines = std::mem::take(&mut self.building);
            if !lines.is_empty() {
                self.finished_frame = Some(Frame {
                    lines: lines.into_iter().map(|l| l.pixels).collect(),
                });
            }
        }
        self.in_vsync = line.vsync;
        if !line.vsync {
            self.building.push(line);
        }
    }

    /// Run to the next instruction boundary. A WSYNC-parked opcode fetch
    /// waits here until the beam wraps and the TIA releases RDY.
    pub fn step_instruction(&mut self) {
        if self.cpu.halted() {
            return;
        }
        while self.cpu.at_instruction_boundary() {
            self.step_clock();
        }
        while !self.cpu.at_instruction_boundary() && !self.cpu.halted() {
            self.step_clock();
        }
    }

    /// Run until a frame completes, bounded so a kernel that never syncs
    /// cannot stall the caller. Returns `None` on budget exhaustion.
    pub fn step_frame(&mut self, budget_lines: usize) -> Option<Frame> {
        let budget_clocks = budget_lines * crate::tia::CLOCKS_PER_LINE as usize;
        for _ in 0..budget_clocks {
            self.step_clock();
            if let Some(frame) = self.finished_frame.take() {
                return Some(frame);
            }
        }
        None
    }
}
