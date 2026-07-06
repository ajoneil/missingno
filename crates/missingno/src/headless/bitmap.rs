use missingno_gb::ppu;
use missingno_gb::ppu::memory::Vram;
use missingno_gb::{Console, Model};
use tiny_http::Response;

pub(super) fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

pub(super) fn respond_bmp(request: tiny_http::Request, bmp: Vec<u8>) {
    let response = Response::from_data(bmp).with_header(
        "Content-Type: image/bmp"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    let _ = request.respond(response);
}

pub(super) fn write_bmp(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let row_stride = ((width * 3 + 3) & !3) as usize;
    let pixel_data_size = row_stride * height as usize;
    let file_size = 54 + pixel_data_size;

    let mut bmp = Vec::with_capacity(file_size);

    // BMP file header (14 bytes)
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());

    // DIB header (40 bytes)
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    bmp.extend_from_slice(&(height as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(pixel_data_size as u32).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());

    // Pixel data (bottom-up; BMP rows are BGR-ordered)
    let padding = row_stride - width as usize * 3;
    for y in (0..height).rev() {
        let row_start = (y * width) as usize * 3;
        for rgb in pixels[row_start..row_start + width as usize * 3].chunks_exact(3) {
            bmp.extend_from_slice(&[rgb[2], rgb[1], rgb[0]]);
        }
        bmp.extend(std::iter::repeat_n(0u8, padding));
    }

    bmp
}

/// Renders all 384 tiles (3 blocks of 128) in a 16-wide grid.
pub(super) fn tiles_bitmap<M: Model>(gb: &Console<M>, bank: u8) -> Vec<u8> {
    let vram = gb.vram().bank(bank);
    let greys: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

    // 16 tiles wide, 24 tiles tall (384 tiles total)
    let cols = 16u32;
    let rows = 24u32;
    let w = cols * 8;
    let h = rows * 8;

    let mut pixels = vec![0u8; (w * h * 3) as usize];

    for block_id in 0..3u8 {
        let block = vram.tile_block(ppu::types::tiles::TileBlockId(block_id));
        for tile_idx in 0..128u8 {
            let tile = block.tile(ppu::types::tiles::TileIndex(tile_idx));
            let global_idx = block_id as u32 * 128 + tile_idx as u32;
            let grid_x = global_idx % cols;
            let grid_y = global_idx / cols;
            for ty in 0..8u8 {
                for tx in 0..8u8 {
                    let shade = greys[tile.pixel(tx, ty).0 as usize];
                    let px = (grid_x * 8 + tx as u32) as usize;
                    let py = (grid_y * 8 + ty as u32) as usize;
                    let offset = (py * w as usize + px) * 3;
                    pixels[offset] = shade;
                    pixels[offset + 1] = shade;
                    pixels[offset + 2] = shade;
                }
            }
        }
    }

    write_bmp(w, h, &pixels)
}
