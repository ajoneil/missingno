use crate::screen::Color555;

/// The CGB boot ROM's default DMG-compatibility palette for a cartridge whose
/// title does not match the boot ROM table (palette combination 0): BG palette
/// 29, and OBJ palettes 0 and 1 both = palette 4. Little-endian RGB555.
pub const DMG_COMPAT_BG: [u16; 4] = [0x7FFF, 0x1BEF, 0x6180, 0x0000];
pub const DMG_COMPAT_OBJ: [u16; 4] = [0x7FFF, 0x421F, 0x1CF2, 0x0000];

/// Reverse-map a DMG-compatibility framebuffer colour to its DMG shade index
/// (0-3), for shade-pattern screenshot comparison. The compat palette is a
/// bijection over the four shades (white→0, BG green / OBJ pink →1, BG blue /
/// OBJ red →2, black→3), so the shade pattern is recoverable independent of the
/// tint. `None` for any off-palette colour.
pub fn dmg_compat_shade(color: Color555) -> Option<u8> {
    DMG_COMPAT_BG
        .iter()
        .chain(DMG_COMPAT_OBJ.iter())
        .position(|&c| Color555(c & 0x7FFF) == color)
        .map(|i| (i % 4) as u8)
}

/// The CGB boot ROM's DMG-compatibility palette selection: a Nintendo-licensee
/// gate, then the title checksum (with a 4th-letter tiebreak for collisions)
/// picks a palette combination. Returns the `(BG, OBJ0, OBJ1)` RGB555 palettes to
/// install in CRAM. A non-Nintendo or unmatched title falls to combination 0 —
/// the well-known green/blue-BG, pink/red-OBJ compatibility palette.
pub(crate) fn dmg_compat_palettes(
    title: &[u8; 16],
    old_licensee: u8,
    new_licensee: [u8; 2],
) -> ([u16; 4], [u16; 4], [u16; 4]) {
    use crate::dmg_palette_data as data;

    let is_nintendo =
        old_licensee == 0x01 || (old_licensee == 0x33 && new_licensee == [b'0', b'1']);

    let mut combo = 0u8;
    if is_nintendo {
        let checksum = title.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        for i in 0..data::TITLE_CHECKSUMS.len() {
            // A collision-region match also has to agree on the 4th title letter,
            // otherwise the search continues.
            if data::TITLE_CHECKSUMS[i] == checksum
                && (i < data::FIRST_DUP_INDEX
                    || data::DUPS_4TH_LETTER[i - data::FIRST_DUP_INDEX] == title[3])
            {
                combo = data::PALETTE_PER_CHECKSUM[i] & 0x7F;
                break;
            }
        }
    }

    let [obj0, obj1, bg] = data::PALETTE_COMBINATIONS[combo as usize];
    (
        data::PALETTES[bg as usize],
        data::PALETTES[obj0 as usize],
        data::PALETTES[obj1 as usize],
    )
}

#[cfg(test)]
mod dmg_palette_tests {
    use super::*;
    use crate::dmg_palette_data;

    fn title(s: &str) -> [u8; 16] {
        let mut t = [0u8; 16];
        for (i, b) in s.bytes().take(16).enumerate() {
            t[i] = b;
        }
        t
    }

    #[test]
    fn non_nintendo_falls_to_compat_default() {
        // Any non-Nintendo licensee gates to combination 0, regardless of title.
        let (bg, obj0, obj1) = dmg_compat_palettes(&title("TETRIS"), 0x00, [0, 0]);
        assert_eq!(bg, DMG_COMPAT_BG);
        assert_eq!(obj0, DMG_COMPAT_OBJ);
        assert_eq!(obj1, DMG_COMPAT_OBJ);
    }

    #[test]
    fn nintendo_title_selects_its_palette() {
        // TETRIS (old licensee $01, checksum $DB) selects combination 3 = palette 24.
        let (bg, _, _) = dmg_compat_palettes(&title("TETRIS"), 0x01, [0, 0]);
        assert_eq!(bg, dmg_palette_data::PALETTES[24]);
        assert_ne!(bg, DMG_COMPAT_BG);
    }

    #[test]
    fn fourth_letter_disambiguates_checksum_collision() {
        // Two titles with the same checksum ($46) but different 4th letters resolve
        // to different table entries (66 = 'E', 80 = 'R') via the tiebreak search.
        let mut e = [0u8; 16];
        e[0] = 0x01;
        e[3] = b'E';
        let mut r = [0u8; 16];
        r[0] = 0xF4;
        r[3] = b'R';
        assert_eq!(e.iter().fold(0u8, |s, &x| s.wrapping_add(x)), 0x46);
        assert_eq!(r.iter().fold(0u8, |s, &x| s.wrapping_add(x)), 0x46);

        let bg_of = |combo: u8| {
            dmg_palette_data::PALETTES
                [dmg_palette_data::PALETTE_COMBINATIONS[combo as usize][2] as usize]
        };
        let combo_e = dmg_palette_data::PALETTE_PER_CHECKSUM[66] & 0x7F;
        let combo_r = dmg_palette_data::PALETTE_PER_CHECKSUM[80] & 0x7F;
        assert_eq!(dmg_compat_palettes(&e, 0x01, [0, 0]).0, bg_of(combo_e));
        assert_eq!(dmg_compat_palettes(&r, 0x01, [0, 0]).0, bg_of(combo_r));
    }
}
