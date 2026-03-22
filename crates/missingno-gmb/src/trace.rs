use std::collections::BTreeMap;
use std::path::Path;

pub use gbtrace::Trigger;
use gbtrace::{BootRom, ParquetTraceWriter, Profile, TraceEntry, TraceHeader};
use sha2::{Digest, Sha256};

use crate::GameBoy;

/// Captures gbtrace-format execution traces from a GameBoy.
pub struct Tracer {
    writer: ParquetTraceWriter,
    fields: Vec<String>,
    memory: BTreeMap<String, u16>,
    dot_count: u64,
    trigger: Trigger,
}

impl Tracer {
    /// Create a new tracer that writes to the given path.
    pub fn create(
        path: impl AsRef<Path>,
        profile: &Profile,
        gb: &GameBoy,
        boot_rom: BootRom,
    ) -> Result<Self, gbtrace::Error> {
        let rom_sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(gb.cartridge().rom());
            format!("{:x}", hasher.finalize())
        };

        let trigger = profile.trigger.clone();

        let header = TraceHeader {
            _header: true,
            format_version: "0.1.0".into(),
            emulator: "missingno".into(),
            emulator_version: env!("CARGO_PKG_VERSION").into(),
            rom_sha256,
            model: "DMG-B".into(),
            boot_rom,
            profile: profile.name.clone(),
            fields: profile.fields.clone(),
            trigger: profile.trigger.clone(),
            cy_unit: gbtrace::CycleUnit::Tcycle,
            notes: String::new(),
        };

        let writer = ParquetTraceWriter::create(path, &header)?;

        Ok(Self {
            writer,
            fields: profile.fields.clone(),
            memory: profile.memory.clone(),
            dot_count: 0,
            trigger,
        })
    }

    /// The trigger granularity for this trace.
    pub fn trigger(&self) -> Trigger {
        self.trigger.clone()
    }

    /// Capture the current state and write a trace entry.
    pub fn capture(&mut self, gb: &GameBoy) -> Result<(), gbtrace::Error> {
        let mut entry = TraceEntry::new();

        for field in &self.fields {
            // Check if this is a memory-mapped field first.
            if let Some(&addr) = self.memory.get(field) {
                entry.set_u8(field, gb.peek(addr));
                continue;
            }

            match field.as_str() {
                "cy" => entry.set_cy(self.dot_count),
                "a" => entry.set_u8("a", gb.cpu().a),
                "f" => entry.set_u8("f", gb.cpu().flags.bits()),
                "b" => entry.set_u8("b", gb.cpu().b),
                "c" => entry.set_u8("c", gb.cpu().c),
                "d" => entry.set_u8("d", gb.cpu().d),
                "e" => entry.set_u8("e", gb.cpu().e),
                "h" => entry.set_u8("h", gb.cpu().h),
                "l" => entry.set_u8("l", gb.cpu().l),
                "sp" => entry.set_u16("sp", gb.cpu().stack_pointer),
                "pc" => entry.set_u16("pc", gb.cpu().program_counter),
                "op" => entry.set_u8("op", gb.peek(gb.cpu().program_counter)),
                "ime" => entry.set_bool("ime", gb.cpu().interrupts_enabled()),
                "lcdc" => entry.set_u8("lcdc", gb.peek(0xFF40)),
                "stat" => entry.set_u8("stat", gb.peek(0xFF41)),
                "ly" => entry.set_u8("ly", gb.peek(0xFF44)),
                "lyc" => entry.set_u8("lyc", gb.peek(0xFF45)),
                "scy" => entry.set_u8("scy", gb.peek(0xFF42)),
                "scx" => entry.set_u8("scx", gb.peek(0xFF43)),
                "wy" => entry.set_u8("wy", gb.peek(0xFF4A)),
                "wx" => entry.set_u8("wx", gb.peek(0xFF4B)),
                "bgp" => entry.set_u8("bgp", gb.peek(0xFF47)),
                "obp0" => entry.set_u8("obp0", gb.peek(0xFF48)),
                "obp1" => entry.set_u8("obp1", gb.peek(0xFF49)),
                "dma" => entry.set_u8("dma", gb.peek(0xFF46)),
                "div" => entry.set_u8("div", gb.peek(0xFF04)),
                "tima" => entry.set_u8("tima", gb.peek(0xFF05)),
                "tma" => entry.set_u8("tma", gb.peek(0xFF06)),
                "tac" => entry.set_u8("tac", gb.peek(0xFF07)),
                "if_" => entry.set_u8("if_", gb.peek(0xFF0F)),
                "ie" => entry.set_u8("ie", gb.peek(0xFFFF)),
                "sb" => entry.set_u8("sb", gb.peek(0xFF01)),
                "sc" => entry.set_u8("sc", gb.peek(0xFF02)),
                _ => {}
            }
        }

        self.writer.write_entry(&entry)
    }

    /// Advance the dot counter by one T-cycle.
    pub fn advance_dot(&mut self) {
        self.dot_count += 1;
    }

    /// Advance the dot counter by multiple T-cycles (for instruction-level tracing).
    pub fn advance(&mut self, dots: u32) {
        self.dot_count += dots as u64;
    }

    /// Current T-cycle count.
    pub fn dot_count(&self) -> u64 {
        self.dot_count
    }

    /// Flush and finalize the trace file.
    pub fn finish(self) -> Result<(), gbtrace::Error> {
        self.writer.finish()
    }
}
