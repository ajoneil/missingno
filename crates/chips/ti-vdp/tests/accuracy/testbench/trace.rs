//! Morepork trace capture for the testbench: one entry per Z80 instruction
//! boundary, columns planned from the SG-1000 state schema so a testbench
//! trace diffs against any other producer's.

use std::path::Path;

use missingno_core::state::StateRecord;
use missingno_sg1000::state_schema::sg1000_state_schema;
use missingno_ti_vdp::Standard;
use missingno_trace::{
    BootRom, Column, Source, TRACE_OBSERVATIONS, TraceIdentity, TraceObservation, TraceScope,
    Trigger, build_columns, create_writer, emit_value, pix_format,
};
use missingno_zilog_z80::Cpu;
use morepork::format::TAG_MEMORY;
use morepork::format::write::MoreporkWriter;
use morepork::snapshot::{MemoryRegion, build_memory_payload};
use sha2::{Digest, Sha256};

use super::{Board, RAM_BASE};

pub struct Tracer {
    writer: MoreporkWriter,
    columns: Vec<Column<TraceObservation>>,
    line: u16,
}

impl Tracer {
    /// Capture is off unless `MOREPORK_PROFILE` is set (any value).
    pub fn create(rom: &str, standard: Standard, board: &Board) -> Option<Self> {
        std::env::var("MOREPORK_PROFILE").ok()?;

        let output_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../receipts/traces");
        std::fs::create_dir_all(&output_dir).unwrap();
        let stem = Path::new(rom).file_stem().unwrap().to_string_lossy();
        let body = match standard {
            Standard::Ntsc => "",
            Standard::Pal => "_pal",
        };
        let path = output_dir.join(format!("{stem}{body}.morepork"));
        eprintln!("morepork: writing {}", path.display());

        let mut hasher = Sha256::new();
        hasher.update(&board.cart);
        let rom_sha256 = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();

        let schema = sg1000_state_schema();
        let (columns, field_defs) = build_columns(schema, TraceScope::Full, TRACE_OBSERVATIONS);
        let writer = create_writer(
            &path,
            schema,
            TraceIdentity {
                rom_sha256,
                model: match standard {
                    Standard::Ntsc => "TMS9918A",
                    Standard::Pal => "TMS9929A",
                },
                scope: TraceScope::Full,
                trigger: Trigger::Instruction,
                pix_format: pix_format(schema.frame.format),
                boot_rom: BootRom::Skip,
                snapshot_kinds: vec!["frame".into(), "memory".into()],
            },
            field_defs,
        )
        .unwrap_or_else(|e| panic!("creating {}: {e}", path.display()));

        Some(Tracer {
            writer,
            columns,
            line: board.vdp.line(),
        })
    }

    pub fn capture(&mut self, cpu: &Cpu, board: &mut Board) {
        let line = board.vdp.line();
        if line < self.line {
            self.writer.mark_frame(None).unwrap();
        }
        self.line = line;

        // The testbench has no PSG or board latches: their columns stay null.
        let mut record = StateRecord::new();
        if let Some(cpu) = cpu.boundary_state() {
            missingno_zilog_z80::record::write_state(&mut record, &cpu);
        }
        missingno_ti_vdp::record::write_state(&mut record, &board.vdp.boundary_state());
        let cycles = cpu.bus_trace().len() as u16;
        let ram_write = board.ram_write.take();

        let w = &mut self.writer;
        for (col, column) in self.columns.iter().enumerate() {
            match &column.source {
                Source::Field(name) => {
                    emit_value(w, col, column.ty, column.nullable, record.get(name))
                }
                Source::Observation(TraceObservation::Cycles) => w.set_u16(col, cycles),
                Source::Observation(TraceObservation::Result) => w.set_u8(col, board.ram[0]),
                Source::Observation(TraceObservation::Code) => w.set_u8(col, board.ram[1]),
                Source::Observation(TraceObservation::Observed) => w.set_u8(col, board.ram[2]),
                Source::Observation(TraceObservation::Expected) => w.set_u8(col, board.ram[3]),
                Source::Observation(TraceObservation::RamWriteAddr) => match ram_write {
                    Some((address, _)) => w.set_u16(col, address),
                    None => w.set_null(col),
                },
                Source::Observation(TraceObservation::RamWriteData) => match ram_write {
                    Some((_, data)) => w.set_u8(col, data),
                    None => w.set_null(col),
                },
            }
        }
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
