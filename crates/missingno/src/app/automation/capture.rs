//! Pure helpers behind the `screenshot` tool: mapping a logical crop rect onto a
//! physical capture, PNG encoding, and base64. Kept apart from the state machine
//! in [`super::update`] so the geometry can be unit-tested on its own.

/// Map a logical-pixel crop rect onto a capture of `capture` physical pixels:
/// scale each edge, round to the nearest pixel, and clamp to the capture bounds.
/// `None` when the clamped rect has no area, so the caller reports an empty crop
/// rather than asking iced to crop nothing.
pub fn physical_crop(
    logical: (f32, f32, f32, f32),
    scale: f32,
    capture: (u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let (lx, ly, lw, lh) = logical;
    let (cap_w, cap_h) = capture;
    let scale = if scale > 0.0 { scale } else { 1.0 };

    let x = (lx * scale).round().clamp(0.0, cap_w as f32) as u32;
    let y = (ly * scale).round().clamp(0.0, cap_h as f32) as u32;
    let w = ((lw * scale).round().max(0.0) as u32).min(cap_w - x);
    let h = ((lh * scale).round().max(0.0) as u32).min(cap_h - y);

    (w != 0 && h != 0).then_some((x, y, w, h))
}

/// Encode RGBA8 pixels as a PNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let mut buffer = std::io::Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new(&mut buffer)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("png encode: {error}"))?;
    Ok(buffer.into_inner())
}

/// Standard base64, for embedding a PNG in a tool result.
pub fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(triple >> 12 & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_at_unit_scale_is_the_logical_rect() {
        assert_eq!(
            physical_crop((10.0, 20.0, 30.0, 40.0), 1.0, (200, 200)),
            Some((10, 20, 30, 40))
        );
    }

    #[test]
    fn crop_scales_every_edge() {
        assert_eq!(
            physical_crop((10.0, 20.0, 30.0, 40.0), 2.0, (200, 200)),
            Some((20, 40, 60, 80))
        );
    }

    #[test]
    fn crop_rounds_to_the_nearest_pixel() {
        // 10.4*1.5=15.6→16, 5.2*1.5=7.8→8, 3.1*1.5=4.65→5
        assert_eq!(
            physical_crop((10.4, 5.2, 3.1, 3.1), 1.5, (200, 200)),
            Some((16, 8, 5, 5))
        );
    }

    #[test]
    fn crop_clamps_width_to_the_capture() {
        // 150*1=150 wide from x=100 in a 200-wide capture leaves only 100.
        assert_eq!(
            physical_crop((100.0, 0.0, 150.0, 50.0), 1.0, (200, 200)),
            Some((100, 0, 100, 50))
        );
    }

    #[test]
    fn crop_clamps_negative_origin_to_zero() {
        assert_eq!(
            physical_crop((-5.0, -5.0, 20.0, 20.0), 1.0, (200, 200)),
            Some((0, 0, 20, 20))
        );
    }

    #[test]
    fn crop_off_the_capture_has_no_area() {
        assert_eq!(
            physical_crop((300.0, 0.0, 20.0, 20.0), 1.0, (200, 200)),
            None
        );
    }

    #[test]
    fn crop_of_zero_size_is_none() {
        assert_eq!(
            physical_crop((10.0, 10.0, 0.0, 40.0), 1.0, (200, 200)),
            None
        );
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn png_round_trips_dimensions() {
        let pixels = vec![0u8; 4 * 4 * 4];
        let png = encode_png(4, 4, &pixels).expect("encode");
        let decoded = image::load_from_memory(&png).expect("decode");
        assert_eq!((decoded.width(), decoded.height()), (4, 4));
    }
}
