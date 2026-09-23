//! The cartridge edge: four chip selects, one per 8 KB window from $8000, and
//! A0-A14. A flat image answers the windows it holds from its own bytes.

/// What a read nothing drives returns — an empty window, an expansion select
/// with no module fitted, an I/O range with no read strobe.
pub const UNDRIVEN: u8 = 0xFF;

/// One 8 KB window behind each select.
pub const WINDOW_SIZE: usize = 0x2000;
/// `EN_80`, `EN_A0`, `EN_C0`, `EN_E0`.
pub const WINDOWS: usize = 4;
/// The most a flat cartridge maps: all four windows.
pub const MAX_FLAT_SIZE: usize = WINDOW_SIZE * WINDOWS;

/// Which of the four cartridge selects a window read drove.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CartWindow(pub u8);

/// Why an image did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CartridgeError {
    /// Larger than the four windows: a bank-switched board, which this core
    /// does not model.
    Banked { size: usize },
}

impl std::fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CartridgeError::Banked { size } => write!(
                f,
                "a {size}-byte image is larger than the 32 KB the cartridge selects map; \
                 bank-switched boards are not modelled"
            ),
        }
    }
}

impl std::error::Error for CartridgeError {}

pub struct Cartridge {
    rom: Vec<u8>,
}

impl Cartridge {
    pub fn load(rom: &[u8]) -> Result<Cartridge, CartridgeError> {
        if rom.len() > MAX_FLAT_SIZE {
            return Err(CartridgeError::Banked { size: rom.len() });
        }
        Ok(Cartridge { rom: rom.to_vec() })
    }

    /// The byte a select drives, or `None` where the image holds none.
    pub fn read(&self, window: CartWindow, offset: u16) -> Option<u8> {
        let index = window.0 as usize * WINDOW_SIZE + offset as usize;
        self.rom.get(index).copied()
    }
}

/// The header's two magic orders: a game the BIOS introduces with its title
/// screen, and a test cartridge it jumps straight into.
const GAME_MAGIC: [u8; 2] = [0xAA, 0x55];
const TEST_MAGIC: [u8; 2] = [0x55, 0xAA];
/// `GAME_NAME`: "LINE/LINE/YEAR", up to 60 bytes.
const GAME_NAME: usize = 0x24;
const GAME_NAME_MAX: usize = 60;

/// The title the header's `GAME_NAME` string carries — its first two lines
/// joined by a space — for an image the BIOS recognises as a cartridge.
pub fn title_from_rom(rom: &[u8]) -> Option<String> {
    let magic = rom.get(..2)?;
    if magic != GAME_MAGIC && magic != TEST_MAGIC {
        return None;
    }
    let field = rom.get(GAME_NAME..)?;
    let field = &field[..field.len().min(GAME_NAME_MAX)];
    let end = field
        .iter()
        .position(|&byte| byte == 0 || !(0x20..0x7F).contains(&byte))
        .unwrap_or(field.len());
    let text = std::str::from_utf8(&field[..end]).ok()?;
    let title = text
        .split('/')
        .take(2)
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned();
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_name(magic: [u8; 2], name: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x100];
        rom[..2].copy_from_slice(&magic);
        rom[GAME_NAME..GAME_NAME + name.len()].copy_from_slice(name);
        rom
    }

    #[test]
    fn an_image_answers_only_the_windows_it_holds() {
        let cart = Cartridge::load(&[0x11; 0x4000]).expect("a flat image");
        assert_eq!(cart.read(CartWindow(0), 0), Some(0x11));
        assert_eq!(cart.read(CartWindow(1), 0x1FFF), Some(0x11));
        assert_eq!(cart.read(CartWindow(2), 0), None);
        assert_eq!(cart.read(CartWindow(3), 0x1FFF), None);
    }

    #[test]
    fn an_image_past_the_four_windows_is_refused() {
        assert_eq!(
            Cartridge::load(&[0; MAX_FLAT_SIZE + 1]).err(),
            Some(CartridgeError::Banked {
                size: MAX_FLAT_SIZE + 1
            })
        );
        assert!(Cartridge::load(&[0; MAX_FLAT_SIZE]).is_ok());
    }

    /// The field runs straight into code where no NUL ends it.
    #[test]
    fn the_name_stops_at_the_first_unprintable_byte() {
        let rom = with_name(GAME_MAGIC, b"A/B/1982\xC3\x00\x80");
        assert_eq!(title_from_rom(&rom).as_deref(), Some("A B"));
    }

    #[test]
    fn the_title_is_the_first_two_lines_of_the_game_name() {
        let rom = with_name(GAME_MAGIC, b"PRESENTS COLECO'S/ DONKEY KONG /1982\0");
        assert_eq!(
            title_from_rom(&rom).as_deref(),
            Some("PRESENTS COLECO'S DONKEY KONG")
        );
        let rom = with_name(TEST_MAGIC, b"TEST/CART/1983");
        assert_eq!(title_from_rom(&rom).as_deref(), Some("TEST CART"));
    }

    #[test]
    fn no_title_without_the_magic_or_before_the_first_printable_byte() {
        let mut rom = with_name(GAME_MAGIC, b"A/B/1982\0");
        rom[0] = 0x00;
        assert_eq!(title_from_rom(&rom), None);
        assert_eq!(
            title_from_rom(&with_name(GAME_MAGIC, b"\x01A/B/1982")),
            None
        );
        assert_eq!(title_from_rom(&with_name(GAME_MAGIC, b"\0")), None);
    }
}
