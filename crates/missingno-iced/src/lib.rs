//! Shared iced presentation for the missingno apps: the emulator screen with
//! its device-simulation shader, and the wgpu texture pipeline behind it.

pub mod screen;
pub mod texture_renderer;

pub use screen::{Frame, IndexedFrame, PalettePolicy, RgbaFrame, ScreenView, iced_color};
pub use texture_renderer::{ScreenOverlay, TextureRenderer};
