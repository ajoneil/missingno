//! The Super Game Boy's part of a DMG save state: its registers as record
//! fields, its RAM and ICD2 captures as off-bus memory spans.

use missingno_core::state::{StateRecord, StateValue};
use missingno_core::system::StateError;

use super::{
    AttributeMap, BorderMapEntry, CommandState, MaskMode, PendingTransfer, Rgb555, Sgb, SgbBorder,
    SgbPalette,
};
use crate::ppu::screen::{self, Screen};

const PALETTE_BYTES: usize = 8;
const ATTRIBUTE_MAP_BYTES: usize = 20 * 18;
const PACKET_BYTES: usize = 16;
const CAPTURE_BYTES: usize = screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize;

fn palette_bytes(palettes: &[SgbPalette]) -> Vec<u8> {
    palettes
        .iter()
        .flat_map(|p| p.colors)
        .flat_map(|c| c.0.to_le_bytes())
        .collect()
}

fn attribute_bytes(maps: &[AttributeMap]) -> Vec<u8> {
    maps.iter()
        .flat_map(|m| m.cells.iter().flatten().copied())
        .collect()
}

fn capture_bytes(capture: &Screen) -> Vec<u8> {
    capture.pixels().map(|p| p.0).collect()
}

fn transfer_code(transfer: PendingTransfer) -> u8 {
    match transfer {
        PendingTransfer::Palettes => 0,
        PendingTransfer::Attributes => 1,
        PendingTransfer::BorderTiles { upper_half: false } => 2,
        PendingTransfer::BorderTiles { upper_half: true } => 3,
        PendingTransfer::BorderMap => 4,
    }
}

fn transfer_from(code: u8) -> Result<PendingTransfer, StateError> {
    Ok(match code {
        0 => PendingTransfer::Palettes,
        1 => PendingTransfer::Attributes,
        2 => PendingTransfer::BorderTiles { upper_half: false },
        3 => PendingTransfer::BorderTiles { upper_half: true },
        4 => PendingTransfer::BorderMap,
        _ => return Err(StateError::Corrupt),
    })
}

fn words(bytes: &[u8]) -> impl Iterator<Item = u16> + '_ {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| u16::from_le_bytes(*w))
}

impl SgbBorder {
    fn capture(&self, out: &mut Vec<(&'static str, Vec<u8>)>) {
        out.push(("sgb_border_tiles", self.tiles.to_vec()));
        out.push((
            "sgb_border_map",
            self.map.iter().flat_map(|e| e.0.to_le_bytes()).collect(),
        ));
        out.push((
            "sgb_border_palettes",
            self.palettes
                .iter()
                .flatten()
                .flat_map(|c| c.0.to_le_bytes())
                .collect(),
        ));
    }

    /// A record from before the border restores an empty one.
    fn from_spans(
        tiles: Option<&[u8]>,
        map: Option<&[u8]>,
        palettes: Option<&[u8]>,
    ) -> Result<SgbBorder, StateError> {
        let mut border = SgbBorder::default();
        let (Some(tiles), Some(map), Some(palettes)) = (tiles, map, palettes) else {
            return Ok(border);
        };
        if tiles.len() != border.tiles.len()
            || map.len() != border.map.len() * 2
            || palettes.len() != border.palettes.len() * 16 * 2
        {
            return Err(StateError::Corrupt);
        }
        border.tiles.copy_from_slice(tiles);
        for (entry, word) in border.map.iter_mut().zip(words(map)) {
            *entry = BorderMapEntry(word);
        }
        for (color, word) in border.palettes.iter_mut().flatten().zip(words(palettes)) {
            *color = Rgb555(word);
        }
        Ok(border)
    }
}

fn palettes_from(bytes: &[u8], count: usize) -> Result<Vec<SgbPalette>, StateError> {
    if bytes.len() != count * PALETTE_BYTES {
        return Err(StateError::Corrupt);
    }
    Ok(bytes
        .as_chunks::<PALETTE_BYTES>()
        .0
        .iter()
        .map(|b| SgbPalette {
            colors: std::array::from_fn(|i| Rgb555::from_bytes(b[i * 2], b[i * 2 + 1])),
        })
        .collect())
}

fn attributes_from(bytes: &[u8], count: usize) -> Result<Vec<AttributeMap>, StateError> {
    if bytes.len() != count * ATTRIBUTE_MAP_BYTES {
        return Err(StateError::Corrupt);
    }
    Ok(bytes
        .as_chunks::<ATTRIBUTE_MAP_BYTES>()
        .0
        .iter()
        .map(|b| {
            let mut map = AttributeMap::new();
            for (row, cells) in map.cells.iter_mut().zip(b.as_chunks::<20>().0) {
                row.copy_from_slice(cells);
            }
            map
        })
        .collect())
}

fn capture_from(bytes: &[u8]) -> Result<Screen, StateError> {
    if bytes.len() != CAPTURE_BYTES {
        return Err(StateError::Corrupt);
    }
    let mut capture = Screen::default();
    capture.restore_front(bytes);
    Ok(capture)
}

impl Sgb {
    pub(crate) fn write_state(&self, r: &mut StateRecord) {
        let (receiver, expected, received, bit_index) = match &self.command_state {
            CommandState::Idle => (0u8, 0, 0, 0),
            CommandState::ReceivingBits {
                packets_expected,
                packets_received,
                bit_index,
                ..
            } => (1, *packets_expected, *packets_received, *bit_index),
            CommandState::AwaitingPacketStart {
                packets_expected,
                packets_received,
                ..
            } => (2, *packets_expected, *packets_received, 0),
        };
        r.set("sgb_mask", self.mask_mode as u8)
            .set("sgb_joypad_index", self.joypad_index)
            .set("sgb_joypad_mask", self.joypad_mask)
            .set("sgb_p15_high", self.prev_p15_high)
            .set("sgb_lines_high", self.prev_lines_high)
            .set("sgb_receiver", receiver)
            .set("sgb_packets_expected", expected)
            .set("sgb_packets_received", received)
            .set("sgb_bit_index", bit_index)
            .set("sgb_preset_in_force", self.preset.is_some());
        if let Some((countdown, transfer)) = self.pending_transfer {
            r.set("sgb_transfer_countdown", countdown)
                .set("sgb_transfer_kind", transfer_code(transfer));
        }
    }

    pub(crate) fn capture_memory(&self, out: &mut Vec<(&'static str, Vec<u8>)>) {
        let (packet, packets): ([u8; PACKET_BYTES], &[u8]) = match &self.command_state {
            CommandState::Idle => ([0; PACKET_BYTES], &[]),
            CommandState::ReceivingBits {
                current_packet,
                all_packets,
                ..
            } => (*current_packet, all_packets),
            CommandState::AwaitingPacketStart { all_packets, .. } => {
                ([0; PACKET_BYTES], all_packets)
            }
        };
        out.push(("sgb_palettes", palette_bytes(&self.palettes)));
        out.push((
            "sgb_attribute_map",
            attribute_bytes(std::slice::from_ref(&self.attribute_map)),
        ));
        out.push(("sgb_system_palettes", palette_bytes(&self.system_palettes)));
        out.push((
            "sgb_attribute_files",
            attribute_bytes(&self.attribute_files),
        ));
        out.push(("sgb_packet", packet.to_vec()));
        out.push(("sgb_packets", packets.to_vec()));
        out.push(("sgb_capture", capture_bytes(&self.last_screen)));
        if let Some(frozen) = &self.frozen_screen {
            out.push(("sgb_frozen_capture", capture_bytes(frozen)));
        }
        if let Some(preset) = &self.preset {
            out.push(("sgb_preset", palette_bytes(std::slice::from_ref(preset))));
        }
        self.border.capture(out);
    }

    /// The SGB a record describes, or `None` if the record carries no SGB.
    pub(crate) fn from_state(
        record: &StateRecord,
        memory: &[(String, Vec<u8>)],
    ) -> Result<Option<Sgb>, StateError> {
        let field = |name: &str| record.get(name).filter(|v| !matches!(v, StateValue::Null));
        let byte = |name: &str| match field(name) {
            Some(StateValue::Int(v)) if *v <= u8::MAX as u32 => Ok(*v as u8),
            _ => Err(StateError::Corrupt),
        };
        let flag = |name: &str| match field(name) {
            Some(StateValue::Bool(b)) => Ok(*b),
            _ => Err(StateError::Corrupt),
        };
        let span = |name: &str| {
            memory
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, data)| data.as_slice())
        };
        let required = |name: &str| span(name).ok_or(StateError::Corrupt);

        if field("sgb_mask").is_none() {
            return Ok(None);
        }
        let mut sgb = Sgb::new();
        sgb.mask_mode = match byte("sgb_mask")? {
            0 => MaskMode::Disabled,
            1 => MaskMode::Freeze,
            2 => MaskMode::Black,
            3 => MaskMode::BackdropColor,
            _ => return Err(StateError::Corrupt),
        };
        sgb.joypad_index = byte("sgb_joypad_index")?;
        sgb.joypad_mask = byte("sgb_joypad_mask")?;
        sgb.prev_p15_high = flag("sgb_p15_high")?;
        sgb.prev_lines_high = flag("sgb_lines_high")?;

        let packets_expected = byte("sgb_packets_expected")?;
        let packets_received = byte("sgb_packets_received")?;
        let current_packet: [u8; PACKET_BYTES] = required("sgb_packet")?
            .try_into()
            .map_err(|_| StateError::Corrupt)?;
        let all_packets = required("sgb_packets")?.to_vec();
        sgb.command_state = match byte("sgb_receiver")? {
            0 => CommandState::Idle,
            1 => CommandState::ReceivingBits {
                packets_expected,
                packets_received,
                current_packet,
                bit_index: byte("sgb_bit_index")?,
                all_packets,
            },
            2 => CommandState::AwaitingPacketStart {
                packets_expected,
                packets_received,
                all_packets,
            },
            _ => return Err(StateError::Corrupt),
        };
        sgb.pending_transfer = match field("sgb_transfer_countdown") {
            None => None,
            Some(_) => Some((
                byte("sgb_transfer_countdown")?,
                transfer_from(byte("sgb_transfer_kind")?)?,
            )),
        };

        sgb.palettes = palettes_from(required("sgb_palettes")?, 4)?
            .try_into()
            .map_err(|_| StateError::Corrupt)?;
        sgb.attribute_map = attributes_from(required("sgb_attribute_map")?, 1)?[0];
        sgb.system_palettes =
            palettes_from(required("sgb_system_palettes")?, sgb.system_palettes.len())?;
        sgb.attribute_files =
            attributes_from(required("sgb_attribute_files")?, sgb.attribute_files.len())?;
        sgb.last_screen = capture_from(required("sgb_capture")?)?;
        sgb.frozen_screen = span("sgb_frozen_capture").map(capture_from).transpose()?;
        sgb.preset = if flag("sgb_preset_in_force")? {
            Some(palettes_from(required("sgb_preset")?, 1)?[0])
        } else {
            None
        };
        sgb.border = SgbBorder::from_spans(
            span("sgb_border_tiles"),
            span("sgb_border_map"),
            span("sgb_border_palettes"),
        )?;
        Ok(Some(sgb))
    }
}
