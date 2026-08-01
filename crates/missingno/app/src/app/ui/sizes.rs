pub fn xs() -> f32 {
    m() * 0.25
}

pub fn s() -> f32 {
    m() * 0.5
}

pub fn m() -> f32 {
    16.0
}

pub fn l() -> f32 {
    m() * 1.5
}

pub fn xl() -> f32 {
    m() * 3.0
}

// Border radii — use these instead of magic numbers.

/// Small elements: buttons, inline badges, overlays.
pub fn border_s() -> f32 {
    4.0
}

/// Cards, containers, activity entries.
pub fn border_m() -> f32 {
    8.0
}

/// Large panels, modals, pill-shaped elements.
pub fn border_l() -> f32 {
    24.0
}
