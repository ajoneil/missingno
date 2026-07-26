//! The seam between the system-agnostic app shell and an emulated system.
//!
//! The app (library, emulator shell, session client, debugger UI) drives a console
//! through these object-safe traits; each system family implements them once,
//! in its own submodule, and registers in its factory. Adding a system means
//! adding a submodule — not extending parallel dispatch enums.

use std::path::Path;

pub use missingno_core::ports::{
    ControlDescriptor, PanelControl, PeripheralId, PortDescriptor, PortId,
};
pub use missingno_core::system::{
    ControlId, ControlInput, ControlRole, SystemConsole, SystemDebugger,
};

pub mod gb;
#[cfg(feature = "nes")]
pub mod nes;
#[cfg(feature = "sms")]
pub mod sms;
pub mod vcs;

/// The platforms the frontend knows, one per family descriptor. The
/// canonical platform identity for library metadata: external sources'
/// platform strings are mapped into it, and display always goes through
/// [`Platform::name`]. Variants are never cfg-gated — a library entry
/// written by a fuller build must still parse.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Platform {
    GameBoy,
    GameBoyColor,
    AtariVcs,
    MasterSystem,
    Nes,
}

impl Platform {
    /// Display name; also the file-dialog filter label.
    pub fn name(self) -> &'static str {
        match self {
            Platform::GameBoy => "Game Boy",
            Platform::GameBoyColor => "Game Boy Color",
            Platform::AtariVcs => "Atari VCS",
            Platform::MasterSystem => "Sega Master System",
            Platform::Nes => "Nintendo Entertainment System",
        }
    }

    /// Short name for compact UI (bindings rows, badges).
    pub fn short_name(self) -> &'static str {
        match self {
            Platform::GameBoy => "GB",
            Platform::GameBoyColor => "GBC",
            Platform::AtariVcs => "VCS",
            Platform::MasterSystem => "SMS",
            Platform::Nes => "NES",
        }
    }

    /// Best-effort mapping from an external platform description — a
    /// Hasheous platform name, or the string an older library entry stored.
    pub fn from_description(text: &str) -> Option<Platform> {
        let text = text.to_ascii_lowercase();
        if text.contains("game boy color") {
            Some(Platform::GameBoyColor)
        } else if text.contains("game boy") && !text.contains("advance") {
            Some(Platform::GameBoy)
        } else if text.contains("2600") || text.contains("atari vcs") {
            Some(Platform::AtariVcs)
        } else if text.contains("master system") {
            Some(Platform::MasterSystem)
        } else if text.contains("nintendo entertainment system") || text.contains("famicom") {
            Some(Platform::Nes)
        } else {
            None
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The broadcast standard a game is authored for. Cross-family library
/// metadata: the VCS consumes it (colour decode + frame timing) and future
/// region-split families (NES, Master System) will too. Persisted on a
/// library entry.
pub use missingno_core::TvStandard;

/// Everything the loader hands a family's console factory. The fields are
/// family-agnostic except the two Game Boy peripheral ones, quarantined here
/// under the same rule as the GB types on the seam traits: generalize when a
/// second family grows an equivalent, not before.
pub struct MediaLoad<'a> {
    /// Soft-patched ROM contents.
    pub rom: &'a [u8],
    /// Display-title fallback (the file stem); families whose media carries
    /// a header title ignore it.
    pub fallback_title: String,
    /// Battery-save contents to restore, if the library holds any.
    pub save_data: Option<Vec<u8>>,
    /// Boot ROM supplied on the CLI; the Game Boy family attaches it.
    pub boot_rom: Option<missingno_gb::BootRom>,
    /// Broadcast standard from the library entry; the VCS uses it, else probes.
    pub tv_standard: Option<TvStandard>,
    /// Cartridge board code from the library entry (VCS), e.g. "F8".
    pub cart_type: Option<String>,
    /// The library entry records this dump as an overdump: padded past the
    /// cartridge's silicon, which the VCS loads on the stated board anyway.
    pub overdump: bool,
    /// Link-cable connection, borrowed mutably so only the family that owns
    /// the concept takes it.
    pub serial_link: &'a mut Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
    /// Where a default-attached Game Boy Printer sends finished prints for the
    /// play log; the Game Boy family wires it into the printer it attaches.
    pub print_sink: Option<crate::printer::PrintSink>,
}

/// Build a console from loaded media; `Err` carries what the core objected to.
pub type CreateConsole = fn(MediaLoad) -> Result<Box<dyn SystemConsole>, String>;

/// A morepork capture request, dispatched through the family table by the
/// `trace` subcommand.
pub struct TraceRequest<'a> {
    pub rom: &'a [u8],
    /// For the Game Boy family's `.sav` sidecar lookup.
    pub rom_path: &'a Path,
    pub profile: &'a missingno_gb::trace::Profile,
    pub output: &'a Path,
    pub cycles: u64,
    pub boot_rom: Option<missingno_gb::BootRom>,
}

/// A family's control surfaces: the integrated pad, the peripherals its ports
/// accept, and the console panel. Every table is static, so a binding's role
/// can be placed on a site — and labelled — without a loaded console.
#[derive(Clone, Copy)]
pub struct ControlMap {
    pub integrated: &'static [ControlDescriptor],
    pub ports: &'static [PortDescriptor],
    pub panel: &'static [PanelControl],
}

impl ControlMap {
    pub const fn new(
        integrated: &'static [ControlDescriptor],
        ports: &'static [PortDescriptor],
        panel: &'static [PanelControl],
    ) -> Self {
        ControlMap {
            integrated,
            ports,
            panel,
        }
    }

    /// Where this family plays `role`: the integrated pad first, then the ports
    /// in order, then the panel. `None` when nothing on the console plays it.
    pub fn resolve(&self, role: ControlRole) -> Option<ControlId> {
        if self.integrated.iter().any(|control| control.role == role) {
            return Some(ControlId::integrated(role));
        }
        for port in self.ports {
            let played = port
                .accepts
                .iter()
                .any(|peripheral| peripheral.controls.iter().any(|c| c.role == role));
            if played {
                return Some(ControlId::port(port.port, role));
            }
        }
        self.panel
            .iter()
            .any(|control| control.role == role)
            .then(|| ControlId::panel(role))
    }

    /// The family's name for `role`, from the surface that plays it.
    pub fn label(&self, role: ControlRole) -> Option<&'static str> {
        let on_peripherals = self
            .ports
            .iter()
            .flat_map(|port| port.accepts)
            .flat_map(|peripheral| peripheral.controls);
        self.integrated
            .iter()
            .chain(on_peripherals)
            .find(|control| control.role == role)
            .map(|control| control.label)
            .or_else(|| {
                self.panel
                    .iter()
                    .find(|control| control.role == role)
                    .map(|control| control.label)
            })
    }
}

/// A family's registration on the load path: how its media is recognised in
/// file dialogs, library scans, and ROM loads, and how a console is built.
pub struct FamilyDescriptor {
    pub platform: Platform,
    pub extensions: &'static [&'static str],
    /// The family's control surfaces, for binding resolution and labelling.
    pub controls: ControlMap,
    /// Whether path and contents identify this family's media. Predicates
    /// across the table are mutually exclusive; table order sets the
    /// file-dialog filter order, not claim precedence.
    pub is_rom: fn(&Path, &[u8]) -> bool,
    /// Title carried in the media's header, for families whose media has
    /// one; `None` falls back to the file stem.
    pub title_from_rom: fn(&[u8]) -> Option<String>,
    pub create_console: CreateConsole,
    /// How this family's ports are configured for a game whose library
    /// metadata names these controllers. Empty leaves the console's power-on
    /// configuration.
    pub port_config: fn(&[missingno_gamedb::Controller]) -> Vec<(PortId, PeripheralId)>,
    /// morepork capture entry point for the `trace` subcommand; `None` for
    /// families without a trace backend.
    pub trace: Option<fn(TraceRequest)>,
}

/// The family registered for a platform.
pub fn family_of(platform: Platform) -> Option<&'static FamilyDescriptor> {
    FAMILIES.iter().find(|family| family.platform == platform)
}

/// The single classification point: the family whose media this is. Media
/// no family claims is unsupported.
pub fn family_for(path: &Path, rom: &[u8]) -> Option<&'static FamilyDescriptor> {
    FAMILIES.iter().find(|family| (family.is_rom)(path, rom))
}

/// Every registered family, in file-dialog filter order.
pub static FAMILIES: &[FamilyDescriptor] = &[
    FamilyDescriptor {
        platform: Platform::GameBoy,
        extensions: gb::ROM_EXTENSIONS,
        controls: gb::CONTROLS,
        is_rom: gb::is_gb_rom,
        title_from_rom: gb::title_from_rom,
        create_console: gb::create_console,
        port_config: |_| Vec::new(),
        trace: Some(crate::trace::trace_gb),
    },
    FamilyDescriptor {
        platform: Platform::GameBoyColor,
        extensions: gb::GBC_ROM_EXTENSIONS,
        controls: gb::CONTROLS,
        is_rom: gb::is_gbc_rom,
        title_from_rom: gb::title_from_rom,
        // The same factory serves both platforms: the header picks the core.
        create_console: gb::create_console,
        port_config: |_| Vec::new(),
        trace: Some(crate::trace::trace_gb),
    },
    FamilyDescriptor {
        platform: Platform::AtariVcs,
        extensions: vcs::ROM_EXTENSIONS,
        controls: vcs::CONTROLS,
        is_rom: vcs::is_vcs_rom,
        title_from_rom: |_| None,
        create_console: |media| {
            vcs::create_console(
                media.rom,
                media.fallback_title,
                media.tv_standard,
                media.cart_type.as_deref(),
                media.overdump,
            )
            .map_err(|error| error.to_string())
        },
        port_config: vcs::port_config,
        trace: Some(crate::trace::trace_vcs),
    },
    #[cfg(feature = "sms")]
    FamilyDescriptor {
        platform: Platform::MasterSystem,
        extensions: sms::ROM_EXTENSIONS,
        controls: sms::CONTROLS,
        is_rom: |path, _| sms::is_sms_rom(path),
        title_from_rom: |_| None,
        create_console: |media| {
            sms::create_console(media.rom, media.fallback_title)
                .map_err(|error| format!("{error:?}"))
        },
        port_config: |_| Vec::new(),
        trace: None,
    },
    #[cfg(feature = "nes")]
    FamilyDescriptor {
        platform: Platform::Nes,
        extensions: nes::ROM_EXTENSIONS,
        controls: nes::CONTROLS,
        is_rom: |_, rom| nes::is_nes_rom(rom),
        title_from_rom: |_| None,
        create_console: |media| {
            nes::create_console(media.rom, media.fallback_title)
                .map_err(|error| format!("{error:?}"))
        },
        port_config: |_| Vec::new(),
        trace: Some(crate::trace::trace_nes),
    },
];
