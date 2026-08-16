//! Diffing a rendered surface against its reference.

use std::fmt::Debug;

/// Format a shade byte the way the greyscale references state it.
pub fn hex_byte(value: &u8) -> String {
    format!("0x{value:02X}")
}

/// Format a pixel by its `Debug` form — the RGB triples and colour indices.
pub fn debug_value<T: Debug>(value: &T) -> String {
    format!("{value:?}")
}

/// Compare a rendered surface against its reference, reporting the first `cap`
/// mismatches with their coordinates and asserting there are none.
///
/// The walk covers the overlap of the two slices, so a reference carrying
/// trailing rows the surface never reached contributes only the rows it did.
/// `subject` leads every line; `describe` states a pixel in the surface's own
/// terms.
pub fn assert_pixels_match<T: PartialEq>(
    subject: &str,
    actual: &[T],
    expected: &[T],
    width: usize,
    cap: usize,
    describe: impl Fn(&T) -> String,
) {
    let mut mismatches = 0usize;
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        if a != e {
            if mismatches < cap {
                let (x, y) = (i % width, i / width);
                eprintln!(
                    "{subject}: pixel ({x},{y}) got {}, expected {}",
                    describe(a),
                    describe(e)
                );
            }
            mismatches += 1;
        }
    }

    assert_eq!(mismatches, 0, "{subject}: {mismatches} pixel mismatches");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identical_surface_matches() {
        let pixels = [0u8, 1, 2, 3];
        assert_pixels_match("identical", &pixels, &pixels, 2, 10, hex_byte);
    }

    /// The walk stops at the shorter slice, so a taller reference does not
    /// itself count as a mismatch.
    #[test]
    fn a_taller_reference_compares_over_the_overlap() {
        assert_pixels_match("overlap", &[0u8, 1], &[0u8, 1, 9, 9], 2, 10, hex_byte);
    }

    #[test]
    #[should_panic(expected = "differing: 1 pixel mismatches")]
    fn a_differing_pixel_fails() {
        assert_pixels_match("differing", &[0u8, 1], &[0u8, 2], 2, 10, hex_byte);
    }

    #[test]
    #[should_panic(expected = "capped: 3 pixel mismatches")]
    fn the_cap_bounds_the_report_not_the_count() {
        assert_pixels_match("capped", &[0u8, 0, 0], &[1u8, 1, 1], 3, 1, hex_byte);
    }

    #[test]
    #[should_panic(expected = "triples: 1 pixel mismatches")]
    fn triples_compare_by_their_debug_form() {
        let actual = [[0u8, 0, 0], [1, 2, 3]];
        let expected = [[0u8, 0, 0], [1, 2, 4]];
        assert_pixels_match("triples", &actual, &expected, 2, 10, debug_value);
    }
}
