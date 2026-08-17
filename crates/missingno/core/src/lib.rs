//! Shared console-core foundation: board/TV vocabulary, analog filters, and
//! debug sidecar formats.
//!
//! A core models its chips up to their pads. What surrounds them — the board's
//! coupling network, the television, the speaker — is not the console, and is
//! shared across consoles that were never related: NTSC is NTSC whether the
//! signal came from a TIA or a VDP, and a resistor-capacitor coupling is the
//! same circuit on every board that has one. Cores depend on this crate to
//! *state* what their hardware is; applying any of it belongs to whatever
//! assembles a console into a working machine.
//!
//! Symbol tables and code/data logs are conventions of the tools around an
//! emulator rather than of any one machine: the `.sym` grammar and the Mesen
//! CDL flag bits mean the same thing whichever CPU produced them. What differs
//! per console — how a CPU address maps onto a ROM offset, what state a
//! watchpoint can name — stays with that console.

pub mod analog;
pub mod cartridge;
pub mod cdl;
pub mod chip;
pub mod disasm;
pub mod graphics;
pub mod inspect;
pub mod isa;
pub mod launch;
pub mod machine;
pub mod ports;
pub mod recording;
pub mod state;
pub mod state_file;
pub mod symbols;
pub mod system;
#[cfg(feature = "morepork")]
pub mod trace;
pub mod tv;
pub mod video;
pub mod waveform;

pub use analog::{HighPass, OnePoleHighPass, RcHighPass};
pub use chip::ClockRatio;
pub use disasm::{ReadMemory, Row};
pub use inspect::{
    FlagName, MemoryRegion, Register, RegisterGroup, ValueStyle, Watch, WatchParam, WatchTerm,
    Watchable,
};
pub use isa::{Flow, Instruction, InstructionSet};
pub use launch::{
    LaunchChoice, LaunchOptionDescriptor, LaunchOptionKind, LaunchValue, LaunchValues,
};
pub use machine::{
    BoundaryState, CoreRun, CoreStop, Machine, MachineConsole, StateIdentity, StopSet,
};
pub use ports::{
    ControlDescriptor, ControlKind, PanelBehaviour, PanelControl, PeripheralDescriptor,
    PeripheralId, PlugError, PortDescriptor, PortId, Provider,
};
pub use state::{
    FieldDef, FieldType, FrameSpec, PixelFormat, Provenance, StateRecord, StateValue,
    SystemStateSchema, Tier,
};
pub use state_file::{StateFile, StateFrame, StateMeta, read_state_file, write_state_file};
pub use tv::TvStandard;
pub use video::{ConsoleFrame, DisplayTechnology, Frame, IndexedFrame, LcdPanel, RgbaFrame};
pub use waveform::{ChannelWave, WaveRing};
