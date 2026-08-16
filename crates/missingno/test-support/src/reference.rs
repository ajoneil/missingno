//! Reference images, decoded once.

use std::path::Path;

/// A decoded reference image. The decode expands palettes and sub-byte depths,
/// so every channel is a byte; which channels a suite reads, and what it
/// requires of the image's shape, stay the caller's choice.
pub struct ReferencePng {
    width: usize,
    height: usize,
    colour_type: png::ColorType,
    stride: usize,
    bytes: Vec<u8>,
}

impl ReferencePng {
    pub fn load(path: &Path) -> Self {
        let file = std::fs::File::open(path)
            .unwrap_or_else(|e| panic!("failed to open reference image {}: {e}", path.display()));
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut bytes = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();

        let stride = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            other => panic!("unsupported reference PNG colour type: {other:?}"),
        };

        Self {
            width: info.width as usize,
            height: info.height as usize,
            colour_type: info.color_type,
            stride,
            bytes,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn colour_type(&self) -> png::ColorType {
        self.colour_type
    }

    /// Panic unless the image carries colour channels — for suites whose
    /// references are colour by construction, where a greyscale file is a
    /// mis-exported reference rather than a shade-only one.
    pub fn require_colour(&self) {
        if self.colour_type == png::ColorType::Grayscale {
            panic!(
                "unsupported reference PNG colour type: {:?}",
                self.colour_type
            );
        }
    }

    /// One byte per pixel: the first channel, which on a greyscale or neutral
    /// reference is the shade.
    pub fn greyscale(&self) -> Vec<u8> {
        (0..self.width * self.height)
            .map(|i| self.bytes[i * self.stride])
            .collect()
    }

    /// Three bytes per pixel; a greyscale reference expands to neutral triples.
    pub fn rgb(&self) -> Vec<[u8; 3]> {
        (0..self.width * self.height)
            .map(|i| self.pixel(i))
            .collect()
    }

    /// The same triples flattened, for surfaces stated as loose RGB888 bytes.
    pub fn rgb_bytes(&self) -> Vec<u8> {
        (0..self.width * self.height)
            .flat_map(|i| self.pixel(i))
            .collect()
    }

    /// One pixel by coordinate, for callers that walk a sub-rectangle.
    pub fn rgb_at(&self, x: usize, y: usize) -> [u8; 3] {
        self.pixel(y * self.width + x)
    }

    fn pixel(&self, index: usize) -> [u8; 3] {
        let p = index * self.stride;
        match self.colour_type {
            png::ColorType::Grayscale => [self.bytes[p]; 3],
            _ => [self.bytes[p], self.bytes[p + 1], self.bytes[p + 2]],
        }
    }
}
