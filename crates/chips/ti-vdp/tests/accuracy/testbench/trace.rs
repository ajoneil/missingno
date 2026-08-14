//! Morepork trace capture for the testbench: one entry per Z80 instruction
//! boundary, columns named by morepork's sg1000 catalogue so a testbench
//! trace diffs against any other producer's.

use std::collections::BTreeMap;
use std::path::Path;

use missingno_zilog_z80::{Cpu, InterruptMode};
use morepork::format::TAG_MEMORY;
use morepork::format::write::MoreporkWriter;
use morepork::header::{ExtensionField, TraceHeader};
use morepork::snapshot::{MemoryRegion, build_memory_payload};
use morepork::{BootRom, FieldType, Trigger};
use sha2::{Digest, Sha256};

use super::{Board, RAM_BASE};

/// A column's value, typed as the catalogue types that column.
enum Value {
    U8(u8),
    U16(u16),
    Bool(bool),
    OptU8(Option<u8>),
    OptU16(Option<u16>),
}

const COLUMNS: usize = 49;

/// Every column of one entry, paired with its catalogue name — the single
/// source of truth the header's field list is built from too.
fn columns(cpu: &Cpu, board: &Board) -> [(&'static str, Value); COLUMNS] {
    // At a boundary the recorded cycles are the retired instruction's: the
    // first drove its opcode fetch, and their count is its T-states.
    let op_addr = cpu
        .bus_trace()
        .first()
        .map_or(cpu.pc, |cycle| cycle.address);
    let cycles = cpu.bus_trace().len() as u8;
    let im = match cpu.im {
        InterruptMode::Mode0 => 0,
        InterruptMode::Mode1 => 1,
        InterruptMode::Mode2 => 2,
    };
    let vdp = &board.vdp;
    let registers = vdp.registers();
    let ram_write = board.ram_write;

    [
        ("pc", Value::U16(cpu.pc)),
        ("op_addr", Value::U16(op_addr)),
        ("sp", Value::U16(cpu.sp)),
        ("a", Value::U8(cpu.a)),
        ("f", Value::U8(cpu.f)),
        ("b", Value::U8(cpu.b)),
        ("c", Value::U8(cpu.c)),
        ("d", Value::U8(cpu.d)),
        ("e", Value::U8(cpu.e)),
        ("h", Value::U8(cpu.h)),
        ("l", Value::U8(cpu.l)),
        ("ix", Value::U16(cpu.ix)),
        ("iy", Value::U16(cpu.iy)),
        ("wz", Value::U16(cpu.wz)),
        ("a_", Value::U8(cpu.a_)),
        ("f_", Value::U8(cpu.f_)),
        ("b_", Value::U8(cpu.b_)),
        ("c_", Value::U8(cpu.c_)),
        ("d_", Value::U8(cpu.d_)),
        ("e_", Value::U8(cpu.e_)),
        ("h_", Value::U8(cpu.h_)),
        ("l_", Value::U8(cpu.l_)),
        ("i", Value::U8(cpu.i)),
        ("r", Value::U8(cpu.r)),
        ("im", Value::U8(im)),
        ("iff1", Value::Bool(cpu.iff1)),
        ("iff2", Value::Bool(cpu.iff2)),
        ("halted", Value::Bool(cpu.halted)),
        ("cycles", Value::U8(cycles)),
        ("reg0", Value::U8(registers[0])),
        ("reg1", Value::U8(registers[1])),
        ("reg2", Value::U8(registers[2])),
        ("reg3", Value::U8(registers[3])),
        ("reg4", Value::U8(registers[4])),
        ("reg5", Value::U8(registers[5])),
        ("reg6", Value::U8(registers[6])),
        ("reg7", Value::U8(registers[7])),
        ("status", Value::U8(vdp.peek_status())),
        ("line", Value::U16(vdp.line())),
        ("dot", Value::U16(vdp.dot())),
        ("addr", Value::U16(vdp.address())),
        ("latch", Value::Bool(vdp.awaiting_second_byte())),
        ("buffer", Value::U8(vdp.read_buffer())),
        ("result", Value::U8(board.ram[0])),
        ("code", Value::U8(board.ram[1])),
        ("observed", Value::U8(board.ram[2])),
        ("expected", Value::U8(board.ram[3])),
        (
            "ram_write_addr",
            Value::OptU16(ram_write.map(|(address, _)| address)),
        ),
        (
            "ram_write_data",
            Value::OptU8(ram_write.map(|(_, data)| data)),
        ),
    ]
}

/// The RAM write tap, which the catalogue has no field for.
fn extension_fields() -> BTreeMap<String, ExtensionField> {
    ["ram_write_addr", "ram_write_data"]
        .into_iter()
        .zip([FieldType::UInt16, FieldType::UInt8])
        .map(|(name, field_type)| {
            (
                name.to_string(),
                ExtensionField {
                    field_type,
                    nullable: true,
                    description: Some("RAM write since the previous entry".into()),
                    source: Some("missingno".into()),
                },
            )
        })
        .collect()
}

pub struct Tracer {
    writer: MoreporkWriter,
    line: u16,
}

impl Tracer {
    /// Capture is off unless `MOREPORK_PROFILE` is set (any value).
    pub fn create(rom: &str, cpu: &Cpu, board: &Board) -> Option<Self> {
        std::env::var("MOREPORK_PROFILE").ok()?;

        let output_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../receipts/traces");
        std::fs::create_dir_all(&output_dir).unwrap();
        let stem = Path::new(rom).file_stem().unwrap().to_string_lossy();
        let path = output_dir.join(format!("{stem}.morepork"));
        eprintln!("morepork: writing {}", path.display());

        let mut hasher = Sha256::new();
        hasher.update(&board.cart);
        let rom_sha256 = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();

        let header = TraceHeader {
            _header: true,
            format_version: "0.1.0".into(),
            emulator: "missingno".into(),
            emulator_version: env!("CARGO_PKG_VERSION").into(),
            rom_sha256,
            system: "sg1000".into(),
            model: "TMS9918A".into(),
            boot_rom: BootRom::Skip,
            profile: "tier1".into(),
            fields: columns(cpu, board)
                .iter()
                .map(|(name, _)| name.to_string())
                .collect(),
            trigger: Trigger::Instruction,
            extension_fields: extension_fields(),
            ..Default::default()
        };

        let writer = MoreporkWriter::create(&path, &header, &[])
            .unwrap_or_else(|e| panic!("creating {}: {e}", path.display()));

        Some(Tracer {
            writer,
            line: board.vdp.line(),
        })
    }

    pub fn capture(&mut self, cpu: &Cpu, board: &mut Board) {
        let line = board.vdp.line();
        if line < self.line {
            self.writer.mark_frame(None).unwrap();
        }
        self.line = line;

        for (column, (_, value)) in columns(cpu, board).iter().enumerate() {
            match value {
                Value::U8(value) => self.writer.set_u8(column, *value),
                Value::U16(value) => self.writer.set_u16(column, *value),
                Value::Bool(value) => self.writer.set_bool(column, *value),
                Value::OptU8(Some(value)) => self.writer.set_u8(column, *value),
                Value::OptU16(Some(value)) => self.writer.set_u16(column, *value),
                Value::OptU8(None) | Value::OptU16(None) => self.writer.set_null(column),
            }
        }
        board.ram_write = None;
        self.writer.finish_entry().unwrap();
    }

    /// Close the trace with the whole test RAM, the RESULT block included.
    pub fn finish(mut self, ram: &[u8]) {
        let payload = build_memory_payload(&[MemoryRegion {
            start: RAM_BASE,
            data: ram.to_vec(),
        }]);
        self.writer.write_snapshot(TAG_MEMORY, &payload).unwrap();
        self.writer.finish().unwrap();
    }
}
