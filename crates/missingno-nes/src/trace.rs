//! morepork capture: emit execution traces in morepork's native format
//! (feature `morepork`). The NES is morepork's second console family; its
//! field catalogue lives there (`morepork/src/family/nes`), and this module
//! captures exactly those fields from a running [`Nes`].

use std::path::Path;

use morepork::format::write::MoreporkWriter;
use morepork::header::{PixFormat, TraceHeader};
use morepork::snapshot::IndexedFrame;
pub use morepork::{Profile, Trigger};
use sha2::{Digest, Sha256};

use crate::console::Nes;
use crate::ppu::{Frame, PIXELS_PER_LINE, VISIBLE_LINES, master_palette};

/// NES pixels on a 4:3 NTSC display are wider than square.
pub const PIXEL_ASPECT: f32 = 8.0 / 7.0;

enum Emitter {
    Pc,
    A,
    X,
    Y,
    S,
    P,
    Rdy,
    Cycles,
    Control,
    Mask,
    Line,
    Dot,
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
        "control" => Emitter::Control,
        "mask" => Emitter::Mask,
        "line" => Emitter::Line,
        "dot" => Emitter::Dot,
        _ => match profile.memory.get(name) {
            Some(&addr) => Emitter::Memory(addr),
            None => {
                return Err(morepork::Error::Profile(format!(
                    "field '{name}' has no NES emitter"
                )));
            }
        },
    })
}

/// Run to the next instruction boundary, returning the CPU cycles
/// consumed. The tracing counterpart of [`Nes::step_instruction`].
pub fn step_instruction_counted(nes: &mut Nes) -> u16 {
    let mut cycles = 0u16;
    while nes.cpu.at_instruction_boundary() && !nes.cpu.halted() {
        nes.step_cycle();
        cycles += 1;
    }
    while !nes.cpu.at_instruction_boundary() && !nes.cpu.halted() {
        nes.step_cycle();
        cycles += 1;
    }
    cycles
}

/// Writes one trace entry per capture and an indexed frame snapshot per
/// completed frame.
pub struct Tracer {
    writer: MoreporkWriter,
    emitters: Vec<(usize, Emitter)>,
}

impl Tracer {
    pub fn create(
        path: impl AsRef<Path>,
        profile: &Profile,
        rom: &[u8],
    ) -> Result<Tracer, morepork::Error> {
        if profile.family != "nes" {
            return Err(morepork::Error::Profile(format!(
                "profile '{}' targets family '{}', not nes",
                profile.name, profile.family
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
            family: "nes".into(),
            model: "NTSC".into(),
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

        Ok(Tracer { writer, emitters })
    }

    /// Write one entry from the console's current state. `cycles` is the
    /// CPU-cycle delta since the previous entry (u16: OAM DMA freezes the
    /// CPU for 513+ cycles inside one instruction).
    pub fn capture(&mut self, nes: &Nes, cycles: u16) -> Result<(), morepork::Error> {
        for (col, emitter) in &self.emitters {
            let col = *col;
            match emitter {
                Emitter::Pc => self.writer.set_u16(col, nes.cpu.pc),
                Emitter::A => self.writer.set_u8(col, nes.cpu.a),
                Emitter::X => self.writer.set_u8(col, nes.cpu.x),
                Emitter::Y => self.writer.set_u8(col, nes.cpu.y),
                Emitter::S => self.writer.set_u8(col, nes.cpu.s),
                Emitter::P => self.writer.set_u8(col, nes.cpu.p),
                Emitter::Rdy => self.writer.set_bool(col, nes.cpu.rdy),
                Emitter::Cycles => self.writer.set_u16(col, cycles),
                Emitter::Control => self.writer.set_u8(col, nes.ppu.control),
                Emitter::Mask => self.writer.set_u8(col, nes.ppu.mask),
                Emitter::Line => self.writer.set_u16(col, nes.ppu.line()),
                Emitter::Dot => self.writer.set_u16(col, nes.ppu.dot()),
                Emitter::Memory(addr) => self.writer.set_u8(col, nes.peek(*addr)),
            }
        }
        self.writer.finish_entry()
    }

    /// Record a frame boundary, with the completed frame as an indexed
    /// snapshot (master-palette indices; the palette rides along so the
    /// payload is self-contained).
    pub fn mark_frame(&mut self, frame: Option<&Frame>) -> Result<(), morepork::Error> {
        let payload = frame.map(|frame| {
            IndexedFrame {
                width: PIXELS_PER_LINE as u16,
                height: VISIBLE_LINES,
                pixel_aspect: PIXEL_ASPECT,
                palette: master_palette()
                    .iter()
                    .map(|&(r, g, b)| [r, g, b])
                    .collect(),
                pixels: frame.pixels.clone(),
            }
            .to_bytes()
        });
        self.writer.mark_frame(payload.as_deref())
    }

    pub fn finish(self) -> Result<(), morepork::Error> {
        self.writer.finish()
    }
}
