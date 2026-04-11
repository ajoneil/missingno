use iced::Color;

// Catppuccin Mocha palette — canonical color definitions for the app.

/// Primary accent (Catppuccin Mocha "mauve") — #CBA6F7
pub const PURPLE: Color = Color::from_rgb(
    0xcb as f32 / 255.0,
    0xa6 as f32 / 255.0,
    0xf7 as f32 / 255.0,
);

/// Primary text (Catppuccin Mocha "text") — #CDD6F4
pub const TEXT: Color = Color::from_rgb(
    0xcd as f32 / 255.0,
    0xd6 as f32 / 255.0,
    0xf4 as f32 / 255.0,
);

/// Secondary text (Catppuccin Mocha "subtext0") — #A6ADC8
pub const MUTED: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xad as f32 / 255.0,
    0xc8 as f32 / 255.0,
);

/// Danger accent (Catppuccin Mocha "red") — #F38BA8
pub const RED: Color = Color::from_rgb(
    0xf3 as f32 / 255.0,
    0x8b as f32 / 255.0,
    0xa8 as f32 / 255.0,
);

/// Hardware accent (Catppuccin Mocha "teal") — #94E2D5
pub const TEAL: Color = Color::from_rgb(
    0x94 as f32 / 255.0,
    0xe2 as f32 / 255.0,
    0xd5 as f32 / 255.0,
);

/// Dim label (Catppuccin Mocha "overlay0") — #6C7086
pub const OVERLAY0: Color = Color::from_rgb(
    0x6c as f32 / 255.0,
    0x70 as f32 / 255.0,
    0x86 as f32 / 255.0,
);

/// Inactive/off state (Catppuccin Mocha "surface2") — #585B70
pub const SURFACE2: Color = Color::from_rgb(
    0x58 as f32 / 255.0,
    0x5b as f32 / 255.0,
    0x70 as f32 / 255.0,
);

/// Positive/on state (Catppuccin Mocha "green") — #A6E3A1
pub const GREEN: Color = Color::from_rgb(
    0xa6 as f32 / 255.0,
    0xe3 as f32 / 255.0,
    0xa1 as f32 / 255.0,
);

/// Changed/warning (Catppuccin Mocha "yellow") — #F9E2AF
pub const YELLOW: Color = Color::from_rgb(
    0xf9 as f32 / 255.0,
    0xe2 as f32 / 255.0,
    0xaf as f32 / 255.0,
);
