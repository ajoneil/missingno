use std::{fs, path::PathBuf};

use crate::app::library::{self, hasheous};

use missingno_gb::cartridge::Cartridge;

pub fn scan_directories(directories: &[PathBuf]) -> Vec<library::GameEntry> {
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

            let sha1 = hasheous::rom_sha1(&rom);

            // Check if already in library
            if let Some((game_dir, mut existing)) = library::find_by_sha1(&sha1) {
                existing.add_rom_path(path);
                library::save_entry(&game_dir, &existing);
                continue;
            }

            // New game
            let title = Cartridge::peek_title(&rom);
            let title = if title.is_empty() {
                "Unknown".to_string()
            } else {
                title
            };

            let entry = library::GameEntry::new(sha1, title, path.clone());

            let game_dir = match library::game_dir_for(&entry.title, &entry.sha1) {
                Some(dir) => dir,
                None => continue,
            };

            // Import .sav from next to ROM if available
            let legacy_sav = path.with_extension("sav");
            if legacy_sav.exists() {
                let mut manifest = library::saves::load_manifest(&game_dir);
                if manifest.saves.is_empty() {
                    if let Ok(data) = fs::read(&legacy_sav) {
                        let save_entry = manifest.record_legacy_import(data.len() as u32);
                        let id = save_entry.id.clone();
                        library::saves::write_save_data(&game_dir, &id, &data);
                        library::saves::save_manifest(&game_dir, &manifest);
                    }
                }
            }

            library::save_entry(&game_dir, &entry);
            new_entries.push(entry);
        }
    }

    new_entries
}

pub fn enrich_library() {
    let mut request_count = 0u32;

    for (game_dir, mut entry) in library::list_all() {
        // Skip already enriched or previously attempted entries
        if entry.enrichment_attempted {
            continue;
        }

        // Rate limit: ~1 request per second to be a good API citizen
        if request_count > 0 {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        let info = match hasheous::lookup(&entry.sha1) {
            Ok(Some(info)) => {
                request_count += 1;
                info
            }
            Ok(None) => {
                // Not found in Hasheous — mark as attempted so we don't retry
                request_count += 1;
                entry.enrichment_attempted = true;
                library::save_entry(&game_dir, &entry);
                continue;
            }
            Err(_) => {
                // Network error — don't mark as attempted, retry next time
                break; // Stop enriching on network errors
            }
        };

        entry.title = info.name;
        entry.platform = info.platform;
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
    }
}

fn is_rom_file(path: &std::path::Path) -> bool {
    path.is_file()
        && matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("gb" | "gbc")
        )
}
