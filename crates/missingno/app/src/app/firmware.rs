//! A firmware socket as a pick list, shared by the launch row that fills one
//! for a single game and the Systems page that sets its default. Both offer the
//! images the folder holds, under the labels the core knows them by.

use missingno_core::firmware::FirmwareSlot;
use missingno_session::FirmwareLibrary;

/// The label of the entry that names no image.
pub const NO_IMAGE: &str = "None";

/// What one entry of a firmware pick list stands for.
#[derive(Clone, PartialEq, Eq)]
pub enum Choice {
    /// Whatever fills the socket when nobody chooses.
    Automatic,
    /// The socket left deliberately empty.
    Empty,
    /// One image the core knows, by id.
    Image(String),
}

/// One entry of a firmware pick list.
#[derive(Clone, PartialEq, Eq)]
pub struct Entry {
    pub choice: Choice,
    pub label: String,
}

impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// What a socket offers: `leading` first, then the images the folder holds, then
/// `chosen` where the folder no longer holds it — a choice with no file behind
/// it still shows, so the refusal at launch has a visible cause. The second
/// element is the entry `chosen` selects, `leading` where nothing does.
pub fn entries(
    slot: &FirmwareSlot,
    library: &FirmwareLibrary,
    chosen: Option<&str>,
    leading: Entry,
) -> (Vec<Entry>, Entry) {
    let mut entries = vec![leading];
    entries.extend(library.present(slot).into_iter().map(|present| Entry {
        choice: Choice::Image(present.image.id.to_string()),
        label: present.image.label.to_string(),
    }));

    let holds = |entries: &[Entry], image: &str| {
        entries
            .iter()
            .any(|entry| entry.choice == Choice::Image(image.to_string()))
    };
    if let Some(chosen) = chosen
        && !holds(&entries, chosen)
    {
        let named = slot
            .image(chosen)
            .map(|image| image.label.to_string())
            .unwrap_or_else(|| chosen.to_string());
        entries.push(Entry {
            choice: Choice::Image(chosen.to_string()),
            label: format!("{named} (not in firmware folder)"),
        });
    }

    let selected = chosen
        .and_then(|chosen| {
            entries
                .iter()
                .find(|entry| entry.choice == Choice::Image(chosen.to_string()))
                .cloned()
        })
        .unwrap_or_else(|| entries[0].clone());
    (entries, selected)
}
