//! The board a ROM is wired for, and what a bare dump reveals about it.
//!
//! A dump is the silicon's contents and nothing else, so its length is the only
//! signal it carries — enough to name a board across the common Atari sizes.
//! The one exception is the Superchip, whose RAM shadows the bottom of every
//! bank and leaves a readable mark in the image.

use missingno_core::cartridge::{BoardNames, row};

use super::{atari, dpc, supercharger, tigervision_ram, tigervision_ram_plus};

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    UnsupportedSize(usize),
    /// The image is the wrong size for the board it was declared as.
    WrongSizeForBoard {
        cart_type: CartType,
        size: usize,
    },
    /// A Supercharger load unit whose checksums don't settle where the
    /// container says they must.
    LoadChecksum {
        unit: usize,
        page: Option<usize>,
    },
    /// A board the catalogue names but the slot does not model.
    BoardNotBuilt(CartType),
}

impl std::fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CartridgeError::UnsupportedSize(size) => write!(f, "unsupported image size {size}"),
            CartridgeError::WrongSizeForBoard { cart_type, size } => match cart_type.image_size() {
                Some(holds) => write!(
                    f,
                    "image is {size} bytes but a {} board holds {holds}",
                    cart_type.code()
                ),
                None => write!(f, "image is {size} bytes, no {} image", cart_type.code()),
            },
            CartridgeError::LoadChecksum { unit, page: None } => {
                write!(f, "Supercharger load {unit}: header checksum error")
            }
            CartridgeError::LoadChecksum {
                unit,
                page: Some(page),
            } => write!(f, "Supercharger load {unit}: page {page} checksum error"),
            CartridgeError::BoardNotBuilt(cart_type) => {
                write!(f, "the {} board is not modelled yet", cart_type.code())
            }
        }
    }
}

impl std::error::Error for CartridgeError {}

/// How a dump's length relates to the board's silicon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DumpFit {
    /// The image is the silicon's contents and nothing else.
    Exact,
    /// A catalogued overdump: the image carries padding past the silicon, so
    /// the stated board's size says where the cartridge ends.
    Overdump,
}

/// The board a ROM is wired for. The Atari codes (F8/F6/F4) name the hotspot
/// ranges that page the 4 KB window; the Superchip variants add Superchip
/// (SARA) cart RAM, which a raw dump can't be told from a plain board by size
/// alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CartType {
    /// 2 KB, mirrored into the window.
    Plain2K,
    /// 4 KB, fills the window.
    Plain4K,
    /// 8 KB across two banks.
    Atari8K,
    Atari8KSuperchip,
    /// 16 KB across four banks.
    Atari16K,
    Atari16KSuperchip,
    /// 32 KB across eight banks.
    Atari32K,
    Atari32KSuperchip,
    /// 12 KB across three banks, with 256 bytes of cart RAM and a
    /// data-bus-gated switch (CBS RAM Plus).
    CbsRamPlus,
    /// 8 KB as eight 1 KB slices in three pageable windows (Parker Bros).
    ParkerBros,
    /// 16 KB as eight 2 KB banks, with 1 KB and 4×256 B cart RAMs (M-Network).
    MNetwork,
    /// 2 KB of ROM over 1 KB of read-low cart RAM (CommaVid).
    Commavid,
    /// 8 KB across two banks, selected from loosely decoded hotspots below the
    /// window (UA Ltd).
    UaLtd,
    /// A 2 KB fixed half over a bus-latched paged half (Tigervision).
    Tigervision,
    /// 8 KB across two banks, picked by an address comparator on $01FE and
    /// data line D5 (Activision).
    Activision,
    /// An F8-banked 8 KB program ROM beside the Display Processor Chip and its
    /// own 2 KB display ROM (Pitfall II).
    Dpc,
    /// 6 KB of tape-loaded RAM and a BIOS, driven entirely by reads (Starpath
    /// Supercharger). The image is a fastload container, not a ROM.
    Supercharger,
    /// 64 KB across sixteen banks, stepped one at a time (Dynacom Megaboy).
    Megaboy,
    /// 16 KB across four banks on scattered hotspots (Tarzan prototype).
    Jane,
    /// 8 KB across two banks, chosen by the written data (Coleco white label).
    ColecoWf8,
    /// 8 KB as eight 1 KB banks filling four segments at once, on a delayed
    /// switch (Wickstead Design prototype).
    WicksteadDesign,
    /// 32 KB across eight banks, latched in two halves and committed
    /// separately (Amiga Power Play Arcade).
    AmigaPowerPlay,
    /// 8 KB across two banks, selected from loosely decoded low-memory
    /// hotspots (Fotomania).
    Fotomania,
    /// 8 KB as eight 1 KB slices in three segments, selected active-low from
    /// low memory (Brazilian Parker Bros).
    ParkerBrosBrazil,
    /// 3F with a cart-RAM path on its own hotspot (homebrew).
    TigervisionRam,
    /// Four independently banked 1 KB segments, each ROM or RAM (homebrew).
    TigervisionRamPlus,
    /// 64 KB across sixteen banks, on the hotspot family's wider run (homebrew).
    Atari64K,
    /// 128 KB across thirty-two banks (homebrew).
    Atari128K,
    /// 256 KB across sixty-four banks (homebrew).
    Atari256K,
    /// 128 KB across thirty-two banks selected from low memory, waking in the
    /// last of them (SuperBanking).
    Superbanking,
    /// 8 KB across two banks, selected by one address line (EconoBanking).
    Econobanking,
    /// 64 KB across sixteen banks, with a second switch riding on TIA writes
    /// (Stocking).
    X07,
    /// 32 KB across eight banks named by an address's low byte, with a one-way
    /// lock (Menu Driven Megacart).
    MenuDrivenMegacart,
    /// DPC's ARM-hosted successor on the Harmony/Melody boards. Catalogued,
    /// not modelled.
    DpcPlus,
    /// The CVC GameLine Master Module modem cartridge. Catalogued, not
    /// modelled.
    GameLine,
    /// CBS RAM Plus lineage with a larger ROM and hotspot page. Catalogued,
    /// not modelled.
    Fa2,
    /// 64 KB + RAM behind the $4A50-decoded switch it is named for.
    /// Catalogued, not modelled.
    FourA50,
}

/// The whole board vocabulary, one row per board — the code a board goes by in
/// interchange (game-db entries, the CLI, a test's board override) and the name
/// shown to a reader. Every name a board answers to derives from here.
const BOARD_NAMES: &[BoardNames<CartType>] = &[
    row(CartType::Plain2K, "2K", "Plain 2K"),
    row(CartType::Plain4K, "4K", "Plain 4K"),
    row(CartType::Atari8K, "F8", "Atari 8K (F8)"),
    row(
        CartType::Atari8KSuperchip,
        "F8SC",
        "Atari 8K + Superchip (F8SC)",
    ),
    row(CartType::Atari16K, "F6", "Atari 16K (F6)"),
    row(
        CartType::Atari16KSuperchip,
        "F6SC",
        "Atari 16K + Superchip (F6SC)",
    ),
    row(CartType::Atari32K, "F4", "Atari 32K (F4)"),
    row(
        CartType::Atari32KSuperchip,
        "F4SC",
        "Atari 32K + Superchip (F4SC)",
    ),
    row(CartType::CbsRamPlus, "FA", "CBS RAM Plus (FA)"),
    row(CartType::ParkerBros, "E0", "Parker Bros (E0)"),
    row(CartType::MNetwork, "E7", "M-Network (E7)"),
    row(CartType::Commavid, "CV", "CommaVid (CV)"),
    row(CartType::UaLtd, "UA", "UA Ltd (UA)"),
    row(CartType::Tigervision, "3F", "Tigervision (3F)"),
    row(CartType::Activision, "FE", "Activision (FE)"),
    row(CartType::Dpc, "DPC", "DPC — Pitfall II (DPC)"),
    row(CartType::Supercharger, "AR", "Starpath Supercharger (AR)"),
    row(CartType::Megaboy, "F0", "Dynacom Megaboy (F0)"),
    row(CartType::Jane, "JANE", "Tarzan prototype (JANE)"),
    row(CartType::ColecoWf8, "WF8", "Coleco (WF8)"),
    row(CartType::WicksteadDesign, "WD", "Wickstead Design (WD)"),
    row(CartType::AmigaPowerPlay, "FC", "Amiga Power Play (FC)"),
    row(CartType::Fotomania, "0FA0", "Fotomania (0FA0)"),
    row(
        CartType::ParkerBrosBrazil,
        "03E0",
        "Parker Bros Brazil (03E0)",
    ),
    row(CartType::TigervisionRam, "3E", "Tigervision + RAM (3E)"),
    row(
        CartType::TigervisionRamPlus,
        "3E+",
        "Tigervision + RAM (3E+)",
    ),
    row(CartType::Atari64K, "EF", "64K Atari-style (EF)"),
    row(CartType::Atari128K, "DF", "128K Atari-style (DF)"),
    row(CartType::Atari256K, "BF", "256K Atari-style (BF)"),
    row(CartType::Superbanking, "SB", "SuperBanking (SB)"),
    row(CartType::Econobanking, "0840", "EconoBanking (0840)"),
    row(CartType::X07, "X07", "X07"),
    row(
        CartType::MenuDrivenMegacart,
        "MDM",
        "Menu Driven Megacart (MDM)",
    ),
    row(CartType::DpcPlus, "DPC+", "DPC+ (Harmony)"),
    row(CartType::GameLine, "GL", "CVC GameLine Master Module (GL)"),
    row(CartType::Fa2, "FA2", "FA2 (CBS RAM Plus lineage)"),
    row(CartType::FourA50, "4A50", "4A50"),
];

missingno_core::board_vocabulary!(CartType, BOARD_NAMES, "unknown Atari VCS board code");

impl CartType {
    /// The board a bare dump is best-effort read as, from its length alone.
    pub(super) fn infer(rom: &[u8]) -> Result<CartType, CartridgeError> {
        Ok(match rom.len() {
            0x800 => CartType::Plain2K,
            0x1000 => CartType::Plain4K,
            0x2000 if has_superchip_signature(rom) => CartType::Atari8KSuperchip,
            0x2000 => CartType::Atari8K,
            0x4000 if has_superchip_signature(rom) => CartType::Atari16KSuperchip,
            0x4000 => CartType::Atari16K,
            0x8000 if has_superchip_signature(rom) => CartType::Atari32KSuperchip,
            0x8000 => CartType::Atari32K,
            // A Supercharger container is whole 8448-byte load units — a size
            // no ROM board shares.
            size if supercharger::is_container(size) => CartType::Supercharger,
            size => return Err(CartridgeError::UnsupportedSize(size)),
        })
    }

    /// Whether an image is sized for the board. Most boards are exact — a dump
    /// is the silicon's contents and nothing else.
    pub(super) fn accepts(self, len: usize) -> bool {
        match self {
            // A DPC dump ends with however much of the RNG stream the dumper
            // happened to catch, so the tail's length varies; the common dump
            // is one byte short of a full page of it.
            CartType::Dpc => self
                .image_size()
                .is_some_and(|with_tail| (dpc::IMAGE_SIZE..=with_tail).contains(&len)),
            // A Supercharger image is a tape container: one load unit per tape
            // load, and a multi-load title carries several.
            CartType::Supercharger => supercharger::is_container(len),
            CartType::TigervisionRam => tigervision_ram::holds(len),
            CartType::TigervisionRamPlus => tigervision_ram_plus::holds(len),
            // The refusal an unbuilt board earns is about the board, not the
            // image.
            _ if !self.built() => true,
            _ => self.image_size() == Some(len),
        }
    }

    /// Whether the slot models this board. The catalogue names more boards
    /// than the core builds; an unbuilt one loads as a stated refusal.
    pub fn built(self) -> bool {
        !matches!(
            self,
            CartType::DpcPlus | CartType::GameLine | CartType::Fa2 | CartType::FourA50
        )
    }

    /// The image size the board is wired for, where its wiring fixes one. A
    /// board that sizes itself from the image has none, so a longer image is
    /// not padding there is anywhere to trim back to.
    pub(super) fn image_size(self) -> Option<usize> {
        Some(match self {
            CartType::Plain2K => 0x800,
            CartType::Plain4K => 0x1000,
            CartType::Atari8K
            | CartType::Atari8KSuperchip
            | CartType::ParkerBros
            | CartType::UaLtd
            | CartType::Tigervision
            | CartType::Activision => 0x2000,
            CartType::Atari16K | CartType::Atari16KSuperchip | CartType::MNetwork => 0x4000,
            CartType::Atari32K | CartType::Atari32KSuperchip => 0x8000,
            CartType::Atari64K | CartType::X07 => 0x10000,
            CartType::Econobanking => 0x2000,
            CartType::MenuDrivenMegacart => 0x8000,
            CartType::Atari128K | CartType::Superbanking => 0x20000,
            CartType::Atari256K => 0x40000,
            CartType::CbsRamPlus => 0x3000,
            CartType::Commavid => 0x800,
            // 8 KB program + 2 KB display, and the dumper's trailing RNG stream.
            CartType::Dpc => 0x2900,
            CartType::Megaboy => 0x10000,
            CartType::Jane => 0x4000,
            CartType::ColecoWf8
            | CartType::WicksteadDesign
            | CartType::Fotomania
            | CartType::ParkerBrosBrazil => 0x2000,
            CartType::AmigaPowerPlay => 0x8000,
            // A Supercharger container holds as many tape loads as the title
            // needs, and a 3E or 3E+ image as many banks as the cart carries.
            CartType::Supercharger | CartType::TigervisionRam | CartType::TigervisionRamPlus => {
                return None;
            }
            // An unbuilt board states no wiring to size an image against.
            CartType::DpcPlus | CartType::GameLine | CartType::Fa2 | CartType::FourA50 => {
                return None;
            }
        })
    }
}

/// The RAM ports shadow the bottom 256 bytes of every bank, so a Superchip
/// dump repeats each bank's first 128 bytes of filler into the next 128.
fn has_superchip_signature(rom: &[u8]) -> bool {
    use atari::SUPERCHIP_RAM_SIZE;
    rom.chunks_exact(0x1000)
        .all(|bank| bank[..SUPERCHIP_RAM_SIZE] == bank[SUPERCHIP_RAM_SIZE..2 * SUPERCHIP_RAM_SIZE])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_board_round_trips_its_code() {
        for row in BOARD_NAMES {
            assert_eq!(CartType::from_code(row.code), Some(row.board));
            assert_eq!(row.board.code(), row.code);
        }
    }

    #[test]
    fn every_board_round_trips_through_ron() {
        for board in CartType::all() {
            let text = ron::to_string(&board).expect("a board serialises");
            assert_eq!(text, format!("{:?}", board.code()));
            assert_eq!(ron::from_str::<CartType>(&text), Ok(board));
        }
    }

    #[test]
    fn an_unlisted_code_names_no_board() {
        let error = ron::from_str::<CartType>("\"MBC1\"").expect_err("no such board");
        assert!(error.to_string().contains("\"MBC1\""), "{error}");
    }
}
