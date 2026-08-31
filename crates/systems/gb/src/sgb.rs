use super::ppu::screen::{self, Screen};
use rgb::RGB8;

/// Convert the rendered screen into a 4KB transfer data buffer.
/// The SGB/SNES reads data from the rendered video signal, not raw VRAM.
/// The game arranges tiles $00-$FF sequentially in the tilemap with identity palette,
/// so the rendered 2bpp pixel data reconstructs to the original tile bytes.
/// Screen is 20×18 tiles (360 tiles), we read the first 256 (tiles $00-$FF) = 4096 bytes.
fn screen_to_transfer_data(screen: &Screen) -> Vec<u8> {
    let mut data = Vec::with_capacity(4096);
    // 20 tiles per row, we need 256 tiles = 12 full rows (240 tiles) + 16 tiles from row 13
    for tile_index in 0..256 {
        let tile_x = (tile_index % 20) * 8;
        let tile_y = (tile_index / 20) * 8;
        for row in 0..8 {
            let py = tile_y + row;
            let mut low_byte = 0u8;
            let mut high_byte = 0u8;
            for col in 0..8 {
                let px = tile_x + col;
                let pixel = if px < screen::PIXELS_PER_LINE as usize
                    && py < screen::NUM_SCANLINES as usize
                {
                    screen.pixel(px as u8, py as u8).0
                } else {
                    0
                };
                // 2bpp planar: bit 0 of pixel goes to low_byte, bit 1 to high_byte
                // MSB first (leftmost pixel = bit 7)
                if pixel & 1 != 0 {
                    low_byte |= 1 << (7 - col);
                }
                if pixel & 2 != 0 {
                    high_byte |= 1 << (7 - col);
                }
            }
            data.push(low_byte);
            data.push(high_byte);
        }
    }
    data
}

/// 15-bit RGB555 color as used by the SNES/SGB.
#[derive(Copy, Clone, Debug, Default)]
pub struct Rgb555(pub u16);

/// Gamma ramp for SNES RGB555 output, derived from SameBoy's SGB-specific
/// color correction curve. Simulates the gamma characteristics of the SNES
/// DAC viewed on a period-appropriate CRT, darkening the full range to
/// produce richer, more saturated colors on a modern LCD.
const GAMMA_RAMP: [u8; 32] = [
    0, 2, 5, 9, 15, 20, 27, 34, 42, 50, 58, 67, 76, 85, 94, 104, 114, 123, 133, 143, 153, 163, 173,
    182, 192, 202, 211, 220, 229, 238, 247, 255,
];

impl Rgb555 {
    pub fn to_rgb8(self) -> RGB8 {
        let r5 = (self.0 & 0x1f) as usize;
        let g5 = ((self.0 >> 5) & 0x1f) as usize;
        let b5 = ((self.0 >> 10) & 0x1f) as usize;
        RGB8::new(GAMMA_RAMP[r5], GAMMA_RAMP[g5], GAMMA_RAMP[b5])
    }

    pub fn from_bytes(low: u8, high: u8) -> Self {
        Self(u16::from_le_bytes([low, high]))
    }
}

/// One SGB palette: 4 colors.
#[derive(Copy, Clone, Debug)]
pub struct SgbPalette {
    pub colors: [Rgb555; 4],
}

impl Default for SgbPalette {
    fn default() -> Self {
        // Grayscale so games are visible before they set palettes
        Self {
            colors: [
                Rgb555(0x7FFF), // White
                Rgb555(0x56B5), // Light gray
                Rgb555(0x294A), // Dark gray
                Rgb555(0x0000), // Black
            ],
        }
    }
}

/// 20x18 attribute map: each cell maps to palette 0-3.
#[derive(Copy, Clone, Debug)]
pub struct AttributeMap {
    pub cells: [[u8; 20]; 18],
}

impl Default for AttributeMap {
    fn default() -> Self {
        Self::new()
    }
}

impl AttributeMap {
    pub fn new() -> Self {
        Self {
            cells: [[0; 20]; 18],
        }
    }

    /// One packed attribute file: 5 bytes per row, 4 cells per byte, leftmost
    /// cell in the most significant bit pair.
    fn from_packed(bytes: &[u8]) -> Self {
        let mut atf = Self::new();
        for y in 0..18 {
            for byte_in_row in 0..5 {
                let Some(&byte_val) = bytes.get(y * 5 + byte_in_row) else {
                    return atf;
                };
                for bit_pair in 0..4 {
                    let x = byte_in_row * 4 + bit_pair;
                    atf.cells[y][x] = (byte_val >> (6 - bit_pair * 2)) & 0x03;
                }
            }
        }
        atf
    }
}

/// Screen masking mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MaskMode {
    Disabled,
    Freeze,
    Black,
    BackdropColor,
}

/// Data needed by the rendering layer.
#[derive(Copy, Clone, Debug)]
pub struct SgbRenderData {
    pub palettes: [SgbPalette; 4],
    pub attribute_map: AttributeMap,
    pub mask_mode: MaskMode,
}

impl SgbRenderData {
    /// The single shared backdrop (CGRAM $00): shade 0 is transparent on the
    /// SNES, so every palette shows palette 0's colour 0.
    pub fn backdrop(&self) -> Rgb555 {
        self.palettes[0].colors[0]
    }

    /// Resolve one screen pixel through the attribute map and its palette.
    pub fn color_at(&self, x: usize, y: usize, shade: u8) -> Rgb555 {
        if shade == 0 {
            return self.backdrop();
        }
        let pal_id = self.attribute_map.cells[y / 8][x / 8] as usize;
        self.palettes[pal_id].colors[shade as usize]
    }
}

enum CommandState {
    Idle,
    ReceivingBits {
        packets_expected: u8,
        packets_received: u8,
        current_packet: [u8; 16],
        bit_index: u8,
        all_packets: Vec<u8>,
    },
    // A non-final packet completed; the next packet begins at its own start pulse
    AwaitingPacketStart {
        packets_expected: u8,
        packets_received: u8,
        all_packets: Vec<u8>,
    },
}

#[derive(Clone, Copy)]
enum PendingTransfer {
    Palettes,
    Attributes,
}

pub struct Sgb {
    palettes: [SgbPalette; 4],
    attribute_map: AttributeMap,
    system_palettes: Vec<SgbPalette>,
    attribute_files: Vec<AttributeMap>,
    pub mask_mode: MaskMode,
    // MLT_REQ joypad selector: a counter ANDed with a 2-bit mask on each advance
    pub(crate) joypad_index: u8,
    joypad_mask: u8,
    prev_p15_high: bool,
    // Whether the last write left both select lines high — pulses are release-framed
    prev_lines_high: bool,
    command_state: CommandState,
    // The ICD2's captured picture, used by TRN commands; a static LD stream holds it
    last_screen: Screen,
    // MASK_EN Freeze halts the displayed picture while the capture continues underneath
    frozen_screen: Option<Screen>,
    // Deferred VRAM transfer: countdown frames + transfer type
    pending_transfer: Option<(u8, PendingTransfer)>,
}

impl Default for Sgb {
    fn default() -> Self {
        Self::new()
    }
}

impl Sgb {
    pub fn new() -> Self {
        Self {
            palettes: [SgbPalette::default(); 4],
            attribute_map: AttributeMap::new(),
            system_palettes: vec![SgbPalette::default(); 512],
            attribute_files: vec![AttributeMap::new(); 45],
            mask_mode: MaskMode::Disabled,
            joypad_index: 0,
            joypad_mask: 0,
            prev_p15_high: false,
            prev_lines_high: false,
            command_state: CommandState::Idle,
            last_screen: Screen::default(),
            frozen_screen: None,
            pending_transfer: None,
        }
    }

    /// Update the stored screen snapshot (called each frame from execute loop).
    pub fn update_screen(&mut self, screen: &Screen) {
        self.last_screen = screen.clone();
        if let Some((countdown, transfer)) = self.pending_transfer {
            if countdown <= 1 {
                self.pending_transfer = None;
                match transfer {
                    PendingTransfer::Palettes => self.cmd_pal_trn(),
                    PendingTransfer::Attributes => self.cmd_attr_trn(),
                }
            } else {
                self.pending_transfer = Some((countdown - 1, transfer));
            }
        }
    }

    /// The picture the SNES side shows: the freeze-time snapshot while masked,
    /// otherwise the live capture.
    pub fn displayed_screen(&self) -> &Screen {
        match (self.mask_mode, &self.frozen_screen) {
            (MaskMode::Freeze, Some(screen)) => screen,
            _ => &self.last_screen,
        }
    }

    pub fn render_data(&self) -> SgbRenderData {
        SgbRenderData {
            palettes: self.palettes,
            attribute_map: self.attribute_map,
            mask_mode: self.mask_mode,
        }
    }

    /// Called on every write to FF00.
    pub fn write_joypad(&mut self, value: u8) {
        let p14_low = value & 0x10 == 0;
        let p15_low = value & 0x20 == 0;
        let both_low = p14_low && p15_low;

        // The joypad counter advances on every P15 low→high edge, packet traffic included
        if !p15_low && !self.prev_p15_high {
            self.joypad_index = (self.joypad_index + 1) & self.joypad_mask;
        }
        self.prev_p15_high = !p15_low;

        let released = self.prev_lines_high;
        self.prev_lines_high = !p14_low && !p15_low;

        match &mut self.command_state {
            CommandState::Idle => {
                if both_low {
                    self.command_state = CommandState::ReceivingBits {
                        packets_expected: 0,
                        packets_received: 0,
                        current_packet: [0; 16],
                        bit_index: 0,
                        all_packets: Vec::new(),
                    };
                }
            }
            CommandState::ReceivingBits {
                packets_expected,
                packets_received,
                current_packet,
                bit_index,
                all_packets,
            } => {
                if both_low {
                    if *bit_index == 0 && *packets_received == 0 {
                        // Got another reset before any data — the previous reset was
                        // just a player-cycle probe. Start fresh.
                        *current_packet = [0; 16];
                    } else {
                        // Mid-packet restart — the in-flight packet is discarded
                        *current_packet = [0; 16];
                        *bit_index = 0;
                    }
                    return;
                }

                if !p14_low && !p15_low {
                    // Both high — release between bits, not a data bit
                    return;
                }

                if !released {
                    // A select change with no release since the last pulse doesn't clock
                    // the receiver — this is what keeps ordinary polling out of packets
                    return;
                }

                if *bit_index < 128 {
                    // Data bit
                    let byte_idx = (*bit_index / 8) as usize;
                    let bit_pos = *bit_index % 8;
                    if p15_low && !p14_low {
                        current_packet[byte_idx] |= 1 << bit_pos;
                    }
                    *bit_index += 1;
                } else {
                    // Stop bit (bit 128) — packet complete
                    let is_first_packet = *packets_received == 0;

                    if is_first_packet {
                        *packets_expected = current_packet[0] & 0x07;
                        if *packets_expected == 0 {
                            self.command_state = CommandState::Idle;
                            return;
                        }
                    }

                    all_packets.extend_from_slice(current_packet);
                    *packets_received += 1;

                    if *packets_received >= *packets_expected {
                        let data = all_packets.clone();
                        self.command_state = CommandState::Idle;
                        self.dispatch_command(&data);
                    } else {
                        let packets_expected = *packets_expected;
                        let packets_received = *packets_received;
                        let all_packets = std::mem::take(all_packets);
                        self.command_state = CommandState::AwaitingPacketStart {
                            packets_expected,
                            packets_received,
                            all_packets,
                        };
                    }
                }
            }
            CommandState::AwaitingPacketStart {
                packets_expected,
                packets_received,
                all_packets,
            } => {
                if both_low {
                    let packets_expected = *packets_expected;
                    let packets_received = *packets_received;
                    let all_packets = std::mem::take(all_packets);
                    self.command_state = CommandState::ReceivingBits {
                        packets_expected,
                        packets_received,
                        current_packet: [0; 16],
                        bit_index: 0,
                        all_packets,
                    };
                }
            }
        }
    }

    fn dispatch_command(&mut self, data: &[u8]) {
        let command_code = data[0] >> 3;
        match command_code {
            0x00 => self.cmd_pal_pair(data, 0, 1),
            0x01 => self.cmd_pal_pair(data, 2, 3),
            0x02 => self.cmd_pal_pair(data, 0, 3),
            0x03 => self.cmd_pal_pair(data, 1, 2),
            0x04 => self.cmd_attr_blk(data),
            0x05 => self.cmd_attr_lin(data),
            0x06 => self.cmd_attr_div(data),
            0x07 => self.cmd_attr_chr(data),
            0x0A => self.cmd_pal_set(data),
            0x0B => self.pending_transfer = Some((3, PendingTransfer::Palettes)),
            0x11 => self.cmd_mlt_req(data),
            0x15 => self.pending_transfer = Some((3, PendingTransfer::Attributes)),
            0x16 => self.cmd_attr_set(data),
            0x17 => self.cmd_mask_en(data),
            0x19 => self.cmd_pal_pri(data),
            // Border/transfer commands — accept but don't render borders
            0x08 | 0x09 | 0x0C | 0x0D | 0x0E | 0x0F | 0x10 | 0x12 | 0x13 | 0x14 | 0x18 => {}
            _ => {}
        }
    }

    // --- Palette commands ---

    fn cmd_pal_pair(&mut self, data: &[u8], pal_a: usize, pal_b: usize) {
        let color0 = Rgb555::from_bytes(data[1], data[2]);
        for p in &mut self.palettes {
            p.colors[0] = color0;
        }
        for i in 0..3 {
            self.palettes[pal_a].colors[i + 1] =
                Rgb555::from_bytes(data[3 + i * 2], data[4 + i * 2]);
        }
        for i in 0..3 {
            self.palettes[pal_b].colors[i + 1] =
                Rgb555::from_bytes(data[9 + i * 2], data[10 + i * 2]);
        }
    }

    fn cmd_pal_set(&mut self, data: &[u8]) {
        for i in 0..4 {
            // Palette IDs are 9 bits wide
            let idx = (u16::from_le_bytes([data[1 + i * 2], data[2 + i * 2]]) & 0x1FF) as usize;
            self.palettes[i] = self.system_palettes[idx];
        }
        let flags = data[9];
        if flags & 0x80 != 0 {
            // Apply attribute file
            let atf_idx = (flags & 0x3F) as usize;
            if atf_idx < self.attribute_files.len() {
                self.attribute_map = self.attribute_files[atf_idx];
            }
        }
        if flags & 0x40 != 0 {
            // Cancel mask
            self.mask_mode = MaskMode::Disabled;
        }
    }

    fn cmd_pal_trn(&mut self) {
        let data = screen_to_transfer_data(&self.last_screen);
        for pal_idx in 0..512 {
            let base = pal_idx * 8;
            for c in 0..4 {
                let offset = base + c * 2;
                self.system_palettes[pal_idx].colors[c] =
                    Rgb555::from_bytes(data[offset], data[offset + 1]);
            }
        }
    }

    fn cmd_pal_pri(&mut self, _data: &[u8]) {
        // Accept but don't change behavior — we always use SGB palettes when active
    }

    // --- Attribute commands ---

    fn cmd_attr_blk(&mut self, data: &[u8]) {
        let num_datasets = data[1] as usize;
        for i in 0..num_datasets {
            let offset = 2 + i * 6;
            if offset + 5 >= data.len() {
                break;
            }
            let control = data[offset];
            let palettes_byte = data[offset + 1];
            let x1 = data[offset + 2] as usize;
            let y1 = data[offset + 3] as usize;
            let x2 = data[offset + 4] as usize;
            let y2 = data[offset + 5] as usize;

            let inside_pal = palettes_byte & 0x03;
            let border_pal = (palettes_byte >> 2) & 0x03;
            let outside_pal = (palettes_byte >> 4) & 0x03;

            let change_inside = control & 0x01 != 0;
            let change_border = control & 0x02 != 0;
            let change_outside = control & 0x04 != 0;

            // If only inside or only outside is set, border follows that one
            let effective_border_pal = if change_border {
                border_pal
            } else if change_inside && !change_outside {
                inside_pal
            } else if change_outside && !change_inside {
                outside_pal
            } else {
                border_pal
            };

            for y in 0..18usize {
                for x in 0..20usize {
                    let is_inside = x > x1 && x < x2 && y > y1 && y < y2;
                    let is_border = x >= x1 && x <= x2 && y >= y1 && y <= y2 && !is_inside;

                    if is_inside && change_inside {
                        self.attribute_map.cells[y][x] = inside_pal;
                    } else if is_border
                        && (change_border
                            || (change_inside && !change_outside)
                            || (change_outside && !change_inside))
                    {
                        self.attribute_map.cells[y][x] = effective_border_pal;
                    } else if !is_inside && !is_border && change_outside {
                        self.attribute_map.cells[y][x] = outside_pal;
                    }
                }
            }
        }
    }

    fn cmd_attr_lin(&mut self, data: &[u8]) {
        let num_datasets = data[1] as usize;
        for i in 0..num_datasets {
            let offset = 2 + i;
            if offset >= data.len() {
                break;
            }
            let dataset = data[offset];
            let line_num = (dataset & 0x1F) as usize;
            let pal = (dataset >> 5) & 0x03;
            let horizontal = dataset & 0x80 != 0;

            if horizontal && line_num < 18 {
                for x in 0..20 {
                    self.attribute_map.cells[line_num][x] = pal;
                }
            } else if !horizontal && line_num < 20 {
                for y in 0..18 {
                    self.attribute_map.cells[y][line_num] = pal;
                }
            }
        }
    }

    fn cmd_attr_div(&mut self, data: &[u8]) {
        let flags = data[1];
        let pal_below_right = flags & 0x03;
        let pal_above_left = (flags >> 2) & 0x03;
        let pal_on_line = (flags >> 4) & 0x03;
        let horizontal = flags & 0x40 != 0;
        let coord = data[2] as usize;

        if horizontal {
            for y in 0..18usize {
                for x in 0..20 {
                    self.attribute_map.cells[y][x] = if y < coord {
                        pal_above_left
                    } else if y == coord {
                        pal_on_line
                    } else {
                        pal_below_right
                    };
                }
            }
        } else {
            for y in 0..18usize {
                for x in 0..20usize {
                    self.attribute_map.cells[y][x] = if x < coord {
                        pal_above_left
                    } else if x == coord {
                        pal_on_line
                    } else {
                        pal_below_right
                    };
                }
            }
        }
    }

    fn cmd_attr_chr(&mut self, data: &[u8]) {
        let start_x = data[1] as usize;
        let start_y = data[2] as usize;
        let count = u16::from_le_bytes([data[3], data[4]]) as usize;
        let direction = data[5]; // 0 = left-to-right, 1 = top-to-bottom

        let mut x = start_x;
        let mut y = start_y;

        // Data starts at byte 6, packed 4 attributes per byte (2 bits each, MSB pair first)
        let mut written = 0;
        let mut byte_offset = 6;
        let mut bit_offset = 0;

        while written < count {
            if byte_offset >= data.len() {
                break;
            }
            let pal = (data[byte_offset] >> (6 - bit_offset)) & 0x03;
            bit_offset += 2;
            if bit_offset >= 8 {
                bit_offset = 0;
                byte_offset += 1;
            }

            if x < 20 && y < 18 {
                self.attribute_map.cells[y][x] = pal;
            }

            if direction == 0 {
                // Left to right, then next row
                x += 1;
                if x >= 20 {
                    x = 0;
                    y += 1;
                }
            } else {
                // Top to bottom, then next column
                y += 1;
                if y >= 18 {
                    y = 0;
                    x += 1;
                }
            }

            written += 1;
        }
    }

    fn cmd_attr_trn(&mut self) {
        let data = screen_to_transfer_data(&self.last_screen);
        // 45 attribute files, each 90 bytes (20x18 / 4 = 90 bytes packed)
        for file_idx in 0..45 {
            self.attribute_files[file_idx] = AttributeMap::from_packed(&data[file_idx * 90..]);
        }
    }

    fn cmd_attr_set(&mut self, data: &[u8]) {
        let atf_idx = (data[1] & 0x3F) as usize;
        if atf_idx < self.attribute_files.len() {
            self.attribute_map = self.attribute_files[atf_idx];
        }
        if data[1] & 0x40 != 0 {
            self.mask_mode = MaskMode::Disabled;
        }
    }

    // --- System commands ---

    fn cmd_mlt_req(&mut self, data: &[u8]) {
        let mask = data[1] & 0x03;
        // The glitched two-player request ($02) advances the counter once before masking
        if mask == 2 {
            self.joypad_index = self.joypad_index.wrapping_add(1);
        }
        self.joypad_mask = mask;
        self.joypad_index &= mask;
    }

    fn cmd_mask_en(&mut self, data: &[u8]) {
        self.mask_mode = match data[1] & 0x03 {
            0 => MaskMode::Disabled,
            1 => MaskMode::Freeze,
            2 => MaskMode::Black,
            3 => MaskMode::BackdropColor,
            _ => unreachable!(),
        };
        self.frozen_screen = match self.mask_mode {
            MaskMode::Freeze => Some(self.last_screen.clone()),
            _ => None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ppu::types::palette::PaletteIndex;

    /// Clock one 16-byte packet through the joypad pulse protocol.
    fn send_packet(sgb: &mut Sgb, bytes: [u8; 16]) {
        sgb.write_joypad(0x00);
        sgb.write_joypad(0x30);
        for byte in bytes {
            for bit in 0..8 {
                sgb.write_joypad(if byte >> bit & 1 != 0 { 0x10 } else { 0x20 });
                sgb.write_joypad(0x30);
            }
        }
        sgb.write_joypad(0x20);
        sgb.write_joypad(0x30);
    }

    /// One frame of an ordinary joypad poll: both selects in turn, no release between
    fn poll_frame(sgb: &mut Sgb) {
        sgb.write_joypad(0x20);
        sgb.write_joypad(0x10);
        sgb.write_joypad(0x30);
    }

    #[test]
    fn polling_after_a_stray_reset_never_dispatches() {
        let mut sgb = Sgb::new();
        sgb.write_joypad(0x30);
        sgb.write_joypad(0x00);
        for _ in 0..200 {
            poll_frame(&mut sgb);
        }
        assert!(matches!(sgb.command_state, CommandState::Idle));
        assert!(sgb.pending_transfer.is_none());

        send_packet(
            &mut sgb,
            [0xB9, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(sgb.mask_mode, MaskMode::Black);
    }

    #[test]
    fn unreleased_select_change_does_not_clock_a_bit() {
        let mut sgb = Sgb::new();
        sgb.write_joypad(0x00);
        sgb.write_joypad(0x30);
        sgb.write_joypad(0x20);
        sgb.write_joypad(0x10);
        match &sgb.command_state {
            CommandState::ReceivingBits {
                bit_index,
                current_packet,
                ..
            } => {
                assert_eq!(*bit_index, 1);
                assert_eq!(current_packet[0], 0);
            }
            _ => panic!("receiver should be mid-packet"),
        }
    }

    #[test]
    fn shade_zero_resolves_to_the_shared_backdrop() {
        let mut data = SgbRenderData {
            palettes: [SgbPalette::default(); 4],
            attribute_map: AttributeMap::new(),
            mask_mode: MaskMode::Disabled,
        };
        data.palettes[0].colors[0] = Rgb555(0x1111);
        data.palettes[2].colors[0] = Rgb555(0x2222);
        data.palettes[2].colors[3] = Rgb555(0x3333);
        data.attribute_map.cells[0][0] = 2;

        assert_eq!(data.color_at(0, 0, 0).0, 0x1111);
        assert_eq!(data.color_at(0, 0, 3).0, 0x3333);
    }

    #[test]
    fn attr_chr_unpacks_cells_msb_pair_first() {
        let mut sgb = Sgb::new();
        let mut packet = [0u8; 16];
        packet[0] = (0x07 << 3) | 1;
        packet[3] = 4;
        packet[6] = 0b00_01_10_11;
        send_packet(&mut sgb, packet);
        assert_eq!(sgb.attribute_map.cells[0][..4], [0, 1, 2, 3]);
    }

    #[test]
    fn continuation_packet_waits_for_its_own_start_pulse() {
        let mut sgb = Sgb::new();
        let mut first = [0xFFu8; 16];
        first[0] = (0x07 << 3) | 2;
        first[1] = 0;
        first[2] = 0;
        first[3] = 60;
        first[4] = 0;
        first[5] = 0;
        send_packet(&mut sgb, first);

        for _ in 0..50 {
            poll_frame(&mut sgb);
        }

        send_packet(&mut sgb, [0xFF; 16]);
        for cell in 0..60usize {
            assert_eq!(sgb.attribute_map.cells[cell / 20][cell % 20], 3);
        }
        assert_eq!(sgb.attribute_map.cells[3][0], 0);
    }

    /// The inverse of `screen_to_transfer_data`: a screen displaying `data` as
    /// tiles $00-$FF in raster order, as a game does during a VRAM transfer.
    fn screen_showing(data: &[u8]) -> Screen {
        let mut screen = Screen::default();
        for tile_index in 0..256 {
            for row in 0..8 {
                let low = data.get(tile_index * 16 + row * 2).copied().unwrap_or(0);
                let high = data
                    .get(tile_index * 16 + row * 2 + 1)
                    .copied()
                    .unwrap_or(0);
                for col in 0..8 {
                    let shade = (low >> (7 - col) & 1) | ((high >> (7 - col) & 1) << 1);
                    let x = (tile_index % 20) * 8 + col;
                    let y = (tile_index / 20) * 8 + row;
                    if x < screen::PIXELS_PER_LINE as usize && y < screen::NUM_SCANLINES as usize {
                        screen.draw_pixel(x as u8, y as u8, PaletteIndex(shade));
                    }
                }
            }
        }
        screen.present();
        screen
    }

    fn run_transfer(sgb: &mut Sgb, command: u8, data: &[u8]) {
        let mut packet = [0u8; 16];
        packet[0] = (command << 3) | 1;
        send_packet(sgb, packet);
        let screen = screen_showing(data);
        for _ in 0..3 {
            sgb.update_screen(&screen);
        }
    }

    #[test]
    fn pal_trn_and_pal_set_load_system_palettes() {
        let mut sgb = Sgb::new();
        let mut data = vec![0u8; 4096];
        // System palette 1: colours 0x1111, 0x2222, 0x3333, 0x4444
        for (i, colour) in [0x1111u16, 0x2222, 0x3333, 0x4444].iter().enumerate() {
            data[8 + i * 2..10 + i * 2].copy_from_slice(&colour.to_le_bytes());
        }
        run_transfer(&mut sgb, 0x0B, &data);

        send_packet(
            &mut sgb,
            [0xB9, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(sgb.mask_mode, MaskMode::Black);

        // PAL_SET: palettes 1,0,1,0; flags = cancel mask
        let mut pal_set = [0u8; 16];
        pal_set[0] = (0x0A << 3) | 1;
        pal_set[1] = 0x01;
        pal_set[5] = 0x01;
        pal_set[9] = 0x40;
        send_packet(&mut sgb, pal_set);

        assert_eq!(sgb.mask_mode, MaskMode::Disabled);
        // The transfer replaced every system palette; palette 0 is now all zeroes
        for (slot, expected) in [(0, 0x1111), (1, 0x0000), (2, 0x1111), (3, 0x0000)] {
            assert_eq!(sgb.palettes[slot].colors[0].0, expected);
        }
        assert_eq!(sgb.palettes[0].colors[3].0, 0x4444);
    }

    #[test]
    fn pal_set_ids_are_nine_bits() {
        let mut sgb = Sgb::new();
        let mut data = vec![0u8; 4096];
        data[8..10].copy_from_slice(&0x5A5Au16.to_le_bytes());
        run_transfer(&mut sgb, 0x0B, &data);

        // ID 0x201 wraps to system palette 1
        let mut pal_set = [0u8; 16];
        pal_set[0] = (0x0A << 3) | 1;
        pal_set[1] = 0x01;
        pal_set[2] = 0x02;
        send_packet(&mut sgb, pal_set);
        assert_eq!(sgb.palettes[0].colors[0].0, 0x5A5A);
    }

    #[test]
    fn attr_trn_unpacks_cells_msb_pair_first() {
        let mut sgb = Sgb::new();
        let mut data = vec![0u8; 4096];
        // ATF 0, row 0: cells 0-3 = 0,1,2,3; ATF 1, row 0: a mid-byte region
        // edge 1,1,2,2 (the pattern the LSB-first order mirrors)
        data[0] = 0b00_01_10_11;
        data[90] = 0b01_01_10_10;
        run_transfer(&mut sgb, 0x15, &data);

        // ATTR_SET file 0
        send_packet(
            &mut sgb,
            [
                (0x16 << 3) | 1,
                0x00,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        );
        assert_eq!(sgb.attribute_map.cells[0][..4], [0, 1, 2, 3]);

        // ATTR_SET file 1
        send_packet(
            &mut sgb,
            [
                (0x16 << 3) | 1,
                0x01,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        );
        assert_eq!(sgb.attribute_map.cells[0][..4], [1, 1, 2, 2]);
    }

    /// A screen filled with one shade, as the capture holds it.
    fn screen_of_shade(shade: u8) -> Screen {
        let mut screen = Screen::default();
        for y in 0..screen::NUM_SCANLINES {
            for x in 0..screen::PIXELS_PER_LINE {
                screen.draw_pixel(x, y, PaletteIndex(shade));
            }
        }
        screen.present();
        screen
    }

    fn mask_en(sgb: &mut Sgb, mode: u8) {
        let mut packet = [0u8; 16];
        packet[0] = (0x17 << 3) | 1;
        packet[1] = mode;
        send_packet(sgb, packet);
    }

    #[test]
    fn freeze_holds_the_picture_while_the_capture_runs_on() {
        let mut sgb = Sgb::new();
        sgb.update_screen(&screen_of_shade(1));
        mask_en(&mut sgb, 1);
        assert_eq!(sgb.mask_mode, MaskMode::Freeze);

        sgb.update_screen(&screen_of_shade(2));
        assert_eq!(sgb.displayed_screen().pixel(0, 0).0, 1);
        assert_eq!(sgb.last_screen.pixel(0, 0).0, 2);

        mask_en(&mut sgb, 0);
        assert_eq!(sgb.displayed_screen().pixel(0, 0).0, 2);
    }

    /// A cartridge declaring SGB support, running `program` from $0100.
    fn sgb_cartridge(program: &[u8]) -> crate::cartridge::Cartridge {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x100 + program.len()].copy_from_slice(program);
        rom[0x146] = 0x03;
        crate::cartridge::Cartridge::new(rom, None, None).unwrap()
    }

    #[test]
    fn the_capture_survives_the_lcd_turning_off() {
        // Paint every shade 3 through BGP, let two frames present, then turn the
        // LCD off in VBlank.
        let wait_for_line = |line: u8| [0xf0, 0x44, 0xfe, line, 0x20, 0xfa];
        let mut program = vec![0x3e, 0xff, 0xe0, 0x47]; // LD A,$FF; LDH ($47),A
        for line in [0x90, 0x00, 0x90, 0x00, 0x90] {
            program.extend_from_slice(&wait_for_line(line));
        }
        program.extend_from_slice(&[0xaf, 0xe0, 0x40, 0x18, 0xfe]); // XOR A; LDH ($40),A; JR -2

        let mut console = crate::chassis::Console::<crate::Dmg>::new(sgb_cartridge(&program), None);
        for _ in 0..2_000_000 {
            console.step();
            if !console.ppu().control().video_enabled() {
                break;
            }
        }
        assert!(!console.ppu().control().video_enabled());
        assert_eq!(console.screen().pixel(0, 0).0, 0);
        assert_eq!(console.sgb().unwrap().displayed_screen().pixel(0, 0).0, 3);
    }

    /// Clock one packet into a running console through its joypad writes.
    fn send_console_packet(console: &mut crate::chassis::Console<crate::Dmg>, bytes: [u8; 16]) {
        use crate::model::Model;
        let mut pulse = |value: u8| console.model.on_joypad_write(value);
        pulse(0x00);
        pulse(0x30);
        for byte in bytes {
            for bit in 0..8 {
                pulse(if byte >> bit & 1 != 0 { 0x10 } else { 0x20 });
                pulse(0x30);
            }
        }
        pulse(0x20);
        pulse(0x30);
    }

    /// The picture and colouring the console delivers for display right now.
    fn delivered_picture(
        console: &crate::chassis::Console<crate::Dmg>,
    ) -> (crate::ppu::screen::Screen, SgbRenderData) {
        use crate::system::ConsoleUi;
        let frame =
            <crate::Dmg as ConsoleUi>::screen_display(console, Some(console.screen().clone()))
                .expect("an SGB console always shows a picture");
        match frame {
            missingno_core::video::Frame::Console(frame) => {
                match frame.as_any().downcast_ref::<crate::frame::GbFrame>() {
                    Some(crate::frame::GbFrame::Sgb(sgb)) => (sgb.screen.clone(), sgb.render_data),
                    _ => panic!("expected an SGB frame"),
                }
            }
            _ => panic!("expected a console frame"),
        }
    }

    #[test]
    fn the_capture_shows_through_the_lcd_coming_back_on() {
        // Paint every shade 3 through BGP, let two frames present, then in VBlank
        // turn the LCD off, repaint through BGP at shade 1, and turn it back on.
        let wait_for_line = |line: u8| [0xf0, 0x44, 0xfe, line, 0x20, 0xfa];
        let mut program = vec![0x3e, 0xff, 0xe0, 0x47]; // LD A,$FF; LDH ($47),A
        for line in [0x90, 0x00, 0x90, 0x00, 0x90] {
            program.extend_from_slice(&wait_for_line(line));
        }
        program.extend_from_slice(&[
            0xaf, 0xe0, 0x40, // XOR A; LDH ($40),A
            0x3e, 0x55, 0xe0, 0x47, // LD A,$55; LDH ($47),A
            0x3e, 0x91, 0xe0, 0x40, // LD A,$91; LDH ($40),A
            0x18, 0xfe, // JR -2
        ]);

        let mut console = crate::chassis::Console::<crate::Dmg>::new(sgb_cartridge(&program), None);
        for _ in 0..2_000_000 {
            console.step();
            if !console.ppu().control().video_enabled() {
                break;
            }
        }
        assert!(!console.ppu().control().video_enabled());
        assert_eq!(console.sgb().unwrap().displayed_screen().pixel(0, 0).0, 3);

        // PAL01: shade 3 of palette 0 becomes green while the LCD is off.
        let mut packet = [0u8; 16];
        packet[0] = 0x01;
        packet[7] = 0xe0;
        packet[8] = 0x03;
        send_console_packet(&mut console, packet);

        for _ in 0..2_000_000 {
            console.step();
            if console.ppu().control().video_enabled() {
                break;
            }
        }
        assert!(console.ppu().control().video_enabled());

        let mut delivered = Vec::new();
        for _ in 0..2_000_000 {
            if console.step().new_screen {
                delivered.push(delivered_picture(&console));
                if delivered.len() == 2 {
                    break;
                }
            }
        }
        assert_eq!(delivered.len(), 2);

        // The first frame after LCD-on never presents, so the SNES side still
        // shows its capture — recoloured by the packet that landed while off.
        let (screen, render_data) = &delivered[0];
        assert_eq!(screen.pixel(0, 0).0, 3);
        assert_eq!(render_data.color_at(0, 0, 3).0, 0x03E0);

        // The second frame presents: the capture becomes the redrawn picture.
        assert_eq!(delivered[1].0.pixel(0, 0).0, 1);
    }
}
