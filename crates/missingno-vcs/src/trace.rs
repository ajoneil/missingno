//! morepork capture: emit execution traces in morepork's native format
//! (feature `morepork`). The VCS is morepork's third console family; its
//! field catalogue lives there (`morepork/src/family/vcs`), and this module
//! captures exactly those fields from a running [`Vcs`]. Frames are
//! emergent from the software's sync pattern, so every frame snapshot
//! carries its own height.

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use morepork::header::{PixFormat, TraceHeader};
use morepork::snapshot::IndexedFrame;
pub use morepork::{Profile, Trigger};
use sha2::{Digest, Sha256};

use crate::TvStandard;
use crate::console::{Frame, Vcs};
use crate::tia::{VISIBLE_CLOCKS, palette, palette_index};
use crate::tv_standard::PIXEL_ASPECT;

enum Emitter {
    Pc,
    A,
    X,
    Y,
    S,
    P,
    Rdy,
    Cycles,
    Line,
    Clock,
    Timer,
    PortA,
    PortB,
    Memory(u16),
}

fn resolve_emitter(name: &str, profile: &Profile) -> Result<Emitter, morepork::Error> {
    Ok(match name {
        "pc" => Emitter::Pc,
        "a" => Emitter::A,
        "x" => Emitter::X,
        "y" => Emitter::Y,
        "s" => Emitter::S,
        "p" => Emitter::P,
        "rdy" => Emitter::Rdy,
        "cycles" => Emitter::Cycles,
        "line" => Emitter::Line,
        "clock" => Emitter::Clock,
        "timer" => Emitter::Timer,
        "port_a" => Emitter::PortA,
        "port_b" => Emitter::PortB,
        _ => match profile.memory.get(name) {
            Some(&addr) => Emitter::Memory(addr),
            None => {
                return Err(morepork::Error::Profile(format!(
                    "field '{name}' has no VCS emitter"
                )));
            }
        },
    })
}

/// Run to the next instruction boundary, returning the CPU cycles
/// consumed (WSYNC parks the CPU, so one store can span most of a
/// scanline). The tracing counterpart of [`Vcs::step_instruction`].
pub fn step_instruction_counted(vcs: &mut Vcs) -> u16 {
    let mut cycles = 0u16;
    while vcs.cpu.at_instruction_boundary() && !vcs.cpu.halted() {
        vcs.step_cpu_cycle();
        cycles += 1;
    }
    while !vcs.cpu.at_instruction_boundary() && !vcs.cpu.halted() {
        vcs.step_cpu_cycle();
        cycles += 1;
    }
    cycles
}

/// Writes one trace entry per capture and an indexed frame snapshot per
/// completed frame.
pub struct Tracer {
    writer: MoreporkWriter,
    emitters: Vec<(usize, Emitter)>,
    region: TvStandard,
}

impl Tracer {
    pub fn create(
        path: impl AsRef<Path>,
        profile: &Profile,
        rom: &[u8],
        region: TvStandard,
    ) -> Result<Tracer, morepork::Error> {
        if profile.system != "vcs" {
            return Err(morepork::Error::Profile(format!(
                "profile '{}' targets system '{}', not vcs",
                profile.name, profile.system
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(rom);
        let rom_sha256 = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();

        let header = TraceHeader {
            _header: true,
            format_version: "0.1.0".into(),
            emulator: "missingno".into(),
            emulator_version: env!("CARGO_PKG_VERSION").into(),
            rom_sha256,
            system: "vcs".into(),
            model: region.name().into(),
            profile: profile.name.clone(),
            fields: profile.fields.clone(),
            trigger: profile.trigger.clone(),
            pix_format: PixFormat::Indexed8,
            ..Default::default()
        };

        // Empty groups: the writer groups columns by the catalogue's
        // subsystem/layer defs.
        let writer = MoreporkWriter::create(path, &header, &[])?;

        let mut emitters = Vec::with_capacity(profile.fields.len());
        for (col, field) in profile.fields.iter().enumerate() {
            emitters.push((col, resolve_emitter(field, profile)?));
        }

        Ok(Tracer {
            writer,
            emitters,
            region,
        })
    }

    /// Write one entry from the console's current state. `cycles` is the
    /// CPU-cycle delta since the previous entry.
    pub fn capture(&mut self, vcs: &Vcs, cycles: u16) -> Result<(), morepork::Error> {
        for (col, emitter) in &self.emitters {
            let col = *col;
            match emitter {
                Emitter::Pc => self.writer.set_u16(col, vcs.cpu.pc),
                Emitter::A => self.writer.set_u8(col, vcs.cpu.a),
                Emitter::X => self.writer.set_u8(col, vcs.cpu.x),
                Emitter::Y => self.writer.set_u8(col, vcs.cpu.y),
                Emitter::S => self.writer.set_u8(col, vcs.cpu.s),
                Emitter::P => self.writer.set_u8(col, vcs.cpu.p),
                Emitter::Rdy => self.writer.set_bool(col, vcs.cpu.rdy),
                Emitter::Cycles => self.writer.set_u16(col, cycles),
                Emitter::Line => self.writer.set_u16(col, vcs.scanline() as u16),
                // The beam counter stays below CLOCKS_PER_LINE (228).
                Emitter::Clock => self.writer.set_u8(col, vcs.tia.beam() as u8),
                Emitter::Timer => self.writer.set_u8(col, vcs.peek(0x284)),
                Emitter::PortA => self.writer.set_u8(col, vcs.peek(0x280)),
                Emitter::PortB => self.writer.set_u8(col, vcs.peek(0x282)),
                Emitter::Memory(addr) => self.writer.set_u8(col, vcs.peek(*addr)),
            }
        }
        self.writer.finish_entry()
    }

    /// Record a frame boundary, with the completed frame as an indexed
    /// snapshot. Height is whatever the kernel produced; the palette
    /// rides along so the payload is self-contained.
    pub fn mark_frame(&mut self, frame: Option<&Frame>) -> Result<(), morepork::Error> {
        let payload = frame.map(|frame| {
            IndexedFrame {
                width: VISIBLE_CLOCKS as u16,
                height: frame.lines.len() as u16,
                pixel_aspect: PIXEL_ASPECT,
                palette: palette(self.region)
                    .iter()
                    .map(|&(r, g, b)| [r, g, b])
                    .collect(),
                pixels: frame
                    .lines
                    .iter()
                    .flat_map(|line| line.iter().map(|&pixel| palette_index(pixel) as u8))
                    .collect(),
            }
            .to_bytes()
        });
        self.writer.mark_frame(payload.as_deref())
    }

    pub fn finish(self) -> Result<(), morepork::Error> {
        self.writer.finish()
    }
}
