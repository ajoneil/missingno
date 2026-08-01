//! The board a ROM is wired for, and what a bare dump reveals about it.
//!
//! A dump is the silicon's contents and nothing else, so its length is the only
//! signal it carries — enough to name a board across the common Atari sizes.
//! The one exception is the Superchip, whose RAM shadows the bottom of every
//! bank and leaves a readable mark in the image.

use super::{ar, atari, dpc};

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
}

impl std::fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CartridgeError::UnsupportedSize(size) => write!(f, "unsupported image size {size}"),
            CartridgeError::WrongSizeForBoard { cart_type, size } => write!(
                f,
                "image is {size} bytes but a {} board holds {}",
                cart_type.code(),
                cart_type.image_size()
            ),
            CartridgeError::LoadChecksum { unit, page: None } => {
                write!(f, "Supercharger load {unit}: header checksum error")
            }
            CartridgeError::LoadChecksum {
                unit,
                page: Some(page),
            } => write!(f, "Supercharger load {unit}: page {page} checksum error"),
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
/// ranges that page the 4 KB window; `*Sc` variants add Superchip (SARA) cart
/// RAM, which a raw dump can't be told from a plain board by size alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CartType {
    /// 2 KB, mirrored into the window.
    Plain2K,
    /// 4 KB, fills the window.
    Plain4K,
    /// 8 KB across two banks.
    F8,
    F8Sc,
    /// 16 KB across four banks.
    F6,
    F6Sc,
    /// 32 KB across eight banks.
    F4,
    F4Sc,
    /// 12 KB across three banks, with 256 bytes of cart RAM and a
    /// data-bus-gated switch (CBS RAM Plus).
    Fa,
    /// 8 KB as eight 1 KB slices in three pageable windows (Parker Bros).
    E0,
    /// 16 KB as eight 2 KB banks, with 1 KB and 4×256 B cart RAMs (M-Network).
    E7,
    /// 2 KB of ROM over 1 KB of read-low cart RAM (CommaVid).
    Cv,
    /// 8 KB across two banks, selected from loosely decoded hotspots below the
    /// window (UA Ltd).
    Ua,
    /// A 2 KB fixed half over a bus-latched paged half (Tigervision).
    ThreeF,
    /// 8 KB across two banks, picked by an address comparator on $01FE and
    /// data line D5 (Activision).
    Fe,
    /// An F8-banked 8 KB program ROM beside the Display Processor Chip and its
    /// own 2 KB display ROM (Pitfall II).
    Dpc,
    /// 6 KB of tape-loaded RAM and a BIOS, driven entirely by reads (Starpath
    /// Supercharger). The image is a fastload container, not a ROM.
    Ar,
    /// 64 KB across sixteen banks, stepped one at a time (Dynacom Megaboy).
    F0,
    /// 16 KB across four banks on scattered hotspots (Tarzan prototype).
    Jane,
    /// 8 KB across two banks, chosen by the written data (Coleco white label).
    Wf8,
    /// 8 KB as eight 1 KB banks filling four segments at once, on a delayed
    /// switch (Wickstead Design prototype).
    Wd,
    /// 32 KB across eight banks, latched in two halves and committed
    /// separately (Amiga Power Play Arcade).
    Fc,
    /// 8 KB across two banks, selected from loosely decoded low-memory
    /// hotspots (Fotomania).
    ZeroFa0,
    /// 8 KB as eight 1 KB slices in three segments, selected active-low from
    /// low memory (Brazilian Parker Bros).
    Zero3E0,
    /// 3F with a cart-RAM path on its own hotspot (homebrew).
    ThreeE,
    /// Four independently banked 1 KB segments, each ROM or RAM (homebrew).
    ThreeEPlus,
    /// 64 KB across sixteen banks, on the hotspot family's wider run (homebrew).
    Ef,
    /// 128 KB across thirty-two banks (homebrew).
    Df,
    /// 256 KB across sixty-four banks (homebrew).
    Bf,
    /// 128 KB across thirty-two banks selected from low memory, waking in the
    /// last of them (SuperBanking).
    Sb,
    /// 8 KB across two banks, selected by one address line (EconoBanking).
    Zero840,
    /// 64 KB across sixteen banks, with a second switch riding on TIA writes
    /// (Stocking).
    X07,
    /// 32 KB across eight banks named by an address's low byte, with a one-way
    /// lock (Menu Driven Megacart).
    Mdm,
}

impl CartType {
    /// The board a bare dump is best-effort read as, from its length alone.
    pub(super) fn infer(rom: &[u8]) -> Result<CartType, CartridgeError> {
        Ok(match rom.len() {
            0x800 => CartType::Plain2K,
            0x1000 => CartType::Plain4K,
            0x2000 if has_superchip_signature(rom) => CartType::F8Sc,
            0x2000 => CartType::F8,
            0x4000 if has_superchip_signature(rom) => CartType::F6Sc,
            0x4000 => CartType::F6,
            0x8000 if has_superchip_signature(rom) => CartType::F4Sc,
            0x8000 => CartType::F4,
            // A Supercharger container is whole 8448-byte load units — a size
            // no ROM board shares.
            size if ar::is_container(size) => CartType::Ar,
            size => return Err(CartridgeError::UnsupportedSize(size)),
        })
    }

    /// The board a game-db board code names.
    pub fn from_code(code: &str) -> Option<CartType> {
        Some(match code {
            "2K" => CartType::Plain2K,
            "4K" => CartType::Plain4K,
            "F8" => CartType::F8,
            "F8SC" => CartType::F8Sc,
            "F6" => CartType::F6,
            "F6SC" => CartType::F6Sc,
            "F4" => CartType::F4,
            "F4SC" => CartType::F4Sc,
            "FA" => CartType::Fa,
            "FC" => CartType::Fc,
            "FE" => CartType::Fe,
            "E0" => CartType::E0,
            "E7" => CartType::E7,
            "CV" => CartType::Cv,
            "UA" => CartType::Ua,
            "3F" => CartType::ThreeF,
            "3E" => CartType::ThreeE,
            "3E+" => CartType::ThreeEPlus,
            "DPC" => CartType::Dpc,
            "AR" => CartType::Ar,
            "F0" => CartType::F0,
            "JANE" => CartType::Jane,
            "WF8" => CartType::Wf8,
            "WD" => CartType::Wd,
            "0FA0" => CartType::ZeroFa0,
            "03E0" => CartType::Zero3E0,
            "0840" => CartType::Zero840,
            "EF" => CartType::Ef,
            "DF" => CartType::Df,
            "BF" => CartType::Bf,
            "SB" => CartType::Sb,
            "X07" => CartType::X07,
            "MDM" => CartType::Mdm,
            _ => return None,
        })
    }

    /// The game-db board code for this board — the inverse of [`from_code`].
    ///
    /// [`from_code`]: CartType::from_code
    pub fn code(self) -> &'static str {
        match self {
            CartType::Plain2K => "2K",
            CartType::Plain4K => "4K",
            CartType::F8 => "F8",
            CartType::F8Sc => "F8SC",
            CartType::F6 => "F6",
            CartType::F6Sc => "F6SC",
            CartType::F4 => "F4",
            CartType::F4Sc => "F4SC",
            CartType::Fa => "FA",
            CartType::Fc => "FC",
            CartType::Fe => "FE",
            CartType::E0 => "E0",
            CartType::E7 => "E7",
            CartType::Cv => "CV",
            CartType::Ua => "UA",
            CartType::ThreeF => "3F",
            CartType::ThreeE => "3E",
            CartType::ThreeEPlus => "3E+",
            CartType::Dpc => "DPC",
            CartType::Ar => "AR",
            CartType::F0 => "F0",
            CartType::Jane => "JANE",
            CartType::Wf8 => "WF8",
            CartType::Wd => "WD",
            CartType::ZeroFa0 => "0FA0",
            CartType::Zero3E0 => "03E0",
            CartType::Zero840 => "0840",
            CartType::Ef => "EF",
            CartType::Df => "DF",
            CartType::Bf => "BF",
            CartType::Sb => "SB",
            CartType::X07 => "X07",
            CartType::Mdm => "MDM",
        }
    }

    /// Whether an image is sized for the board. Most boards are exact — a dump
    /// is the silicon's contents and nothing else.
    pub(super) fn accepts(self, len: usize) -> bool {
        match self {
            // A DPC dump ends with however much of the RNG stream the dumper
            // happened to catch, so the tail's length varies; the common dump
            // is one byte short of a full page of it.
            CartType::Dpc => (dpc::IMAGE_SIZE..=self.image_size()).contains(&len),
            // A Supercharger image is a tape container: one load unit per tape
            // load, and a multi-load title carries several.
            CartType::Ar => ar::is_container(len),
            _ => len == self.image_size(),
        }
    }

    /// The image size the board is wired for.
    pub(super) fn image_size(self) -> usize {
        match self {
            CartType::Plain2K => 0x800,
            CartType::Plain4K => 0x1000,
            CartType::F8
            | CartType::F8Sc
            | CartType::E0
            | CartType::Ua
            | CartType::ThreeF
            | CartType::Fe => 0x2000,
            CartType::F6 | CartType::F6Sc | CartType::E7 => 0x4000,
            CartType::F4 | CartType::F4Sc => 0x8000,
            CartType::Ef | CartType::X07 => 0x10000,
            CartType::Zero840 => 0x2000,
            CartType::Mdm => 0x8000,
            CartType::Df | CartType::Sb => 0x20000,
            CartType::Bf => 0x40000,
            CartType::Fa => 0x3000,
            CartType::Cv => 0x800,
            // 8 KB program + 2 KB display, and the dumper's trailing RNG stream.
            CartType::Dpc => 0x2900,
            CartType::Ar => ar::IMAGE_SIZE,
            CartType::F0 => 0x10000,
            CartType::Jane => 0x4000,
            CartType::Wf8
            | CartType::Wd
            | CartType::ZeroFa0
            | CartType::Zero3E0
            | CartType::ThreeE
            | CartType::ThreeEPlus => 0x2000,
            CartType::Fc => 0x8000,
        }
    }
}

/// The RAM ports shadow the bottom 256 bytes of every bank, so a Superchip
/// dump repeats each bank's first 128 bytes of filler into the next 128.
fn has_superchip_signature(rom: &[u8]) -> bool {
    use atari::SUPERCHIP_RAM_SIZE;
    rom.chunks_exact(0x1000)
        .all(|bank| bank[..SUPERCHIP_RAM_SIZE] == bank[SUPERCHIP_RAM_SIZE..2 * SUPERCHIP_RAM_SIZE])
}
