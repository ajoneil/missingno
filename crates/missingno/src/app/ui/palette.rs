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
