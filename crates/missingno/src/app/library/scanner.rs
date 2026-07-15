use std::{fs, path::PathBuf};

use crate::app::library::{self, catalogue::Catalogue, hasheous};
use crate::app::system;

pub fn scan_directories(directories: &[PathBuf], catalogue: &Catalogue) -> Vec<library::GameEntry> {
    let mut new_entries = Vec::new();

    for dir in directories {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for dir_entry in entries.flatten() {
            let path = dir_entry.path();
            if !is_rom_file(&path) {
                continue;
            }

            let rom = match fs::read(&path) {
                Ok(rom) => rom,
                Err(_) => continue,
            };

            // Skip media no family claims — an unlaunchable library entry
            // helps nobody.
            let Some(family) = system::family_for(&path, &rom) else {
                continue;
            };

            let sha1 = hasheous::rom_sha1(&rom);

            // Check if already in library; older entries may predate
            // platform classification, so stamp it while the ROM is at hand.
            if let Some((game_dir, mut existing)) = library::find_by_sha1(&sha1) {
                existing.platform.get_or_insert(family.platform);
                // What cartridge this is comes from the catalogue and nowhere
                // else, so re-read it rather than trusting what an older scan
                // recorded — a correction upstream reaches the library only if
                // a rescan can revise these.
                if let Some(cat_entry) = catalogue.lookup_hash(&sha1) {
                    existing.tv_standard = cat_entry.manifest.tv_format;
                    existing.cart_type = cat_entry.manifest.cart_type.clone();
                }
                existing.add_rom_path(path);
                library::save_entry(&game_dir, &existing);
                continue;
            }

            let header_title = (family.title_from_rom)(&rom);

            // Try catalogue first for a good title, fall back to the header
            // title or the file stem.
            let mut entry = if let Some(cat_entry) = catalogue.lookup_hash(&sha1) {
                let mut e =
                    library::GameEntry::new(sha1, cat_entry.manifest.title.clone(), path.clone());
                e.platform = Some(family.platform);
                e.publisher = cat_entry
                    .manifest
                    .publisher
                    .clone()
                    .or(cat_entry.manifest.developer.clone());
                e.year = cat_entry.manifest.date.clone();
                e.description = cat_entry.manifest.description.clone();
                e.tv_standard = cat_entry.manifest.tv_format;
                e.cart_type = cat_entry.manifest.cart_type.clone();
                e.enrichment_attempted = false; // still want Hasheous for covers
                e
            } else {
                let title = header_title
                    .clone()
                    .unwrap_or_else(|| crate::app::file_stem_title(&path));
                let mut e = library::GameEntry::new(sha1, title, path.clone());
                e.platform = Some(family.platform);
                e
            };
            entry.header_title = header_title;

            let game_dir = match library::game_dir_for(&entry.title, &entry.sha1) {
                Some(dir) => dir,
                None => continue,
            };

            // Import .sav from next to ROM if available
            let legacy_sav = path.with_extension("sav");
            if legacy_sav.exists() {
                library::activity::import_legacy_sav(&game_dir, &legacy_sav);
            }

            library::save_entry(&game_dir, &entry);
            new_entries.push(entry);
        }
    }

    new_entries
}

/// Result of enriching a single game.
#[derive(Debug, Clone)]
pub struct EnrichResult {
    /// SHA1 of the game that was enriched, if any.
    pub sha1: Option<String>,
    /// Whether there may be more games to enrich.
    pub has_more: bool,
    /// Whether visible data changed (title, cover, metadata).
    pub data_changed: bool,
}

/// Enrich the next unenriched game in the library.
pub fn enrich_next() -> EnrichResult {
    // Rate limit: sleep 1s before each request
    std::thread::sleep(std::time::Duration::from_secs(1));

    let Some((game_dir, mut entry)) = library::list_all()
        .into_iter()
        .find(|(_, e)| !e.enrichment_attempted)
    else {
        return EnrichResult {
            sha1: None,
            has_more: false,
            data_changed: false,
        };
    };

    let sha1 = entry.sha1.clone();

    let info = match hasheous::lookup(&entry.sha1) {
        Ok(Some(info)) => info,
        Ok(None) => {
            entry.enrichment_attempted = true;
            library::save_entry(&game_dir, &entry);
            return EnrichResult {
                sha1: Some(sha1),
                has_more: true,
                data_changed: false,
            };
        }
        Err(_) => {
            return EnrichResult {
                sha1: None,
                has_more: false,
                data_changed: false,
            };
        }
    };

    entry.title = info.name;
    // Hasheous's platform string never overrides the header-derived
    // classification; it only fills a gap, mapped to our own type.
    if entry.platform.is_none() {
        entry.platform = info
            .platform
            .as_deref()
            .and_then(system::Platform::from_description);
    }
    entry.publisher = info.publisher;
    entry.year = info.year;
    entry.description = info.description;
    entry.wikipedia_url = info.wikipedia_url;
    entry.igdb_url = info.igdb_url;
    entry.enrichment_attempted = true;
    library::save_entry(&game_dir, &entry);

    if let Some(bytes) = &info.cover_art {
        library::save_cover(&game_dir, bytes);
    }

    EnrichResult {
        sha1: Some(sha1),
        has_more: true,
        data_changed: true,
    }
}

fn is_rom_file(path: &std::path::Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                system::FAMILIES
                    .iter()
                    .any(|family| family.extensions.contains(&ext))
            })
}
