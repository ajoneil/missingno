use super::video::screen::{self, Screen};
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
#[derive(Copy, Clone, Debug)]
pub struct Rgb555(pub u16);

impl Rgb555 {
    pub fn to_rgb8(self) -> RGB8 {
        // 5-bit to 8-bit: multiply by 8 and OR with top bits for proper rounding
        let r5 = (self.0 & 0x1f) as u8;
        let g5 = ((self.0 >> 5) & 0x1f) as u8;
        let b5 = ((self.0 >> 10) & 0x1f) as u8;
        RGB8::new(
            (r5 << 3) | (r5 >> 2),
            (g5 << 3) | (g5 >> 2),
            (b5 << 3) | (b5 >> 2),
        )
    }

    pub fn from_bytes(low: u8, high: u8) -> Self {
        Self(u16::from_le_bytes([low, high]))
    }
}

impl Default for Rgb555 {
    fn default() -> Self {
        Self(0)
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

impl AttributeMap {
    pub fn new() -> Self {
        Self {
            cells: [[0; 20]; 18],
        }
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
    pub video_enabled: bool,
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
    pub player_count: u8,
    pub current_player: u8,
    command_state: CommandState,
    // Track previous write for player cycling
    prev_p14_p15_both_low: bool,
    // Snapshot of the last rendered screen, used by TRN commands
    last_screen: Screen,
    // Deferred VRAM transfer: countdown frames + transfer type
    pending_transfer: Option<(u8, PendingTransfer)>,
}

impl Sgb {
    pub fn new() -> Self {
        Self {
            palettes: [SgbPalette::default(); 4],
            attribute_map: AttributeMap::new(),
            system_palettes: vec![SgbPalette::default(); 512],
            attribute_files: vec![AttributeMap::new(); 45],
            mask_mode: MaskMode::Disabled,
            player_count: 1,
            current_player: 0,
            command_state: CommandState::Idle,
            prev_p14_p15_both_low: false,
            last_screen: Screen::new(),
            pending_transfer: None,
        }
    }

    /// Update the stored screen snapshot (called each frame from execute loop).
    pub fn update_screen(&mut self, screen: &Screen) {
        self.last_screen = *screen;
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

    pub fn render_data(&self, video_enabled: bool) -> SgbRenderData {
        SgbRenderData {
            palettes: self.palettes,
            attribute_map: self.attribute_map,
            mask_mode: self.mask_mode,
            video_enabled,
        }
    }

    /// Called on every write to FF00.
    pub fn write_joypad(&mut self, value: u8) {
        let p14_low = value & 0x10 == 0;
        let p15_low = value & 0x20 == 0;
        let both_low = p14_low && p15_low;

        // Player cycling for MLT_REQ: cycle on every reset pulse (both-low falling edge)
        if both_low && !self.prev_p14_p15_both_low && self.player_count > 1 {
            self.current_player = (self.current_player + 1) % self.player_count;
        }
        self.prev_p14_p15_both_low = both_low;

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
                        // Between packets in a multi-packet command, or mid-packet restart
                        *current_packet = [0; 16];
                        *bit_index = 0;
                    }
                    return;
                }

                if !p14_low && !p15_low {
                    // Both high — release between bits, not a data bit
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
                        // More packets to come — wait for next reset + data bit
                        *current_packet = [0; 16];
                        *bit_index = 0;
                    }
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
            let idx = u16::from_le_bytes([data[1 + i * 2], data[2 + i * 2]]) as usize;
            if idx < self.system_palettes.len() {
                self.palettes[i] = self.system_palettes[idx];
            }
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

        // Data starts at byte 6, packed 4 attributes per byte (2 bits each, LSB first)
        let mut written = 0;
        let mut byte_offset = 6;
        let mut bit_offset = 0;

        while written < count {
            if byte_offset >= data.len() {
                break;
            }
            let pal = (data[byte_offset] >> bit_offset) & 0x03;
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
            let base = file_idx * 90;
            let mut atf = AttributeMap::new();
            for y in 0..18 {
                // 5 bytes per row (20 cells * 2 bits = 40 bits = 5 bytes)
                for byte_in_row in 0..5 {
                    let offset = base + y * 5 + byte_in_row;
                    if offset >= data.len() {
                        break;
                    }
                    let byte_val = data[offset];
                    for bit_pair in 0..4 {
                        let x = byte_in_row * 4 + bit_pair;
                        if x < 20 {
                            atf.cells[y][x] = (byte_val >> (bit_pair * 2)) & 0x03;
                        }
                    }
                }
            }
            self.attribute_files[file_idx] = atf;
        }
    }

    fn cmd_attr_set(&mut self, data: &[u8]) {
        let atf_idx = (data[1] & 0x3F) as usize;
        if atf_idx < self.attribute_files.len() {
            self.attribute_map = self.attribute_files[atf_idx].clone();
        }
        if data[1] & 0x40 != 0 {
            self.mask_mode = MaskMode::Disabled;
        }
    }

    // --- System commands ---

    fn cmd_mlt_req(&mut self, data: &[u8]) {
        self.player_count = match data[1] & 0x03 {
            0 => 1,
            1 => 2,
            3 => 4,
            _ => 1,
        };
        // After MLT_REQ, current_player starts at count-1 so the first read
        // returns a non-zero player ID, which games use to detect SGB presence.
        if self.player_count > 1 {
            self.current_player = self.player_count - 1;
        } else {
            self.current_player = 0;
        }
    }

    fn cmd_mask_en(&mut self, data: &[u8]) {
        self.mask_mode = match data[1] & 0x03 {
            0 => MaskMode::Disabled,
            1 => MaskMode::Freeze,
            2 => MaskMode::Black,
            3 => MaskMode::BackdropColor,
            _ => unreachable!(),
        };
    }
}
