//! The chip's state as one capturable value, and putting it back.
//!
//! What a consumer cannot reach through the two CPU ports: the raster
//! counters, the port engine's in-flight memory access, the instants the
//! status flags were set at, the sprite scanner's place in its lattice, and
//! the latched fetch the raster is walking. The DRAM and the three pixel
//! buffers travel beside this as byte regions.
//!
//! Every instant the model compares against is a difference from the running
//! XTAL count, and no comparison reaches back further than a memory cycle — so
//! the state carries elapsed periods rather than an absolute clock, and a
//! restore re-founds that clock on an origin of its own choosing.

use crate::Vdp;
use crate::port::{FLAG_RELEASE_XTALS, InFlightAccess, PortTransfer, TRANSFER_LOCK_XTALS};
use crate::render::Segment;
use crate::scan::ScanStop;
use crate::standard::XTALS_PER_LINE;
use crate::vram::VramAddress;

/// The whole chip at one instant, less its DRAM and its pixel buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VdpState {
    /// The eight write-only registers.
    pub registers: [u8; 8],
    /// The vertical counter, and the XTAL period reached within its line.
    pub line: u16,
    pub line_xtal: u32,
    /// Visible rasters completed since power-on.
    pub fields_completed: u64,
    pub port: PortState,
    pub status: StatusState,
    pub scanner: ScannerState,
    pub segment: SegmentState,
}

/// The CPU port: its pointer, control-port latch, read-ahead buffer, and the
/// transfer waiting on a memory cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortState {
    /// The auto-incrementing VRAM pointer.
    pub address: u16,
    /// A first control-port byte is held, awaiting its second.
    pub awaiting_second_byte: bool,
    pub read_buffer: u8,
    /// The transfer a servicing cycle would sample, the one it replaced, and
    /// how long ago the replacement was written — a lock inside the settle
    /// window merges the two.
    pub transfer: PortTransfer,
    pub prior_transfer: PortTransfer,
    pub transfer_written_ago: u32,
    /// The address latched by the request that raised the flag.
    pub pending_address: u16,
    pub pending_flag: bool,
    /// The access a memory cycle has claimed, while one is in flight.
    pub access: Option<AccessState>,
}

/// A claimed CPU access: the address latched at claim, and the XTAL periods
/// since — the lock and release instants follow from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessState {
    pub address: u16,
    pub claimed_ago: u8,
}

/// The three status flags, the latched fifth-sprite index they present, and
/// how long ago the two race-resolving flags were set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusState {
    pub frame: bool,
    pub fifth_sprite: bool,
    pub coincidence: bool,
    pub frame_set_ago: u32,
    pub fifth_sprite_set_ago: u32,
    pub sprite_field: u8,
}

/// The sprite pre-processing scanner: its counter, where this line's ramp
/// ends, and the boundary window around its latest step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScannerState {
    pub counter: u8,
    pub stop: ScanStop,
    pub stepped_ago: u32,
    pub step_from: u8,
    /// The fifth match's hold on the presented field, while one is armed.
    pub field_hold: Option<u8>,
    pub fifth_match_this_scan: bool,
}

/// The latched fetch the raster is drawing from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentState {
    pub bits: u8,
    pub foreground: u8,
    pub background: u8,
    pub start_x: u16,
    pub end_x: u16,
}

impl Vdp {
    /// The chip's state at this instant.
    pub fn boundary_state(&self) -> VdpState {
        VdpState {
            registers: self.registers,
            line: self.line,
            line_xtal: self.xtal_in_line,
            fields_completed: self.frames_completed,
            port: PortState {
                address: self.port.address.value(),
                awaiting_second_byte: self.port.awaiting_second_byte,
                read_buffer: self.port.read_buffer,
                transfer: self.port.transfer,
                prior_transfer: self.port.prior_transfer,
                transfer_written_ago: self.ago(self.port.transfer_written_at),
                pending_address: self.port.pending_address.value(),
                pending_flag: self.port.pending_flag,
                access: self.port.in_flight.as_ref().map(|access| AccessState {
                    address: access.address.value(),
                    claimed_ago: (TRANSFER_LOCK_XTALS - (access.lock_at - self.xtal_total)) as u8,
                }),
            },
            status: StatusState {
                frame: self.status.frame,
                fifth_sprite: self.status.fifth_sprite,
                coincidence: self.status.coincidence,
                frame_set_ago: self.ago(self.status.frame_set_at),
                fifth_sprite_set_ago: self.ago(self.status.fifth_sprite_set_at),
                sprite_field: self.status.sprite_field,
            },
            scanner: ScannerState {
                counter: self.scanner.counter,
                stop: self.scanner.stop,
                stepped_ago: self.ago(self.scanner.stepped_at),
                step_from: self.scanner.step_from,
                field_hold: self.scanner.field_hold,
                fifth_match_this_scan: self.scanner.fifth_match_this_scan,
            },
            segment: SegmentState {
                bits: self.segment.bits,
                foreground: self.segment.fg,
                background: self.segment.bg,
                start_x: self.segment.start_x as u16,
                end_x: self.segment.end_x as u16,
            },
        }
    }

    /// Reseat the chip on a captured state. The raster counters are taken
    /// modulo the standard's geometry, so a state from a part cut for another
    /// standard lands inside this one's frame rather than off it.
    pub fn restore_boundary(&mut self, state: &VdpState) {
        self.registers = state.registers;
        self.line = state.line % self.standard.lines_per_frame();
        self.xtal_in_line = state.line_xtal % XTALS_PER_LINE;
        self.frames_completed = state.fields_completed;

        // The captured instants are all behind the current one, and an
        // in-flight access's release may already have passed, so the restored
        // clock starts one memory cycle past the furthest of them.
        let now = [
            state.port.transfer_written_ago,
            state.status.frame_set_ago,
            state.status.fifth_sprite_set_ago,
            state.scanner.stepped_ago,
        ]
        .into_iter()
        .max()
        .unwrap_or(0) as u64
            + TRANSFER_LOCK_XTALS;
        self.xtal_total = now;

        self.port.address = VramAddress::new(state.port.address);
        self.port.awaiting_second_byte = state.port.awaiting_second_byte;
        self.port.read_buffer = state.port.read_buffer;
        self.port.transfer = state.port.transfer;
        self.port.prior_transfer = state.port.prior_transfer;
        self.port.transfer_written_at = now - state.port.transfer_written_ago as u64;
        self.port.pending_address = VramAddress::new(state.port.pending_address);
        self.port.pending_flag = state.port.pending_flag;
        self.port.in_flight = state.port.access.map(|access| InFlightAccess {
            address: VramAddress::new(access.address),
            lock_at: now + TRANSFER_LOCK_XTALS - access.claimed_ago as u64,
            release_at: now + FLAG_RELEASE_XTALS - access.claimed_ago as u64,
        });

        self.status.frame = state.status.frame;
        self.status.fifth_sprite = state.status.fifth_sprite;
        self.status.coincidence = state.status.coincidence;
        self.status.frame_set_at = now - state.status.frame_set_ago as u64;
        self.status.fifth_sprite_set_at = now - state.status.fifth_sprite_set_ago as u64;
        self.status.sprite_field = state.status.sprite_field;

        self.scanner.counter = state.scanner.counter;
        self.scanner.stop = state.scanner.stop;
        self.scanner.stepped_at = now - state.scanner.stepped_ago as u64;
        self.scanner.step_from = state.scanner.step_from;
        self.scanner.field_hold = state.scanner.field_hold;
        self.scanner.fifth_match_this_scan = state.scanner.fifth_match_this_scan;

        self.segment = Segment {
            bits: state.segment.bits,
            fg: state.segment.foreground,
            bg: state.segment.background,
            start_x: state.segment.start_x as usize,
            end_x: state.segment.end_x as usize,
        };
    }

    /// The row being composited, as far as the raster has reached across it.
    pub fn line_buffer(&self) -> &[u8] {
        &self.line_pixels
    }

    /// The line-latched sprite plane of the row being emitted; 0 is no sprite
    /// pixel.
    pub fn sprite_plane(&self) -> &[u8] {
        &self.sprite_line
    }

    pub fn restore_vram(&mut self, bytes: &[u8]) {
        copy_into(&mut self.vram, bytes);
    }

    pub fn restore_line_buffer(&mut self, bytes: &[u8]) {
        copy_into(&mut self.line_pixels, bytes);
    }

    pub fn restore_sprite_plane(&mut self, bytes: &[u8]) {
        copy_into(&mut self.sprite_line, bytes);
    }

    /// The visible raster as it stands — the rows this field has already
    /// emitted, which no later state can reconstruct.
    pub fn restore_raster(&mut self, pixels: &[u8]) {
        copy_into(&mut self.frame.pixels, pixels);
    }

    /// XTAL periods since an instant, saturating: every window the model
    /// compares against is shorter than a memory cycle, so a saturated value
    /// restores to the same behaviour.
    fn ago(&self, at: u64) -> u32 {
        u32::try_from(self.xtal_total - at).unwrap_or(u32::MAX)
    }
}

/// Copy as much of `source` as fits, leaving any remainder untouched — a
/// truncated region restores what it carries rather than panicking.
fn copy_into(destination: &mut [u8], source: &[u8]) {
    let len = destination.len().min(source.len());
    destination[..len].copy_from_slice(&source[..len]);
}

#[cfg(test)]
mod tests {
    use crate::{Standard, Vdp};

    /// Everything the model reads is a difference from the XTAL count, so a
    /// captured state restores to one that captures identically.
    #[test]
    fn a_captured_state_survives_its_own_restore() {
        let mut vdp = Vdp::new(Standard::Ntsc);
        vdp.write_control(0x00);
        vdp.write_control(0x40);
        for value in 0..64u8 {
            vdp.write_data(value);
        }
        vdp.tick(5_000);
        vdp.write_control(0x82);
        vdp.write_data(0x5A);
        vdp.tick(7);

        let state = vdp.boundary_state();
        let mut restored = Vdp::new(Standard::Ntsc);
        restored.restore_vram(vdp.vram());
        restored.restore_boundary(&state);
        assert_eq!(restored.boundary_state(), state);

        // And the two parts stay in step as the raster runs on.
        let mut original = vdp;
        for _ in 0..4 {
            original.tick(700);
            restored.tick(700);
            assert_eq!(restored.boundary_state(), original.boundary_state());
            assert_eq!(restored.vram(), original.vram());
        }
    }
}
